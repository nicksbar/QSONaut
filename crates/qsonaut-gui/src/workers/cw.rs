use super::super::*;

pub(super) const CW_DECODE_WINDOW_SAMPLES: usize = 12_000 * 8;

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
            let text = text.trim().to_string();
            shared.digital_decodes.push_back(DigitalDecodeEntry {
                mode: WorkspaceMode::Cw,
                period: window_id,
                utc,
                snr_db: 0.0,
                dt_s: 0.0,
                freq_hz: tone_hz,
                message: text.clone(),
            });
            while shared.digital_decodes.len() > 300 {
                shared.digital_decodes.pop_front();
            }
            shared.digital_decode_status = format!("LIVE: CW decoded {} characters", text.len());
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
}
