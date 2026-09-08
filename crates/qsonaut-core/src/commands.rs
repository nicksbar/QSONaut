use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, Instant};

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
    use super::*;
    use std::time::Duration;

    fn command(id: &str, timeout_ms: u64) -> CommandEnvelope {
        CommandEnvelope {
            id: CommandId::new(id),
            kind: CommandKind::SetPtt,
            timeout_ms,
        }
    }

    #[test]
    fn command_wire_values_and_timeout_are_stable() {
        let value = command("tx-1", 1_000);
        assert_eq!(value.timeout(), Duration::from_millis(1_000));
        assert_eq!(
            serde_json::to_string(&value).unwrap(),
            r#"{"id":"tx-1","kind":"set_ptt","timeout_ms":1000}"#
        );
    }

    #[test]
    fn tracker_correlates_completion_and_cancels_generation() {
        let now = Instant::now();
        let mut tracker = CommandTracker::default();
        let pending = command("radio-1", 100);
        assert_eq!(
            tracker.begin(pending.clone(), now).outcome,
            CommandOutcome::Accepted
        );
        assert_eq!(
            tracker
                .finish(&pending.id, CommandOutcome::Completed, "done")
                .unwrap()
                .outcome,
            CommandOutcome::Completed
        );
        let pending = command("radio-2", 100);
        tracker.begin(pending.clone(), now);
        assert_eq!(
            tracker.advance_generation()[0].outcome,
            CommandOutcome::Canceled
        );
        assert!(tracker
            .finish(&pending.id, CommandOutcome::Completed, "late")
            .is_none());
    }

    #[test]
    fn tracker_expires_timed_out_commands() {
        let now = Instant::now();
        let mut tracker = CommandTracker::default();
        tracker.begin(command("radio-3", 10), now);
        assert_eq!(
            tracker.expire(now + Duration::from_millis(10))[0].outcome,
            CommandOutcome::TimedOut
        );
    }
}
