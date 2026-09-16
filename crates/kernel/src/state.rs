use thiserror::Error;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PluginState {
    Discovered,
    Verified,
    Resolved,
    Staged,
    InstalledDisabled,
    Enabled,
    Draining,
    Disabled,
    Uninstalled,
    Quarantined,
    Revoked,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunState {
    Queued,
    Running,
    WaitingApproval,
    WaitingRetry,
    Blocked,
    CancelRequested,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApprovalState {
    Pending,
    Approved,
    Rejected,
    Cancelled,
    Expired,
    Stale,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffectState {
    Prepared,
    WaitingApproval,
    Authorized,
    Executing,
    Succeeded,
    FailedConfirmed,
    OutcomeUnknown,
    Reconciling,
    ManualReview,
    Cancelled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CostState {
    Reserved,
    Settled,
    Unknown,
    Released,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MachineState {
    Plugin(PluginState),
    Run(RunState),
    Approval(ApprovalState),
    Effect(EffectState),
    Cost(CostState),
}

impl MachineState {
    pub fn parse(machine: &str, state: &str) -> Result<Self, StateError> {
        match (machine, state) {
            ("plugin", "DISCOVERED") => Ok(Self::Plugin(PluginState::Discovered)),
            ("plugin", "VERIFIED") => Ok(Self::Plugin(PluginState::Verified)),
            ("plugin", "RESOLVED") => Ok(Self::Plugin(PluginState::Resolved)),
            ("plugin", "STAGED") => Ok(Self::Plugin(PluginState::Staged)),
            ("plugin", "INSTALLED_DISABLED") => Ok(Self::Plugin(PluginState::InstalledDisabled)),
            ("plugin", "ENABLED") => Ok(Self::Plugin(PluginState::Enabled)),
            ("plugin", "DRAINING") => Ok(Self::Plugin(PluginState::Draining)),
            ("plugin", "DISABLED") => Ok(Self::Plugin(PluginState::Disabled)),
            ("plugin", "UNINSTALLED") => Ok(Self::Plugin(PluginState::Uninstalled)),
            ("plugin", "QUARANTINED") => Ok(Self::Plugin(PluginState::Quarantined)),
            ("plugin", "REVOKED") => Ok(Self::Plugin(PluginState::Revoked)),
            ("run", "QUEUED") => Ok(Self::Run(RunState::Queued)),
            ("run", "RUNNING") => Ok(Self::Run(RunState::Running)),
            ("run", "WAITING_APPROVAL") => Ok(Self::Run(RunState::WaitingApproval)),
            ("run", "WAITING_RETRY") => Ok(Self::Run(RunState::WaitingRetry)),
            ("run", "BLOCKED") => Ok(Self::Run(RunState::Blocked)),
            ("run", "CANCEL_REQUESTED") => Ok(Self::Run(RunState::CancelRequested)),
            ("run", "SUCCEEDED") => Ok(Self::Run(RunState::Succeeded)),
            ("run", "FAILED") => Ok(Self::Run(RunState::Failed)),
            ("run", "CANCELLED") => Ok(Self::Run(RunState::Cancelled)),
            ("approval", "PENDING") => Ok(Self::Approval(ApprovalState::Pending)),
            ("approval", "APPROVED") => Ok(Self::Approval(ApprovalState::Approved)),
            ("approval", "REJECTED") => Ok(Self::Approval(ApprovalState::Rejected)),
            ("approval", "CANCELLED") => Ok(Self::Approval(ApprovalState::Cancelled)),
            ("approval", "EXPIRED") => Ok(Self::Approval(ApprovalState::Expired)),
            ("approval", "STALE") => Ok(Self::Approval(ApprovalState::Stale)),
            ("effect", "PREPARED") => Ok(Self::Effect(EffectState::Prepared)),
            ("effect", "WAITING_APPROVAL") => Ok(Self::Effect(EffectState::WaitingApproval)),
            ("effect", "AUTHORIZED") => Ok(Self::Effect(EffectState::Authorized)),
            ("effect", "EXECUTING") => Ok(Self::Effect(EffectState::Executing)),
            ("effect", "SUCCEEDED") => Ok(Self::Effect(EffectState::Succeeded)),
            ("effect", "FAILED_CONFIRMED") => Ok(Self::Effect(EffectState::FailedConfirmed)),
            ("effect", "OUTCOME_UNKNOWN") => Ok(Self::Effect(EffectState::OutcomeUnknown)),
            ("effect", "RECONCILING") => Ok(Self::Effect(EffectState::Reconciling)),
            ("effect", "MANUAL_REVIEW") => Ok(Self::Effect(EffectState::ManualReview)),
            ("effect", "CANCELLED") => Ok(Self::Effect(EffectState::Cancelled)),
            ("cost", "RESERVED") => Ok(Self::Cost(CostState::Reserved)),
            ("cost", "SETTLED") => Ok(Self::Cost(CostState::Settled)),
            ("cost", "UNKNOWN") => Ok(Self::Cost(CostState::Unknown)),
            ("cost", "RELEASED") => Ok(Self::Cost(CostState::Released)),
            _ => Err(StateError::UnknownState {
                machine: machine.to_owned(),
                state: state.to_owned(),
            }),
        }
    }

    pub fn can_transition_to(self, target: Self) -> bool {
        match (self, target) {
            (Self::Plugin(from), Self::Plugin(to)) => plugin_edge(from, to),
            (Self::Run(from), Self::Run(to)) => run_edge(from, to),
            (Self::Approval(from), Self::Approval(to)) => approval_edge(from, to),
            (Self::Effect(from), Self::Effect(to)) => effect_edge(from, to),
            (Self::Cost(from), Self::Cost(to)) => cost_edge(from, to),
            _ => false,
        }
    }
}

fn plugin_edge(from: PluginState, to: PluginState) -> bool {
    use PluginState::*;
    matches!(
        (from, to),
        (Discovered, Verified)
            | (Verified, Resolved)
            | (Resolved, Staged)
            | (Staged, InstalledDisabled)
            | (InstalledDisabled, Enabled)
            | (Enabled, Draining)
            | (Draining, Disabled)
            | (Disabled, Enabled)
            | (Disabled, Uninstalled)
            | (InstalledDisabled, Uninstalled)
            | (Discovered, Quarantined)
            | (Verified, Quarantined)
            | (Staged, Quarantined)
            | (Enabled, Revoked)
            | (Draining, Revoked)
            | (Disabled, Revoked)
            | (InstalledDisabled, Revoked)
    )
}

fn run_edge(from: RunState, to: RunState) -> bool {
    use RunState::*;
    matches!(
        (from, to),
        (Queued, Running)
            | (Running, WaitingApproval)
            | (WaitingApproval, Running)
            | (Running, WaitingRetry)
            | (WaitingRetry, Running)
            | (Running, Blocked)
            | (Blocked, Running)
            | (Running, Succeeded)
            | (Running, Failed)
            | (Queued, Cancelled)
            | (WaitingApproval, Cancelled)
            | (WaitingRetry, Cancelled)
            | (Blocked, Cancelled)
            | (Running, CancelRequested)
            | (CancelRequested, Cancelled)
            | (CancelRequested, Succeeded)
    )
}

fn approval_edge(from: ApprovalState, to: ApprovalState) -> bool {
    use ApprovalState::*;
    matches!(
        (from, to),
        (Pending, Approved)
            | (Pending, Rejected)
            | (Pending, Cancelled)
            | (Pending, Expired)
            | (Pending, Stale)
            | (Approved, Stale)
            | (Approved, Expired)
            | (Approved, Cancelled)
    )
}

fn effect_edge(from: EffectState, to: EffectState) -> bool {
    use EffectState::*;
    matches!(
        (from, to),
        (Prepared, WaitingApproval)
            | (Prepared, Authorized)
            | (WaitingApproval, Authorized)
            | (Authorized, Executing)
            | (Executing, Succeeded)
            | (Executing, FailedConfirmed)
            | (Executing, OutcomeUnknown)
            | (OutcomeUnknown, Reconciling)
            | (Reconciling, Succeeded)
            | (Reconciling, FailedConfirmed)
            | (Reconciling, Authorized)
            | (Reconciling, ManualReview)
            | (ManualReview, Reconciling)
            | (Prepared, Cancelled)
            | (WaitingApproval, Cancelled)
            | (Authorized, Cancelled)
    )
}

fn cost_edge(from: CostState, to: CostState) -> bool {
    use CostState::*;
    matches!(
        (from, to),
        (Reserved, Settled)
            | (Reserved, Unknown)
            | (Reserved, Released)
            | (Unknown, Settled)
            | (Unknown, Released)
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VersionedState {
    state: MachineState,
    version: u64,
}

impl VersionedState {
    pub fn new(state: MachineState) -> Self {
        Self { state, version: 1 }
    }

    pub fn transition(&mut self, target: MachineState) -> Result<(), StateError> {
        if !self.state.can_transition_to(target) {
            return Err(StateError::IllegalTransition);
        }
        self.state = target;
        self.version += 1;
        Ok(())
    }

    pub fn state(&self) -> MachineState {
        self.state
    }

    pub fn version(&self) -> u64 {
        self.version
    }
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum StateError {
    #[error("ILLEGAL_TRANSITION")]
    IllegalTransition,
    #[error("unknown state {machine}:{state}")]
    UnknownState { machine: String, state: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    // MASONWING@1.0.1 REQ-073 / AC-077 / TC-AC-077.
    #[test]
    fn terminal_run_cannot_resume_and_version_does_not_change() {
        let mut run = VersionedState::new(MachineState::Run(RunState::Succeeded));
        assert_eq!(
            run.transition(MachineState::Run(RunState::Running)),
            Err(StateError::IllegalTransition)
        );
        assert_eq!(run.state(), MachineState::Run(RunState::Succeeded));
        assert_eq!(run.version(), 1);
    }
}
