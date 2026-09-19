//! Approval admission: one reviewer, one decision, one winner.
//! A decision never transmits anything; it only terminates the proposal
//! and lets the durable workflow observe the terminal state.

use chrono::Utc;
use masonwing_contracts::wire::{PrincipalKind, PrincipalRef};
use masonwing_kernel::runtime::{AuthorizedCommand, CommandFailure, require_expected_version};
use serde_json::json;
use sqlx::Row;
use uuid::Uuid;

use crate::store::{Mutation, PostgresStore, Tx, database_failure, field, parse_uuid};

impl PostgresStore {
    pub(crate) async fn decide_approval(
        &self,
        tx: &mut Tx<'_>,
        command: &AuthorizedCommand,
    ) -> Result<Mutation, CommandFailure> {
        let tenant = parse_uuid(command.actor.tenant_id.as_str())?;
        let principal = parse_uuid(command.actor.principal_id.as_str())?;
        let proposal = parse_uuid(&field::<String>(&command.input, "proposal_id")?)?;
        let decision: String = field(&command.input, "decision")?;
        let expected: i32 = field(&command.input, "expected_version")?;
        let content_digest: String = field(&command.input, "content_digest")?;
        let reason: String = field(&command.input, "reason")?;
        if reason.trim().is_empty() {
            return Err(CommandFailure::invalid("REASON_REQUIRED"));
        }
        // The row lock serializes concurrent decisions: the second transaction
        // observes the first one's terminal state and loses the race.
        let row = sqlx::query("SELECT * FROM approvals WHERE tenant_id=$1 AND id=$2 FOR UPDATE")
            .bind(tenant)
            .bind(proposal)
            .fetch_optional(&mut **tx)
            .await
            .map_err(database_failure)?
            .ok_or_else(CommandFailure::not_found)?;
        let state: String = row.try_get("state").map_err(database_failure)?;
        let next = match state.as_str() {
            "REQUESTED" => decision_state(&decision)?,
            // Terminal states keep their decision; a second decide is the
            // race loser, not a version conflict.
            "APPROVED" | "REJECTED" | "CANCELLED" => {
                return Err(CommandFailure::conflict("DECISION_CONFLICT"));
            }
            "EXPIRED" => return Err(CommandFailure::precondition("APPROVAL_EXPIRED")),
            "INVALIDATED" => return Err(CommandFailure::precondition("APPROVAL_STALE")),
            _ => return Err(CommandFailure::unavailable("APPROVAL_STATE_INVALID")),
        };
        // Four-eyes: the author of a proposal can never be its decider.
        let created_by: Uuid = row.try_get("created_by").map_err(database_failure)?;
        if created_by == principal {
            return Err(CommandFailure::denied("FOUR_EYES_REQUIRED"));
        }
        // A REQUESTED proposal past its expiry is closed, not decided.
        let expires_at = row
            .try_get::<chrono::DateTime<chrono::Utc>, _>("expires_at")
            .map_err(database_failure)?;
        let now: chrono::DateTime<Utc> = sqlx::query_scalar("SELECT clock_timestamp()")
            .fetch_one(&mut **tx)
            .await
            .map_err(database_failure)?;
        if now >= expires_at {
            return Err(CommandFailure::precondition("APPROVAL_EXPIRED"));
        }
        // The reviewer decides over exactly the previewed content; a different
        // digest means the proposal no longer matches what was reviewed.
        let bound_digest: String = row.try_get("content_digest").map_err(database_failure)?;
        if bound_digest != content_digest {
            return Err(CommandFailure::conflict("REVIEW_CONFLICT"));
        }
        let version =
            require_expected_version(row.try_get("version").map_err(database_failure)?, expected)?;
        // Conditional update: even under the row lock, only a still-REQUESTED
        // proposal accepts a decision. One winner; every loser conflicts.
        let updated = sqlx::query("UPDATE approvals SET state=$3,decided_by=$4,decided_issuer=$5,decision_reason=$6,version=$7,updated_at=clock_timestamp() WHERE tenant_id=$1 AND id=$2 AND state='REQUESTED' RETURNING action,target,content_digest,scope_digest,policy_version,expires_at,max_cost_microunits,currency,created_at,updated_at")
            .bind(tenant).bind(proposal).bind(next).bind(principal).bind(&command.actor.issuer)
            .bind(&reason).bind(version)
            .fetch_one(&mut **tx).await.map_err(database_failure)?;
        let target: serde_json::Value = updated.try_get("target").map_err(database_failure)?;
        let approved_by = PrincipalRef {
            kind: PrincipalKind::User,
            id: command.actor.principal_id.clone(),
            issuer: command.actor.issuer.clone(),
        };
        let mut value = json!({
            "proposal_id": proposal, "tenant_id": tenant,
            "action": updated.try_get::<String, _>("action").map_err(database_failure)?,
            "target": target,
            "content_digest": updated.try_get::<String, _>("content_digest").map_err(database_failure)?,
            "scope_digest": updated.try_get::<String, _>("scope_digest").map_err(database_failure)?,
            "policy_version": updated.try_get::<String, _>("policy_version").map_err(database_failure)?,
            "expires_at": updated.try_get::<chrono::DateTime<chrono::Utc>, _>("expires_at").map_err(database_failure)?,
            "max_cost_microunits": updated.try_get::<i64, _>("max_cost_microunits").map_err(database_failure)?,
            "currency": updated.try_get::<String, _>("currency").map_err(database_failure)?,
            "state": next,
            "decided_by": approved_by,
            "decision_reason": reason,
            "version": version,
            "created_at": updated.try_get::<chrono::DateTime<chrono::Utc>, _>("created_at").map_err(database_failure)?,
            "updated_at": updated.try_get::<chrono::DateTime<chrono::Utc>, _>("updated_at").map_err(database_failure)?,
        });
        if next == "APPROVED" {
            // The binding carries the decider that authorized the exact digest.
            value["approved_by"] = serde_json::to_value(&approved_by)
                .map_err(|_| CommandFailure::unavailable("APPROVAL_PROJECTION_INVALID"))?;
        } else {
            // A rejection or cancellation authorizes nothing, so the binding
            // records no approver rather than an approver of the wrong action.
            value["approved_by"] = serde_json::Value::Null;
        }
        let mut mutation = Mutation::one(
            "Approval",
            proposal,
            version,
            value,
            format!("approval {decision}"),
        );
        // The durable run that requested this decision resumes through the
        // outbox event recorded with this projection change.
        let run_id: Uuid = row.try_get("run_id").map_err(database_failure)?;
        mutation.run_id = Some(run_id);
        Ok(mutation)
    }
}

fn decision_state(decision: &str) -> Result<&'static str, CommandFailure> {
    match decision {
        "APPROVE" => Ok("APPROVED"),
        "REJECT" => Ok("REJECTED"),
        "CANCEL" => Ok("CANCELLED"),
        _ => Err(CommandFailure::invalid("DECISION_INVALID")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // MASONWING@1.0.1 REQ-078 / AC-082: the decision verb maps onto the
    // terminal state, never a blanket permission.
    #[test]
    fn decision_verbs_map_to_terminal_states() {
        assert_eq!(decision_state("APPROVE").unwrap(), "APPROVED");
        assert_eq!(decision_state("REJECT").unwrap(), "REJECTED");
        assert_eq!(decision_state("CANCEL").unwrap(), "CANCELLED");
        assert_eq!(
            decision_state("GRANT").unwrap_err().code,
            "DECISION_INVALID"
        );
    }
}
