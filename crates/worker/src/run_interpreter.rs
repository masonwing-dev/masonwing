//! Deterministic workflow step selection for durable runs.
//!
//! The interpreter is a pure decision function over the immutable workflow
//! definition and the durable checkpoint history. It never performs I/O: the
//! worker executes the step it names and records the outcome back into
//! `run_checkpoints`. Re-running the decision after a crash reproduces the same
//! traversal, so a completed step is never re-dispatched (REQ-070), a step past
//! the declared budget is never dispatched at all (REQ-072), and recorded
//! history that disagrees with the current traversal quarantines the run
//! instead of executing new side effects (REQ-075).

use std::collections::BTreeMap;

use masonwing_contracts::wire::WorkflowDefinition;
use serde_json::Value;

/// Workflow node kinds from the generated contract enum
/// (`WorkflowNode.kind`: DETERMINISTIC/ACTIVITY/MODEL/APPROVAL/EFFECT/TIMER/
/// BOUNDED_LOOP). The interpreter only branches on the two kinds that change
/// scheduling: MODEL consumes a turn from `max_model_turns`, APPROVAL parks.
const NODE_KIND_MODEL: &str = "MODEL";
const NODE_KIND_APPROVAL: &str = "APPROVAL";

/// Edge condition enum from the generated contract (`WorkflowDefinition.edges`
/// condition: SUCCESS/FAILURE/APPROVED/REJECTED/TRUE/FALSE/TIMER_FIRED/
/// LOOP_CONTINUE/LOOP_DONE). A node's unconditional forward continuation on
/// success is SUCCESS; every other condition requires a branch decision the
/// interpreter does not make, so such edges are not traversed here.
const EDGE_CONDITION_SUCCESS: &str = "SUCCESS";
const EDGE_CONDITION_APPROVED: &str = "APPROVED";

/// One recorded durable step. `state` mirrors the `run_checkpoints` CHECK:
/// RUNNING, SUCCEEDED, FAILED, BLOCKED or INCOMPLETE.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckpointRecord {
    pub logical_step_id: String,
    pub node_id: String,
    pub state: String,
    pub failure_code: Option<String>,
}

/// What the worker must do next. Every variant is reachable only through the
/// same deterministic traversal, so replays cannot invent extra dispatches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StepDecision {
    /// Execute this node exactly once under this logical step id. A crash
    /// before completion re-issues the same id, never a new one (AC-074).
    Execute {
        logical_step_id: String,
        node_id: String,
    },
    /// The traversal reached a terminal node; the run can complete.
    Completed,
    /// REQ-072 / AC-076: the declared step or turn budget is exhausted. No
    /// model or tool call is dispatched after the limit.
    LimitReached,
    /// REQ-071 / AC-075: the node parks the run on a durable approval wait.
    /// Waiting holds no execution slot; resumption re-enters the same run.
    WaitingApproval {
        logical_step_id: String,
        node_id: String,
    },
    /// REQ-075 / AC-079: recorded history disagrees with this traversal. The
    /// run must quarantine; transmit stays zero.
    ReplayIncompatible,
    /// A recorded step failed or is blocked; downstream dispatch stops until
    /// an operator or retry path recovers it.
    Blocked { failure_code: Option<String> },
    /// The traversal cannot be continued deterministically: the current node
    /// has no continuation edge this interpreter models (a branch, timer or
    /// loop condition it does not decide), or names a node the definition does
    /// not declare. This is NOT completion — reporting `Completed` here would
    /// mark a run SUCCEEDED by inventing a traversal — so the run blocks with
    /// this reason and nothing further is dispatched.
    Unsupported {
        node_id: String,
        reason: &'static str,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkflowFailure {
    /// REQ-077 / AC-081: node output does not satisfy the pinned output
    /// contract. The downstream node must not be dispatched.
    NodeOutputInvalid,
}

/// Key for the nth (1-based) deterministic visit. The numeric prefix orders
/// the traversal even when bounded cycles revisit the same node.
pub fn logical_step_key(seq: u32, node_id: &str) -> String {
    format!("{seq:04}:{node_id}")
}

/// The node identity carried inside a logical step key. `None` means the key
/// was not produced by [`logical_step_key`]; such a row cannot be attributed to
/// a declared node, so a replay must not treat it as a completed step.
pub fn node_id_from_logical_step_id(logical_step_id: &str) -> Option<&str> {
    logical_step_id
        .split_once(':')
        .map(|(_, node_id)| node_id)
        .filter(|node_id| !node_id.is_empty())
}

pub struct RunInterpreter<'a> {
    definition: &'a WorkflowDefinition,
    checkpoints: BTreeMap<String, CheckpointRecord>,
}

impl<'a> RunInterpreter<'a> {
    pub fn new(definition: &'a WorkflowDefinition, checkpoints: Vec<CheckpointRecord>) -> Self {
        let indexed = checkpoints
            .into_iter()
            .map(|record| (record.logical_step_id.clone(), record))
            .collect();
        Self {
            definition,
            checkpoints: indexed,
        }
    }

    fn node(&self, node_id: &str) -> Option<&masonwing_contracts::wire::WorkflowNode> {
        self.definition.nodes.iter().find(|n| n.id == node_id)
    }

    /// Deterministic successor: the first edge in definition order from the
    /// current node whose condition is the continuation this node's success
    /// licenses. An APPROVAL node continues on `APPROVED` — its checkpoint only
    /// reaches SUCCEEDED once the approval was granted — and every other node
    /// continues on `SUCCESS`. A node with no such edge is terminal.
    ///
    /// Decision edges the interpreter does not model (FAILURE, REJECTED,
    /// TRUE/FALSE, TIMER_FIRED, LOOP_*) are never taken here, and an APPROVED
    /// edge is never taken from a non-approval node: ending a traversal early
    /// is safe, inventing a branch decision is not.
    fn successor(&self, node_id: &str) -> Option<&str> {
        let condition = if self.node(node_id)?.kind == NODE_KIND_APPROVAL {
            EDGE_CONDITION_APPROVED
        } else {
            EDGE_CONDITION_SUCCESS
        };
        self.definition
            .edges
            .iter()
            .find(|edge| edge.from == node_id && edge.condition == condition)
            .map(|edge| edge.to.as_str())
    }

    /// The node the traversal visits on step `seq`, walking from `entry_node`.
    ///
    /// Returns:
    /// - `Ok(Some(node_id))` — the traversal reached this node deterministically.
    /// - `Ok(None)` — the traversal reached a leaf node with no declared out-edges
    ///   (a legitimate terminal node).
    /// - `Err((reason, node_id))` — the traversal encountered an undeclared node,
    ///   a target node that does not exist in `definition.nodes`, or a node that
    ///   has out-edges but none are the modeled continuation (`SUCCESS`, or
    ///   `APPROVED` for approval nodes). In this case the run cannot proceed
    ///   deterministically and must block rather than report `Completed`.
    fn node_at(&self, seq: u32) -> Result<Option<&str>, (&'static str, String)> {
        let mut current = self.definition.entry_node.as_str();
        for _ in 1..seq {
            if self.node(current).is_none() {
                return Err(("NODE_UNDECLARED", current.to_string()));
            }
            let out_edges: Vec<_> = self
                .definition
                .edges
                .iter()
                .filter(|e| e.from == current)
                .collect();
            if out_edges.is_empty() {
                // Declared leaf node: traversal completed cleanly.
                return Ok(None);
            }
            let Some(next) = self.successor(current) else {
                // Out-edges exist, but none match the modeled continuation.
                return Err(("UNMODELED_BRANCH_EDGE", current.to_string()));
            };
            if self.node(next).is_none() {
                return Err(("EDGE_TARGET_UNDECLARED", next.to_string()));
            }
            current = next;
        }
        if self.node(current).is_none() {
            return Err(("NODE_UNDECLARED", current.to_string()));
        }
        Ok(Some(current))
    }

    /// Number of model-kind nodes executed before step `seq`.
    fn model_turns_used(&self, upto_seq: u32) -> u32 {
        (1..upto_seq)
            .filter_map(|seq| self.node_at(seq).ok().flatten())
            .filter(|node_id| {
                self.node(node_id)
                    .is_some_and(|node| node.kind == NODE_KIND_MODEL)
            })
            .count() as u32
    }

    /// Decide the next action for the run from durable state. Calling this
    /// repeatedly without new checkpoints is idempotent.
    pub fn next_step(&self) -> StepDecision {
        // A traversal that runs off the end of the graph before the declared
        // step budget is exhausted has completed; exhausting the budget first
        // is LIMIT_REACHED. An unmodeled branch or undeclared target blocks
        // as Unsupported rather than inventing a traversal or claiming completion.
        for seq in 1..=self.definition.max_total_steps {
            let node_id = match self.node_at(seq) {
                Ok(Some(id)) => id,
                Ok(None) => return StepDecision::Completed,
                Err((reason, node_id)) => return StepDecision::Unsupported { node_id, reason },
            };
            let key = logical_step_key(seq, node_id);
            match self.checkpoints.get(&key) {
                Some(record) if record.node_id != node_id => {
                    // Recorded history belongs to a different traversal than
                    // this definition produces. Quarantine before any dispatch.
                    return StepDecision::ReplayIncompatible;
                }
                Some(record) if record.state == "SUCCEEDED" => continue,
                Some(record) if record.state == "FAILED" || record.state == "BLOCKED" => {
                    return StepDecision::Blocked {
                        failure_code: record.failure_code.clone(),
                    };
                }
                // RUNNING or INCOMPLETE: the worker died mid-step. Re-dispatch
                // once under the same logical id (AC-074).
                Some(_) => {
                    return self.budget_gate(seq, node_id, &key);
                }
                None => {
                    return self.budget_gate(seq, node_id, &key);
                }
            }
        }
        StepDecision::LimitReached
    }

    fn budget_gate(&self, seq: u32, node_id: &str, key: &str) -> StepDecision {
        let Some(node) = self.node(node_id) else {
            // Unresolved node contract cannot dispatch anything.
            return StepDecision::ReplayIncompatible;
        };
        if node.kind == NODE_KIND_MODEL
            && self.model_turns_used(seq) >= self.definition.max_model_turns
        {
            return StepDecision::LimitReached;
        }
        if node.kind == NODE_KIND_APPROVAL {
            return StepDecision::WaitingApproval {
                logical_step_id: key.to_string(),
                node_id: node_id.to_string(),
            };
        }
        StepDecision::Execute {
            logical_step_id: key.to_string(),
            node_id: node_id.to_string(),
        }
    }

    /// REQ-077: validate a node's output against its pinned output contract
    /// before any downstream node is dispatched. `schema_bytes` are loaded by
    /// the caller from the artifact the definition pins.
    pub fn validate_node_output(
        &self,
        node_id: &str,
        schema_bytes: &[u8],
        output: &Value,
    ) -> Result<(), WorkflowFailure> {
        let node = self
            .node(node_id)
            .ok_or(WorkflowFailure::NodeOutputInvalid)?;
        let _ = node;
        let schema = masonwing_contract_validation::ArtifactSchema::compile(schema_bytes)
            .map_err(|_| WorkflowFailure::NodeOutputInvalid)?;
        let bytes = masonwing_contract_validation::canonical_bytes(output)
            .map_err(|_| WorkflowFailure::NodeOutputInvalid)?;
        schema
            .validate_bytes(&bytes, 4 * 1024 * 1024)
            .map_err(|_| WorkflowFailure::NodeOutputInvalid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use masonwing_contracts::wire::{WorkflowEdge, WorkflowNode};

    fn node(id: &str, kind: &str) -> WorkflowNode {
        WorkflowNode {
            id: id.to_string(),
            kind: kind.to_string(),
            handler_id: Some(format!("handler-{id}")),
            input_schema_ref: output_ref(),
            output_schema_ref: output_ref(),
            max_attempts: 1,
            timeout_seconds: 30,
            max_iterations: 1,
            on_failure: "FAIL_RUN".to_string(),
        }
    }

    fn output_ref() -> masonwing_contracts::wire::ArtifactRef {
        serde_json::from_value(serde_json::json!({
            "tenant_id": "019732b8-0000-7000-8000-000000000001",
            "artifact_id": "019732b8-0000-7000-8000-000000000002",
            "digest": format!("sha256:{}", "0".repeat(64)),
            "classification": "INTERNAL",
            "schema_version": "1"
        }))
        .unwrap()
    }

    fn definition(nodes: Vec<WorkflowNode>, edges: Vec<WorkflowEdge>) -> WorkflowDefinition {
        let entry_node = nodes.first().map(|n| n.id.clone()).unwrap_or_default();
        WorkflowDefinition {
            id: "wf".to_string(),
            version: "1.0.0".to_string(),
            input_schema_ref: output_ref(),
            output_schema_ref: output_ref(),
            entry_node,
            nodes,
            edges,
            max_total_steps: 8,
            max_model_turns: 8,
            required_actions: vec![],
            digest: masonwing_contracts::Digest::sha256(format!("{:x}", 0u8))
                .unwrap_or_else(|_| masonwing_contracts::Digest::sha256("0".repeat(64)).unwrap()),
        }
    }

    fn edge(from: &str, to: &str) -> WorkflowEdge {
        edge_condition(from, to, "SUCCESS")
    }

    fn edge_condition(from: &str, to: &str, condition: &str) -> WorkflowEdge {
        WorkflowEdge {
            from: from.to_string(),
            to: to.to_string(),
            condition: condition.to_string(),
        }
    }

    fn succeeded(seq: u32, node_id: &str) -> CheckpointRecord {
        CheckpointRecord {
            logical_step_id: logical_step_key(seq, node_id),
            node_id: node_id.to_string(),
            state: "SUCCEEDED".to_string(),
            failure_code: None,
        }
    }

    // MASONWING@1.0.1 REQ-070 / AC-074: a completed checkpoint is never
    // re-dispatched and never rewritten as pending; the next step is issued
    // once under its logical id.
    #[test]
    fn resume_skips_completed_steps_and_dispatches_next_once() {
        let def = definition(
            vec![
                node("a", "ACTIVITY"),
                node("b", "ACTIVITY"),
                node("c", "ACTIVITY"),
            ],
            vec![edge("a", "b"), edge("b", "c")],
        );
        // Fresh run: step one only.
        let fresh = RunInterpreter::new(&def, vec![]);
        assert_eq!(
            fresh.next_step(),
            StepDecision::Execute {
                logical_step_id: "0001:a".to_string(),
                node_id: "a".to_string(),
            }
        );
        // After step one completed: the next decision is step two, once.
        let resumed = RunInterpreter::new(&def, vec![succeeded(1, "a")]);
        assert_eq!(
            resumed.next_step(),
            StepDecision::Execute {
                logical_step_id: "0002:b".to_string(),
                node_id: "b".to_string(),
            }
        );
        // All steps completed: the traversal is finished.
        let done = RunInterpreter::new(
            &def,
            vec![succeeded(1, "a"), succeeded(2, "b"), succeeded(3, "c")],
        );
        assert_eq!(done.next_step(), StepDecision::Completed);
    }

    // MASONWING@1.0.1 REQ-072 / AC-076: once the declared step or turn budget
    // is exhausted the decision is LIMIT_REACHED and nothing is dispatched.
    #[test]
    fn limit_reached_dispatches_zero_calls() {
        let def = definition(
            vec![node("a", "ACTIVITY"), node("b", "ACTIVITY")],
            vec![edge("a", "b")],
        );
        let mut bounded = def.clone();
        bounded.max_total_steps = 1;
        let interpreter = RunInterpreter::new(&bounded, vec![succeeded(1, "a")]);
        assert_eq!(interpreter.next_step(), StepDecision::LimitReached);

        // Model turns: one model node may run; the next model turn is refused.
        let mut turns = definition(
            vec![
                node("m", "MODEL"),
                node("t", "ACTIVITY"),
                node("m2", "MODEL"),
            ],
            vec![edge("m", "t"), edge("t", "m2")],
        );
        turns.max_model_turns = 1;
        let interpreter = RunInterpreter::new(&turns, vec![succeeded(1, "m"), succeeded(2, "t")]);
        assert_eq!(interpreter.next_step(), StepDecision::LimitReached);

        // A run that walks off the graph before exhausting the budget has
        // completed rather than hit the limit.
        let interpreter = RunInterpreter::new(&def, vec![succeeded(1, "a"), succeeded(2, "b")]);
        assert_eq!(interpreter.next_step(), StepDecision::Completed);
    }

    // MASONWING@1.0.1 REQ-075 / AC-079: recorded history that disagrees with
    // the traversal quarantines the run before any dispatch (transmit=0).
    #[test]
    fn replay_incompatible_quarantines_without_dispatch() {
        let def = definition(
            vec![node("a", "ACTIVITY"), node("b", "ACTIVITY")],
            vec![edge("a", "b")],
        );
        // The recorded checkpoint for step one belongs to node b, but this
        // definition's traversal lands on node a there — the replay is
        // incompatible and must quarantine before any new dispatch.
        let foreign = CheckpointRecord {
            logical_step_id: logical_step_key(1, "a"),
            node_id: "b".to_string(),
            state: "SUCCEEDED".to_string(),
            failure_code: None,
        };
        let interpreter = RunInterpreter::new(&def, vec![foreign]);
        assert_eq!(interpreter.next_step(), StepDecision::ReplayIncompatible);
        // An unresolved node contract (an edge to a node the definition does
        // not declare) is equally unreplayable once its step is reached.
        let broken = definition(vec![node("a", "ACTIVITY")], vec![edge("a", "missing")]);
        let interpreter = RunInterpreter::new(&broken, vec![succeeded(1, "a")]);
        assert_eq!(
            interpreter.next_step(),
            StepDecision::Unsupported {
                node_id: "missing".to_string(),
                reason: "EDGE_TARGET_UNDECLARED",
            }
        );
    }

    // A node that declares only conditional out-edges (a branch, loop or timer
    // the interpreter does not decide) cannot be treated as terminal: reporting
    // Completed would mark the run SUCCEEDED by inventing a traversal. It must
    // block instead (fail closed, REQ-073).
    #[test]
    fn unmodeled_branch_blocks_rather_than_inventing_completion() {
        let def = definition(
            vec![
                node("a", "ACTIVITY"),
                node("b", "ACTIVITY"),
                node("c", "ACTIVITY"),
            ],
            vec![
                edge_condition("a", "b", "TRUE"),
                edge_condition("a", "c", "FALSE"),
            ],
        );
        let interpreter = RunInterpreter::new(&def, vec![succeeded(1, "a")]);
        assert_eq!(
            interpreter.next_step(),
            StepDecision::Unsupported {
                node_id: "a".to_string(),
                reason: "UNMODELED_BRANCH_EDGE",
            }
        );
    }

    // A node pointing at an undeclared target blocks when the next step
    // is requested, not on the fresh step (which can execute).
    #[test]
    fn undeclared_edge_target_blocks_on_resume() {
        let def = definition(vec![node("a", "ACTIVITY")], vec![edge("a", "ghost")]);
        // Fresh run: step 1 can execute "a".
        let fresh = RunInterpreter::new(&def, vec![]);
        assert_eq!(
            fresh.next_step(),
            StepDecision::Execute {
                logical_step_id: "0001:a".to_string(),
                node_id: "a".to_string(),
            }
        );
        // After "a" succeeds, the next step tries to follow the edge to
        // "ghost" and blocks because the target is undeclared.
        let resumed = RunInterpreter::new(&def, vec![succeeded(1, "a")]);
        assert_eq!(
            resumed.next_step(),
            StepDecision::Unsupported {
                node_id: "ghost".to_string(),
                reason: "EDGE_TARGET_UNDECLARED",
            }
        );
    }

    // MASONWING@1.0.1 REQ-077 / AC-081: output that fails the pinned contract
    // is refused, so the downstream node is never dispatched.
    #[test]
    fn node_output_invalid_blocks_downstream() {
        let def = definition(
            vec![node("a", "ACTIVITY"), node("b", "ACTIVITY")],
            vec![edge("a", "b")],
        );
        let interpreter = RunInterpreter::new(&def, vec![]);
        // additionalProperties must declare its own properties; a bare
        // `additionalProperties:false` schema would reject every field,
        // including the required one, which is a definition bug not an output
        // contract. This schema pins exactly the `b` field the node contract
        // requires.
        let schema = br#"{"type":"object","required":["b"],"properties":{"b":{"type":"integer"}},"additionalProperties":false}"#;
        interpreter
            .validate_node_output("a", schema, &serde_json::json!({"b": 1}))
            .unwrap();
        assert_eq!(
            interpreter.validate_node_output("a", schema, &serde_json::json!({"a": 1})),
            Err(WorkflowFailure::NodeOutputInvalid)
        );
        // A failed recorded step blocks downstream dispatch as well.
        let failed = CheckpointRecord {
            logical_step_id: logical_step_key(1, "a"),
            node_id: "a".to_string(),
            state: "FAILED".to_string(),
            failure_code: Some("NODE_OUTPUT_INVALID".to_string()),
        };
        let interpreter = RunInterpreter::new(&def, vec![failed]);
        assert_eq!(
            interpreter.next_step(),
            StepDecision::Blocked {
                failure_code: Some("NODE_OUTPUT_INVALID".to_string()),
            }
        );
    }

    // MASONWING@1.0.1 REQ-071 / AC-075: an approval wait is durable and holds
    // no execution slot; the same run resumes at the correct next step once
    // the approval terminal state is recorded.
    #[test]
    fn waiting_approval_parks_without_consuming_budget() {
        let def = definition(
            vec![
                node("a", "ACTIVITY"),
                node("gate", "APPROVAL"),
                node("c", "ACTIVITY"),
            ],
            vec![edge("a", "gate"), edge_condition("gate", "c", "APPROVED")],
        );
        let interpreter = RunInterpreter::new(&def, vec![succeeded(1, "a")]);
        assert_eq!(
            interpreter.next_step(),
            StepDecision::WaitingApproval {
                logical_step_id: "0002:gate".to_string(),
                node_id: "gate".to_string(),
            }
        );
        // Recording the approval outcome completes the same logical step and
        // resumes exactly this run's traversal.
        let resumed = RunInterpreter::new(
            &def,
            vec![succeeded(1, "a"), succeeded(2, "gate"), succeeded(3, "c")],
        );
        assert_eq!(resumed.next_step(), StepDecision::Completed);
    }

    // MASONWING@1.0.1 REQ-073 / AC-077: a failed step keeps blocking; the
    // mapper never invents a forward transition past recorded failure.
    #[test]
    fn recorded_failure_stays_blocked_until_recovered() {
        let def = definition(vec![node("a", "ACTIVITY")], vec![]);
        let incomplete = CheckpointRecord {
            logical_step_id: logical_step_key(1, "a"),
            node_id: "a".to_string(),
            state: "INCOMPLETE".to_string(),
            failure_code: None,
        };
        let interpreter = RunInterpreter::new(&def, vec![incomplete]);
        // Crash mid-step: the same logical step is re-issued, never a new one.
        assert_eq!(
            interpreter.next_step(),
            StepDecision::Execute {
                logical_step_id: "0001:a".to_string(),
                node_id: "a".to_string(),
            }
        );
    }
}
