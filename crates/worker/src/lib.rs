//! Masonwing durable worker: workflow step selection and run orchestration.
//!
//! `run_interpreter` is the deterministic decision layer — pure over the
//! workflow definition plus durable checkpoints, so crash/resume replays
//! cannot invent new dispatches.

pub mod relay;
pub mod run_interpreter;
pub mod run_resume;

pub use relay::{RelayConfig, relay_once, run_relay};
