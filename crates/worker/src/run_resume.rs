//! Bind durable checkpoint rows to the deterministic interpreter.
//!
//! A worker that crashes mid-run restarts with no memory of the traversal it
//! was executing. The only record of what already happened is `run_checkpoints`
//! in PostgreSQL, so resume is a pure function of those rows plus the pinned
//! workflow definition — never of in-process state (REQ-070 / AC-074).

use masonwing_contracts::wire::WorkflowDefinition;
use masonwing_data_postgres::DurableCheckpoint;

use crate::run_interpreter::{
    CheckpointRecord, RunInterpreter, StepDecision, node_id_from_logical_step_id,
};

/// Rebuild the interpreter for a run from its durable history.
///
/// A row whose logical step id does not carry a node identity is dropped: the
/// interpreter would otherwise index it as a completed step it cannot attribute
/// to any node. Dropping it is fail-safe — the step is treated as not executed,
/// so the worst case is one re-issued dispatch under its own logical id, which
/// the `run_checkpoints` primary key then makes a no-op. Inventing a node
/// identity for the row would instead let a corrupt record skip a real step.
pub fn interpreter_from_checkpoints<'a>(
    definition: &'a WorkflowDefinition,
    checkpoints: &[DurableCheckpoint],
) -> RunInterpreter<'a> {
    let records = checkpoints
        .iter()
        .filter_map(|checkpoint| {
            node_id_from_logical_step_id(&checkpoint.logical_step_id).map(|node_id| {
                CheckpointRecord {
                    logical_step_id: checkpoint.logical_step_id.clone(),
                    node_id: node_id.to_string(),
                    state: checkpoint.state.clone(),
                    failure_code: checkpoint.failure_code.clone(),
                }
            })
        })
        .collect();
    RunInterpreter::new(definition, records)
}

/// The fence a checkpoint write from this traversal must carry.
///
/// `None` means the run has no durable checkpoint history yet, in which case
/// the caller uses the run's current fence from its own read.
pub fn checkpoint_fence(checkpoints: &[DurableCheckpoint]) -> Option<i64> {
    checkpoints.first().map(|checkpoint| checkpoint.fence)
}

/// Whether a decision permits dispatch. Used by the worker loop to decide
/// between executing, parking, quarantining and completing without re-deriving
/// the interpreter's rules at the call site.
pub fn dispatches(decision: &StepDecision) -> bool {
    matches!(decision, StepDecision::Execute { .. })
}

#[cfg(test)]
mod tests {
    use super::*;
    use masonwing_contracts::wire::{ArtifactRef, WorkflowEdge, WorkflowNode};

    fn reference() -> ArtifactRef {
        serde_json::from_value(serde_json::json!({
            "tenant_id": "019732b8-0000-7000-8000-000000000001",
            "artifact_id": "019732b8-0000-7000-8000-000000000002",
            "digest": format!("sha256:{}", "0".repeat(64)),
            "classification": "INTERNAL",
            "schema_version": "1"
        }))
        .unwrap()
    }

    fn definition() -> WorkflowDefinition {
        WorkflowDefinition {
            id: "wf".to_string(),
            version: "1.0.0".to_string(),
            input_schema_ref: reference(),
            output_schema_ref: reference(),
            entry_node: "a".to_string(),
            nodes: ["a", "b", "c"]
                .into_iter()
                .map(|id| WorkflowNode {
                    id: id.to_string(),
                    kind: "ACTIVITY".to_string(),
                    handler_id: Some(format!("handler-{id}")),
                    input_schema_ref: reference(),
                    output_schema_ref: reference(),
                    max_attempts: 1,
                    timeout_seconds: 30,
                    max_iterations: 1,
                    on_failure: "FAIL_RUN".to_string(),
                })
                .collect(),
            edges: vec![
                WorkflowEdge {
                    from: "a".to_string(),
                    to: "b".to_string(),
                    condition: "SUCCESS".to_string(),
                },
                WorkflowEdge {
                    from: "b".to_string(),
                    to: "c".to_string(),
                    condition: "SUCCESS".to_string(),
                },
            ],
            max_total_steps: 8,
            max_model_turns: 4,
            required_actions: vec![],
            digest: masonwing_contracts::Digest::sha256("0".repeat(64)).unwrap(),
        }
    }

    fn checkpoint(logical_step_id: &str, state: &str) -> DurableCheckpoint {
        DurableCheckpoint {
            logical_step_id: logical_step_id.to_string(),
            state: state.to_string(),
            failure_code: None,
            fence: 1,
        }
    }

    // MASONWING@1.0.1 REQ-070 / AC-074: a worker that died after step one
    // resumes at step two, and step one is never re-issued.
    #[test]
    fn resume_continues_at_the_first_unfinished_step() {
        let def = definition();
        let rows = vec![checkpoint("0001:a", "SUCCEEDED")];
        let interpreter = interpreter_from_checkpoints(&def, &rows);
        let decision = interpreter.next_step();
        assert_eq!(
            decision,
            StepDecision::Execute {
                logical_step_id: "0002:b".to_string(),
                node_id: "b".to_string(),
            }
        );
        assert!(dispatches(&decision));
        assert_eq!(checkpoint_fence(&rows), Some(1));
    }

    // A step interrupted mid-flight (RUNNING, never completed) is re-issued
    // exactly once under its own logical id — the same id it had before the
    // crash, so the durable primary key makes the retry a no-op if the original
    // write had actually landed.
    #[test]
    fn interrupted_step_is_reissued_under_its_own_logical_id() {
        let def = definition();
        let rows = vec![checkpoint("0001:a", "RUNNING")];
        let interpreter = interpreter_from_checkpoints(&def, &rows);
        assert_eq!(
            interpreter.next_step(),
            StepDecision::Execute {
                logical_step_id: "0001:a".to_string(),
                node_id: "a".to_string(),
            }
        );
    }

    // A completed traversal reports Completed, never another dispatch.
    #[test]
    fn fully_completed_history_dispatches_nothing() {
        let def = definition();
        let rows = vec![
            checkpoint("0001:a", "SUCCEEDED"),
            checkpoint("0002:b", "SUCCEEDED"),
            checkpoint("0003:c", "SUCCEEDED"),
        ];
        let interpreter = interpreter_from_checkpoints(&def, &rows);
        assert_eq!(interpreter.next_step(), StepDecision::Completed);
        assert!(!dispatches(&interpreter.next_step()));
    }

    // A row that carries no node identity is dropped rather than attributed to
    // an invented node; the affected step is re-issued under its own id.
    #[test]
    fn row_without_node_identity_cannot_skip_a_step() {
        let def = definition();
        let rows = vec![checkpoint("garbage", "SUCCEEDED")];
        let interpreter = interpreter_from_checkpoints(&def, &rows);
        assert_eq!(
            interpreter.next_step(),
            StepDecision::Execute {
                logical_step_id: "0001:a".to_string(),
                node_id: "a".to_string(),
            }
        );
    }

    #[test]
    fn node_identity_is_parsed_from_the_step_key() {
        assert_eq!(node_id_from_logical_step_id("0002:b"), Some("b"));
        assert_eq!(node_id_from_logical_step_id("0002:a:b"), Some("a:b"));
        assert_eq!(node_id_from_logical_step_id("0002:"), None);
        assert_eq!(node_id_from_logical_step_id("0002"), None);
    }
}
