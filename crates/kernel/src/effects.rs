use thiserror::Error;

use crate::state::EffectState;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectRecord {
    state: EffectState,
    transmit_count: u32,
    reconciliation_queued: bool,
}

impl EffectRecord {
    pub fn authorized() -> Self {
        Self {
            state: EffectState::Authorized,
            transmit_count: 0,
            reconciliation_queued: false,
        }
    }

    pub fn transmit(&mut self) -> Result<(), EffectError> {
        if self.state != EffectState::Authorized {
            return Err(match self.state {
                EffectState::OutcomeUnknown
                | EffectState::Reconciling
                | EffectState::ManualReview => EffectError::OutcomeUnknownBlocksResend,
                _ => EffectError::InvalidEffectState,
            });
        }
        self.state = EffectState::Executing;
        self.transmit_count += 1;
        Ok(())
    }

    pub fn mark_ambiguous_timeout(&mut self) -> Result<(), EffectError> {
        if self.state != EffectState::Executing {
            return Err(EffectError::InvalidEffectState);
        }
        self.state = EffectState::OutcomeUnknown;
        self.reconciliation_queued = true;
        Ok(())
    }

    pub fn begin_reconciliation(&mut self) -> Result<(), EffectError> {
        if !matches!(
            self.state,
            EffectState::OutcomeUnknown | EffectState::ManualReview
        ) {
            return Err(EffectError::InvalidEffectState);
        }
        self.state = EffectState::Reconciling;
        self.reconciliation_queued = false;
        Ok(())
    }

    pub fn reconcile(&mut self, evidence: ReconciliationEvidence) -> Result<(), EffectError> {
        if self.state != EffectState::Reconciling {
            return Err(EffectError::InvalidEffectState);
        }
        self.state = match evidence {
            ReconciliationEvidence::ConfirmedPresent => EffectState::Succeeded,
            ReconciliationEvidence::ConfirmedFailure => EffectState::FailedConfirmed,
            ReconciliationEvidence::ProvenAbsentAndReauthorized => EffectState::Authorized,
            ReconciliationEvidence::Unresolved => EffectState::ManualReview,
        };
        Ok(())
    }

    pub fn state(&self) -> EffectState {
        self.state
    }

    pub fn transmit_count(&self) -> u32 {
        self.transmit_count
    }

    pub fn reconciliation_queued(&self) -> bool {
        self.reconciliation_queued
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReconciliationEvidence {
    ConfirmedPresent,
    ConfirmedFailure,
    ProvenAbsentAndReauthorized,
    Unresolved,
}

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum EffectError {
    #[error("OUTCOME_UNKNOWN_BLOCKS_RESEND")]
    OutcomeUnknownBlocksResend,
    #[error("ILLEGAL_EFFECT_STATE")]
    InvalidEffectState,
}

#[cfg(test)]
mod tests {
    use super::*;

    // MASONWING@1.0.1 REQ-085 / AC-089 / TC-AC-089.
    #[test]
    fn ambiguous_timeout_queues_reconciliation_and_never_blind_resends() {
        let mut effect = EffectRecord::authorized();
        effect.transmit().unwrap();
        effect.mark_ambiguous_timeout().unwrap();

        assert_eq!(effect.state(), EffectState::OutcomeUnknown);
        assert_eq!(effect.transmit_count(), 1);
        assert!(effect.reconciliation_queued());
        assert_eq!(
            effect.transmit(),
            Err(EffectError::OutcomeUnknownBlocksResend)
        );
        assert_eq!(effect.transmit_count(), 1);
    }

    // MASONWING@1.0.1 REQ-087 / AC-091 / TC-AC-091.
    #[test]
    fn unresolved_reconciliation_moves_to_manual_review_without_resend() {
        let mut effect = EffectRecord::authorized();
        effect.transmit().unwrap();
        effect.mark_ambiguous_timeout().unwrap();
        effect.begin_reconciliation().unwrap();
        effect
            .reconcile(ReconciliationEvidence::Unresolved)
            .unwrap();

        assert_eq!(effect.state(), EffectState::ManualReview);
        assert_eq!(
            effect.transmit(),
            Err(EffectError::OutcomeUnknownBlocksResend)
        );
        assert_eq!(effect.transmit_count(), 1);
    }
}
