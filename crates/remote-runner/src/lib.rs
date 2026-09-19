//! Remote execution seam.
//!
//! No authenticated leased transport has been qualified yet, so remote plugin
//! invocation is deliberately unavailable. The fence validation below is the
//! state-mutation guard future transports must call before accepting completion.

use masonwing_contracts::{Digest, ResourceId};
use thiserror::Error;

pub const REMOTE_WORKER_UNAVAILABLE: &str = "REMOTE_WORKER_UNAVAILABLE";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecutionFence {
    pub invocation_id: ResourceId,
    pub lease_epoch: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteCompletion {
    pub fence: ExecutionFence,
    pub output_digest: Digest,
}

pub fn validate_completion_fence(
    expected: &ExecutionFence,
    completion: &RemoteCompletion,
) -> Result<(), RemoteRunnerError> {
    if expected != &completion.fence {
        return Err(RemoteRunnerError::StaleFence);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Default)]
pub struct RemoteRunner;

impl RemoteRunner {
    pub fn qualified(&self) -> bool {
        false
    }

    pub fn invoke(&self) -> Result<RemoteCompletion, RemoteRunnerError> {
        Err(RemoteRunnerError::Unavailable)
    }
}

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum RemoteRunnerError {
    #[error("REMOTE_WORKER_UNAVAILABLE")]
    Unavailable,
    #[error("STALE_FENCE")]
    StaleFence,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_execution_is_explicitly_unqualified() {
        let runner = RemoteRunner;
        assert!(!runner.qualified());
        assert_eq!(runner.invoke(), Err(RemoteRunnerError::Unavailable));
    }

    #[test]
    fn stale_completion_fence_is_rejected() {
        let expected = ExecutionFence {
            invocation_id: ResourceId::new("invoke_remote_1").unwrap(),
            lease_epoch: 2,
        };
        let completion = RemoteCompletion {
            fence: ExecutionFence {
                invocation_id: ResourceId::new("invoke_remote_1").unwrap(),
                lease_epoch: 1,
            },
            output_digest: Digest::parse(
                "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            )
            .unwrap(),
        };
        assert_eq!(
            validate_completion_fence(&expected, &completion),
            Err(RemoteRunnerError::StaleFence)
        );
    }
}
