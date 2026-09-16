use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoundedLoop {
    max_turns: u32,
    completed_turns: u32,
}

impl BoundedLoop {
    pub fn new(max_turns: u32) -> Self {
        Self {
            max_turns,
            completed_turns: 0,
        }
    }

    pub fn run_next<T>(&mut self, dispatch: impl FnOnce() -> T) -> Result<T, LoopError> {
        if self.completed_turns >= self.max_turns {
            return Err(LoopError::LimitReached);
        }
        self.completed_turns += 1;
        Ok(dispatch())
    }

    pub fn completed_turns(&self) -> u32 {
        self.completed_turns
    }
}

#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum LoopError {
    #[error("LIMIT_REACHED")]
    LimitReached,
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    // MASONWING@1.0.1 REQ-072 / AC-076.
    #[test]
    fn dispatch_callback_is_never_called_after_turn_limit() {
        let mut guard = BoundedLoop::new(2);
        let calls = Cell::new(0);
        for _ in 0..2 {
            guard
                .run_next(|| calls.set(calls.get() + 1))
                .expect("within limit");
        }

        assert_eq!(
            guard.run_next(|| calls.set(99)),
            Err(LoopError::LimitReached)
        );
        assert_eq!(calls.get(), 2);
        assert_eq!(guard.completed_turns(), 2);
    }
}
