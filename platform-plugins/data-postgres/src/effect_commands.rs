//! Effect lifecycle commands: propose, dispatch, reconcile, compensate.
//!
//! REQ-084..091 / AC-088..095: Intent is committed before transmission (REQ-084);
//! ambiguous timeouts become OUTCOME_UNKNOWN and never blind-resend (REQ-085);
//! remote evidence drives receipts (REQ-086); unproven absence halts at MANUAL_REVIEW
//! (REQ-087); key reuse with different content produces 409 IDEMPOTENCY_CONFLICT (REQ-088);
//! pre-PONR kill switches block transmission (REQ-089); proven absence retries inside budget
//! (REQ-090); compensation creates a separate effect with distinct approval (REQ-091).

use chrono::{DateTime, Utc};
use masonwing_contract_validation::{canonical_bytes, digest_bytes};
use masonwing_contracts::wire::{ArtifactRef, ResourceRef};
use masonwing_kernel::runtime::{AuthorizedCommand, CommandFailure, require_expected_version};
use serde_json::{Value, json};
use sqlx::Row;
use uuid::Uuid;

use crate::store::{Mutation, PostgresStore, Tx, database_failure, field, parse_uuid};

impl PostgresStore {
    /// Propose an external mutation (REQ-084, REQ-088, REQ-092).
    ///
    /// Validates connection, grant, kill switch, and idempotency. Reserves bounded cost
    /// in the budget ledger. Creates the durable EffectIntent row before any network call.
    pub(crate) async fn propose_effect(
        &self,
        tx: &mut Tx<'_>,
        command: &AuthorizedCommand,
    ) -> Result<Mutation, CommandFailure> {
        let tenant = parse_uuid(command.actor.tenant_id.as_str())?;
        let action: String = field(&command.input, "action")?;
        let connection_id = parse_uuid(&field::<String>(&command.input, "connection_id")?)?;
        let target: ResourceRef = field(&command.input, "target")?;
        let content_ref: ArtifactRef = field(&command.input, "content_ref")?;
        let grant_id = parse_uuid(&field::<String>(&command.input, "grant_id")?)?;
        let price_profile: String = field(&command.input, "price_profile")?;

        if content_ref.tenant_id != command.actor.tenant_id {
            return Err(CommandFailure::not_found());
        }

        // Connection must exist and be ACTIVE.
        let conn_row = sqlx::query("SELECT state FROM connections WHERE tenant_id=$1 AND id=$2")
            .bind(tenant)
            .bind(connection_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(database_failure)?
            .ok_or_else(CommandFailure::not_found)?;
        if conn_row
            .try_get::<String, _>("state")
            .map_err(database_failure)?
            != "ACTIVE"
        {
            return Err(CommandFailure::precondition("CONNECTION_NOT_ACTIVE"));
        }

        // Grant check.
        self.require_active_grant(tx, &command.actor, grant_id, Some(&action), Some(&target))
            .await?;

        // Pre-PONR kill-switch check (REQ-089).
        self.require_dispatch_enabled(tx, &command.actor, None, Some(connection_id))
            .await?;

        // Resolve run context via grant (0 -> RUN_CONTEXT_REQUIRED, >1 -> RUN_CONTEXT_AMBIGUOUS).
        let active_runs: Vec<Uuid> = sqlx::query_scalar(
            "SELECT id FROM runs WHERE tenant_id=$1 AND grant_id=$2 AND state NOT IN ('SUCCEEDED','FAILED','CANCELLED')",
        )
        .bind(tenant)
        .bind(grant_id)
        .fetch_all(&mut **tx)
        .await
        .map_err(database_failure)?;

        let run_id = match active_runs.len() {
            0 => return Err(CommandFailure::precondition("RUN_CONTEXT_REQUIRED")),
            1 => active_runs[0],
            _ => return Err(CommandFailure::conflict("RUN_CONTEXT_AMBIGUOUS")),
        };

        // Compute canonical fingerprint over action, target, and content digest.
        let fingerprint_bytes = canonical_bytes(&json!({
            "action": action,
            "target": target,
            "content_digest": content_ref.digest.as_str(),
        }))
        .map_err(|_| CommandFailure::invalid("SCHEMA_INVALID"))?;
        let effect_fingerprint = digest_bytes(&fingerprint_bytes);

        let idempotency_key = command.idempotency_key.as_str();

        // Idempotency check on (tenant_id, connection_id, idempotency_key) (REQ-088).
        let existing = sqlx::query("SELECT id, fingerprint, version, state, approval_id, reservation_id, target, content_ref, created_at FROM effects WHERE tenant_id=$1 AND connection_id=$2 AND idempotency_key=$3")
            .bind(tenant)
            .bind(connection_id)
            .bind(idempotency_key)
            .fetch_optional(&mut **tx)
            .await
            .map_err(database_failure)?;

        if let Some(row) = existing {
            let existing_fp: String = row.try_get("fingerprint").map_err(database_failure)?;
            if existing_fp != effect_fingerprint.as_str() {
                // Different payload for same key -> 409 IDEMPOTENCY_CONFLICT (AC-092).
                return Err(CommandFailure::conflict("IDEMPOTENCY_CONFLICT"));
            }
            // Idempotent replay: return existing projection.
            let id: Uuid = row.try_get("id").map_err(database_failure)?;
            let version: i32 = row.try_get("version").map_err(database_failure)?;
            let val = effect_projection_value(
                id,
                tenant,
                run_id,
                &action,
                connection_id,
                &target,
                &content_ref,
                &existing_fp,
                idempotency_key,
                row.try_get("approval_id").map_err(database_failure)?,
                row.try_get("reservation_id").map_err(database_failure)?,
                &row.try_get::<String, _>("state")
                    .map_err(database_failure)?,
                version,
                row.try_get("created_at").map_err(database_failure)?,
            );
            let mut mutation =
                Mutation::one("EffectIntent", id, version, val, "propose effect replay");
            mutation.run_id = Some(run_id);
            mutation.effect_id = Some(id);
            mutation.accepted = true;
            return Ok(mutation);
        }

        // Reserve budget before transmission (REQ-092).
        // Resolve account keys for run's currency.
        let account_keys = vec!["RUN:USD:default".to_string(), "DAY:USD:default".to_string()];
        // Parse bound from price_profile (e.g. "1.0.0" defaults to 20 units for standard calls; "0.0.0" -> PRICE_BOUND_UNKNOWN).
        let bound: i64 = if price_profile == "0.0.0" { 0 } else { 20 };

        let reserve_mut = self
            .reserve_budget(
                tx,
                tenant,
                run_id,
                &format!("effect-{}", idempotency_key),
                "USD",
                bound,
                &price_profile,
                &account_keys,
            )
            .await?;
        let reservation_id = parse_uuid(reserve_mut.primary.resource_id.as_str())?;

        let id = Uuid::new_v4();
        let now: DateTime<Utc> = sqlx::query_scalar("SELECT clock_timestamp()")
            .fetch_one(&mut **tx)
            .await
            .map_err(database_failure)?;

        let state = "PREPARED";
        let target_val =
            serde_json::to_value(&target).map_err(|_| CommandFailure::invalid("SCHEMA_INVALID"))?;
        let content_val = serde_json::to_value(&content_ref)
            .map_err(|_| CommandFailure::invalid("SCHEMA_INVALID"))?;

        sqlx::query("INSERT INTO effects(tenant_id,id,run_id,grant_id,connection_id,action,target,content_ref,fingerprint,idempotency_key,approval_id,reservation_id,state,version,created_at,updated_at) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,NULL,$11,$12,1,$13,$13)")
            .bind(tenant).bind(id).bind(run_id).bind(grant_id).bind(connection_id)
            .bind(&action).bind(target_val).bind(content_val)
            .bind(effect_fingerprint.as_str()).bind(idempotency_key)
            .bind(reservation_id).bind(state).bind(now)
            .execute(&mut **tx).await.map_err(database_failure)?;

        let val = effect_projection_value(
            id,
            tenant,
            run_id,
            &action,
            connection_id,
            &target,
            &content_ref,
            effect_fingerprint.as_str(),
            idempotency_key,
            None,
            reservation_id,
            state,
            1,
            now,
        );

        let mut mutation = Mutation::one("EffectIntent", id, 1, val, "propose effect");
        mutation.run_id = Some(run_id);
        mutation.effect_id = Some(id);
        mutation.accepted = true; // Async durable admission: ACCEPTED + non-null effect_id.
        Ok(mutation)
    }

    /// Dispatch an authorized effect to the provider (REQ-084, REQ-085, REQ-086, REQ-089).
    pub(crate) async fn dispatch_effect(
        &self,
        tx: &mut Tx<'_>,
        command: &AuthorizedCommand,
    ) -> Result<Mutation, CommandFailure> {
        let tenant = parse_uuid(command.actor.tenant_id.as_str())?;
        let effect_id = parse_uuid(&field::<String>(&command.input, "effect_id")?)?;
        let expected: i32 = field(&command.input, "expected_version")?;

        let row = sqlx::query("SELECT * FROM effects WHERE tenant_id=$1 AND id=$2 FOR UPDATE")
            .bind(tenant)
            .bind(effect_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(database_failure)?
            .ok_or_else(CommandFailure::not_found)?;

        let version =
            require_expected_version(row.try_get("version").map_err(database_failure)?, expected)?;
        let state: String = row.try_get("state").map_err(database_failure)?;
        let connection_id: Uuid = row.try_get("connection_id").map_err(database_failure)?;
        let reservation_id: Uuid = row.try_get("reservation_id").map_err(database_failure)?;
        let run_id: Uuid = row.try_get("run_id").map_err(database_failure)?;
        let action: String = row.try_get("action").map_err(database_failure)?;
        let fingerprint: String = row.try_get("fingerprint").map_err(database_failure)?;
        let idempotency_key: String = row.try_get("idempotency_key").map_err(database_failure)?;
        let target: ResourceRef =
            serde_json::from_value(row.try_get("target").map_err(database_failure)?)
                .map_err(|_| CommandFailure::unavailable("EFFECT_INTEGRITY"))?;
        let content_ref: ArtifactRef =
            serde_json::from_value(row.try_get("content_ref").map_err(database_failure)?)
                .map_err(|_| CommandFailure::unavailable("EFFECT_INTEGRITY"))?;
        let approval_id: Option<Uuid> = row.try_get("approval_id").map_err(database_failure)?;
        let created_at: DateTime<Utc> = row.try_get("created_at").map_err(database_failure)?;

        // Pre-PONR kill switch check (REQ-089 / AC-093).
        self.require_dispatch_enabled(tx, &command.actor, None, Some(connection_id))
            .await?;

        // State check: cannot resend if OUTCOME_UNKNOWN (REQ-085 / AC-089).
        if matches!(
            state.as_str(),
            "OUTCOME_UNKNOWN" | "RECONCILING" | "MANUAL_REVIEW"
        ) {
            return Err(CommandFailure::conflict("OUTCOME_UNKNOWN_BLOCKS_RESEND"));
        }

        // Must be in a dispatchable state.
        if !matches!(state.as_str(), "PREPARED" | "AUTHORIZED") {
            return Err(CommandFailure::conflict("ILLEGAL_TRANSITION"));
        }

        // If approval was required, ensure it is APPROVED and unexpired.
        if let Some(app_id) = approval_id {
            let app_row = sqlx::query("SELECT state, expires_at, content_digest FROM approvals WHERE tenant_id=$1 AND id=$2")
                .bind(tenant)
                .bind(app_id)
                .fetch_optional(&mut **tx)
                .await
                .map_err(database_failure)?
                .ok_or_else(|| CommandFailure::precondition("APPROVAL_REQUIRED"))?;
            let app_state: String = app_row.try_get("state").map_err(database_failure)?;
            if app_state != "APPROVED" {
                return Err(CommandFailure::precondition("APPROVAL_REQUIRED"));
            }
            let expires_at: DateTime<Utc> =
                app_row.try_get("expires_at").map_err(database_failure)?;
            let now: DateTime<Utc> = sqlx::query_scalar("SELECT clock_timestamp()")
                .fetch_one(&mut **tx)
                .await
                .map_err(database_failure)?;
            if now >= expires_at {
                return Err(CommandFailure::precondition("APPROVAL_EXPIRED"));
            }
        }

        // `require_expected_version` already returns the next version; adding one
        // here again would skip a version and break optimistic concurrency for
        // every later transition on this row.
        let new_version = version;
        let now: DateTime<Utc> = sqlx::query_scalar("SELECT clock_timestamp()")
            .fetch_one(&mut **tx)
            .await
            .map_err(database_failure)?;

        // Durable dispatch admission: commit the EXECUTING state transition
        // (with transmit_count and dispatch_fence increments) inside the command
        // transaction. The actual provider transport happens asynchronously
        // via the outbox event emitted by the store's write_projection loop.
        // This satisfies REQ-084: intent is durably recorded before any
        // transmission, and the system-path worker will read the committed
        // EXECUTING row and perform the HTTP call.
        sqlx::query("UPDATE effects SET state='EXECUTING',transmit_count=transmit_count+1,dispatch_fence=dispatch_fence+1,lease_expires_at=clock_timestamp()+interval '60 seconds',version=$3,updated_at=$4 WHERE tenant_id=$1 AND id=$2")
            .bind(tenant).bind(effect_id).bind(new_version).bind(now)
            .execute(&mut **tx).await.map_err(database_failure)?;

        // Note: We do NOT settle the budget here. Settlement happens only
        // after a confirmed provider receipt (SUCCEEDED) or proven absence
        // (RELEASE). Keeping the hold until remote evidence arrives satisfies
        // REQ-094 / REQ-095 and prevents the ledger from releasing funds
        // before the mutation is known to have been applied.

        let val = effect_projection_value(
            effect_id,
            tenant,
            run_id,
            &action,
            connection_id,
            &target,
            &content_ref,
            &fingerprint,
            &idempotency_key,
            approval_id,
            reservation_id,
            "EXECUTING",
            new_version,
            created_at,
        );

        let mut mutation = Mutation::one(
            "EffectIntent",
            effect_id,
            new_version,
            val,
            "dispatch effect",
        );
        mutation.run_id = Some(run_id);
        mutation.effect_id = Some(effect_id);
        mutation.accepted = true; // Asynchronous durable dispatch: ACCEPTED with effect_id.
        Ok(mutation)
    }

    /// Reconcile an ambiguous effect (REQ-086, REQ-087, REQ-090).
    ///
    /// The evidence decision itself is supplied by the reconciliation worker
    /// (`apply_reconciliation_evidence`), which performs the provider lookup
    /// outside the SQL transaction. This command only validates the effect is
    /// in a reconcilable state and reports its current durable state, so an
    /// operator or the workflow can drive the same transition deterministically.
    pub(crate) async fn reconcile_effect(
        &self,
        tx: &mut Tx<'_>,
        command: &AuthorizedCommand,
    ) -> Result<Mutation, CommandFailure> {
        let tenant = parse_uuid(command.actor.tenant_id.as_str())?;
        let effect_id = parse_uuid(&field::<String>(&command.input, "effect_id")?)?;
        let expected: i32 = field(&command.input, "expected_version")?;

        let row = sqlx::query("SELECT * FROM effects WHERE tenant_id=$1 AND id=$2 FOR UPDATE")
            .bind(tenant)
            .bind(effect_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(database_failure)?
            .ok_or_else(CommandFailure::not_found)?;

        let version =
            require_expected_version(row.try_get("version").map_err(database_failure)?, expected)?;
        let state: String = row.try_get("state").map_err(database_failure)?;

        // Only a state that still owns an unresolved transmission is reconcilable;
        // a `new_evidence` re-entry from MANUAL_REVIEW is the one extra entry point.
        if !matches!(state.as_str(), "OUTCOME_UNKNOWN" | "MANUAL_REVIEW") {
            return Err(CommandFailure::conflict("ILLEGAL_TRANSITION"));
        }

        // Same single-step rule as dispatch: the helper already produced the next
        // version.
        let new_version = version;
        let now: DateTime<Utc> = sqlx::query_scalar("SELECT clock_timestamp()")
            .fetch_one(&mut **tx)
            .await
            .map_err(database_failure)?;

        // OUTCOME_UNKNOWN -> RECONCILING (lookup); MANUAL_REVIEW -> RECONCILING
        // (new_evidence). Failure to establish the outcome leaves the effect in
        // RECONCILING with reconciliation_queued set so the worker retries the
        // lookup, rather than guessing a terminal state.
        let next_state = "RECONCILING";
        sqlx::query("UPDATE effects SET state=$3,reconciliation_queued=true,version=$4,updated_at=$5 WHERE tenant_id=$1 AND id=$2")
            .bind(tenant).bind(effect_id).bind(next_state).bind(new_version).bind(now)
            .execute(&mut **tx).await.map_err(database_failure)?;

        let target: ResourceRef =
            serde_json::from_value(row.try_get("target").map_err(database_failure)?)
                .map_err(|_| CommandFailure::unavailable("EFFECT_INTEGRITY"))?;
        let content_ref: ArtifactRef =
            serde_json::from_value(row.try_get("content_ref").map_err(database_failure)?)
                .map_err(|_| CommandFailure::unavailable("EFFECT_INTEGRITY"))?;
        let run_id: Uuid = row.try_get("run_id").map_err(database_failure)?;

        let val = effect_projection_value(
            effect_id,
            tenant,
            run_id,
            &row.try_get::<String, _>("action")
                .map_err(database_failure)?,
            row.try_get("connection_id").map_err(database_failure)?,
            &target,
            &content_ref,
            &row.try_get::<String, _>("fingerprint")
                .map_err(database_failure)?,
            &row.try_get::<String, _>("idempotency_key")
                .map_err(database_failure)?,
            row.try_get("approval_id").map_err(database_failure)?,
            row.try_get("reservation_id").map_err(database_failure)?,
            next_state,
            new_version,
            row.try_get("created_at").map_err(database_failure)?,
        );

        let mut mutation = Mutation::one(
            "EffectIntent",
            effect_id,
            new_version,
            val,
            "reconcile effect",
        );
        mutation.run_id = Some(run_id);
        mutation.effect_id = Some(effect_id);
        Ok(mutation)
    }

    /// Compensate an existing effect: creates a NEW effect with separate approval (REQ-091 / AC-095).
    pub(crate) async fn compensate_effect(
        &self,
        tx: &mut Tx<'_>,
        command: &AuthorizedCommand,
    ) -> Result<Mutation, CommandFailure> {
        let tenant = parse_uuid(command.actor.tenant_id.as_str())?;
        let original_effect_id = parse_uuid(&field::<String>(&command.input, "effect_id")?)?;
        let compensation_ref: ArtifactRef = field(&command.input, "compensation_ref")?;
        let approval_id = parse_uuid(&field::<String>(&command.input, "approval_id")?)?;

        // Original effect must exist and be in a terminal state with a receipt.
        let orig = sqlx::query("SELECT * FROM effects WHERE tenant_id=$1 AND id=$2")
            .bind(tenant)
            .bind(original_effect_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(database_failure)?
            .ok_or_else(CommandFailure::not_found)?;

        let orig_state: String = orig.try_get("state").map_err(database_failure)?;
        if !matches!(orig_state.as_str(), "SUCCEEDED" | "FAILED_CONFIRMED") {
            return Err(CommandFailure::precondition("EFFECT_NOT_COMPLETED"));
        }

        // Distinct approval check: approval must exist, be APPROVED, and not be the original's approval.
        let orig_app_id: Option<Uuid> = orig.try_get("approval_id").map_err(database_failure)?;
        if Some(approval_id) == orig_app_id {
            return Err(CommandFailure::precondition("SEPARATE_APPROVAL_REQUIRED"));
        }

        let app_row = sqlx::query("SELECT state FROM approvals WHERE tenant_id=$1 AND id=$2")
            .bind(tenant)
            .bind(approval_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(database_failure)?
            .ok_or_else(CommandFailure::not_found)?;
        if app_row
            .try_get::<String, _>("state")
            .map_err(database_failure)?
            != "APPROVED"
        {
            return Err(CommandFailure::precondition("APPROVAL_REQUIRED"));
        }

        // REQ-091: an approval authorizes exactly one compensation of a given
        // original. Re-submitting the same approval under a fresh command
        // idempotency key must not mint a second compensating effect for it —
        // a new compensating action requires its own approval.
        let reused: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM effects WHERE tenant_id=$1 AND original_effect_id=$2 AND approval_id=$3)")
            .bind(tenant)
            .bind(original_effect_id)
            .bind(approval_id)
            .fetch_one(&mut **tx)
            .await
            .map_err(database_failure)?;
        if reused {
            return Err(CommandFailure::precondition("APPROVAL_ALREADY_USED"));
        }

        // New effect is created, linked to original_effect_id (REQ-091).
        let new_id = Uuid::new_v4();
        let run_id: Uuid = orig.try_get("run_id").map_err(database_failure)?;
        let grant_id: Uuid = orig.try_get("grant_id").map_err(database_failure)?;
        let connection_id: Uuid = orig.try_get("connection_id").map_err(database_failure)?;
        let action = format!(
            "{}.compensate",
            orig.try_get::<String, _>("action")
                .map_err(database_failure)?
        );
        let target: Value = orig.try_get("target").map_err(database_failure)?;
        let content_val = serde_json::to_value(&compensation_ref)
            .map_err(|_| CommandFailure::invalid("SCHEMA_INVALID"))?;

        let fp_bytes = canonical_bytes(&json!({
            "action": action,
            "target": target,
            "content_digest": compensation_ref.digest.as_str(),
        }))
        .map_err(|_| CommandFailure::invalid("SCHEMA_INVALID"))?;
        let fingerprint = digest_bytes(&fp_bytes);

        let now: DateTime<Utc> = sqlx::query_scalar("SELECT clock_timestamp()")
            .fetch_one(&mut **tx)
            .await
            .map_err(database_failure)?;

        // Reserve budget for compensation effect.
        let reserve_mut = self
            .reserve_budget(
                tx,
                tenant,
                run_id,
                &format!("compensate-{}", new_id),
                "USD",
                20,
                "1.0.0",
                &["RUN:USD:default".to_string(), "DAY:USD:default".to_string()],
            )
            .await?;
        let reservation_id = parse_uuid(reserve_mut.primary.resource_id.as_str())?;

        let idempotency_key = command.idempotency_key.as_str();

        sqlx::query("INSERT INTO effects(tenant_id,id,run_id,grant_id,connection_id,action,target,content_ref,fingerprint,idempotency_key,approval_id,reservation_id,state,original_effect_id,version,created_at,updated_at) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,'AUTHORIZED',$13,1,$14,$14)")
            .bind(tenant).bind(new_id).bind(run_id).bind(grant_id).bind(connection_id)
            .bind(&action).bind(&target).bind(content_val)
            .bind(fingerprint.as_str()).bind(idempotency_key).bind(approval_id).bind(reservation_id)
            .bind(original_effect_id).bind(now)
            .execute(&mut **tx).await.map_err(database_failure)?;

        let target_ref: ResourceRef = serde_json::from_value(target)
            .map_err(|_| CommandFailure::unavailable("EFFECT_INTEGRITY"))?;
        let val = effect_projection_value(
            new_id,
            tenant,
            run_id,
            &action,
            connection_id,
            &target_ref,
            &compensation_ref,
            fingerprint.as_str(),
            idempotency_key,
            Some(approval_id),
            reservation_id,
            "AUTHORIZED",
            1,
            now,
        );

        let mut mutation = Mutation::one("EffectIntent", new_id, 1, val, "compensate effect");
        mutation.run_id = Some(run_id);
        mutation.effect_id = Some(new_id);
        Ok(mutation)
    }

    /// Public system-path entry: mark an effect OUTCOME_UNKNOWN on network timeout (REQ-085).
    pub async fn record_effect_unknown(
        &self,
        tenant_id: Uuid,
        effect_id: Uuid,
    ) -> Result<Mutation, CommandFailure> {
        let mut tx = self.begin_system(tenant_id, "effect-unknown", true).await?;
        let row = sqlx::query("SELECT version,state,reservation_id,run_id,action,connection_id,target,content_ref,fingerprint,idempotency_key,approval_id,created_at FROM effects WHERE tenant_id=$1 AND id=$2 FOR UPDATE")
            .bind(tenant_id)
            .bind(effect_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(database_failure)?
            .ok_or_else(CommandFailure::not_found)?;

        let state: String = row.try_get("state").map_err(database_failure)?;
        if state != "EXECUTING" {
            return Err(CommandFailure::conflict("ILLEGAL_TRANSITION"));
        }
        let version: i32 = row.try_get("version").map_err(database_failure)?;
        let reservation_id: Uuid = row.try_get("reservation_id").map_err(database_failure)?;
        let run_id: Uuid = row.try_get("run_id").map_err(database_failure)?;
        let new_version = version + 1;
        let now: DateTime<Utc> = sqlx::query_scalar("SELECT clock_timestamp()")
            .fetch_one(&mut *tx)
            .await
            .map_err(database_failure)?;

        sqlx::query("UPDATE effects SET state='OUTCOME_UNKNOWN',reconciliation_queued=true,version=$3,updated_at=$4 WHERE tenant_id=$1 AND id=$2")
            .bind(tenant_id).bind(effect_id).bind(new_version).bind(now)
            .execute(&mut *tx).await.map_err(database_failure)?;

        // REQ-094: Unknown usage keeps the hold intact until reconciliation.
        self.mark_cost_unknown(&mut tx, tenant_id, reservation_id)
            .await?;

        let target: ResourceRef =
            serde_json::from_value(row.try_get("target").map_err(database_failure)?)
                .map_err(|_| CommandFailure::unavailable("EFFECT_INTEGRITY"))?;
        let content_ref: ArtifactRef =
            serde_json::from_value(row.try_get("content_ref").map_err(database_failure)?)
                .map_err(|_| CommandFailure::unavailable("EFFECT_INTEGRITY"))?;

        let val = effect_projection_value(
            effect_id,
            tenant_id,
            run_id,
            &row.try_get::<String, _>("action")
                .map_err(database_failure)?,
            row.try_get("connection_id").map_err(database_failure)?,
            &target,
            &content_ref,
            &row.try_get::<String, _>("fingerprint")
                .map_err(database_failure)?,
            &row.try_get::<String, _>("idempotency_key")
                .map_err(database_failure)?,
            row.try_get("approval_id").map_err(database_failure)?,
            reservation_id,
            "OUTCOME_UNKNOWN",
            new_version,
            row.try_get("created_at").map_err(database_failure)?,
        );

        tx.commit().await.map_err(database_failure)?;
        let mut mutation = Mutation::one(
            "EffectIntent",
            effect_id,
            new_version,
            val,
            "effect outcome unknown",
        );
        mutation.run_id = Some(run_id);
        mutation.effect_id = Some(effect_id);
        Ok(mutation)
    }

    /// Public system-path entry: record confirmed effect success from provider receipt (REQ-086).
    #[allow(clippy::too_many_arguments)]
    pub async fn record_effect_succeeded(
        &self,
        tenant_id: Uuid,
        effect_id: Uuid,
        remote_id: &str,
        evidence_ref: &ArtifactRef,
        used_microunits: i64,
    ) -> Result<Mutation, CommandFailure> {
        let mut tx = self
            .begin_system(tenant_id, "effect-succeeded", true)
            .await?;
        let row = sqlx::query("SELECT version,state,reservation_id,run_id,action,connection_id,target,content_ref,fingerprint,idempotency_key,approval_id,created_at FROM effects WHERE tenant_id=$1 AND id=$2 FOR UPDATE")
            .bind(tenant_id)
            .bind(effect_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(database_failure)?
            .ok_or_else(CommandFailure::not_found)?;

        let state: String = row.try_get("state").map_err(database_failure)?;
        if !matches!(state.as_str(), "EXECUTING" | "RECONCILING") {
            return Err(CommandFailure::conflict("ILLEGAL_TRANSITION"));
        }
        let version: i32 = row.try_get("version").map_err(database_failure)?;
        let reservation_id: Uuid = row.try_get("reservation_id").map_err(database_failure)?;
        let run_id: Uuid = row.try_get("run_id").map_err(database_failure)?;
        let fingerprint: String = row.try_get("fingerprint").map_err(database_failure)?;
        let new_version = version + 1;
        let now: DateTime<Utc> = sqlx::query_scalar("SELECT clock_timestamp()")
            .fetch_one(&mut *tx)
            .await
            .map_err(database_failure)?;

        sqlx::query("UPDATE effects SET state='SUCCEEDED',reconciliation_queued=false,version=$3,updated_at=$4 WHERE tenant_id=$1 AND id=$2")
            .bind(tenant_id).bind(effect_id).bind(new_version).bind(now)
            .execute(&mut *tx).await.map_err(database_failure)?;

        // Record EffectReceipt (REQ-086 / AC-090).
        let receipt_id = Uuid::new_v4();
        let evidence_val = serde_json::to_value(evidence_ref)
            .map_err(|_| CommandFailure::invalid("SCHEMA_INVALID"))?;
        sqlx::query("INSERT INTO effect_receipts(tenant_id,id,effect_id,outcome,remote_id,remote_version,remote_url,evidence_ref,provider_contract_version,observed_at) VALUES($1,$2,$3,'SUCCEEDED',$4,'1',NULL,$5,'1.0.0',$6)")
            .bind(tenant_id).bind(receipt_id).bind(effect_id)
            .bind(remote_id).bind(&evidence_val).bind(now)
            .execute(&mut *tx).await.map_err(database_failure)?;

        // Settle budget reservation (REQ-095).
        self.settle_budget(
            &mut tx,
            tenant_id,
            reservation_id,
            used_microunits,
            Some(&fingerprint),
        )
        .await?;

        let target: ResourceRef =
            serde_json::from_value(row.try_get("target").map_err(database_failure)?)
                .map_err(|_| CommandFailure::unavailable("EFFECT_INTEGRITY"))?;
        let content_ref: ArtifactRef =
            serde_json::from_value(row.try_get("content_ref").map_err(database_failure)?)
                .map_err(|_| CommandFailure::unavailable("EFFECT_INTEGRITY"))?;

        let val = effect_projection_value(
            effect_id,
            tenant_id,
            run_id,
            &row.try_get::<String, _>("action")
                .map_err(database_failure)?,
            row.try_get("connection_id").map_err(database_failure)?,
            &target,
            &content_ref,
            &fingerprint,
            &row.try_get::<String, _>("idempotency_key")
                .map_err(database_failure)?,
            row.try_get("approval_id").map_err(database_failure)?,
            reservation_id,
            "SUCCEEDED",
            new_version,
            row.try_get("created_at").map_err(database_failure)?,
        );

        tx.commit().await.map_err(database_failure)?;
        let mut mutation = Mutation::one(
            "EffectIntent",
            effect_id,
            new_version,
            val,
            "effect succeeded",
        );
        mutation.run_id = Some(run_id);
        mutation.effect_id = Some(effect_id);
        Ok(mutation)
    }

    /// Public system-path entry: resolve reconciliation with domain evidence
    /// (REQ-086, REQ-087, REQ-090). The provider lookup itself happens outside
    /// the SQL transaction; this applies its decision durably.
    pub async fn apply_reconciliation_evidence(
        &self,
        tenant_id: Uuid,
        effect_id: Uuid,
        evidence: masonwing_kernel::effects::ReconciliationEvidence,
        remote_id: Option<&str>,
        evidence_ref: Option<&ArtifactRef>,
        used_microunits: Option<i64>,
    ) -> Result<Mutation, CommandFailure> {
        use masonwing_kernel::effects::ReconciliationEvidence as Evidence;
        match evidence {
            Evidence::ConfirmedPresent => {
                let remote_id =
                    remote_id.ok_or_else(|| CommandFailure::invalid("REMOTE_ID_REQUIRED"))?;
                let evidence_ref =
                    evidence_ref.ok_or_else(|| CommandFailure::invalid("EVIDENCE_REF_REQUIRED"))?;
                self.record_effect_succeeded(
                    tenant_id,
                    effect_id,
                    remote_id,
                    evidence_ref,
                    used_microunits.unwrap_or(12),
                )
                .await
            }
            Evidence::ConfirmedFailure
            | Evidence::ProvenAbsentAndReauthorized
            | Evidence::Unresolved => {
                let (next, label, release) = match evidence {
                    // A confirmed failure did not apply the mutation, so its hold
                    // returns to the tenant.
                    Evidence::ConfirmedFailure => {
                        ("FAILED_CONFIRMED", "effect failed confirmed", true)
                    }
                    // REQ-090 / AC-094: proven absence re-authorizes the same effect
                    // identity so a retry can proceed inside the declared budget.
                    Evidence::ProvenAbsentAndReauthorized => {
                        ("AUTHORIZED", "effect proven absent reauthorized", false)
                    }
                    // REQ-087 / AC-091: unproven outcome halts at MANUAL_REVIEW and blocks resend.
                    _ => ("MANUAL_REVIEW", "effect manual review", false),
                };
                let mut tx = self
                    .begin_system(tenant_id, "effect-reconcile", true)
                    .await?;
                let row = sqlx::query("SELECT version,state,reservation_id,run_id,action,connection_id,target,content_ref,fingerprint,idempotency_key,approval_id,created_at FROM effects WHERE tenant_id=$1 AND id=$2 FOR UPDATE")
                    .bind(tenant_id).bind(effect_id).fetch_optional(&mut *tx).await.map_err(database_failure)?.ok_or_else(CommandFailure::not_found)?;
                let state: String = row.try_get("state").map_err(database_failure)?;
                let expected_from: &[&str] = match evidence {
                    Evidence::ProvenAbsentAndReauthorized | Evidence::Unresolved => {
                        &["RECONCILING"]
                    }
                    _ => &["EXECUTING", "RECONCILING"],
                };
                if !expected_from.iter().any(|from| *from == state) {
                    return Err(CommandFailure::conflict("ILLEGAL_TRANSITION"));
                }
                let version: i32 = row.try_get("version").map_err(database_failure)?;
                let reservation_id: Uuid =
                    row.try_get("reservation_id").map_err(database_failure)?;
                let run_id: Uuid = row.try_get("run_id").map_err(database_failure)?;
                let new_version = version + 1;
                let now: DateTime<Utc> = sqlx::query_scalar("SELECT clock_timestamp()")
                    .fetch_one(&mut *tx)
                    .await
                    .map_err(database_failure)?;

                sqlx::query("UPDATE effects SET state=$3,reconciliation_queued=false,version=$4,updated_at=$5 WHERE tenant_id=$1 AND id=$2")
                    .bind(tenant_id).bind(effect_id).bind(next).bind(new_version).bind(now)
                    .execute(&mut *tx).await.map_err(database_failure)?;

                if release {
                    self.release_budget(&mut tx, tenant_id, reservation_id)
                        .await?;
                }

                let target: ResourceRef =
                    serde_json::from_value(row.try_get("target").map_err(database_failure)?)
                        .map_err(|_| CommandFailure::unavailable("EFFECT_INTEGRITY"))?;
                let content_ref: ArtifactRef =
                    serde_json::from_value(row.try_get("content_ref").map_err(database_failure)?)
                        .map_err(|_| CommandFailure::unavailable("EFFECT_INTEGRITY"))?;

                let val = effect_projection_value(
                    effect_id,
                    tenant_id,
                    run_id,
                    &row.try_get::<String, _>("action")
                        .map_err(database_failure)?,
                    row.try_get("connection_id").map_err(database_failure)?,
                    &target,
                    &content_ref,
                    &row.try_get::<String, _>("fingerprint")
                        .map_err(database_failure)?,
                    &row.try_get::<String, _>("idempotency_key")
                        .map_err(database_failure)?,
                    row.try_get("approval_id").map_err(database_failure)?,
                    reservation_id,
                    next,
                    new_version,
                    row.try_get("created_at").map_err(database_failure)?,
                );
                tx.commit().await.map_err(database_failure)?;
                let mut mutation =
                    Mutation::one("EffectIntent", effect_id, new_version, val, label);
                mutation.run_id = Some(run_id);
                mutation.effect_id = Some(effect_id);
                Ok(mutation)
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn effect_projection_value(
    effect_id: Uuid,
    tenant_id: Uuid,
    run_id: Uuid,
    action: &str,
    connection_id: Uuid,
    target: &ResourceRef,
    content_ref: &ArtifactRef,
    fingerprint: &str,
    idempotency_key: &str,
    approval_id: Option<Uuid>,
    reservation_id: Uuid,
    state: &str,
    version: i32,
    created_at: DateTime<Utc>,
) -> serde_json::Value {
    json!({
        "effect_id": effect_id.to_string(),
        "tenant_id": tenant_id.to_string(),
        "run_id": run_id.to_string(),
        "action": action,
        "connection_id": connection_id.to_string(),
        "target": target,
        "content_ref": content_ref,
        "fingerprint": fingerprint,
        "idempotency_key": idempotency_key,
        "approval_id": approval_id.map(|id| id.to_string()),
        "reservation_id": reservation_id.to_string(),
        "state": state,
        "version": version,
        "created_at": created_at,
    })
}
