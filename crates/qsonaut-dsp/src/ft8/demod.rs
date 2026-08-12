// FT8 soft-symbol extractor: given 12 kHz samples and a sync candidate,
// compute 8-tone magnitudes for all 79 symbols.
//
// Key insight: FFT bin spacing = 12000/1920 = 6.25 Hz = exactly FT8 tone spacing.
// So a 1920-point FFT on each symbol window produces bins that are 1-to-1 with tones.
// No baseband mixing needed — just read 8 consecutive bins starting at floor(f0/6.25).

use qsonaut_modes::ft8::params::{N, N_SYM, SAMPLES_PER_SYM, SAMPLE_RATE};
use rustfft::{num_complex::Complex, FftPlanner};
use std::f32::consts::PI;

const FS: f32 = SAMPLE_RATE as f32;
// Bin spacing with 1920-point FFT at 12 kHz: exactly 6.25 Hz = one FT8 tone slot.
const BIN_HZ: f32 = FS / SAMPLES_PER_SYM as f32; // 6.25

fn interp_power(spec: &[Complex<f32>], bin_pos: f32) -> f32 {
    let max_bin = (spec.len() / 2).saturating_sub(2) as f32;
    if max_bin <= 1.0 {
        return 0.0;
    }
    let p = bin_pos.clamp(1.0, max_bin);
    let k0 = p.floor() as usize;
    let k1 = (k0 + 1).min(spec.len() / 2 - 1);
    let t = p - k0 as f32;
    let a = spec[k0].norm_sqr();
    let b = spec[k1].norm_sqr();
    a + (b - a) * t
}

/// Extract per-symbol 8-tone magnitudes for a given sync candidate.
///
/// `samples`  — 12 kHz, 15-second window (up to 180 000 samples).
/// `f0_hz`    — base frequency of the signal (lowest tone).
/// `t0_s`     — timing offset from period start returned by the sync search.
///
/// Returns `s8[sym][tone]` power values for all 79 symbols.
pub fn extract_symbol_magnitudes(samples: &[f32], f0_hz: f32, t0_s: f32) -> [[f32; 8]; N_SYM] {
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(SAMPLES_PER_SYM);
    let mut fbuf = vec![Complex::new(0.0f32, 0.0f32); SAMPLES_PER_SYM];

    // FT8 transmissions start 0.5 s into the 15-second period.
    let i0 = ((t0_s + 0.5) * FS).round() as i64;

    // First tone falls near this (possibly fractional) FFT bin; subsequent tones
    // are exactly one bin apart at FT8 tone spacing.
    let tone0_bin = f0_hz / BIN_HZ;

    let n = samples.len();
    let mut s8 = [[0f32; 8]; N_SYM];

    for sym in 0..N_SYM {
        let start = (i0 + (sym * SAMPLES_PER_SYM) as i64).max(0) as usize;

        for (k, b) in fbuf.iter_mut().enumerate() {
            let idx = start + k;
            let x = if idx < n { samples[idx] } else { 0.0 };
            let w = 0.5 - 0.5 * (2.0 * PI * k as f32 / (SAMPLES_PER_SYM - 1) as f32).cos();
            *b = Complex::new(x * w, 0.0);
        }

        fft.process(&mut fbuf);

        for tone in 0..8 {
            let bin_pos = tone0_bin + tone as f32;
            s8[sym][tone] = interp_power(&fbuf, bin_pos);
        }
    }

    s8
}

/// Full pipeline for one candidate: extract magnitudes → channel LLRs.
pub fn demodulate(samples: &[f32], f0_hz: f32, t0_s: f32) -> [f32; N] {
    use qsonaut_modes::ft8::decode::symbol_magnitudes_to_llr;
    let s8 = extract_symbol_magnitudes(samples, f0_hz, t0_s);
    symbol_magnitudes_to_llr(&s8)
}

#[cfg(test)]
mod tests {
    use super::*;
    use qsonaut_modes::ft8::decode::decode_llr;
    use qsonaut_modes::ft8::{encode::message_to_tones, pack77::Ft8Message};

    /// Synthesise a clean FT8 signal, demodulate it, and check decoding.
    #[test]
    fn synthesise_and_decode() {
        let msg = Ft8Message::Standard {
            call1: "W1AW".into(),
            call2: "K1JT".into(),
            report: "FN20".into(),
            ir: false,
        };
        let tones = message_to_tones(&msg).expect("encode");
        let f0: f32 = 1_500.0; // 1.5 kHz baseband
        let fs: f32 = FS;
        let dt = 1.0 / fs;
        let tone_spacing = 6.25f32;
        let _sym_dur = SAMPLES_PER_SYM as f32 / fs;

        // Synthesise clean FSK waveform at 12 kHz.
        let mut wave = vec![0.0f32; 15 * SAMPLE_RATE as usize];
        let start = (0.5 * fs) as usize; // FT8 starts 0.5 s into the period
        for (sym_idx, &tone) in tones.iter().enumerate() {
            let freq = f0 + tone as f32 * tone_spacing;
            let twopi = 2.0 * std::f32::consts::PI;
            for k in 0..SAMPLES_PER_SYM {
                let t = (start + sym_idx * SAMPLES_PER_SYM + k) as f32 * dt;
                let idx = start + sym_idx * SAMPLES_PER_SYM + k;
                if idx < wave.len() {
                    wave[idx] += (twopi * freq * t).sin();
                }
            }
        }

        let llr = demodulate(&wave, f0, 0.0);
        let result = decode_llr(&llr);
        // With a perfect noiseless signal the decoder must succeed.
        assert!(result.is_some(), "decode failed on synthesised signal");
        if let Some(d) = result {
            assert_eq!(d.message.to_string(), msg.to_string());
        }
    }

    #[test]
    fn synthesise_and_decode_with_fractional_base_freq() {
        let msg = Ft8Message::Standard {
            call1: "W1AW".into(),
            call2: "K1JT".into(),
            report: "FN20".into(),
            ir: false,
        };
        let tones = message_to_tones(&msg).expect("encode");
        let f0: f32 = 1_501.8; // intentionally off-bin for 6.25 Hz grid
        let fs: f32 = FS;
        let dt = 1.0 / fs;
        let tone_spacing = 6.25f32;

        let mut wave = vec![0.0f32; 15 * SAMPLE_RATE as usize];
        let start = (0.5 * fs) as usize;
        for (sym_idx, &tone) in tones.iter().enumerate() {
            let freq = f0 + tone as f32 * tone_spacing;
            let omega = 2.0 * std::f32::consts::PI * freq;
            for k in 0..SAMPLES_PER_SYM {
                let t = (start + sym_idx * SAMPLES_PER_SYM + k) as f32 * dt;
                let idx = start + sym_idx * SAMPLES_PER_SYM + k;
                if idx < wave.len() {
                    wave[idx] += (omega * t).sin();
                }
            }
        }

        let llr = demodulate(&wave, f0, 0.0);
        let result = decode_llr(&llr);
        assert!(result.is_some(), "decode failed on fractional-bin signal");
        if let Some(d) = result {
            assert_eq!(d.message.to_string(), msg.to_string());
        }
    }
}
