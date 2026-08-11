// Polyphase decimator: 48 kHz → 12 kHz (factor 4).
// Uses a windowed-sinc low-pass FIR (48-tap, Kaiser window, α=5).

use std::sync::OnceLock;

const FACTOR: usize = 4;
const NTAPS: usize = 48;

static FIR: OnceLock<[f32; NTAPS]> = OnceLock::new();

fn fir_coeffs() -> &'static [f32; NTAPS] {
    FIR.get_or_init(|| {
        use std::f64::consts::PI;
        const FC: f64 = 0.25; // normalised cutoff
        const ALPHA: f64 = 5.0;
        let m = (NTAPS - 1) as f64;
        let i0a = bessel_i0(ALPHA);
        let mut h = [0f32; NTAPS];
        for k in 0..NTAPS {
            let n = k as f64 - m / 2.0;
            let sinc = if n == 0.0 {
                2.0 * FC
            } else {
                (2.0 * FC * PI * n).sin() / (PI * n)
            };
            let t = 2.0 * k as f64 / m - 1.0;
            let arg = ALPHA * (1.0 - t * t).max(0.0).sqrt();
            let w = bessel_i0(arg) / i0a;
            h[k] = (sinc * w) as f32;
        }
        h
    })
}

fn bessel_i0(x: f64) -> f64 {
    let mut sum = 1.0f64;
    let mut term = 1.0f64;
    let hx = x / 2.0;
    for k in 1u32..=25 {
        term *= hx / k as f64;
        term *= hx / k as f64;
        sum += term;
        if term < 1e-15 { break; }
    }
    sum
}

/// Decimator state — holds the FIR delay line.
pub struct Decimator {
    buf: Vec<f32>,
    pos: usize,
    phase: usize,
    input_rate: u32,
}

impl Decimator {
    pub fn new(input_rate: u32) -> Self {
        assert_eq!(input_rate % 12_000, 0, "input rate must be a multiple of 12 kHz");
        let factor = (input_rate / 12_000) as usize;
        assert_eq!(factor, FACTOR, "only 4× decimation (48→12 kHz) is implemented");
        Self { buf: vec![0.0; NTAPS], pos: 0, phase: 0, input_rate }
    }

    pub fn output_rate(&self) -> u32 { 12_000 }

    /// Process a chunk of input samples, returning one output sample per FACTOR inputs.
    pub fn process(&mut self, input: &[f32]) -> Vec<f32> {
        let mut out = Vec::with_capacity(input.len() / FACTOR + 1);
        for &x in input {
            self.buf[self.pos] = x;
            self.pos = (self.pos + 1) % NTAPS;
            if self.phase == FACTOR - 1 {
                let y: f32 = (0..NTAPS)
                    .map(|k| {
                        let idx = (self.pos + k) % NTAPS;
                        fir_coeffs()[k] * self.buf[idx]
                    })
                    .sum();
                out.push(y);
            }
            self.phase = (self.phase + 1) % FACTOR;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decimator_length() {
        let mut dec = Decimator::new(48_000);
        let input = vec![0.0f32; 48_000]; // 1 second at 48 kHz
        let out = dec.process(&input);
        assert_eq!(out.len(), 12_000, "1 s of 48 kHz → 12 kHz samples");
    }

    #[test]
    fn decimator_chunked_matches_one_shot() {
        let input: Vec<f32> = (0..96_000)
            .map(|n| {
                // Mixed tones + slight envelope to avoid trivial periodic alias.
                let t = n as f32 / 48_000.0;
                (2.0 * std::f32::consts::PI * 700.0 * t).sin() * (0.7 + 0.3 * (2.0 * std::f32::consts::PI * 2.0 * t).sin())
                    + 0.4 * (2.0 * std::f32::consts::PI * 1500.0 * t).sin()
            })
            .collect();

        let mut one = Decimator::new(48_000);
        let out_one = one.process(&input);

        let mut chunked = Decimator::new(48_000);
        let mut out_chunked = Vec::new();
        for chunk in input.chunks(1379) {
            out_chunked.extend(chunked.process(chunk));
        }

        assert_eq!(out_one.len(), out_chunked.len());
        for (a, b) in out_one.iter().zip(out_chunked.iter()) {
            assert!((a - b).abs() < 1e-6, "chunked decimator mismatch: {a} vs {b}");
        }
    }
}
