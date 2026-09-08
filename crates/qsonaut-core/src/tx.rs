use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TxState {
    #[default]
    Disarmed,
    Armed,
    Queued,
    Transmitting,
}

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
                TxState::Armed | TxState::Queued | TxState::Transmitting => {
                    return Err(TxError::AlreadyArmed)
                }
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
                TxState::Disarmed => TxState::Disarmed,
                TxState::Armed | TxState::Queued | TxState::Transmitting => TxState::Disarmed,
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
    use super::*;

    #[test]
    fn normal_transmission_returns_to_armed_after_completion() {
        let mut gate = TxGate::default();
        assert_eq!(gate.apply(TxAction::Arm), Ok(TxState::Armed));
        assert_eq!(gate.apply(TxAction::Queue), Ok(TxState::Queued));
        assert_eq!(gate.apply(TxAction::Start), Ok(TxState::Transmitting));
        assert_eq!(gate.apply(TxAction::Complete), Ok(TxState::Armed));
    }

    #[test]
    fn cancellation_failure_and_radio_loss_disarm() {
        for action in [TxAction::Cancel, TxAction::Fail, TxAction::RadioUnavailable] {
            let mut gate = TxGate::default();
            gate.apply(TxAction::Arm).unwrap();
            gate.apply(TxAction::Queue).unwrap();
            assert_eq!(gate.apply(action), Ok(TxState::Disarmed));
            assert_eq!(gate.state(), TxState::Disarmed);
        }
    }
}
