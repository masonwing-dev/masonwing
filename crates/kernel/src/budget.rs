use thiserror::Error;

use crate::state::CostState;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CostReservation {
    upper_bound_microunits: u64,
    held_microunits: u64,
    charged_microunits: Option<u64>,
    state: CostState,
    reconciliation_queued: bool,
}

impl CostReservation {
    pub fn reserve(upper_bound_microunits: u64) -> Result<Self, BudgetError> {
        if upper_bound_microunits == 0 {
            return Err(BudgetError::PriceBoundUnknown);
        }
        Ok(Self {
            upper_bound_microunits,
            held_microunits: upper_bound_microunits,
            charged_microunits: None,
            state: CostState::Reserved,
            reconciliation_queued: false,
        })
    }

    pub fn mark_usage_unknown(&mut self) -> Result<(), BudgetError> {
        if self.state != CostState::Reserved {
            return Err(BudgetError::IllegalCostState);
        }
        self.state = CostState::Unknown;
        self.reconciliation_queued = true;
        Ok(())
    }

    pub fn settle_authoritative_usage(&mut self, used_microunits: u64) -> Result<(), BudgetError> {
        if self.state == CostState::Settled {
            return if self.charged_microunits == Some(used_microunits) {
                Ok(())
            } else {
                Err(BudgetError::SettlementConflict)
            };
        }
        if !matches!(self.state, CostState::Reserved | CostState::Unknown) {
            return Err(BudgetError::IllegalCostState);
        }
        if used_microunits > self.upper_bound_microunits {
            return Err(BudgetError::UsageExceedsReservation);
        }
        self.state = CostState::Settled;
        self.charged_microunits = Some(used_microunits);
        self.held_microunits = 0;
        self.reconciliation_queued = false;
        Ok(())
    }

    pub fn state(&self) -> CostState {
        self.state
    }

    pub fn held_microunits(&self) -> u64 {
        self.held_microunits
    }

    pub fn charged_microunits(&self) -> Option<u64> {
        self.charged_microunits
    }

    pub fn reconciliation_queued(&self) -> bool {
        self.reconciliation_queued
    }
}

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum BudgetError {
    #[error("PRICE_BOUND_UNKNOWN")]
    PriceBoundUnknown,
    #[error("ILLEGAL_COST_STATE")]
    IllegalCostState,
    #[error("SETTLEMENT_CONFLICT")]
    SettlementConflict,
    #[error("USAGE_EXCEEDS_RESERVATION")]
    UsageExceedsReservation,
}

#[cfg(test)]
mod tests {
    use super::*;

    // MASONWING@1.0.1 REQ-094 / AC-098 / TC-AC-098.
    #[test]
    fn unknown_usage_retains_full_reservation_until_reconciled() {
        let mut reservation = CostReservation::reserve(20).unwrap();
        reservation.mark_usage_unknown().unwrap();

        assert_eq!(reservation.state(), CostState::Unknown);
        assert_eq!(reservation.held_microunits(), 20);
        assert_eq!(reservation.charged_microunits(), None);
        assert!(reservation.reconciliation_queued());
    }

    // MASONWING@1.0.1 REQ-095 / AC-099.
    #[test]
    fn authoritative_settlement_is_idempotent_for_same_usage() {
        let mut reservation = CostReservation::reserve(20).unwrap();
        reservation.settle_authoritative_usage(12).unwrap();
        reservation.settle_authoritative_usage(12).unwrap();
        assert_eq!(reservation.charged_microunits(), Some(12));
        assert_eq!(reservation.held_microunits(), 0);
    }
}
