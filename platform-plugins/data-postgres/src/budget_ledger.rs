//! Budget ledger: reserve/settle/unknown/release with atomic budget checks.
//!
//! REQ-092..098 / AC-096..102: reservation happens before dispatch (REQ-092),
//! concurrent excess is rejected atomically by the `budget_accounts` CHECK
//! constraint — no select-then-check (REQ-093), unknown usage keeps the hold
//! (REQ-094), settlement is once-only and idempotent (REQ-095). The caller
//! (effect admission) resolves the price profile into `upper_bound_microunits`;
//! an unresolvable profile never reaches this ledger (REQ-096). BYOK and other
//! credentials ride the same reservation path, so platform quotas still apply
//! (REQ-098). Tenant execution fairness is a worker scheduling concern
//! (per-tenant concurrency cap), not ledger state (REQ-097).

use chrono::{DateTime, Utc};
use masonwing_kernel::runtime::CommandFailure;
use masonwing_kernel::state::MachineState;
use serde_json::json;
use sqlx::Row;
use uuid::Uuid;

use crate::store::{Mutation, PostgresStore, Tx, database_failure};

impl PostgresStore {
    /// Public system-path entry: reserve on behalf of a run's admission.
    /// Used by the effect proposer (WP-013) and the run interpreter, which act
    /// as the platform on a run that was already authorized at admission.
    #[allow(clippy::too_many_arguments)]
    pub async fn reserve_run_budget(
        &self,
        tenant_id: Uuid,
        run_id: Uuid,
        logical_call_id: &str,
        currency: &str,
        upper_bound_microunits: i64,
        price_profile: &str,
        account_keys: &[String],
    ) -> Result<Mutation, CommandFailure> {
        let mut tx = self.begin_system(tenant_id, "budget-reserve", true).await?;
        let mutation = self
            .reserve_budget(
                &mut tx,
                tenant_id,
                run_id,
                logical_call_id,
                currency,
                upper_bound_microunits,
                price_profile,
                account_keys,
            )
            .await?;
        tx.commit().await.map_err(database_failure)?;
        Ok(mutation)
    }

    /// Public system-path entry: settle authoritative usage from a receipt.
    pub async fn settle_run_budget(
        &self,
        tenant_id: Uuid,
        reservation_id: Uuid,
        used_microunits: i64,
        usage_fingerprint: Option<&str>,
    ) -> Result<Mutation, CommandFailure> {
        let mut tx = self.begin_system(tenant_id, "budget-settle", true).await?;
        let mutation = self
            .settle_budget(
                &mut tx,
                tenant_id,
                reservation_id,
                used_microunits,
                usage_fingerprint,
            )
            .await?;
        tx.commit().await.map_err(database_failure)?;
        Ok(mutation)
    }

    /// Public system-path entry: mark usage unknown after an ambiguous outcome.
    pub async fn mark_run_cost_unknown(
        &self,
        tenant_id: Uuid,
        reservation_id: Uuid,
    ) -> Result<Mutation, CommandFailure> {
        let mut tx = self.begin_system(tenant_id, "budget-unknown", true).await?;
        let mutation = self
            .mark_cost_unknown(&mut tx, tenant_id, reservation_id)
            .await?;
        tx.commit().await.map_err(database_failure)?;
        Ok(mutation)
    }

    /// Public system-path entry: release a hold (proven absent / terminal run).
    pub async fn release_run_budget(
        &self,
        tenant_id: Uuid,
        reservation_id: Uuid,
    ) -> Result<Mutation, CommandFailure> {
        let mut tx = self.begin_system(tenant_id, "budget-release", true).await?;
        let mutation = self
            .release_budget(&mut tx, tenant_id, reservation_id)
            .await?;
        tx.commit().await.map_err(database_failure)?;
        Ok(mutation)
    }

    /// Reserve the bounded maximum cost of one billable call (REQ-092).
    ///
    /// Charges every account key by incrementing `held_microunits`; the
    /// `CHECK (held_microunits <= limit_microunits - charged_microunits)`
    /// constraint is the atomic arbiter — a 23514 violation maps to
    /// BUDGET_EXCEEDED (REQ-093). Records the RESERVED reservation and its
    /// cost_ledger event in the same transaction as the hold.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn reserve_budget(
        &self,
        tx: &mut Tx<'_>,
        tenant: Uuid,
        run_id: Uuid,
        logical_call_id: &str,
        currency: &str,
        upper_bound_microunits: i64,
        price_profile: &str,
        account_keys: &[String],
    ) -> Result<Mutation, CommandFailure> {
        // REQ-096: an operation without a bounded price is never admitted; the
        // caller passes 0 (or a non-positive resolved bound) to be refused here.
        if upper_bound_microunits <= 0 {
            return Err(CommandFailure::precondition("PRICE_BOUND_UNKNOWN"));
        }
        let id = Uuid::new_v4();
        // Each charged account must exist with a positive limit: a dimension
        // the tenant never configured cannot hold a reservation.
        for key in account_keys {
            let updated = sqlx::query("UPDATE budget_accounts SET held_microunits=held_microunits+$3 WHERE tenant_id=$1 AND account_key=$2")
                .bind(tenant).bind(key).bind(upper_bound_microunits)
                .execute(&mut **tx).await;
            match updated {
                Ok(result) if result.rows_affected() == 1 => {}
                Ok(_) => {
                    // Unknown account: seed it from the period's configured
                    // limit (the key prefix IS the period) or refuse.
                    let period = key.split(':').next().unwrap_or_default();
                    let limit: Option<i64> = sqlx::query_scalar("SELECT limit_microunits FROM budget_settings WHERE tenant_id=$1 AND currency=$2 AND period=$3")
                        .bind(tenant).bind(currency).bind(period)
                        .fetch_optional(&mut **tx).await.map_err(database_failure)?;
                    let limit = limit
                        .filter(|limit| *limit > 0)
                        .ok_or_else(|| CommandFailure::precondition("BUDGET_NOT_CONFIGURED"))?;
                    sqlx::query("INSERT INTO budget_accounts(tenant_id,account_key,currency,limit_microunits,held_microunits) VALUES($1,$2,$3,$4,$5)")
                        .bind(tenant).bind(key).bind(currency).bind(limit)
                        .bind(upper_bound_microunits)
                        .execute(&mut **tx).await.map_err(|error| {
                            budget_constraint_failure(error)
                        })?;
                }
                Err(error) => return Err(budget_constraint_failure(error)),
            }
        }
        let now: DateTime<Utc> = sqlx::query_scalar("SELECT clock_timestamp()")
            .fetch_one(&mut **tx)
            .await
            .map_err(database_failure)?;
        sqlx::query("INSERT INTO cost_reservations(tenant_id,id,run_id,logical_call_id,currency,upper_bound_microunits,state,price_profile,account_keys,version,created_at) VALUES($1,$2,$3,$4,$5,$6,'RESERVED',$7,$8,1,$9)")
            .bind(tenant).bind(id).bind(run_id).bind(logical_call_id)
            .bind(currency).bind(upper_bound_microunits).bind(price_profile)
            .bind(account_keys).bind(now)
            .execute(&mut **tx).await.map_err(database_failure)?;
        insert_cost_event(
            tx,
            tenant,
            id,
            "RESERVED",
            upper_bound_microunits,
            0,
            None,
            now,
        )
        .await?;
        let value = reservation_value(
            id,
            tenant,
            run_id,
            currency,
            upper_bound_microunits,
            None,
            "RESERVED",
            price_profile,
            1,
            account_keys,
        );
        Ok(Mutation::one(
            "CostReservation",
            id,
            1,
            value,
            "reserve budget",
        ))
    }

    /// Settle authoritative usage exactly once (REQ-095).
    ///
    /// Releases the whole hold, charges the used amount, and stamps the
    /// reservation SETTLED. Same-usage replay is a no-op returning the settled
    /// projection; different usage on a settled reservation is
    /// SETTLEMENT_CONFLICT; usage beyond the bound is refused before any
    /// account moves.
    pub(crate) async fn settle_budget(
        &self,
        tx: &mut Tx<'_>,
        tenant: Uuid,
        reservation_id: Uuid,
        used_microunits: i64,
        usage_fingerprint: Option<&str>,
    ) -> Result<Mutation, CommandFailure> {
        let row = locked_reservation(tx, tenant, reservation_id).await?;
        let state: String = row.try_get("state").map_err(database_failure)?;
        let upper_bound: i64 = row
            .try_get("upper_bound_microunits")
            .map_err(database_failure)?;
        let version: i32 = row.try_get("version").map_err(database_failure)?;
        let account_keys: Vec<String> = row.try_get("account_keys").map_err(database_failure)?;
        let run_id: Uuid = row.try_get("run_id").map_err(database_failure)?;
        let currency: String = row.try_get("currency").map_err(database_failure)?;
        let price_profile: String = row.try_get("price_profile").map_err(database_failure)?;
        if state == "SETTLED" {
            let settled: i64 = row
                .try_get("settled_microunits")
                .map_err(database_failure)?;
            if settled == used_microunits {
                // Idempotent replay: the first receipt already charged; this
                // one must not subtract again (AC-099).
                let value = reservation_value(
                    reservation_id,
                    tenant,
                    run_id,
                    &currency,
                    upper_bound,
                    Some(used_microunits),
                    "SETTLED",
                    &price_profile,
                    version,
                    &account_keys,
                );
                return Ok(Mutation::one(
                    "CostReservation",
                    reservation_id,
                    version,
                    value,
                    "settle budget",
                ));
            }
            return Err(CommandFailure::conflict("SETTLEMENT_CONFLICT"));
        }
        require_cost_transition(&state, "SETTLED")?;
        if used_microunits < 0 || used_microunits > upper_bound {
            return Err(CommandFailure::precondition("USAGE_EXCEEDS_RESERVATION"));
        }
        for key in &account_keys {
            sqlx::query("UPDATE budget_accounts SET held_microunits=held_microunits-$3,charged_microunits=charged_microunits+$4 WHERE tenant_id=$1 AND account_key=$2")
                .bind(tenant).bind(key).bind(upper_bound).bind(used_microunits)
                .execute(&mut **tx).await.map_err(database_failure)?;
        }
        let now: DateTime<Utc> = sqlx::query_scalar("SELECT clock_timestamp()")
            .fetch_one(&mut **tx)
            .await
            .map_err(database_failure)?;
        let new_version = version + 1;
        sqlx::query("UPDATE cost_reservations SET state='SETTLED',settled_microunits=$3,usage_fingerprint=$4,version=$5 WHERE tenant_id=$1 AND id=$2")
            .bind(tenant).bind(reservation_id).bind(used_microunits)
            .bind(usage_fingerprint).bind(new_version)
            .execute(&mut **tx).await.map_err(database_failure)?;
        insert_cost_event(
            tx,
            tenant,
            reservation_id,
            "SETTLED",
            -upper_bound,
            used_microunits,
            usage_fingerprint,
            now,
        )
        .await?;
        let value = reservation_value(
            reservation_id,
            tenant,
            run_id,
            &currency,
            upper_bound,
            Some(used_microunits),
            "SETTLED",
            &price_profile,
            new_version,
            &account_keys,
        );
        Ok(Mutation::one(
            "CostReservation",
            reservation_id,
            new_version,
            value,
            "settle budget",
        ))
    }

    /// Keep the hold when usage is unknown (REQ-094 / AC-098).
    ///
    /// An ambiguous provider outcome never settles to zero and never frees the
    /// bound: the reservation moves to UNKNOWN with the hold intact so
    /// reconciliation owns the final state.
    pub(crate) async fn mark_cost_unknown(
        &self,
        tx: &mut Tx<'_>,
        tenant: Uuid,
        reservation_id: Uuid,
    ) -> Result<Mutation, CommandFailure> {
        let row = locked_reservation(tx, tenant, reservation_id).await?;
        let state: String = row.try_get("state").map_err(database_failure)?;
        require_cost_transition(&state, "UNKNOWN")?;
        let upper_bound: i64 = row
            .try_get("upper_bound_microunits")
            .map_err(database_failure)?;
        let version: i32 = row.try_get("version").map_err(database_failure)?;
        let account_keys: Vec<String> = row.try_get("account_keys").map_err(database_failure)?;
        let run_id: Uuid = row.try_get("run_id").map_err(database_failure)?;
        let currency: String = row.try_get("currency").map_err(database_failure)?;
        let price_profile: String = row.try_get("price_profile").map_err(database_failure)?;
        let now: DateTime<Utc> = sqlx::query_scalar("SELECT clock_timestamp()")
            .fetch_one(&mut **tx)
            .await
            .map_err(database_failure)?;
        let new_version = version + 1;
        sqlx::query(
            "UPDATE cost_reservations SET state='UNKNOWN',version=$3 WHERE tenant_id=$1 AND id=$2",
        )
        .bind(tenant)
        .bind(reservation_id)
        .bind(new_version)
        .execute(&mut **tx)
        .await
        .map_err(database_failure)?;
        insert_cost_event(tx, tenant, reservation_id, "UNKNOWN", 0, 0, None, now).await?;
        let value = reservation_value(
            reservation_id,
            tenant,
            run_id,
            &currency,
            upper_bound,
            None,
            "UNKNOWN",
            &price_profile,
            new_version,
            &account_keys,
        );
        Ok(Mutation::one(
            "CostReservation",
            reservation_id,
            new_version,
            value,
            "mark budget unknown",
        ))
    }

    /// Release a hold back to the tenant (proven absent / run terminal).
    ///
    /// RESERVED and UNKNOWN reservations release the full bound; a settled
    /// reservation has no hold left, so releasing it is an illegal state.
    pub(crate) async fn release_budget(
        &self,
        tx: &mut Tx<'_>,
        tenant: Uuid,
        reservation_id: Uuid,
    ) -> Result<Mutation, CommandFailure> {
        let row = locked_reservation(tx, tenant, reservation_id).await?;
        let state: String = row.try_get("state").map_err(database_failure)?;
        require_cost_transition(&state, "RELEASED")?;
        let upper_bound: i64 = row
            .try_get("upper_bound_microunits")
            .map_err(database_failure)?;
        let version: i32 = row.try_get("version").map_err(database_failure)?;
        let account_keys: Vec<String> = row.try_get("account_keys").map_err(database_failure)?;
        let run_id: Uuid = row.try_get("run_id").map_err(database_failure)?;
        let currency: String = row.try_get("currency").map_err(database_failure)?;
        let price_profile: String = row.try_get("price_profile").map_err(database_failure)?;
        for key in &account_keys {
            sqlx::query("UPDATE budget_accounts SET held_microunits=held_microunits-$3 WHERE tenant_id=$1 AND account_key=$2")
                .bind(tenant).bind(key).bind(upper_bound)
                .execute(&mut **tx).await.map_err(database_failure)?;
        }
        let now: DateTime<Utc> = sqlx::query_scalar("SELECT clock_timestamp()")
            .fetch_one(&mut **tx)
            .await
            .map_err(database_failure)?;
        let new_version = version + 1;
        sqlx::query(
            "UPDATE cost_reservations SET state='RELEASED',version=$3 WHERE tenant_id=$1 AND id=$2",
        )
        .bind(tenant)
        .bind(reservation_id)
        .bind(new_version)
        .execute(&mut **tx)
        .await
        .map_err(database_failure)?;
        insert_cost_event(
            tx,
            tenant,
            reservation_id,
            "RELEASED",
            -upper_bound,
            0,
            None,
            now,
        )
        .await?;
        let value = reservation_value(
            reservation_id,
            tenant,
            run_id,
            &currency,
            upper_bound,
            None,
            "RELEASED",
            &price_profile,
            new_version,
            &account_keys,
        );
        Ok(Mutation::one(
            "CostReservation",
            reservation_id,
            new_version,
            value,
            "release budget",
        ))
    }
}

async fn locked_reservation(
    tx: &mut Tx<'_>,
    tenant: Uuid,
    reservation_id: Uuid,
) -> Result<sqlx::postgres::PgRow, CommandFailure> {
    sqlx::query("SELECT * FROM cost_reservations WHERE tenant_id=$1 AND id=$2 FOR UPDATE")
        .bind(tenant)
        .bind(reservation_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(database_failure)?
        .ok_or_else(CommandFailure::not_found)
}

/// Every cost transition passes the kernel state machine; unknown mappings are
/// adapter integrity failures, never silent acceptances.
fn require_cost_transition(state: &str, next: &str) -> Result<(), CommandFailure> {
    let from = MachineState::parse("cost", state)
        .map_err(|_| CommandFailure::unavailable("COST_INTEGRITY"))?;
    let to = MachineState::parse("cost", next)
        .map_err(|_| CommandFailure::unavailable("COST_INTEGRITY"))?;
    if !from.can_transition_to(to) {
        return Err(CommandFailure::conflict("ILLEGAL_TRANSITION"));
    }
    Ok(())
}

/// The ledger CHECK is the arbiter: only its violation means the budget is
/// exhausted; every other database error stays an adapter failure.
fn budget_constraint_failure(error: sqlx::Error) -> CommandFailure {
    if error.as_database_error().and_then(|e| e.code()).as_deref() == Some("23514") {
        return CommandFailure::conflict("BUDGET_EXCEEDED");
    }
    database_failure(error)
}

#[allow(clippy::too_many_arguments)]
async fn insert_cost_event(
    tx: &mut Tx<'_>,
    tenant: Uuid,
    reservation_id: Uuid,
    event_type: &str,
    held_delta: i64,
    charged_delta: i64,
    usage_fingerprint: Option<&str>,
    created_at: DateTime<Utc>,
) -> Result<(), CommandFailure> {
    sqlx::query("INSERT INTO cost_ledger(tenant_id,id,reservation_id,event_type,held_delta,charged_delta,usage_fingerprint,created_at) VALUES($1,$2,$3,$4,$5,$6,$7,$8)")
        .bind(tenant).bind(Uuid::new_v4()).bind(reservation_id)
        .bind(event_type).bind(held_delta).bind(charged_delta)
        .bind(usage_fingerprint).bind(created_at)
        .execute(&mut **tx).await.map_err(database_failure)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn reservation_value(
    reservation_id: Uuid,
    tenant: Uuid,
    run_id: Uuid,
    currency: &str,
    upper_bound: i64,
    settled: Option<i64>,
    state: &str,
    price_profile: &str,
    version: i32,
    // Not part of the wire CostReservation contract (additionalProperties:
    // false); kept on the DB row so settle/release can refund every charged
    // account.
    _account_keys: &[String],
) -> serde_json::Value {
    json!({
        "reservation_id": reservation_id.to_string(),
        "tenant_id": tenant.to_string(),
        "run_id": run_id.to_string(),
        "currency": currency,
        "upper_bound_microunits": upper_bound,
        "settled_microunits": settled,
        "state": state,
        "price_profile": price_profile,
        "version": version,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // MASONWING@1.0.1 REQ-093 / AC-097: every legal cost transition passes the
    // kernel edge table; RESERVED->RESERVED and SETTLED->anything are refused.
    #[test]
    fn cost_transitions_follow_kernel_edges() {
        assert!(require_cost_transition("RESERVED", "SETTLED").is_ok());
        assert!(require_cost_transition("RESERVED", "UNKNOWN").is_ok());
        assert!(require_cost_transition("RESERVED", "RELEASED").is_ok());
        assert!(require_cost_transition("UNKNOWN", "SETTLED").is_ok());
        assert!(require_cost_transition("UNKNOWN", "RELEASED").is_ok());
        assert_eq!(
            require_cost_transition("RESERVED", "RESERVED")
                .unwrap_err()
                .code,
            "ILLEGAL_TRANSITION"
        );
        assert_eq!(
            require_cost_transition("SETTLED", "RELEASED")
                .unwrap_err()
                .code,
            "ILLEGAL_TRANSITION"
        );
        assert_eq!(
            require_cost_transition("RELEASED", "SETTLED")
                .unwrap_err()
                .code,
            "ILLEGAL_TRANSITION"
        );
    }
}
