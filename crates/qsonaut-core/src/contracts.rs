use serde::{Deserialize, Serialize};

/// Audio format shared by capture, monitor, waterfall, and decoder paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AudioFormat {
    pub sample_rate_hz: u32,
    pub channels: u16,
    pub block_samples: u32,
}

impl AudioFormat {
    pub const CANONICAL: Self = Self {
        sample_rate_hz: 48_000,
        channels: 2,
        block_samples: 960,
    };

    pub const fn is_supported(self) -> bool {
        self.sample_rate_hz == Self::CANONICAL.sample_rate_hz
            && (self.channels == 1 || self.channels == Self::CANONICAL.channels)
            && self.block_samples > 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogOutcome {
    Accepted,
    Persisted,
    Duplicate,
    Rejected,
    Failed,
}

#[cfg(test)]
mod tests {
    use super::{AudioFormat, LogOutcome};

    #[test]
    fn canonical_audio_contract_is_supported() {
        assert!(AudioFormat::CANONICAL.is_supported());
        assert!(AudioFormat {
            channels: 1,
            ..AudioFormat::CANONICAL
        }
        .is_supported());
        assert!(!AudioFormat {
            sample_rate_hz: 44_100,
            ..AudioFormat::CANONICAL
        }
        .is_supported());
    }

    #[test]
    fn logging_outcomes_have_stable_wire_names() {
        assert_eq!(
            serde_json::to_string(&LogOutcome::Persisted).unwrap(),
            "\"persisted\""
        );
    }
}
