// Polyphase decimator: 48 kHz → 12 kHz (factor 4).
// Uses a windowed-sinc low-pass FIR (48-tap, Kaiser window, α=5).

use std::sync::OnceLock;

const FACTOR: usize = 4;
const NTAPS: usize = 48;

static FIR: OnceLock<[f32; NTAPS]> = OnceLock::new();

fn fir_coeffs() -> &'static [f32; NTAPS] {
    FIR.get_or_init(|| {
        use std::f64::consts::PI;
        const FC: f64 = 0.25;
        const ALPHA: f64 = 5.0;
        let m = (NTAPS - 1) as f64;
        let i0a = bessel_i0(ALPHA);
        let mut h = [0f32; NTAPS];
        for (k, hk) in h.iter_mut().enumerate().take(NTAPS) {
            let n = k as f64 - m / 2.0;
            let sinc = if n == 0.0 {
                2.0 * FC
            } else {
                (2.0 * FC * PI * n).sin() / (PI * n)
            };
            let t = 2.0 * k as f64 / m - 1.0;
            let arg = ALPHA * (1.0 - t * t).max(0.0).sqrt();
            let w = bessel_i0(arg) / i0a;
            *hk = (sinc * w) as f32;
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
        if term < 1e-15 {
            break;
        }
    }
    sum
}

/// Stateful high-quality 48 kHz → 12 kHz FIR decimator.
pub struct Decimator {
    buf: Vec<f32>,
    pos: usize,
    phase: usize,
}

impl Decimator {
    pub fn new(input_rate: u32) -> Self {
        assert_eq!(
            input_rate % 12_000,
            0,
            "input rate must be a multiple of 12 kHz"
        );
        let factor = (input_rate / 12_000) as usize;
        assert_eq!(
            factor, FACTOR,
            "only 4× decimation (48→12 kHz) is implemented"
        );
        Self {
            buf: vec![0.0; NTAPS],
            pos: 0,
            phase: 0,
        }
    }

    pub fn output_rate(&self) -> u32 {
        12_000
    }

    pub fn process(&mut self, input: &[f32]) -> Vec<f32> {
        let mut out = Vec::with_capacity(input.len() / FACTOR + 1);
        for &sample in input {
            self.buf[self.pos] = sample;
            self.pos = (self.pos + 1) % NTAPS;
            if self.phase == FACTOR - 1 {
                let value = (0..NTAPS)
                    .map(|index| {
                        let buffer_index = (self.pos + index) % NTAPS;
                        fir_coeffs()[index] * self.buf[buffer_index]
                    })
                    .sum();
                out.push(value);
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
        let mut decimator = Decimator::new(48_000);
        let output = decimator.process(&vec![0.0; 48_000]);
        assert_eq!(output.len(), 12_000);
    }

    #[test]
    fn decimator_chunked_matches_one_shot() {
        let input: Vec<f32> = (0..96_000)
            .map(|sample| {
                let time = sample as f32 / 48_000.0;
                (2.0 * std::f32::consts::PI * 700.0 * time).sin()
                    * (0.7 + 0.3 * (2.0 * std::f32::consts::PI * 2.0 * time).sin())
                    + 0.4 * (2.0 * std::f32::consts::PI * 1_500.0 * time).sin()
            })
            .collect();

        let mut one_shot = Decimator::new(48_000);
        let expected = one_shot.process(&input);
        let mut chunked = Decimator::new(48_000);
        let actual: Vec<f32> = input
            .chunks(1_379)
            .flat_map(|chunk| chunked.process(chunk))
            .collect();

        assert_eq!(expected.len(), actual.len());
        assert!(expected
            .iter()
            .zip(actual.iter())
            .all(|(left, right)| (left - right).abs() < 1e-6));
    }
}
