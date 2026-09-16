use std::{fs, path::PathBuf};

use masonwing_kernel::{MachineState, StateError, VersionedState};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct StateVectorFile {
    vectors: Vec<StateVector>,
}

#[derive(Debug, Deserialize)]
struct StateVector {
    id: String,
    machine: String,
    from: String,
    to: String,
    expected: String,
}

// Runtime adjacency oracle for the complete immutable MASONWING@1.0.1 vector set.
// Guard semantics are exercised separately by output-based kernel tests.
#[test]
fn runtime_state_oracle_executes_all_immutable_baseline_vectors() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../masonwing-requirements-v1.0.1/06-testing/state-vectors.json");
    let bytes = fs::read(&path).expect("immutable state vector fixture is readable");
    let fixture: StateVectorFile =
        serde_json::from_slice(&bytes).expect("immutable state vector fixture parses");

    assert_eq!(fixture.vectors.len(), 314, "baseline vector count drifted");

    for vector in fixture.vectors {
        let from = MachineState::parse(&vector.machine, &vector.from)
            .unwrap_or_else(|error| panic!("{} from state: {error}", vector.id));
        let to = MachineState::parse(&vector.machine, &vector.to)
            .unwrap_or_else(|error| panic!("{} to state: {error}", vector.id));
        let mut runtime = VersionedState::new(from);
        let legal = vector.expected.starts_with("transition to target");
        let result = runtime.transition(to);

        if legal {
            assert_eq!(result, Ok(()), "{} expected legal edge", vector.id);
            assert_eq!(runtime.state(), to, "{} target state", vector.id);
            assert_eq!(
                runtime.version(),
                2,
                "{} audit version increment",
                vector.id
            );
        } else {
            assert_eq!(
                result,
                Err(StateError::IllegalTransition),
                "{} expected illegal edge",
                vector.id
            );
            assert_eq!(runtime.state(), from, "{} state preserved", vector.id);
            assert_eq!(runtime.version(), 1, "{} version preserved", vector.id);
        }
    }
}
