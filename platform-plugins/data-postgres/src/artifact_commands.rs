//! Artifact metadata and upload state machine. Object bytes become reachable only
//! after length/hash/MIME/scanner verification and a current-authority commit.

use std::{collections::BTreeMap, io::SeekFrom, time::Duration};

use chrono::{DateTime, Utc};
use masonwing_contracts::{
    Digest,
    wire::{Classification, ResourceRef},
};
use masonwing_kernel::runtime::{
    AuthorizedCommand, CommandActor, CommandFailure, FailureKind, ScanVerdict, StoredArtifact,
};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use sqlx::Row;
use tokio::{
    fs::File,
    io::{AsyncRead, AsyncReadExt, AsyncSeekExt, AsyncWriteExt},
};
use uuid::Uuid;

use crate::store::{
    Mutation, PostgresStore, ProjectionChange, Tx, database_failure, field, parse_uuid,
    resource_ref,
};

const MAX_ARTIFACT_BYTES: u64 = 1024 * 1024 * 1024;
const UPLOAD_TTL_SECONDS: i64 = 300;
const UPLOAD_DEADLINE: Duration = Duration::from_secs(30);

pub struct VerifiedArtifactDownload {
    pub metadata: StoredArtifact,
    pub file: File,
    pub permit: tokio::sync::OwnedSemaphorePermit,
}

impl PostgresStore {
    pub(crate) async fn begin_artifact(
        &self,
        tx: &mut Tx<'_>,
        command: &AuthorizedCommand,
    ) -> Result<Mutation, CommandFailure> {
        let tenant = parse_uuid(command.actor.tenant_id.as_str())?;
        let principal = parse_uuid(command.actor.principal_id.as_str())?;
        let classification: Classification = field(&command.input, "classification")?;
        let content_type: String = field(&command.input, "content_type")?;
        let size: i64 = field(&command.input, "size_bytes")?;
        let digest: Digest = field(&command.input, "expected_digest")?;
        validate_declared_mime(&content_type)?;
        let open_count:i64=sqlx::query_scalar("SELECT count(*) FROM uploads WHERE tenant_id=$1 AND principal_id=$2 AND state='OPEN' AND expires_at>clock_timestamp()")
            .bind(tenant).bind(principal).fetch_one(&mut **tx).await.map_err(database_failure)?;
        if open_count >= 20 {
            return Err(CommandFailure::new(
                "UPLOAD_QUEUE_FULL",
                FailureKind::Exhausted,
            ));
        }
        let artifact = Uuid::new_v4();
        let upload = Uuid::new_v4();
        let expires: DateTime<Utc> =
            sqlx::query_scalar("SELECT clock_timestamp()+make_interval(secs=>$1)")
                .bind(UPLOAD_TTL_SECONDS as f64)
                .fetch_one(&mut **tx)
                .await
                .map_err(database_failure)?;
        sqlx::query("INSERT INTO artifacts(tenant_id,id,digest,classification,content_type,size_bytes,storage_key,state,creator_id) VALUES($1,$2,$3,$4,$5,$6,$7,'PENDING',$8)")
            .bind(tenant).bind(artifact).bind(digest.as_str()).bind(classification.as_str()).bind(&content_type).bind(size)
            .bind(format!("pending/{tenant}/{artifact}")).bind(principal).execute(&mut **tx).await.map_err(database_failure)?;
        sqlx::query("INSERT INTO uploads(tenant_id,id,artifact_id,principal_id,expires_at,state) VALUES($1,$2,$3,$4,$5,'OPEN')")
            .bind(tenant).bind(upload).bind(artifact).bind(principal).bind(expires).execute(&mut **tx).await.map_err(database_failure)?;
        let value = upload_json(UploadProjection {
            upload,
            tenant,
            artifact,
            size,
            digest: digest.as_str(),
            content_type: &content_type,
            expires,
            state: "OPEN",
        });
        Ok(
            Mutation::one("UploadSession", upload, 1, value, "Pending file upload")
                .with_source_artifact(artifact),
        )
    }

    /// A PUT is not a command receipt: it durably stages verified bytes and
    /// returns no final ArtifactRef. A different upload attempt cannot overwrite
    /// a completed attempt or adopt a stale lease's result.
    pub async fn put_upload(
        &self,
        actor: &CommandActor,
        upload_id: &str,
        reader: &mut (dyn AsyncRead + Send + Unpin),
        content_length: Option<u64>,
    ) -> Result<(), CommandFailure> {
        let _permit = self
            .upload_permits
            .try_acquire()
            .map_err(|_| CommandFailure::new("UPLOAD_CAPACITY_FULL", FailureKind::Exhausted))?;
        let upload = parse_uuid(upload_id)?;
        let tenant = parse_uuid(actor.tenant_id.as_str())?;
        let principal = parse_uuid(actor.principal_id.as_str())?;
        let lease = Uuid::new_v4();
        let mut tx = self.begin(actor, true).await?;
        let row=sqlx::query("SELECT u.*,a.size_bytes,a.digest,a.content_type,a.state AS artifact_state,clock_timestamp() AS checked_at FROM uploads u JOIN artifacts a ON a.tenant_id=u.tenant_id AND a.id=u.artifact_id WHERE u.tenant_id=$1 AND u.id=$2 AND u.principal_id=$3 FOR UPDATE OF u,a")
            .bind(tenant).bind(upload).bind(principal).fetch_optional(&mut *tx).await.map_err(database_failure)?.ok_or_else(CommandFailure::not_found)?;
        let now: DateTime<Utc> = row.try_get("checked_at").map_err(database_failure)?;
        if row
            .try_get::<DateTime<Utc>, _>("expires_at")
            .map_err(database_failure)?
            <= now
        {
            return Err(CommandFailure::new("UPLOAD_EXPIRED", FailureKind::Expired));
        }
        if row
            .try_get::<String, _>("state")
            .map_err(database_failure)?
            != "OPEN"
            || row
                .try_get::<String, _>("artifact_state")
                .map_err(database_failure)?
                != "PENDING"
        {
            return Err(CommandFailure::conflict("IMMUTABLE_UPLOAD"));
        }
        if row
            .try_get::<Option<DateTime<Utc>>, _>("lease_expires_at")
            .map_err(database_failure)?
            .is_some_and(|end| end > now)
        {
            return Err(CommandFailure::conflict("UPLOAD_IN_PROGRESS"));
        }
        let size = row
            .try_get::<i64, _>("size_bytes")
            .map_err(database_failure)? as u64;
        if !(1..=MAX_ARTIFACT_BYTES).contains(&size)
            || content_length.is_some_and(|value| value != size)
        {
            return Err(CommandFailure::invalid("ARTIFACT_SIZE_MISMATCH"));
        }
        let declared_digest: String = row.try_get("digest").map_err(database_failure)?;
        let content_type: String = row.try_get("content_type").map_err(database_failure)?;
        let artifact: Uuid = row.try_get("artifact_id").map_err(database_failure)?;
        sqlx::query("UPDATE uploads SET lease_id=$3,lease_expires_at=clock_timestamp()+interval '40 seconds' WHERE tenant_id=$1 AND id=$2")
            .bind(tenant).bind(upload).bind(lease).execute(&mut *tx).await.map_err(database_failure)?;
        tx.commit().await.map_err(database_failure)?;

        let outcome = tokio::time::timeout(UPLOAD_DEADLINE, async {
            let (mut file, actual_digest) = self.spool_exact(reader, size).await?;
            if actual_digest.as_str() != declared_digest {
                return Err(CommandFailure::precondition("ARTIFACT_DIGEST_MISMATCH"));
            }
            self.validate_file_mime(&mut file, &content_type, size)
                .await?;
            file.rewind().await.map_err(spool_failure)?;
            match self.scanner.scan_reader(&mut file, size).await? {
                ScanVerdict::Clean => {}
                ScanVerdict::Quarantined { .. } => {
                    return Err(CommandFailure::precondition("ARTIFACT_QUARANTINED"));
                }
            }
            file.rewind().await.map_err(spool_failure)?;
            let key = format!(
                "uploads/{tenant}/{artifact}/{lease}/{}",
                actual_digest.as_str().trim_start_matches("sha256:")
            );
            self.objects
                .put_reader(&key, &mut file, size, &content_type)
                .await?;
            Ok::<_, CommandFailure>((key, actual_digest))
        })
        .await
        .unwrap_or_else(|_| Err(CommandFailure::unavailable("UPLOAD_TIMEOUT")));

        let mut tx = self.begin(actor, true).await?;
        let current=sqlx::query("SELECT u.*,a.digest,a.content_type,a.size_bytes,clock_timestamp() AS checked_at FROM uploads u JOIN artifacts a ON a.tenant_id=u.tenant_id AND a.id=u.artifact_id WHERE u.tenant_id=$1 AND u.id=$2 AND u.principal_id=$3 FOR UPDATE OF u,a")
            .bind(tenant).bind(upload).bind(principal).fetch_optional(&mut *tx).await.map_err(database_failure)?.ok_or_else(CommandFailure::not_found)?;
        if current
            .try_get::<Option<Uuid>, _>("lease_id")
            .map_err(database_failure)?
            != Some(lease)
            || current
                .try_get::<String, _>("state")
                .map_err(database_failure)?
                != "OPEN"
            || current
                .try_get::<Option<DateTime<Utc>>, _>("lease_expires_at")
                .map_err(database_failure)?
                .is_none_or(|until| until <= current.get::<DateTime<Utc>, _>("checked_at"))
        {
            return Err(CommandFailure::conflict("UPLOAD_LEASE_STALE"));
        }
        let (state, scan_code, failure) = match outcome {
            Ok((key, digest)) => {
                if current
                    .try_get::<DateTime<Utc>, _>("expires_at")
                    .map_err(database_failure)?
                    <= current.get::<DateTime<Utc>, _>("checked_at")
                {
                    (
                        "EXPIRED",
                        Some("UPLOAD_EXPIRED"),
                        Some(CommandFailure::new("UPLOAD_EXPIRED", FailureKind::Expired)),
                    )
                } else {
                    sqlx::query("UPDATE artifacts SET storage_key=$3 WHERE tenant_id=$1 AND id=$2 AND state='PENDING'")
                        .bind(tenant).bind(artifact).bind(key).execute(&mut *tx).await.map_err(database_failure)?;
                    sqlx::query("UPDATE uploads SET observed_digest=$3,observed_size_bytes=$4,scanned_at=clock_timestamp() WHERE tenant_id=$1 AND id=$2")
                        .bind(tenant).bind(upload).bind(digest.as_str()).bind(size as i64).execute(&mut *tx).await.map_err(database_failure)?;
                    ("UPLOADED", Some("CLEAN"), None)
                }
            }
            Err(error) => {
                let quarantine = matches!(
                    error.code,
                    "ARTIFACT_DIGEST_MISMATCH"
                        | "ARTIFACT_QUARANTINED"
                        | "ARTIFACT_MIME_MISMATCH"
                        | "ARTIFACT_POLYGLOT"
                        | "ARTIFACT_JSON_INVALID"
                );
                if quarantine {
                    sqlx::query("UPDATE artifacts SET state='QUARANTINED',version=version+1 WHERE tenant_id=$1 AND id=$2")
                        .bind(tenant).bind(artifact).execute(&mut *tx).await.map_err(database_failure)?;
                }
                (
                    if quarantine { "FAILED" } else { "OPEN" },
                    Some(error.code),
                    Some(error),
                )
            }
        };
        let version = current
            .try_get::<i32, _>("version")
            .map_err(database_failure)?
            .checked_add(1)
            .ok_or_else(|| CommandFailure::precondition("VERSION_EXHAUSTED"))?;
        sqlx::query("UPDATE uploads SET state=$3,scan_code=$4,lease_id=NULL,lease_expires_at=NULL,version=$5 WHERE tenant_id=$1 AND id=$2")
            .bind(tenant).bind(upload).bind(state).bind(scan_code).bind(version).execute(&mut *tx).await.map_err(database_failure)?;
        let value = upload_json(UploadProjection {
            upload,
            tenant,
            artifact,
            size: size as i64,
            digest: &declared_digest,
            content_type: &content_type,
            expires: current.try_get("expires_at").map_err(database_failure)?,
            state,
        });
        let change = ProjectionChange {
            resource: resource_ref("UploadSession", upload, version),
            value,
            label: format!("File upload {state}"),
            source_artifacts: vec![artifact],
        };
        self.record_change(&mut tx, actor, "artifact.upload", &change, Uuid::new_v4())
            .await?;
        tx.commit().await.map_err(database_failure)?;
        if let Some(error) = failure {
            return Err(error);
        }
        Ok(())
    }

    pub(crate) async fn finalize_artifact(
        &self,
        tx: &mut Tx<'_>,
        command: &AuthorizedCommand,
    ) -> Result<Mutation, CommandFailure> {
        let tenant = parse_uuid(command.actor.tenant_id.as_str())?;
        let artifact = parse_uuid(&field::<String>(&command.input, "artifact_id")?)?;
        let observed: Digest = field(&command.input, "observed_digest")?;
        let row=sqlx::query("SELECT u.*,a.digest,a.content_type,a.size_bytes,a.classification,a.version AS artifact_version,a.state AS artifact_state FROM uploads u JOIN artifacts a ON a.tenant_id=u.tenant_id AND a.id=u.artifact_id WHERE u.tenant_id=$1 AND u.artifact_id=$2 AND u.principal_id=$3 FOR UPDATE OF u,a")
            .bind(tenant).bind(artifact).bind(parse_uuid(command.actor.principal_id.as_str())?)
            .fetch_optional(&mut **tx).await.map_err(database_failure)?.ok_or_else(CommandFailure::not_found)?;
        if row
            .try_get::<String, _>("state")
            .map_err(database_failure)?
            != "UPLOADED"
            || row
                .try_get::<String, _>("artifact_state")
                .map_err(database_failure)?
                != "PENDING"
            || row
                .try_get::<Option<String>, _>("scan_code")
                .map_err(database_failure)?
                .as_deref()
                != Some("CLEAN")
        {
            return Err(CommandFailure::precondition("ARTIFACT_NOT_VERIFIED"));
        }
        let expected: String = row.try_get("digest").map_err(database_failure)?;
        if expected != observed.as_str()
            || row
                .try_get::<Option<String>, _>("observed_digest")
                .map_err(database_failure)?
                .as_deref()
                != Some(expected.as_str())
            || row
                .try_get::<Option<i64>, _>("observed_size_bytes")
                .map_err(database_failure)?
                != Some(row.try_get("size_bytes").map_err(database_failure)?)
        {
            return Err(CommandFailure::precondition("ARTIFACT_DIGEST_MISMATCH"));
        }
        let upload: Uuid = row.try_get("id").map_err(database_failure)?;
        let version = row
            .try_get::<i32, _>("version")
            .map_err(database_failure)?
            .checked_add(1)
            .ok_or_else(|| CommandFailure::precondition("VERSION_EXHAUSTED"))?;
        sqlx::query(
            "UPDATE artifacts SET state='ACTIVE',version=version+1 WHERE tenant_id=$1 AND id=$2",
        )
        .bind(tenant)
        .bind(artifact)
        .execute(&mut **tx)
        .await
        .map_err(database_failure)?;
        sqlx::query("UPDATE uploads SET state='FINALIZED',version=$3 WHERE tenant_id=$1 AND id=$2")
            .bind(tenant)
            .bind(upload)
            .bind(version)
            .execute(&mut **tx)
            .await
            .map_err(database_failure)?;
        // A manual upload does not assert permission to redistribute or disclose
        // to an AI provider. Those rights require a separately evidenced grant.
        sqlx::query("INSERT INTO artifact_rights(tenant_id,artifact_id,rights_id,access_mode,allow_acquire,allow_ai_analysis,allow_transform,allow_public_redistribution,state) VALUES($1,$2,$3,'MANUAL_IMPORT',true,false,true,false,'ACTIVE')")
            .bind(tenant).bind(artifact).bind(Uuid::new_v4()).execute(&mut **tx).await.map_err(database_failure)?;
        let value = upload_json(UploadProjection {
            upload,
            tenant,
            artifact,
            size: row.try_get("size_bytes").map_err(database_failure)?,
            digest: &expected,
            content_type: &row
                .try_get::<String, _>("content_type")
                .map_err(database_failure)?,
            expires: row.try_get("expires_at").map_err(database_failure)?,
            state: "FINALIZED",
        });
        Ok(Mutation::one(
            "UploadSession",
            upload,
            version,
            value,
            "Verified file upload",
        )
        .with_source_artifact(artifact))
    }

    pub(crate) async fn request_deletion(
        &self,
        tx: &mut Tx<'_>,
        command: &AuthorizedCommand,
    ) -> Result<Mutation, CommandFailure> {
        let target: ResourceRef = field(&command.input, "resource")?;
        let reason: String = field(&command.input, "reason")?;
        let confirmation: String = field(&command.input, "confirmation")?;
        if reason.trim().is_empty() || confirmation != format!("DELETE {}", target.resource_id) {
            return Err(CommandFailure::precondition(
                "DELETION_CONFIRMATION_REQUIRED",
            ));
        }
        let tenant = parse_uuid(command.actor.tenant_id.as_str())?;
        if target.resource_type != "Artifact" {
            return Err(CommandFailure::precondition(
                "DELETION_RESOURCE_UNSUPPORTED",
            ));
        }
        let id = parse_uuid(target.resource_id.as_str())?;
        let row=sqlx::query("SELECT version,legal_hold,state FROM artifacts WHERE tenant_id=$1 AND id=$2 FOR UPDATE")
            .bind(tenant).bind(id).fetch_optional(&mut **tx).await.map_err(database_failure)?.ok_or_else(CommandFailure::not_found)?;
        if row.try_get::<i32, _>("version").map_err(database_failure)? != target.version {
            return Err(CommandFailure::conflict("STALE_VERSION"));
        }
        if row
            .try_get::<bool, _>("legal_hold")
            .map_err(database_failure)?
        {
            return Err(CommandFailure::precondition("LEGAL_HOLD_ACTIVE"));
        }
        let artifacts:Vec<Uuid>=sqlx::query_scalar("WITH RECURSIVE affected(id) AS (SELECT $2::uuid UNION SELECT l.derived_id FROM artifact_lineage l JOIN affected a ON a.id=l.source_id WHERE l.tenant_id=$1) SELECT id FROM affected ORDER BY id")
            .bind(tenant).bind(id).fetch_all(&mut **tx).await.map_err(database_failure)?;
        let any_hold:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM artifacts WHERE tenant_id=$1 AND id=ANY($2) AND legal_hold)")
            .bind(tenant).bind(&artifacts).fetch_one(&mut **tx).await.map_err(database_failure)?;
        if any_hold {
            return Err(CommandFailure::precondition("LEGAL_HOLD_ACTIVE"));
        }
        let policy = self.read_policy(tx, &command.actor).await?;
        if !policy.permits(
            "deletion.request",
            "Artifact",
            target.resource_id.as_str(),
            BTreeMap::from([("version".into(), json!(target.version))]),
        ) {
            return Err(CommandFailure::denied("DELETION_DENIED"));
        }
        let external:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM effects WHERE tenant_id=$1 AND (content_ref->>'artifact_id')=ANY($2) AND state IN ('EXECUTING','SUCCEEDED','OUTCOME_UNKNOWN','RECONCILING','MANUAL_REVIEW'))")
            .bind(tenant).bind(artifacts.iter().map(ToString::to_string).collect::<Vec<_>>()).fetch_one(&mut **tx).await.map_err(database_failure)?;
        for artifact in &artifacts {
            sqlx::query("INSERT INTO deletion_tombstones(tenant_id,id,resource_type,resource_id,requested_by,reason) VALUES($1,$2,'Artifact',$3,$4,$5) ON CONFLICT(tenant_id,resource_type,resource_id) DO NOTHING")
                .bind(tenant).bind(Uuid::new_v4()).bind(artifact.to_string()).bind(parse_uuid(command.actor.principal_id.as_str())?).bind(&reason)
                .execute(&mut **tx).await.map_err(database_failure)?;
        }
        sqlx::query("UPDATE artifacts SET state='TOMBSTONED',version=version+1 WHERE tenant_id=$1 AND id=ANY($2) AND state<>'PURGED'")
            .bind(tenant).bind(&artifacts).execute(&mut **tx).await.map_err(database_failure)?;
        sqlx::query("UPDATE artifact_rights SET state='REVOKED',version=version+1 WHERE tenant_id=$1 AND artifact_id=ANY($2) AND state='ACTIVE'")
            .bind(tenant).bind(&artifacts).execute(&mut **tx).await.map_err(database_failure)?;
        let request = Uuid::new_v4();
        let state = if external {
            "PENDING_EXTERNAL"
        } else {
            "PURGE_PENDING"
        };
        sqlx::query("INSERT INTO deletion_requests(tenant_id,id,resource_type,resource_id,state,affected_artifact_ids,requested_by,reason) VALUES($1,$2,'Artifact',$3,$4,$5,$6,$7)")
            .bind(tenant).bind(request).bind(id.to_string()).bind(state).bind(&artifacts).bind(parse_uuid(command.actor.principal_id.as_str())?).bind(reason)
            .execute(&mut **tx).await.map_err(database_failure)?;
        Ok(Mutation::one(
            "DeletionRequest",
            request,
            1,
            json!({"request_id":request,"resource":target,"state":state,"affected_artifact_ids":artifacts,"physical_purge_complete":false,"external_copy_removal_complete":false,"version":1}),
            "File deletion request",
        ))
    }

    pub(crate) async fn spool_exact(
        &self,
        reader: &mut (dyn AsyncRead + Send + Unpin),
        size: u64,
    ) -> Result<(File, Digest), CommandFailure> {
        if !(1..=MAX_ARTIFACT_BYTES).contains(&size) {
            return Err(CommandFailure::invalid("ARTIFACT_SIZE_INVALID"));
        }
        let directory = self.spool_directory.clone();
        let raw = tokio::task::spawn_blocking(move || tempfile::tempfile_in(directory))
            .await
            .map_err(|_| CommandFailure::unavailable("ARTIFACT_SPOOL_UNAVAILABLE"))?
            .map_err(spool_failure)?;
        let mut file = File::from_std(raw);
        let mut remaining = size;
        let mut buffer = vec![0_u8; 64 * 1024];
        let mut digest = Sha256::new();
        while remaining > 0 {
            let count = remaining.min(buffer.len() as u64) as usize;
            reader
                .read_exact(&mut buffer[..count])
                .await
                .map_err(|_| CommandFailure::invalid("ARTIFACT_STREAM_TRUNCATED"))?;
            digest.update(&buffer[..count]);
            file.write_all(&buffer[..count])
                .await
                .map_err(spool_failure)?;
            remaining -= count as u64;
        }
        let mut extra = [0_u8; 1];
        if reader
            .read(&mut extra)
            .await
            .map_err(|_| CommandFailure::invalid("ARTIFACT_STREAM_INVALID"))?
            != 0
        {
            return Err(CommandFailure::invalid("ARTIFACT_SIZE_MISMATCH"));
        }
        file.flush().await.map_err(spool_failure)?;
        file.rewind().await.map_err(spool_failure)?;
        let digest = Digest::sha256(format!("{:x}", digest.finalize()))
            .map_err(|_| CommandFailure::unavailable("DIGEST_FAILURE"))?;
        Ok((file, digest))
    }

    async fn validate_file_mime(
        &self,
        file: &mut File,
        content_type: &str,
        size: u64,
    ) -> Result<(), CommandFailure> {
        validate_declared_mime(content_type)?;
        file.rewind().await.map_err(spool_failure)?;
        let mut prefix = vec![0; size.min(8192) as usize];
        file.read_exact(&mut prefix).await.map_err(spool_failure)?;
        file.seek(SeekFrom::Start(size.saturating_sub(1024)))
            .await
            .map_err(spool_failure)?;
        let mut tail = vec![0; size.min(1024) as usize];
        file.read_exact(&mut tail).await.map_err(spool_failure)?;
        validate_mime_boundaries(content_type, &prefix, &tail)?;
        if content_type == "image/png" || content_type == "image/jpeg" {
            file.rewind().await.map_err(spool_failure)?;
            let raw = file
                .try_clone()
                .await
                .map_err(spool_failure)?
                .into_std()
                .await;
            let format = if content_type == "image/png" {
                image::ImageFormat::Png
            } else {
                image::ImageFormat::Jpeg
            };
            tokio::task::spawn_blocking(move || {
                let reader = std::io::BufReader::new(DeadlineReader::new(raw));
                let mut image = image::ImageReader::with_format(reader, format);
                let mut limits = image::Limits::default();
                limits.max_image_width = Some(8192);
                limits.max_image_height = Some(8192);
                limits.max_alloc = Some(32 * 1024 * 1024);
                image.limits(limits);
                image
                    .decode()
                    .map(|_| ())
                    .map_err(|_| CommandFailure::precondition("ARTIFACT_MIME_MISMATCH"))
            })
            .await
            .map_err(|_| CommandFailure::unavailable("ARTIFACT_VALIDATION_UNAVAILABLE"))??;
        } else if content_type == "application/json" {
            file.rewind().await.map_err(spool_failure)?;
            let raw = file
                .try_clone()
                .await
                .map_err(spool_failure)?
                .into_std()
                .await;
            tokio::task::spawn_blocking(move || {
                let mut decoder = serde_json::Deserializer::from_reader(std::io::BufReader::new(
                    DeadlineReader::new(raw),
                ));
                serde::de::IgnoredAny::deserialize(&mut decoder)
                    .map_err(|_| CommandFailure::precondition("ARTIFACT_JSON_INVALID"))?;
                decoder
                    .end()
                    .map_err(|_| CommandFailure::precondition("ARTIFACT_JSON_INVALID"))
            })
            .await
            .map_err(|_| CommandFailure::unavailable("ARTIFACT_VALIDATION_UNAVAILABLE"))??;
        } else if content_type == "text/plain" || content_type == "text/csv" {
            file.rewind().await.map_err(spool_failure)?;
            let mut validator = Utf8Stream::default();
            let mut buffer = vec![0; 64 * 1024];
            loop {
                let count = file.read(&mut buffer).await.map_err(spool_failure)?;
                if count == 0 {
                    break;
                }
                validator.push(&buffer[..count])?;
            }
            validator.finish()?;
        }
        Ok(())
    }

    /// A verified spool prevents corrupted object-store bytes from reaching the
    /// browser before their declared digest is checked. Authorization is checked
    /// again after the download and before handing the file to the HTTP layer.
    pub async fn verified_artifact_download(
        &self,
        actor: &CommandActor,
        id: &str,
    ) -> Result<VerifiedArtifactDownload, CommandFailure> {
        let permit = self
            .upload_permits
            .clone()
            .try_acquire_owned()
            .map_err(|_| {
                CommandFailure::new("ARTIFACT_TRANSFER_CAPACITY_FULL", FailureKind::Exhausted)
            })?;
        let metadata = self.artifact_metadata(actor, id).await?;
        let mut tx = self.begin(actor, false).await?;
        let key: String = sqlx::query_scalar(
            "SELECT storage_key FROM artifacts WHERE tenant_id=$1 AND id=$2 AND state='ACTIVE'",
        )
        .bind(parse_uuid(actor.tenant_id.as_str())?)
        .bind(parse_uuid(id)?)
        .fetch_optional(&mut *tx)
        .await
        .map_err(database_failure)?
        .ok_or_else(CommandFailure::not_found)?;
        tx.commit().await.map_err(database_failure)?;
        let (file, digest) = tokio::time::timeout(UPLOAD_DEADLINE, async {
            let mut source = self
                .objects
                .open_reader(&key, metadata.size_bytes as u64)
                .await?;
            if source.size_bytes != metadata.size_bytes as u64 {
                return Err(CommandFailure::precondition("ARTIFACT_INTEGRITY"));
            }
            self.spool_exact(&mut source.reader, source.size_bytes)
                .await
        })
        .await
        .map_err(|_| CommandFailure::unavailable("ARTIFACT_DOWNLOAD_TIMEOUT"))??;
        if digest != metadata.reference.digest {
            return Err(CommandFailure::precondition("ARTIFACT_INTEGRITY"));
        }
        let current = self.artifact_metadata(actor, id).await?;
        if current.reference != metadata.reference {
            return Err(CommandFailure::precondition("ARTIFACT_INTEGRITY"));
        }
        Ok(VerifiedArtifactDownload {
            metadata: current,
            file,
            permit,
        })
    }
}

struct UploadProjection<'a> {
    upload: Uuid,
    tenant: Uuid,
    artifact: Uuid,
    size: i64,
    digest: &'a str,
    content_type: &'a str,
    expires: DateTime<Utc>,
    state: &'a str,
}

fn upload_json(
    UploadProjection {
        upload,
        tenant,
        artifact,
        size,
        digest,
        content_type,
        expires,
        state,
    }: UploadProjection<'_>,
) -> Value {
    json!({"upload_id":upload,"tenant_id":tenant,"artifact_id":artifact,"upload_path":format!("/v1/tenants/{tenant}/uploads/{upload}"),"expected_size_bytes":size,"expected_digest":digest,"content_type":content_type,"expires_at":expires,"state":state})
}

fn spool_failure(_: std::io::Error) -> CommandFailure {
    CommandFailure::unavailable("ARTIFACT_SPOOL_UNAVAILABLE")
}

struct DeadlineReader {
    file: std::fs::File,
    deadline: std::time::Instant,
}
impl DeadlineReader {
    fn new(file: std::fs::File) -> Self {
        Self {
            file,
            deadline: std::time::Instant::now() + Duration::from_secs(10),
        }
    }
    fn check(&self) -> std::io::Result<()> {
        if std::time::Instant::now() >= self.deadline {
            Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "artifact validation deadline",
            ))
        } else {
            Ok(())
        }
    }
}
impl std::io::Read for DeadlineReader {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        self.check()?;
        std::io::Read::read(&mut self.file, bytes)
    }
}
impl std::io::Seek for DeadlineReader {
    fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
        self.check()?;
        std::io::Seek::seek(&mut self.file, position)
    }
}

fn validate_declared_mime(content_type: &str) -> Result<(), CommandFailure> {
    if !matches!(
        content_type,
        "application/octet-stream"
            | "application/json"
            | "text/plain"
            | "text/csv"
            | "application/pdf"
            | "image/png"
            | "image/jpeg"
            | "application/wasm"
    ) {
        return Err(CommandFailure::invalid("ARTIFACT_MIME_UNSUPPORTED"));
    }
    Ok(())
}

fn validate_mime_boundaries(
    content_type: &str,
    prefix: &[u8],
    tail: &[u8],
) -> Result<(), CommandFailure> {
    let trimmed_tail = tail.trim_ascii_end();
    let recognized = match content_type {
        "image/png" => {
            prefix.starts_with(b"\x89PNG\r\n\x1a\n") && tail.ends_with(b"\0\0\0\0IEND\xaeB`\x82")
        }
        "image/jpeg" => prefix.starts_with(b"\xff\xd8\xff") && tail.ends_with(b"\xff\xd9"),
        "application/pdf" => prefix.starts_with(b"%PDF-") && trimmed_tail.ends_with(b"%%EOF"),
        "application/wasm" => prefix.starts_with(b"\0asm") && prefix.len() >= 8,
        "application/json" | "text/plain" | "text/csv" => !prefix.contains(&0),
        "application/octet-stream" => true,
        _ => false,
    };
    if !recognized {
        return Err(CommandFailure::precondition("ARTIFACT_MIME_MISMATCH"));
    }
    if matches!(content_type, "image/png" | "image/jpeg" | "application/pdf") {
        for bytes in [prefix, tail] {
            let lower = bytes.to_ascii_lowercase();
            if [b"<script".as_slice(), b"<!doctype html", b"<html", b"<?php"]
                .iter()
                .any(|marker| lower.windows(marker.len()).any(|slice| slice == *marker))
            {
                return Err(CommandFailure::precondition("ARTIFACT_POLYGLOT"));
            }
        }
    }
    Ok(())
}

#[derive(Default)]
struct Utf8Stream {
    carry: Vec<u8>,
}
impl Utf8Stream {
    fn push(&mut self, bytes: &[u8]) -> Result<(), CommandFailure> {
        if bytes.contains(&0) {
            return Err(CommandFailure::precondition("ARTIFACT_MIME_MISMATCH"));
        }
        self.carry.extend_from_slice(bytes);
        match std::str::from_utf8(&self.carry) {
            Ok(_) => self.carry.clear(),
            Err(error) if error.error_len().is_none() => {
                self.carry.drain(..error.valid_up_to());
                if self.carry.len() > 3 {
                    return Err(CommandFailure::precondition("ARTIFACT_MIME_MISMATCH"));
                }
            }
            Err(_) => return Err(CommandFailure::precondition("ARTIFACT_MIME_MISMATCH")),
        }
        Ok(())
    }
    fn finish(self) -> Result<(), CommandFailure> {
        if self.carry.is_empty() {
            Ok(())
        } else {
            Err(CommandFailure::precondition("ARTIFACT_MIME_MISMATCH"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use masonwing_contract_validation::{SchemaCatalog, digest_bytes};
    #[test]
    fn mime_spoof_and_active_content_polyglot_do_not_become_clean() {
        assert_eq!(
            validate_mime_boundaries("image/jpeg", b"<html>file", b"\xff\xd9")
                .unwrap_err()
                .code,
            "ARTIFACT_MIME_MISMATCH"
        );
        assert_eq!(
            validate_mime_boundaries("image/jpeg", b"\xff\xd8\xff<script>bad", b"\xff\xd9")
                .unwrap_err()
                .code,
            "ARTIFACT_POLYGLOT"
        );
        assert!(validate_declared_mime("text/html").is_err());
        assert!(validate_declared_mime("image/svg+xml").is_err());
    }
    #[test]
    fn utf8_validation_handles_split_multibyte_and_rejects_truncation() {
        let mut good = Utf8Stream::default();
        for byte in "Xin chào 👋".as_bytes() {
            good.push(&[*byte]).unwrap();
        }
        good.finish().unwrap();
        let mut bad = Utf8Stream::default();
        bad.push(&[0xf0, 0x9f]).unwrap();
        assert!(bad.finish().is_err());
        assert!(Utf8Stream::default().push(&[0xff]).is_err());
    }
    #[test]
    fn upload_projection_matches_immutable_contract() {
        let value = upload_json(UploadProjection {
            upload: Uuid::new_v4(),
            tenant: Uuid::new_v4(),
            artifact: Uuid::new_v4(),
            size: 1,
            digest: digest_bytes(b"a").as_str(),
            content_type: "text/plain",
            expires: Utc::now(),
            state: "OPEN",
        });
        SchemaCatalog::shared()
            .unwrap()
            .validate("UploadSession", &value)
            .unwrap();
    }
}
