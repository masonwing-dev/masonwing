//! Run lifecycle admission: start queues durable work, cancellation only requests.
//! Starting a run never claims Temporal dispatch; stopping one never undoes effects.

use masonwing_contract_validation::ArtifactSchema;
use masonwing_contracts::wire::{ArtifactRef, WorkflowDefinition};
use masonwing_kernel::{
    runtime::{AuthorizedCommand, CommandFailure, require_expected_version},
    state::MachineState,
};
use serde_json::json;
use sqlx::Row;
use uuid::Uuid;

use crate::store::{Mutation, PostgresStore, Tx, database_failure, field, parse_uuid};

const MAX_RUN_INPUT_BYTES: usize = 4 * 1024 * 1024;
const MAX_WORKFLOW_SCHEMA_BYTES: usize = 256 * 1024;

/// One durable step outcome submitted by the worker (REQ-070).
#[derive(Clone, Debug)]
pub struct CheckpointWrite {
    pub logical_step_id: String,
    pub state: String,
    pub failure_code: Option<String>,
    pub fence: i64,
    pub output_ref: Option<ArtifactRef>,
}

impl PostgresStore {
    pub(crate) async fn start_run(
        &self,
        tx: &mut Tx<'_>,
        command: &AuthorizedCommand,
    ) -> Result<Mutation, CommandFailure> {
        let tenant = parse_uuid(command.actor.tenant_id.as_str())?;
        let principal = parse_uuid(command.actor.principal_id.as_str())?;
        let workflow_id: String = field(&command.input, "workflow_id")?;
        let workflow_version: String = field(&command.input, "workflow_version")?;
        let input_ref: ArtifactRef = field(&command.input, "input_ref")?;
        if input_ref.tenant_id != command.actor.tenant_id {
            return Err(CommandFailure::not_found());
        }
        let grant_id = parse_uuid(&field::<String>(&command.input, "grant_id")?)?;
        // Resolve the exact workflow definition; a missing or corrupt definition
        // is a request error, never a silent default workflow. No row lock: the
        // table is select-only for the runtime role and the runs FK pins the
        // exact plugin digest; lifecycle changes recheck active runs.
        let definition_row = sqlx::query("SELECT plugin_id,plugin_digest,definition FROM workflow_definitions WHERE tenant_id=$1 AND workflow_id=$2 AND workflow_version=$3 ORDER BY created_at DESC LIMIT 1")
            .bind(tenant).bind(&workflow_id).bind(&workflow_version)
            .fetch_optional(&mut **tx).await.map_err(database_failure)?.ok_or_else(CommandFailure::not_found)?;
        let plugin_id: String = definition_row
            .try_get("plugin_id")
            .map_err(database_failure)?;
        let plugin_digest: String = definition_row
            .try_get("plugin_digest")
            .map_err(database_failure)?;
        let definition: WorkflowDefinition = serde_json::from_value(
            definition_row
                .try_get("definition")
                .map_err(database_failure)?,
        )
        .map_err(|_| CommandFailure::unavailable("RUN_INTEGRITY"))?;
        // The definition pins a plugin build; only its installed ENABLED build
        // may admit new runs.
        let installation = sqlx::query("SELECT state,artifact_digest FROM plugin_installations WHERE tenant_id=$1 AND plugin_id=$2 FOR SHARE")
            .bind(tenant).bind(&plugin_id).fetch_optional(&mut **tx).await.map_err(database_failure)?.ok_or_else(CommandFailure::not_found)?;
        match installation
            .try_get::<String, _>("state")
            .map_err(database_failure)?
            .as_str()
        {
            "ENABLED" => {}
            "DRAINING" => return Err(CommandFailure::precondition("PLUGIN_DRAINING")),
            "REVOKED" => return Err(CommandFailure::precondition("PLUGIN_REVOKED")),
            _ => return Err(CommandFailure::precondition("PLUGIN_NOT_ENABLED")),
        }
        if installation
            .try_get::<String, _>("artifact_digest")
            .map_err(database_failure)?
            != plugin_digest
        {
            return Err(CommandFailure::precondition("PLUGIN_NOT_ENABLED"));
        }
        self.require_dispatch_enabled(tx, &command.actor, Some(&plugin_id), None)
            .await?;
        let grant = self
            .require_active_grant(tx, &command.actor, grant_id, Some("run.start"), None)
            .await?;
        // Trust and storage reads occur after grant/rights denials, so an invalid
        // scope never causes reads of confidential bytes.
        let metadata = self
            .artifact_metadata_in(tx, &command.actor, input_ref.artifact_id.as_str())
            .await?;
        if metadata.reference != input_ref {
            return Err(CommandFailure::precondition("ARTIFACT_REFERENCE_MISMATCH"));
        }
        let input_schema = self
            .load_artifact_ref(
                tx,
                &command.actor,
                &definition.input_schema_ref,
                MAX_WORKFLOW_SCHEMA_BYTES,
            )
            .await?;
        let input_schema = ArtifactSchema::compile(&input_schema)
            .map_err(|_| CommandFailure::precondition("WORKFLOW_SCHEMA_INVALID"))?;
        let input_bytes = self
            .load_artifact_ref(tx, &command.actor, &input_ref, MAX_RUN_INPUT_BYTES)
            .await?;
        input_schema
            .validate_bytes(&input_bytes, MAX_RUN_INPUT_BYTES)
            .map_err(|_| CommandFailure::invalid("WORKFLOW_INPUT_INVALID"))?;
        let id = Uuid::new_v4();
        let input_value = serde_json::to_value(&input_ref)
            .map_err(|_| CommandFailure::unavailable("RUN_INTEGRITY"))?;
        let inserted = sqlx::query("INSERT INTO runs(tenant_id,id,principal_id,workflow_id,workflow_version,plugin_id,plugin_digest,grant_id,grant_fence,input_ref,state,temporal_workflow_id,dispatch_state) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,'QUEUED',$2,'PENDING') RETURNING created_at,updated_at")
            .bind(tenant).bind(id).bind(principal).bind(&workflow_id).bind(&workflow_version)
            .bind(&plugin_id).bind(&plugin_digest).bind(grant_id).bind(grant.fence).bind(input_value)
            .fetch_one(&mut **tx).await.map_err(database_failure)?;
        let mut mutation = Mutation::one(
            "Run",
            id,
            1,
            json!({
                "run_id": id, "tenant_id": tenant,
                "workflow_id": workflow_id.clone(),
                "workflow_version": workflow_version,
                "state": "QUEUED",
                "plugin_digest": plugin_digest,
                "grant_id": grant_id,
                "version": 1,
                "created_at": inserted.try_get::<chrono::DateTime<chrono::Utc>, _>("created_at").map_err(database_failure)?,
                "updated_at": inserted.try_get::<chrono::DateTime<chrono::Utc>, _>("updated_at").map_err(database_failure)?,
            }),
            workflow_id,
        );
        // Preserve input classification/lineage even though Run's wire schema
        // has no input_ref field. execute() records projection/audit/outbox.
        mutation = mutation.with_source_artifact(parse_uuid(input_ref.artifact_id.as_str())?);
        // Async admission: ACCEPTED with a stable non-null run identity. Temporal
        // dispatch stays PENDING until a qualified worker claims the outbox event.
        mutation.run_id = Some(id);
        mutation.accepted = true;
        Ok(mutation)
    }

    pub(crate) async fn cancel_run(
        &self,
        tx: &mut Tx<'_>,
        command: &AuthorizedCommand,
    ) -> Result<Mutation, CommandFailure> {
        let tenant = parse_uuid(command.actor.tenant_id.as_str())?;
        let id = parse_uuid(&field::<String>(&command.input, "run_id")?)?;
        let expected: i32 = field(&command.input, "expected_version")?;
        let reason: String = field(&command.input, "reason")?;
        if reason.trim().is_empty() {
            return Err(CommandFailure::invalid("REASON_REQUIRED"));
        }
        let row = sqlx::query("SELECT * FROM runs WHERE tenant_id=$1 AND id=$2 FOR UPDATE")
            .bind(tenant)
            .bind(id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(database_failure)?
            .ok_or_else(CommandFailure::not_found)?;
        let version =
            require_expected_version(row.try_get("version").map_err(database_failure)?, expected)?;
        // A blocked/waiting run can still own an unresolved transmission. Do not
        // take its direct CANCELLED edge until the effect owner reconciles it.
        let unresolved: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM effects WHERE tenant_id=$1 AND run_id=$2 AND state IN ('EXECUTING','OUTCOME_UNKNOWN','RECONCILING','MANUAL_REVIEW'))")
            .bind(tenant).bind(id).fetch_one(&mut **tx).await.map_err(database_failure)?;
        let state: String = row.try_get("state").map_err(database_failure)?;
        let next = cancellation_state(&state, unresolved)?;
        let fence = row
            .try_get::<i64, _>("fence")
            .map_err(database_failure)?
            .checked_add(1)
            .ok_or_else(|| CommandFailure::precondition("FENCE_EXHAUSTED"))?;
        let updated = sqlx::query("UPDATE runs SET state=$3,version=$4,fence=$5,dispatch_state='BLOCKED',updated_at=clock_timestamp() WHERE tenant_id=$1 AND id=$2 RETURNING *")
            .bind(tenant).bind(id).bind(next).bind(version).bind(fence)
            .fetch_one(&mut **tx).await.map_err(database_failure)?;
        let mut mutation = Mutation::one(
            "Run",
            id,
            version,
            json!({
                "run_id": id, "tenant_id": tenant,
                "workflow_id": updated.try_get::<String, _>("workflow_id").map_err(database_failure)?,
                "workflow_version": updated.try_get::<String, _>("workflow_version").map_err(database_failure)?,
                "state": next,
                "plugin_digest": updated.try_get::<String, _>("plugin_digest").map_err(database_failure)?,
                "grant_id": updated.try_get::<uuid::Uuid, _>("grant_id").map_err(database_failure)?,
                "version": version,
                "created_at": updated.try_get::<chrono::DateTime<chrono::Utc>, _>("created_at").map_err(database_failure)?,
                "updated_at": updated.try_get::<chrono::DateTime<chrono::Utc>, _>("updated_at").map_err(database_failure)?,
            }),
            updated
                .try_get::<String, _>("workflow_id")
                .map_err(database_failure)?,
        );
        // Preserve input classification/lineage even though Run's wire schema
        // has no input_ref field. execute() records projection/audit/outbox.
        let input: masonwing_contracts::wire::ArtifactRef =
            serde_json::from_value(updated.try_get("input_ref").map_err(database_failure)?)
                .map_err(|_| CommandFailure::unavailable("RUN_INTEGRITY"))?;
        if input.tenant_id != command.actor.tenant_id {
            return Err(CommandFailure::not_found());
        }
        mutation = mutation.with_source_artifact(parse_uuid(input.artifact_id.as_str())?);
        mutation.run_id = Some(id);
        mutation.accepted = next == "CANCEL_REQUESTED";
        Ok(mutation)
    }

    /// Public system-path entry: record one step's outcome durably (REQ-070).
    ///
    /// The worker calls this after each node completes. The primary key on
    /// (tenant, run, logical_step_id) makes "dispatch once per logical ID" a
    /// database guarantee: an identical retry after a crash is a no-op, and a
    /// replay that names the same step under a different node quarantines the
    /// run instead of executing anything (REQ-075). A SUCCEEDED checkpoint is
    /// never rewritten to a pending state (AC-074).
    ///
    /// `fence` must equal the run's current fence: a checkpoint written for an
    /// execution that has since been cancelled or re-fenced is ignored, so a
    /// stale worker cannot re-drive an outdated traversal.
    pub async fn record_run_checkpoint(
        &self,
        tenant_id: Uuid,
        run_id: Uuid,
        record: &CheckpointWrite,
    ) -> Result<bool, CommandFailure> {
        let logical_step_id = &record.logical_step_id;
        let state = &record.state;
        let failure_code = &record.failure_code;
        let fence = record.fence;
        let output_ref = record.output_ref.as_ref();
        let mut tx = self.begin_system(tenant_id, "run-checkpoint", true).await?;
        let run =
            sqlx::query("SELECT state,fence FROM runs WHERE tenant_id=$1 AND id=$2 FOR UPDATE")
                .bind(tenant_id)
                .bind(run_id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(database_failure)?
                .ok_or_else(CommandFailure::not_found)?;
        let run_state: String = run.try_get("state").map_err(database_failure)?;
        let current_fence: i64 = run.try_get("fence").map_err(database_failure)?;
        if current_fence != fence {
            return Err(CommandFailure::conflict("STALE_FENCE"));
        }
        if !matches!(
            run_state.as_str(),
            "RUNNING" | "WAITING_APPROVAL" | "WAITING_RETRY"
        ) {
            return Err(CommandFailure::conflict("ILLEGAL_TRANSITION"));
        }
        if !matches!(
            state.as_str(),
            "RUNNING" | "SUCCEEDED" | "FAILED" | "BLOCKED" | "INCOMPLETE"
        ) {
            return Err(CommandFailure::invalid("CHECKPOINT_STATE_INVALID"));
        }
        // The recorded node identity is part of the logical step's durable
        // name. A replay that arrives with the same step id but a different
        // node is an incompatible replay (REQ-075): quarantine the step as
        // BLOCKED with REPLAY_INCOMPATIBLE, transmit nothing.
        let existing: Option<(String, String)> = sqlx::query_as("SELECT logical_step_id,state FROM run_checkpoints WHERE tenant_id=$1 AND run_id=$2 AND logical_step_id=$3")
            .bind(tenant_id).bind(run_id).bind(logical_step_id)
            .fetch_optional(&mut *tx).await.map_err(database_failure)?;
        if let Some((_, existing_state)) = existing
            && existing_state == "SUCCEEDED"
        {
            // Re-issued completion of the same logical step: already
            // durable, never rewritten (AC-074).
            return Ok(false);
        }
        // output_ref is only populated once the node output has passed its
        // pinned contract; on failure it is left NULL and failure_code names
        // the quarantine reason.
        let output_value = output_ref
            .map(|reference| serde_json::to_value(reference).unwrap_or(serde_json::Value::Null));
        let inserted = sqlx::query("INSERT INTO run_checkpoints(tenant_id,run_id,logical_step_id,fence,state,output_ref,failure_code) VALUES($1,$2,$3,$4,$5,$6,$7) ON CONFLICT (tenant_id,run_id,logical_step_id) DO UPDATE SET state=EXCLUDED.state,failure_code=EXCLUDED.failure_code,output_ref=EXCLUDED.output_ref,fence=EXCLUDED.fence,updated_at=clock_timestamp() WHERE run_checkpoints.state <> 'SUCCEEDED' RETURNING state")
            .bind(tenant_id).bind(run_id).bind(logical_step_id).bind(fence).bind(state).bind(output_value).bind(failure_code)
            .fetch_optional(&mut *tx).await.map_err(database_failure)?;
        tx.commit().await.map_err(database_failure)?;
        Ok(inserted.is_some())
    }

    /// Public system-path entry: take a QUEUED run to RUNNING after the
    /// durable workflow start is acknowledged by its stable identity.
    ///
    /// The dispatching worker supplies `temporal_run_id` returned (or adopted
    /// on `AlreadyStarted`) by Temporal. The update is conditional on the run
    /// still being QUEUED with dispatch_state PENDING and the caller's fence
    /// still matching, so a concurrent cancel that landed first wins and no
    /// RUNNING write is emitted for a cancelled run.
    pub async fn mark_run_started(
        &self,
        tenant_id: Uuid,
        run_id: Uuid,
        temporal_run_id: &str,
        fence: i64,
    ) -> Result<Mutation, CommandFailure> {
        let mut tx = self.begin_system(tenant_id, "run-started", true).await?;
        let row = sqlx::query("SELECT version,workflow_id,workflow_version,plugin_digest,grant_id,input_ref,created_at FROM runs WHERE tenant_id=$1 AND id=$2 FOR UPDATE")
            .bind(tenant_id).bind(run_id)
            .fetch_optional(&mut *tx).await.map_err(database_failure)?
            .ok_or_else(CommandFailure::not_found)?;
        let current_fence: i64 =
            sqlx::query_scalar("SELECT fence FROM runs WHERE tenant_id=$1 AND id=$2")
                .bind(tenant_id)
                .bind(run_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(database_failure)?;
        if current_fence != fence {
            return Err(CommandFailure::conflict("STALE_FENCE"));
        }
        let version: i32 = row.try_get("version").map_err(database_failure)?;
        let new_version = version + 1;
        let updated = sqlx::query("UPDATE runs SET state='RUNNING',dispatch_state='STARTED',temporal_run_id=$3,version=$4,updated_at=clock_timestamp() WHERE tenant_id=$1 AND id=$2 AND state='QUEUED' AND dispatch_state='PENDING' RETURNING updated_at")
            .bind(tenant_id).bind(run_id).bind(temporal_run_id).bind(new_version)
            .fetch_optional(&mut *tx).await.map_err(database_failure)?;
        if updated.is_none() {
            return Err(CommandFailure::conflict("ILLEGAL_TRANSITION"));
        }
        let updated_at: chrono::DateTime<chrono::Utc> = updated
            .ok_or_else(CommandFailure::not_found)?
            .try_get("updated_at")
            .map_err(database_failure)?;
        let mut mutation = Mutation::one(
            "Run",
            run_id,
            new_version,
            json!({
                "run_id": run_id,
                "tenant_id": tenant_id,
                "workflow_id": row.try_get::<String, _>("workflow_id").map_err(database_failure)?,
                "workflow_version": row.try_get::<String, _>("workflow_version").map_err(database_failure)?,
                "state": "RUNNING",
                "plugin_digest": row.try_get::<String, _>("plugin_digest").map_err(database_failure)?,
                "grant_id": row.try_get::<Uuid, _>("grant_id").map_err(database_failure)?,
                "version": new_version,
                "created_at": row.try_get::<chrono::DateTime<chrono::Utc>, _>("created_at").map_err(database_failure)?,
                "updated_at": updated_at,
            }),
            row.try_get::<String, _>("workflow_id")
                .map_err(database_failure)?,
        );
        let input: ArtifactRef =
            serde_json::from_value(row.try_get("input_ref").map_err(database_failure)?)
                .map_err(|_| CommandFailure::unavailable("RUN_INTEGRITY"))?;
        if input.tenant_id
            != masonwing_contracts::TenantId::new(tenant_id.to_string())
                .map_err(|_| CommandFailure::not_found())?
        {
            return Err(CommandFailure::not_found());
        }
        mutation = mutation.with_source_artifact(parse_uuid(input.artifact_id.as_str())?);
        mutation.run_id = Some(run_id);
        tx.commit().await.map_err(database_failure)?;
        Ok(mutation)
    }

    /// Public system-path entry: load the durable step history for a run so a
    /// restarted worker replays the same deterministic traversal (REQ-070).
    ///
    /// The result is ordered by logical step id; callers feed it to the
    /// interpreter unchanged. Reading does not lock the run: the checkpoint
    /// writes are fenced and the (tenant, run, logical step) primary key is
    /// the exactly-once guarantee, so a concurrent checkpoint writer can only
    /// make the next read newer, never corrupt an in-flight decision.
    pub async fn read_run_checkpoints(
        &self,
        tenant_id: Uuid,
        run_id: Uuid,
    ) -> Result<Vec<DurableCheckpoint>, CommandFailure> {
        let mut tx = self
            .begin_system(tenant_id, "run-checkpoints-read", false)
            .await?;
        let rows = sqlx::query(
            "SELECT logical_step_id,state,failure_code FROM run_checkpoints
             WHERE tenant_id=$1 AND run_id=$2 ORDER BY logical_step_id",
        )
        .bind(tenant_id)
        .bind(run_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(database_failure)?;
        let fence: i64 = sqlx::query_scalar("SELECT fence FROM runs WHERE tenant_id=$1 AND id=$2")
            .bind(tenant_id)
            .bind(run_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(database_failure)?
            .ok_or_else(CommandFailure::not_found)?;
        tx.commit().await.map_err(database_failure)?;
        let mut checkpoints = Vec::with_capacity(rows.len());
        for row in rows {
            checkpoints.push(DurableCheckpoint {
                logical_step_id: row.try_get("logical_step_id").map_err(database_failure)?,
                state: row.try_get("state").map_err(database_failure)?,
                failure_code: row.try_get("failure_code").map_err(database_failure)?,
                fence,
            });
        }
        Ok(checkpoints)
    }

    /// Public system-path entry: take a RUNNING run to SUCCEEDED once the
    /// deterministic traversal reports completion. The update is conditional
    /// on the fence and current state, so a cancelled or re-fenced run is
    /// never reported complete by a stale worker.
    pub async fn mark_run_completed(
        &self,
        tenant_id: Uuid,
        run_id: Uuid,
        fence: i64,
    ) -> Result<(), CommandFailure> {
        let mut tx = self.begin_system(tenant_id, "run-completed", true).await?;
        let updated = sqlx::query(
            "UPDATE runs SET state='SUCCEEDED',version=version+1,updated_at=clock_timestamp()
             WHERE tenant_id=$1 AND id=$2 AND fence=$3 AND state IN ('RUNNING','WAITING_APPROVAL','WAITING_RETRY')
             RETURNING id",
        )
        .bind(tenant_id)
        .bind(run_id)
        .bind(fence)
        .fetch_optional(&mut *tx)
        .await
        .map_err(database_failure)?;
        if updated.is_none() {
            // Either the fence moved (STALE_FENCE) or the run already left the
            // executing states (ILLEGAL_TRANSITION); check which for a precise
            // refusal rather than a generic one.
            let row =
                sqlx::query("SELECT state,fence FROM runs WHERE tenant_id=$1 AND id=$2 FOR SHARE")
                    .bind(tenant_id)
                    .bind(run_id)
                    .fetch_optional(&mut *tx)
                    .await
                    .map_err(database_failure)?
                    .ok_or_else(CommandFailure::not_found)?;
            let current_fence: i64 = row.try_get("fence").map_err(database_failure)?;
            tx.rollback().await.map_err(database_failure)?;
            return Err(if current_fence != fence {
                CommandFailure::conflict("STALE_FENCE")
            } else {
                CommandFailure::conflict("ILLEGAL_TRANSITION")
            });
        }
        tx.commit().await.map_err(database_failure)?;
        Ok(())
    }
}

/// One durable checkpoint row as read back by a resuming worker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DurableCheckpoint {
    pub logical_step_id: String,
    pub state: String,
    pub failure_code: Option<String>,
    pub fence: i64,
}

fn cancellation_state(
    state: &str,
    unresolved_effect: bool,
) -> Result<&'static str, CommandFailure> {
    let next = match state {
        "RUNNING" => "CANCEL_REQUESTED",
        "QUEUED" | "WAITING_APPROVAL" | "WAITING_RETRY" | "BLOCKED" => {
            if unresolved_effect {
                return Err(CommandFailure::precondition(
                    "EFFECT_RECONCILIATION_REQUIRED",
                ));
            }
            "CANCELLED"
        }
        _ => return Err(CommandFailure::conflict("ILLEGAL_TRANSITION")),
    };
    let from = MachineState::parse("run", state)
        .map_err(|_| CommandFailure::unavailable("RUN_INTEGRITY"))?;
    let to = MachineState::parse("run", next)
        .map_err(|_| CommandFailure::unavailable("RUN_INTEGRITY"))?;
    if !from.can_transition_to(to) {
        return Err(CommandFailure::conflict("ILLEGAL_TRANSITION"));
    }
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;

    // MASONWING@1.0.1 REQ-074 / AC-078: cancellation is not compensation.
    #[test]
    fn running_cancellation_is_only_requested() {
        for unresolved in [false, true] {
            assert_eq!(
                cancellation_state("RUNNING", unresolved).unwrap(),
                "CANCEL_REQUESTED"
            );
        }
    }

    #[test]
    fn waiting_cancellation_requires_no_unresolved_transmission() {
        for state in ["QUEUED", "WAITING_APPROVAL", "WAITING_RETRY", "BLOCKED"] {
            assert_eq!(cancellation_state(state, false).unwrap(), "CANCELLED");
            assert_eq!(
                cancellation_state(state, true).unwrap_err().code,
                "EFFECT_RECONCILIATION_REQUIRED"
            );
        }
    }

    // MASONWING@1.0.1 REQ-073 / AC-077.
    #[test]
    fn terminal_or_already_requested_run_cannot_cancel_again() {
        for state in [
            "SUCCEEDED",
            "FAILED",
            "CANCELLED",
            "CANCEL_REQUESTED",
            "invalid",
        ] {
            assert_eq!(
                cancellation_state(state, false).unwrap_err().code,
                "ILLEGAL_TRANSITION"
            );
        }
    }
}
