use serde::{Deserialize, Serialize};
use thiserror::Error;

/// The shared ownership state for every QSONaut transmit path.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TxState {
    #[default]
    Disarmed,
    Armed,
    Queued,
    Transmitting,
}

/// Explicit transitions accepted by the shared TX safety gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TxAction {
    Arm,
    Queue,
    Start,
    Complete,
    Cancel,
    Fail,
    Disarm,
    RadioUnavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum TxError {
    #[error("TX is already armed")]
    AlreadyArmed,
    #[error("TX is not armed")]
    NotArmed,
    #[error("TX has no queued transmission")]
    NothingQueued,
    #[error("TX is not transmitting")]
    NotTransmitting,
}

/// Deterministic, UI-independent TX safety state machine.
///
/// `Disarm` and `RadioUnavailable` are unconditional and idempotent. This is
/// deliberate: safety callers must not need to know which mode currently owns
/// a queued or active transmission before stopping it.
#[derive(Debug, Default)]
pub struct TxGate {
    state: TxState,
}

impl TxGate {
    pub fn state(&self) -> TxState {
        self.state
    }

    pub fn apply(&mut self, action: TxAction) -> Result<TxState, TxError> {
        let next = match action {
            TxAction::Disarm | TxAction::RadioUnavailable => TxState::Disarmed,
            TxAction::Arm => match self.state {
                TxState::Disarmed => TxState::Armed,
                TxState::Armed => return Err(TxError::AlreadyArmed),
                TxState::Queued | TxState::Transmitting => return Err(TxError::AlreadyArmed),
            },
            TxAction::Queue => match self.state {
                TxState::Armed => TxState::Queued,
                TxState::Disarmed => return Err(TxError::NotArmed),
                TxState::Queued | TxState::Transmitting => return Err(TxError::AlreadyArmed),
            },
            TxAction::Start => match self.state {
                TxState::Queued => TxState::Transmitting,
                TxState::Disarmed | TxState::Armed => return Err(TxError::NothingQueued),
                TxState::Transmitting => return Err(TxError::AlreadyArmed),
            },
            TxAction::Complete => match self.state {
                TxState::Transmitting => TxState::Armed,
                TxState::Disarmed | TxState::Armed | TxState::Queued => {
                    return Err(TxError::NotTransmitting)
                }
            },
            TxAction::Cancel | TxAction::Fail => match self.state {
                TxState::Queued | TxState::Transmitting | TxState::Armed => TxState::Disarmed,
                TxState::Disarmed => TxState::Disarmed,
            },
        };
        self.state = next;
        Ok(next)
    }

    pub fn disarm(&mut self) {
        let _ = self.apply(TxAction::Disarm);
    }
}

#[cfg(test)]
mod tests {
    use super::{TxAction, TxError, TxGate, TxState};

    #[test]
    fn normal_transmission_returns_to_armed_after_completion() {
        let mut gate = TxGate::default();
        assert_eq!(gate.apply(TxAction::Arm), Ok(TxState::Armed));
        assert_eq!(gate.apply(TxAction::Queue), Ok(TxState::Queued));
        assert_eq!(gate.apply(TxAction::Start), Ok(TxState::Transmitting));
        assert_eq!(gate.apply(TxAction::Complete), Ok(TxState::Armed));
    }

    #[test]
    fn cancellation_and_failure_always_disarm() {
        for action in [TxAction::Cancel, TxAction::Fail] {
            let mut gate = TxGate::default();
            gate.apply(TxAction::Arm).unwrap();
            gate.apply(TxAction::Queue).unwrap();
            gate.apply(TxAction::Start).unwrap();
            assert_eq!(gate.apply(action), Ok(TxState::Disarmed));
            assert_eq!(gate.state(), TxState::Disarmed);
        }
    }

    #[test]
    fn global_disarm_is_idempotent_and_clears_queued_work() {
        let mut gate = TxGate::default();
        gate.apply(TxAction::Arm).unwrap();
        gate.apply(TxAction::Queue).unwrap();
        assert_eq!(gate.apply(TxAction::Disarm), Ok(TxState::Disarmed));
        assert_eq!(gate.apply(TxAction::Disarm), Ok(TxState::Disarmed));
        assert_eq!(gate.apply(TxAction::Start), Err(TxError::NothingQueued));
    }

    #[test]
    fn radio_loss_cannot_restore_previous_tx_state() {
        let mut gate = TxGate::default();
        gate.apply(TxAction::Arm).unwrap();
        gate.apply(TxAction::Queue).unwrap();
        assert_eq!(
            gate.apply(TxAction::RadioUnavailable),
            Ok(TxState::Disarmed)
        );
        assert_eq!(gate.apply(TxAction::Arm), Ok(TxState::Armed));
        assert_eq!(gate.apply(TxAction::Start), Err(TxError::NothingQueued));
    }

    #[test]
    fn invalid_transitions_do_not_mutate_state() {
        let mut gate = TxGate::default();
        assert_eq!(gate.apply(TxAction::Start), Err(TxError::NothingQueued));
        assert_eq!(gate.state(), TxState::Disarmed);
        assert_eq!(gate.apply(TxAction::Queue), Err(TxError::NotArmed));
        assert_eq!(gate.state(), TxState::Disarmed);
    }
}
