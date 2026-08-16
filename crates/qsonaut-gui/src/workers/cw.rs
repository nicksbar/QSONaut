use super::super::*;

pub(super) const CW_DECODE_WINDOW_SAMPLES: usize = 12_000 * 8;
pub(super) const CW_DECODE_MIN_SAMPLES: usize = 12_000 * 3;
pub(super) const CW_DECODE_HOP_SAMPLES: usize = 12_000;

fn is_no_signal_error(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    [
        "dominant frequency",
        "not enough audio",
        "not enough signal",
        "no power signal",
        "audio buffer is empty",
        "power signal is empty",
    ]
    .iter()
    .any(|needle| message.contains(needle))
}

fn normalize_cw_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn incremental_cw_suffix(previous: &str, current: &str) -> String {
    let previous = normalize_cw_text(previous);
    let current = normalize_cw_text(current);
    if current.is_empty() || previous == current || previous.contains(&current) {
        return String::new();
    }
    if previous.is_empty() {
        return current;
    }
    if let Some(remainder) = current.strip_prefix(&previous) {
        return remainder.trim().to_string();
    }

    let max_overlap = previous.len().min(current.len());
    for overlap in (2..=max_overlap).rev() {
        if previous.ends_with(&current[..overlap]) {
            return current[overlap..].trim().to_string();
        }
    }
    current
}

pub(super) fn run_cw_decode(
    samples: Vec<f32>,
    window_id: u64,
    utc: String,
    tone_hz: u32,
    state: Arc<Mutex<GuiState>>,
) {
    let result = ditdah::decode_samples(&samples, 12_000);
    let mut shared = state.lock().expect("ui state lock poisoned");
    match result {
        Ok(text) if !text.trim().is_empty() => {
            let text = normalize_cw_text(&text);
            let incremental = incremental_cw_suffix(&shared.cw_last_window_text, &text);
            shared.cw_live_text = text.clone();
            shared.cw_last_window_text = text;
            if !incremental.is_empty() {
                shared.digital_decodes.push_back(DigitalDecodeEntry {
                    mode: WorkspaceMode::Cw,
                    period: window_id,
                    utc,
                    snr_db: 0.0,
                    dt_s: 0.0,
                    freq_hz: tone_hz,
                    message: incremental,
                });
                while shared.digital_decodes.len() > 300 {
                    shared.digital_decodes.pop_front();
                }
            }
            shared.digital_decode_status = format!(
                "LIVE CW · rolling 8 s context · {} characters",
                shared.cw_live_text.len()
            );
        }
        Ok(_) => {
            shared.digital_decode_status = "LIVE: no CW decoded in latest window".to_string();
        }
        Err(error) if is_no_signal_error(&error.to_string()) => {
            shared.digital_decode_status = "LIVE: no CW signal in latest window".to_string();
        }
        Err(error) => {
            shared.digital_decode_status = format!("CW decode error: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_expected_quiet_window_errors() {
        assert!(is_no_signal_error(
            "Could not find a dominant frequency in the specified range."
        ));
        assert!(is_no_signal_error(
            "Not enough audio data for pitch detection"
        ));
        assert!(!is_no_signal_error("resampler configuration failed"));
    }

    #[test]
    fn extracts_only_new_text_from_overlapping_cw_windows() {
        assert_eq!(incremental_cw_suffix("", "CQ CQ"), "CQ CQ");
        assert_eq!(incremental_cw_suffix("CQ CQ", "CQ CQ DE W1AW"), "DE W1AW");
        assert_eq!(incremental_cw_suffix("CQ CQ DE W1", "Q DE W1AW"), "AW");
        assert_eq!(incremental_cw_suffix("CQ CQ", "CQ CQ"), "");
        assert_eq!(incremental_cw_suffix("CQ CQ DE W1AW", "DE W1"), "");
    }
}
