use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;
use std::time::Instant;

/// Correlates a request, its terminal result, and any diagnostic events.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct CommandId(pub String);

impl CommandId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandKind {
    SetPtt,
    Tune,
    SetMode,
    StartAudio,
    StopAudio,
    Transmit,
    CancelTransmit,
    SaveLog,
    SetPower,
    SetControl,
    ApplyWorkspace,
    StartTuner,
    StartSwrSweep,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandEnvelope {
    pub id: CommandId,
    pub kind: CommandKind,
    pub timeout_ms: u64,
}

impl CommandEnvelope {
    pub fn timeout(&self) -> Duration {
        Duration::from_millis(self.timeout_ms)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandOutcome {
    Accepted,
    Completed,
    Canceled,
    TimedOut,
    Rejected,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandResult {
    pub id: CommandId,
    pub outcome: CommandOutcome,
    pub detail: String,
}

#[derive(Debug, Clone)]
struct PendingCommand {
    envelope: CommandEnvelope,
    generation: u64,
    accepted_at: Instant,
}

/// Tracks worker command ownership across acceptance, terminal completion,
/// cancellation, generation changes, and timeouts.
#[derive(Debug, Default)]
pub struct CommandTracker {
    generation: u64,
    pending: HashMap<CommandId, PendingCommand>,
}

impl CommandTracker {
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn begin(&mut self, envelope: CommandEnvelope, now: Instant) -> CommandResult {
        let id = envelope.id.clone();
        self.pending.insert(
            id.clone(),
            PendingCommand {
                envelope,
                generation: self.generation,
                accepted_at: now,
            },
        );
        CommandResult {
            id,
            outcome: CommandOutcome::Accepted,
            detail: "command accepted for execution".to_string(),
        }
    }

    pub fn finish(
        &mut self,
        id: &CommandId,
        outcome: CommandOutcome,
        detail: impl Into<String>,
    ) -> Option<CommandResult> {
        let pending = self.pending.remove(id)?;
        if pending.generation != self.generation {
            return Some(CommandResult {
                id: id.clone(),
                outcome: CommandOutcome::Rejected,
                detail: "command belongs to a stale worker generation".to_string(),
            });
        }
        Some(CommandResult {
            id: id.clone(),
            outcome,
            detail: detail.into(),
        })
    }

    pub fn cancel(&mut self, id: &CommandId, detail: impl Into<String>) -> Option<CommandResult> {
        self.finish(id, CommandOutcome::Canceled, detail)
    }

    pub fn advance_generation(&mut self) -> Vec<CommandResult> {
        self.generation = self.generation.wrapping_add(1);
        self.pending
            .drain()
            .map(|(id, _)| CommandResult {
                id,
                outcome: CommandOutcome::Canceled,
                detail: "worker generation stopped".to_string(),
            })
            .collect()
    }

    pub fn expire(&mut self, now: Instant) -> Vec<CommandResult> {
        let expired = self
            .pending
            .iter()
            .filter(|(_, pending)| {
                now.duration_since(pending.accepted_at) >= pending.envelope.timeout()
            })
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        expired
            .into_iter()
            .filter_map(|id| {
                self.finish(&id, CommandOutcome::TimedOut, "command execution timed out")
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CommandEnvelope, CommandId, CommandKind, CommandOutcome, CommandResult, CommandTracker,
    };
    use std::time::Duration;

    #[test]
    fn command_results_keep_correlation_and_timeout_contract() {
        let command = CommandEnvelope {
            id: CommandId::new("radio-17"),
            kind: CommandKind::SetPtt,
            timeout_ms: 750,
        };
        assert_eq!(command.timeout(), Duration::from_millis(750));

        let result = CommandResult {
            id: command.id.clone(),
            outcome: CommandOutcome::Completed,
            detail: "PTT released".to_string(),
        };
        assert_eq!(result.id, command.id);
        assert_eq!(result.outcome, CommandOutcome::Completed);
    }

    #[test]
    fn command_wire_values_are_stable() {
        let command = CommandEnvelope {
            id: CommandId::new("tx-1"),
            kind: CommandKind::CancelTransmit,
            timeout_ms: 1_000,
        };
        assert_eq!(
            serde_json::to_string(&command).unwrap(),
            r#"{"id":"tx-1","kind":"cancel_transmit","timeout_ms":1000}"#
        );
    }

    #[test]
    fn tracker_correlates_completion_and_rejects_stale_generation() {
        let now = std::time::Instant::now();
        let mut tracker = CommandTracker::default();
        assert_eq!(tracker.generation(), 0);
        let command = CommandEnvelope {
            id: CommandId::new("radio-1"),
            kind: CommandKind::Tune,
            timeout_ms: 100,
        };
        assert_eq!(
            tracker.begin(command.clone(), now).outcome,
            CommandOutcome::Accepted
        );
        assert_eq!(
            tracker
                .finish(&command.id, CommandOutcome::Completed, "frequency observed")
                .unwrap()
                .outcome,
            CommandOutcome::Completed
        );

        tracker.begin(command.clone(), now);
        let canceled = tracker.advance_generation();
        assert_eq!(canceled[0].outcome, CommandOutcome::Canceled);
        assert!(tracker
            .finish(&command.id, CommandOutcome::Completed, "stale")
            .is_none());

        let cancel_id = CommandId::new("cancel-me");
        tracker.begin(
            CommandEnvelope {
                id: cancel_id.clone(),
                kind: CommandKind::Tune,
                timeout_ms: 100,
            },
            now,
        );
        assert_eq!(
            tracker
                .cancel(&cancel_id, "operator canceled")
                .unwrap()
                .outcome,
            CommandOutcome::Canceled
        );
    }

    #[test]
    fn tracker_expires_pending_commands_by_declared_timeout() {
        let now = std::time::Instant::now();
        let mut tracker = CommandTracker::default();
        tracker.begin(
            CommandEnvelope {
                id: CommandId::new("radio-timeout"),
                kind: CommandKind::SetPtt,
                timeout_ms: 10,
            },
            now,
        );
        let expired = tracker.expire(now + Duration::from_millis(11));
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].outcome, CommandOutcome::TimedOut);
    }
}
