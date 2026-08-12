// FT8 sync detector: scan a 15-second 12 kHz buffer for Costas-array candidates.
//
// Algorithm follows WSJTX sync8.f90:
//   1. Compute short-time FFT spectra stepping by NSTEP = NSPS/4 = 480 samples.
//   2. For each (frequency bin, time lag) position, correlate the three Costas
//      7×7 arrays embedded in the 79-symbol sequence.
//   3. Return candidate list sorted by sync strength.

use qsonaut_modes::ft8::params::{COSTAS, SAMPLES_PER_SYM, SAMPLE_RATE};
use rustfft::{num_complex::Complex, FftPlanner};

pub const NSTEP: usize = SAMPLES_PER_SYM / 4; // 480 — quarter-symbol steps
pub const NFFT: usize = 2 * SAMPLES_PER_SYM; // 3840 — FFT size
pub const NH: usize = NFFT / 2; // 1920 useful bins
pub const NMAX: usize = (15 * SAMPLE_RATE) as usize; // 180 000 samples
pub const NHSYM: usize = NMAX / NSTEP - 3; // ≈ 372 time steps

// Number of fractional-symbol steps per symbol (= 4)
pub const NSSY: usize = SAMPLES_PER_SYM / NSTEP;
// Number of frequency-bin oversampling steps per tone (= 2 with NFFT=2×NSPS)
pub const NFOS: usize = NFFT / SAMPLES_PER_SYM;

/// A sync candidate: (audio_freq_hz, time_offset_s, sync_score).
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub freq_hz: f32,
    pub time_offset_s: f32,
    pub sync: f32,
}

/// Compute per-step power spectra from a 15-second 12 kHz input buffer.
/// Returns `s[bin][step]` — only the NH/2 lower bins are populated.
fn compute_spectra(samples: &[f32]) -> Vec<Vec<f32>> {
    let n = samples.len().min(NMAX);
    let n_steps = n / NSTEP;
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(NFFT);
    let mut buf = vec![Complex::new(0.0f32, 0.0); NFFT];

    // s[freq_bin][time_step]
    let mut s = vec![vec![0f32; n_steps]; NH];

    let fac = 1.0 / 300.0;
    for step in 0..n_steps {
        let ia = step * NSTEP;
        let ib = (ia + SAMPLES_PER_SYM).min(n);
        for (k, b) in buf.iter_mut().enumerate() {
            *b = if k < (ib - ia) {
                Complex::new(fac * samples[ia + k], 0.0)
            } else {
                Complex::new(0.0, 0.0)
            };
        }
        fft.process(&mut buf);
        for i in 1..NH {
            s[i][step] = buf[i].norm_sqr();
        }
    }
    s
}

/// Correlation of the three Costas arrays at a given (freq_bin, lag_step).
fn costas_correlation(s: &[Vec<f32>], freq_bin: usize, lag: i32, n_steps: i32) -> f32 {
    let jstrt = (0.5 / (NSTEP as f64 / SAMPLE_RATE as f64)) as i32; // ≈ 12 steps
    let nssy = NSSY as i32;
    let mut t = 0.0f32;
    let mut t0 = 0.0f32;

    for (block, joffset) in [(0, 0i32), (1, 36 * nssy), (2, 72 * nssy)] {
        let _ = block;
        for n in 0..7i32 {
            let m = lag + jstrt + nssy * n + joffset;
            if m >= 0 && m < n_steps {
                let m = m as usize;
                let tone_bin = freq_bin + NFOS * COSTAS[n as usize] as usize;
                if tone_bin < NH {
                    t += s[tone_bin].get(m).copied().unwrap_or(0.0);
                }
                for j in 0..7usize {
                    let b = freq_bin + NFOS * j;
                    if b < NH {
                        t0 += s[b].get(m).copied().unwrap_or(0.0);
                    }
                }
            }
        }
    }
    let t0 = (t0 - t) / 6.0;
    if t0 > 0.0 {
        t / t0
    } else {
        0.0
    }
}

/// Scan `samples` (12 kHz, 15-second window) for FT8 candidates.
///
/// `nfa` and `nfb` are the search frequency bounds in Hz.
pub fn find_candidates(samples: &[f32], nfa: f32, nfb: f32, min_sync: f32) -> Vec<Candidate> {
    let n_steps = (samples.len().min(NMAX) / NSTEP) as i32;
    let s = compute_spectra(samples);

    let df = SAMPLE_RATE as f32 / NFFT as f32; // ≈ 3.125 Hz
    let ia = (nfa / df).round() as usize;
    let ib = ((nfb / df).round() as usize).min(NH - 6);
    let tstep = NSTEP as f32 / SAMPLE_RATE as f32;

    const JZ: i32 = 62; // ±2.5 s search range
    let mut best: Vec<(usize, i32, f32)> = Vec::new(); // (bin, lag, score)

    for i in ia..=ib {
        let mut peak_score = 0.0f32;
        let mut peak_lag = 0i32;
        for lag in -JZ..=JZ {
            let score = costas_correlation(&s, i, lag, n_steps);
            if score > peak_score {
                peak_score = score;
                peak_lag = lag;
            }
        }
        if peak_score >= min_sync {
            best.push((i, peak_lag, peak_score));
        }
    }

    // Sort by score descending.
    best.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));

    // Deduplicate within 4 Hz / 40 ms.
    let mut cands: Vec<Candidate> = Vec::new();
    'outer: for &(i, lag, score) in &best {
        let freq = i as f32 * df;
        let t = (lag as f32 - 0.5) * tstep;
        for c in &cands {
            if (freq - c.freq_hz).abs() < 4.0 && (t - c.time_offset_s).abs() < 0.04 {
                continue 'outer;
            }
        }
        cands.push(Candidate {
            freq_hz: freq,
            time_offset_s: t,
            sync: score,
        });
    }
    cands
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_buffer_gives_no_candidates() {
        let samples = vec![0f32; NMAX];
        let cands = find_candidates(&samples, 200.0, 2800.0, 2.0);
        assert!(cands.is_empty() || cands.iter().all(|c| c.sync < 5.0));
    }
}
