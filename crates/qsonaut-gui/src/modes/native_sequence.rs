use super::super::*;
use super::exchange::DEFAULT_PTT_LEAD_SECONDS;

/// Shared manual/automatic exchange controller for native slot modes. The
/// decoder and waveform remain mode-specific; this layer only handles the
/// common standard-message exchange lifecycle.
pub(crate) fn next_native_period(now_s: f64, slot_seconds: f64) -> u64 {
    (now_s / slot_seconds).floor() as u64 + 1
}

pub(crate) fn native_reply_allowed(
    now_s: f64,
    source_period: u64,
    slot_seconds: f64,
    audio_start_s: f64,
) -> bool {
    let target_start = source_period as f64 * slot_seconds + slot_seconds;
    now_s <= target_start + audio_start_s - DEFAULT_PTT_LEAD_SECONDS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_native_period_rolls_forward() {
        assert_eq!(next_native_period(60.01, 60.0), 2);
    }

    #[test]
    fn native_reply_closes_when_ptt_window_opens() {
        assert!(native_reply_allowed(60.1, 0, 60.0, 1.0));
        assert!(!native_reply_allowed(119.9, 0, 60.0, 1.0));
    }
}
