use super::super::*;

pub(super) const CW_DECODE_WINDOW_SAMPLES: usize = 12_000 * 8;
pub(super) const CW_DECODE_MIN_SAMPLES: usize = 12_000 * 3;
pub(super) const CW_DECODE_HOP_SAMPLES: usize = 12_000;

const CW_FILTER_BANDWIDTH_HZ: f32 = 120.0;
const CW_MIN_PEAK_DBFS: f32 = -65.0;
const CW_MIN_KEYING_CONTRAST_DB: f32 = 9.0;
const CW_MIN_TONE_SHARE: f32 = 0.10;

#[derive(Debug, Clone, Copy)]
pub(crate) struct CwSignalQuality {
    peak_dbfs: f32,
    keying_contrast_db: f32,
    tone_share: f32,
    active_fraction: f32,
}

fn block_rms(samples: &[f32], block_len: usize) -> Vec<f32> {
    samples
        .chunks_exact(block_len.max(1))
        .map(|block| {
            (block.iter().map(|sample| sample * sample).sum::<f32>() / block.len() as f32)
                .sqrt()
        })
        .collect()
}

fn percentile(sorted: &[f32], fraction: f32) -> f32 {
    let index = ((sorted.len().saturating_sub(1)) as f32 * fraction)
        .round()
        .clamp(0.0, sorted.len().saturating_sub(1) as f32) as usize;
    sorted.get(index).copied().unwrap_or_default()
}

fn narrow_cw_bandpass(samples: &[f32], sample_rate: u32, tone_hz: u32) -> Vec<f32> {
    let sample_rate = sample_rate.max(1) as f32;
    let tone_hz = (tone_hz as f32).clamp(200.0, sample_rate * 0.45);
    let q = (tone_hz / CW_FILTER_BANDWIDTH_HZ).max(0.5);
    let omega = 2.0 * PI * tone_hz / sample_rate;
    let alpha = omega.sin() / (2.0 * q);
    let a0 = 1.0 + alpha;
    let b0 = alpha / a0;
    let b1 = 0.0;
    let b2 = -alpha / a0;
    let a1 = -2.0 * omega.cos() / a0;
    let a2 = (1.0 - alpha) / a0;
    let mut x1 = 0.0_f32;
    let mut x2 = 0.0_f32;
    let mut y1 = 0.0_f32;
    let mut y2 = 0.0_f32;
    samples
        .iter()
        .map(|&x0| {
            let y0 = b0 * x0 + b1 * x1 + b2 * x2 - a1 * y1 - a2 * y2;
            x2 = x1;
            x1 = x0;
            y2 = y1;
            y1 = y0;
            y0
        })
        .collect()
}

pub(crate) fn prepare_cw_signal(
    samples: &[f32],
    sample_rate: u32,
    tone_hz: u32,
) -> Result<(Vec<f32>, CwSignalQuality), String> {
    let filtered = narrow_cw_bandpass(samples, sample_rate, tone_hz);
    let block_len = (sample_rate as usize / 50).max(32);
    let mut tone_levels = block_rms(&filtered, block_len);
    let mut broadband_levels = block_rms(samples, block_len);
    if tone_levels.len() < 20 || broadband_levels.len() != tone_levels.len() {
        return Err("waiting for enough CW audio".to_string());
    }
    tone_levels.sort_unstable_by(|left, right| left.total_cmp(right));
    broadband_levels.sort_unstable_by(|left, right| left.total_cmp(right));
    let low = percentile(&tone_levels, 0.20).max(1e-9);
    let high = percentile(&tone_levels, 0.90).max(low);
    let broadband_high = percentile(&broadband_levels, 0.90).max(1e-9);
    let threshold = (low * high).sqrt();
    let active_fraction = tone_levels
        .iter()
        .filter(|level| **level > threshold)
        .count() as f32
        / tone_levels.len() as f32;
    let quality = CwSignalQuality {
        peak_dbfs: 20.0 * high.log10(),
        keying_contrast_db: 20.0 * (high / low).log10(),
        tone_share: high / broadband_high,
        active_fraction,
    };
    if quality.peak_dbfs < CW_MIN_PEAK_DBFS {
        return Err(format!("selected tone below {:.0} dBFS", CW_MIN_PEAK_DBFS));
    }
    if quality.tone_share < CW_MIN_TONE_SHARE {
        return Err("no dominant signal near selected CW tone".to_string());
    }
    if quality.keying_contrast_db < CW_MIN_KEYING_CONTRAST_DB
        || !(0.03..=0.85).contains(&quality.active_fraction)
    {
        return Err("tone is not showing a CW keying envelope".to_string());
    }
    Ok((filtered, quality))
}

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
    let (samples, quality) = match prepare_cw_signal(&samples, 12_000, tone_hz) {
        Ok(prepared) => prepared,
        Err(reason) => {
            state
                .lock()
                .expect("ui state lock poisoned")
                .digital_decode_status = format!("LIVE CW · {reason}");
            return;
        }
    };
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
                "LIVE CW · {:.0} dBFS · {:.0} dB keying · {} characters",
                quality.peak_dbfs,
                quality.keying_contrast_db,
                shared.cw_live_text.len(),
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

    fn keyed_tone(sample_rate: u32, tone_hz: f32, seconds: f32) -> Vec<f32> {
        let len = (sample_rate as f32 * seconds) as usize;
        let dot_samples = (sample_rate as f32 * 0.06) as usize;
        (0..len)
            .map(|index| {
                let keyed = (index / dot_samples) % 2 == 0;
                if keyed {
                    (2.0 * PI * tone_hz * index as f32 / sample_rate as f32).sin() * 0.25
                } else {
                    0.0
                }
            })
            .collect()
    }

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

    #[test]
    fn accepts_keyed_tone_at_selected_cw_pitch() {
        let samples = keyed_tone(12_000, 600.0, 3.0);
        let (_, quality) = prepare_cw_signal(&samples, 12_000, 600).unwrap();
        assert!(quality.keying_contrast_db >= CW_MIN_KEYING_CONTRAST_DB);
        assert!(quality.tone_share >= CW_MIN_TONE_SHARE);
    }

    #[test]
    fn rejects_unkeyed_carrier_and_broadband_noise() {
        let carrier = (0..36_000)
            .map(|index| (2.0 * PI * 600.0 * index as f32 / 12_000.0).sin() * 0.25)
            .collect::<Vec<_>>();
        assert!(prepare_cw_signal(&carrier, 12_000, 600).is_err());

        let mut seed = 0x1234_5678_u32;
        let noise = (0..36_000)
            .map(|_| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (seed as f32 / u32::MAX as f32 - 0.5) * 0.2
            })
            .collect::<Vec<_>>();
        assert!(prepare_cw_signal(&noise, 12_000, 600).is_err());
    }
}
