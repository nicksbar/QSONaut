// Polyphase decimator: 48 kHz → 12 kHz (factor 4).
// Uses a windowed-sinc low-pass FIR (48-tap, Kaiser window, α=5).

use std::sync::OnceLock;

use std::collections::VecDeque;

const SINC_HALF_TAPS: isize = 16;
const SINC_TAPS: usize = (SINC_HALF_TAPS * 2) as usize;
const SINC_PHASES: usize = 1_024;

/// Stateful mono sample-rate converter using a windowed-sinc low-pass filter.
///
/// The converter retains history and look-ahead across callback boundaries so
/// device buffer sizes do not affect the resulting canonical stream.
pub struct BandlimitedResampler {
    input_rate_hz: u32,
    output_rate_hz: u32,
    source_position_numerator: u64,
    source: VecDeque<f32>,
    kernels: Box<[[f32; SINC_TAPS]]>,
}

impl BandlimitedResampler {
    pub fn new(input_rate_hz: u32, output_rate_hz: u32) -> Self {
        assert!(input_rate_hz > 0 && output_rate_hz > 0);
        let mut source = VecDeque::with_capacity((SINC_HALF_TAPS * 4) as usize);
        source.resize(SINC_HALF_TAPS as usize, 0.0);
        let cutoff = (output_rate_hz as f64 / input_rate_hz as f64).min(1.0) * 0.94;
        Self {
            input_rate_hz,
            output_rate_hz,
            source_position_numerator: SINC_HALF_TAPS as u64 * u64::from(output_rate_hz),
            source,
            kernels: build_sinc_kernels(cutoff),
        }
    }

    pub fn process(&mut self, samples: &[f32]) -> Vec<f32> {
        if samples.is_empty() {
            return Vec::new();
        }
        if self.input_rate_hz == self.output_rate_hz {
            return samples.to_vec();
        }

        self.source.extend(samples.iter().copied());
        let mut output = Vec::with_capacity(
            ((samples.len() as f64 * self.output_rate_hz as f64 / self.input_rate_hz as f64).ceil()
                as usize)
                .saturating_add(1),
        );

        while (self.source_position_numerator / u64::from(self.output_rate_hz)) as usize
            + (SINC_HALF_TAPS as usize)
            < self.source.len()
        {
            let center = (self.source_position_numerator / u64::from(self.output_rate_hz)) as isize;
            let fraction_numerator =
                self.source_position_numerator % u64::from(self.output_rate_hz);
            let phase = ((fraction_numerator * SINC_PHASES as u64
                + u64::from(self.output_rate_hz) / 2)
                / u64::from(self.output_rate_hz))
            .min((SINC_PHASES - 1) as u64) as usize;
            let mut value = 0.0_f32;
            for (tap, offset) in ((-SINC_HALF_TAPS + 1)..=SINC_HALF_TAPS).enumerate() {
                let index = center + offset;
                value += self.source[index as usize] * self.kernels[phase][tap];
            }
            output.push(value);
            self.source_position_numerator += u64::from(self.input_rate_hz);
        }

        let consumed = (self.source_position_numerator / u64::from(self.output_rate_hz)) as usize;
        let consumed = consumed.saturating_sub(SINC_HALF_TAPS as usize);
        if consumed > 0 {
            self.source.drain(..consumed.min(self.source.len()));
            self.source_position_numerator -= consumed as u64 * u64::from(self.output_rate_hz);
        }
        output
    }
}

fn build_sinc_kernels(cutoff: f64) -> Box<[[f32; SINC_TAPS]]> {
    (0..SINC_PHASES)
        .map(|phase| {
            let fraction = phase as f64 / SINC_PHASES as f64;
            let mut kernel = [0.0_f32; SINC_TAPS];
            let mut sum = 0.0_f64;
            for (tap, offset) in ((-SINC_HALF_TAPS + 1)..=SINC_HALF_TAPS).enumerate() {
                let distance = offset as f64 - fraction;
                let normalized = distance / SINC_HALF_TAPS as f64;
                let window = 0.42
                    + 0.5 * (std::f64::consts::PI * normalized).cos()
                    + 0.08 * (2.0 * std::f64::consts::PI * normalized).cos();
                let argument = cutoff * distance;
                let sinc = if argument.abs() < f64::EPSILON {
                    1.0
                } else {
                    (std::f64::consts::PI * argument).sin() / (std::f64::consts::PI * argument)
                };
                let weight = cutoff * sinc * window;
                kernel[tap] = weight as f32;
                sum += weight;
            }
            if sum.abs() > f64::EPSILON {
                for weight in &mut kernel {
                    *weight /= sum as f32;
                }
            }
            kernel
        })
        .collect::<Vec<_>>()
        .into_boxed_slice()
}

pub fn resample_f32(samples: &[f32], input_rate_hz: u32, output_rate_hz: u32) -> Vec<f32> {
    if samples.is_empty() || input_rate_hz == output_rate_hz {
        return samples.to_vec();
    }
    let expected_len = ((samples.len() as f64 * output_rate_hz as f64 / input_rate_hz as f64)
        .round() as usize)
        .max(1);
    let mut resampler = BandlimitedResampler::new(input_rate_hz, output_rate_hz);
    let mut output = resampler.process(samples);
    output.extend(resampler.process(&vec![0.0; (SINC_HALF_TAPS * 2) as usize]));
    output.truncate(expected_len);
    output
}

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

    #[test]
    fn bandlimited_resampler_is_independent_of_callback_boundaries() {
        let input = (0..44_100)
            .map(|sample| (2.0 * std::f32::consts::PI * 1_000.0 * sample as f32 / 44_100.0).sin())
            .collect::<Vec<_>>();
        let mut one_chunk = BandlimitedResampler::new(44_100, 48_000);
        let expected = one_chunk.process(&input);
        let mut chunked = BandlimitedResampler::new(44_100, 48_000);
        let actual = input
            .chunks(317)
            .flat_map(|chunk| chunked.process(chunk))
            .collect::<Vec<_>>();
        assert_eq!(actual.len(), expected.len());
        assert!(actual
            .iter()
            .zip(expected.iter())
            .all(|(left, right)| (left - right).abs() < 1e-6));
    }

    #[test]
    fn one_shot_resampler_produces_the_requested_rate_length() {
        let input = vec![0.25; 44_100];
        let output = resample_f32(&input, 44_100, 48_000);
        assert_eq!(output.len(), 48_000);
        assert!(output[100..47_900]
            .iter()
            .all(|sample| (*sample - 0.25).abs() < 1e-4));
    }

    #[test]
    fn downsampling_rejects_audio_above_the_output_nyquist_limit() {
        let input = (0..96_000)
            .map(|sample| (2.0 * std::f32::consts::PI * 30_000.0 * sample as f32 / 96_000.0).sin())
            .collect::<Vec<_>>();
        let output = resample_f32(&input, 96_000, 48_000);
        let body = &output[500..output.len() - 500];
        let rms =
            (body.iter().map(|sample| sample * sample).sum::<f32>() / body.len() as f32).sqrt();
        assert!(rms < 0.05, "aliased out-of-band RMS was {rms}");
    }
}
