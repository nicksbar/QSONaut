//! Minimal, reusable analog SSTV modem support.
//!
//! The first supported format is Martin M1 (VIS 44), the common 320 x 256
//! RGB mode. Audio is mono 12 kHz PCM and follows the conventional
//! 1500 Hz black / 2300 Hz white mapping.

use std::f32::consts::TAU;

pub const SAMPLE_RATE_HZ: u32 = 12_000;
pub const WIDTH: usize = 320;
pub const HEIGHT: usize = 256;
pub const VIS_CODE_MARTIN_M1: u8 = 0x2c;

const LEADER_MS: f64 = 300.0;
const VIS_BREAK_MS: f64 = 10.0;
const VIS_BIT_MS: f64 = 30.0;
const SYNC_MS: f64 = 4.862;
const GAP_MS: f64 = 0.572;
const CHANNEL_MS: f64 = 146.432;
const LINE_MS: f64 = SYNC_MS + 4.0 * GAP_MS + 3.0 * CHANNEL_MS;
const HEADER_MS: f64 = LEADER_MS * 2.0 + VIS_BREAK_MS + VIS_BIT_MS * 10.0;
const IMAGE_MS: f64 = LINE_MS * HEIGHT as f64;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SstvError {
    #[error("Martin M1 needs exactly 320 x 256 RGB pixels")]
    InvalidImage,
    #[error("audio does not contain a complete Martin M1 image")]
    IncompleteAudio,
    #[error("VIS header is not Martin M1")]
    UnsupportedVis,
}

#[derive(Debug, Clone)]
pub struct DecodedImage {
    pub width: usize,
    pub height: usize,
    pub rgb: Vec<u8>,
}

/// Streaming receiver with VIS detection and bounded buffering.
#[derive(Debug, Default)]
pub struct MartinM1Receiver {
    buffer: Vec<f32>,
    image_start: Option<usize>,
    search_from: usize,
    last_vis: Option<u8>,
    frequency_offset_hz: Option<f32>,
}

impl MartinM1Receiver {
    pub fn reset(&mut self) {
        self.buffer.clear();
        self.image_start = None;
        self.search_from = 0;
        self.last_vis = None;
        self.frequency_offset_hz = None;
    }

    pub fn progress(&self) -> Option<f32> {
        let start = self.image_start?;
        Some(
            ((self.buffer.len().saturating_sub(start)) as f64 / ms_samples(IMAGE_MS) as f64)
                .clamp(0.0, 1.0) as f32,
        )
    }

    /// Most recent parity-valid VIS code observed since reset.
    pub fn detected_vis(&self) -> Option<u8> {
        self.last_vis
    }

    /// Frequency correction inferred from the 1900 Hz VIS leader.
    pub fn frequency_offset_hz(&self) -> Option<f32> {
        self.frequency_offset_hz
    }

    pub fn push(&mut self, samples: &[f32]) -> Option<DecodedImage> {
        self.buffer.extend_from_slice(samples);
        if self.image_start.is_none() {
            self.find_header();
        }
        if let Some(start) = self.image_start {
            let needed = start + ms_samples(IMAGE_MS);
            if self.buffer.len() >= needed {
                let decoded = decode_image_audio(
                    &self.buffer[start..needed],
                    self.frequency_offset_hz.unwrap_or(0.0),
                );
                self.reset();
                return decoded.ok();
            }
        } else {
            let keep = ms_samples(HEADER_MS + 400.0);
            if self.buffer.len() > keep {
                let drain = self.buffer.len() - keep;
                self.buffer.drain(..drain);
                self.search_from = self.search_from.saturating_sub(drain);
            }
        }
        None
    }

    fn find_header(&mut self) {
        let header = ms_samples(HEADER_MS);
        let step = ms_samples(10.0);
        while self.search_from + header <= self.buffer.len() {
            if let Some((vis, frequency_offset_hz)) =
                decode_vis_header(&self.buffer[self.search_from..self.search_from + header])
            {
                self.last_vis = Some(vis);
                self.frequency_offset_hz = Some(frequency_offset_hz);
                if vis == VIS_CODE_MARTIN_M1 {
                    self.image_start = Some(self.search_from + header);
                    return;
                }
                // Skip this complete unsupported header while retaining enough
                // trailing audio to find the next transmission.
                self.search_from += header;
                continue;
            }
            self.search_from += step;
        }
    }
}

pub fn encode_martin_m1(rgb: &[u8]) -> Result<Vec<i16>, SstvError> {
    encode_martin_m1_with_offset(rgb, 0.0)
}

fn encode_martin_m1_with_offset(
    rgb: &[u8],
    frequency_offset_hz: f64,
) -> Result<Vec<i16>, SstvError> {
    if rgb.len() != WIDTH * HEIGHT * 3 {
        return Err(SstvError::InvalidImage);
    }
    let total = ms_samples(HEADER_MS + IMAGE_MS) + 8;
    let mut out = Vec::with_capacity(total);
    let mut phase = 0.0_f64;
    let mut fractional_samples = 0.0_f64;
    let mut tone = |frequency: f64, duration_ms: f64, out: &mut Vec<i16>| {
        fractional_samples += duration_ms * SAMPLE_RATE_HZ as f64 / 1000.0;
        let count = fractional_samples.floor() as usize;
        fractional_samples -= count as f64;
        let step =
            std::f64::consts::TAU * (frequency + frequency_offset_hz) / SAMPLE_RATE_HZ as f64;
        for _ in 0..count {
            out.push((phase.sin() * 18_000.0).round() as i16);
            phase = (phase + step) % std::f64::consts::TAU;
        }
    };

    append_vis_header(VIS_CODE_MARTIN_M1, &mut tone, &mut out);

    for y in 0..HEIGHT {
        tone(1200.0, SYNC_MS, &mut out);
        tone(1500.0, GAP_MS, &mut out);
        for channel in [1_usize, 2, 0] {
            for x in 0..WIDTH {
                let value = rgb[(y * WIDTH + x) * 3 + channel];
                tone(
                    1500.0 + 800.0 * f64::from(value) / 255.0,
                    CHANNEL_MS / WIDTH as f64,
                    &mut out,
                );
            }
            tone(1500.0, GAP_MS, &mut out);
        }
    }
    Ok(out)
}

pub fn decode_martin_m1(audio: &[f32]) -> Result<DecodedImage, SstvError> {
    let header = ms_samples(HEADER_MS);
    if audio.len() < header + ms_samples(IMAGE_MS) {
        return Err(SstvError::IncompleteAudio);
    }
    let Some((vis, frequency_offset_hz)) = decode_vis_header(&audio[..header]) else {
        return Err(SstvError::UnsupportedVis);
    };
    if vis != VIS_CODE_MARTIN_M1 {
        return Err(SstvError::UnsupportedVis);
    }
    decode_image_audio(&audio[header..], frequency_offset_hz)
}

fn decode_image_audio(audio: &[f32], frequency_offset_hz: f32) -> Result<DecodedImage, SstvError> {
    if audio.len() < ms_samples(IMAGE_MS) {
        return Err(SstvError::IncompleteAudio);
    }
    let frequencies = crossing_frequency(audio);
    let mut rgb = vec![0_u8; WIDTH * HEIGHT * 3];
    for y in 0..HEIGHT {
        let line_ms = y as f64 * LINE_MS;
        let mut channel_start_ms = line_ms + SYNC_MS + GAP_MS;
        for channel in [1_usize, 2, 0] {
            for x in 0..WIDTH {
                let center_ms = channel_start_ms + (x as f64 + 0.5) * CHANNEL_MS / WIDTH as f64;
                let index = ms_samples(center_ms).min(frequencies.len() - 1);
                let frequency = (frequencies[index] - frequency_offset_hz).clamp(1500.0, 2300.0);
                rgb[(y * WIDTH + x) * 3 + channel] =
                    (((frequency - 1500.0) * 255.0 / 800.0).round() as i32).clamp(0, 255) as u8;
            }
            channel_start_ms += CHANNEL_MS + GAP_MS;
        }
    }
    Ok(DecodedImage {
        width: WIDTH,
        height: HEIGHT,
        rgb,
    })
}

pub fn vis_mode_name(vis: u8) -> &'static str {
    match vis {
        0x2c => "Martin M1",
        0x28 => "Martin M2",
        0x3c => "Scottie S1",
        0x38 => "Scottie S2",
        0x4c => "Scottie DX",
        0x08 => "Robot 36",
        0x0c => "Robot 72",
        0x63 => "PD90",
        0x5f => "PD120",
        0x62 => "PD160",
        0x60 => "PD180",
        0x61 => "PD240",
        0x5e => "PD290",
        _ => "unknown SSTV mode",
    }
}

fn decode_vis_header(audio: &[f32]) -> Option<(u8, f32)> {
    let leader1_audio = slice_ms(audio, 40.0, 260.0);
    if dominant_frequency(leader1_audio, 0.0) != 1900.0 {
        return None;
    }
    let actual_leader_hz = peak_frequency(leader1_audio, 1_700, 2_100, 10);
    let frequency_offset_hz = actual_leader_hz - 1900.0;
    if frequency_offset_hz.abs() > 150.0 {
        return None;
    }
    let classify = |start_ms, end_ms| {
        dominant_frequency(slice_ms(audio, start_ms, end_ms), frequency_offset_hz)
    };
    if classify(301.0, 309.0) != 1200.0 || classify(350.0, 570.0) != 1900.0 {
        return None;
    }
    if classify(614.0, 636.0) != 1200.0 {
        return None;
    }
    let bits_start = LEADER_MS * 2.0 + VIS_BREAK_MS + VIS_BIT_MS;
    let mut vis = 0_u8;
    let mut ones = 0;
    for bit in 0..7 {
        let start = bits_start + bit as f64 * VIS_BIT_MS;
        let frequency = classify(start + 4.0, start + 26.0);
        if frequency == 1100.0 {
            vis |= 1 << bit;
            ones += 1;
        } else if frequency != 1300.0 {
            return None;
        }
    }
    let parity_start = bits_start + 7.0 * VIS_BIT_MS;
    let parity = classify(parity_start + 4.0, parity_start + 26.0);
    let parity_ok = if ones % 2 == 1 {
        parity == 1100.0
    } else {
        parity == 1300.0
    };
    let stop_start = parity_start + VIS_BIT_MS;
    let stop = classify(stop_start + 4.0, stop_start + 26.0);
    (parity_ok && stop == 1200.0).then_some((vis, frequency_offset_hz))
}

fn append_vis_header<F>(vis_code: u8, tone: &mut F, out: &mut Vec<i16>)
where
    F: FnMut(f64, f64, &mut Vec<i16>),
{
    tone(1900.0, LEADER_MS, out);
    tone(1200.0, VIS_BREAK_MS, out);
    tone(1900.0, LEADER_MS, out);
    tone(1200.0, VIS_BIT_MS, out);
    let mut ones = 0;
    for bit in 0..7 {
        let one = vis_code & (1 << bit) != 0;
        ones += usize::from(one);
        tone(if one { 1100.0 } else { 1300.0 }, VIS_BIT_MS, out);
    }
    tone(if ones % 2 == 1 { 1100.0 } else { 1300.0 }, VIS_BIT_MS, out);
    tone(1200.0, VIS_BIT_MS, out);
}

fn crossing_frequency(audio: &[f32]) -> Vec<f32> {
    let mut result = vec![1900.0; audio.len()];
    let mut previous: Option<usize> = None;
    for index in 1..audio.len() {
        if audio[index - 1] <= 0.0 && audio[index] > 0.0 {
            if let Some(last) = previous {
                let period = index - last;
                if period > 0 {
                    let frequency = SAMPLE_RATE_HZ as f32 / period as f32;
                    for value in &mut result[last..=index] {
                        *value = frequency;
                    }
                }
            }
            previous = Some(index);
        }
    }
    result
}

fn dominant_frequency(audio: &[f32], offset_hz: f32) -> f32 {
    let mut best = (0.0_f32, 0.0_f32);
    for frequency in [1100, 1200, 1300, 1900] {
        let power = tone_power(audio, frequency as f32 + offset_hz);
        if power > best.1 {
            best = (frequency as f32, power);
        }
    }
    best.0
}

fn peak_frequency(audio: &[f32], start_hz: u32, end_hz: u32, step_hz: usize) -> f32 {
    (start_hz..=end_hz)
        .step_by(step_hz)
        .map(|frequency| (frequency as f32, tone_power(audio, frequency as f32)))
        .max_by(|left, right| left.1.total_cmp(&right.1))
        .map(|(frequency, _)| frequency)
        .unwrap_or(1900.0)
}

fn tone_power(audio: &[f32], frequency_hz: f32) -> f32 {
    let omega = TAU * frequency_hz / SAMPLE_RATE_HZ as f32;
    let coeff = 2.0 * omega.cos();
    let (mut q1, mut q2) = (0.0_f32, 0.0_f32);
    for &sample in audio {
        let q0 = coeff * q1 - q2 + sample;
        q2 = q1;
        q1 = q0;
    }
    q1 * q1 + q2 * q2 - coeff * q1 * q2
}

fn slice_ms(audio: &[f32], start_ms: f64, end_ms: f64) -> &[f32] {
    let start = ms_samples(start_ms).min(audio.len());
    let end = ms_samples(end_ms).min(audio.len()).max(start);
    &audio[start..end]
}

fn ms_samples(ms: f64) -> usize {
    (ms * SAMPLE_RATE_HZ as f64 / 1000.0).round() as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn martin_m1_round_trip_preserves_color_structure() {
        let mut source = vec![0_u8; WIDTH * HEIGHT * 3];
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                let pixel = &mut source[(y * WIDTH + x) * 3..][..3];
                pixel.copy_from_slice(&[x as u8, y as u8, ((x + y) / 2) as u8]);
            }
        }
        let pcm = encode_martin_m1(&source).unwrap();
        let audio: Vec<f32> = pcm
            .iter()
            .map(|sample| *sample as f32 / i16::MAX as f32)
            .collect();
        let decoded = decode_martin_m1(&audio).unwrap();
        assert_eq!((decoded.width, decoded.height), (WIDTH, HEIGHT));
        let mean_error = source
            .iter()
            .zip(&decoded.rgb)
            .map(|(a, b)| a.abs_diff(*b) as u64)
            .sum::<u64>() as f64
            / source.len() as f64;
        assert!(mean_error < 35.0, "mean channel error was {mean_error}");
    }

    #[test]
    fn streaming_receiver_reports_progress_and_finishes() {
        let source = vec![127_u8; WIDTH * HEIGHT * 3];
        let pcm = encode_martin_m1(&source).unwrap();
        let audio: Vec<f32> = pcm
            .iter()
            .map(|sample| *sample as f32 / i16::MAX as f32)
            .collect();
        let mut receiver = MartinM1Receiver::default();
        let mut result = None;
        for chunk in audio.chunks(4096) {
            result = receiver.push(chunk).or(result);
        }
        assert!(result.is_some());
    }

    #[test]
    fn streaming_receiver_reports_an_unsupported_vis_mode() {
        let mut pcm = Vec::new();
        let mut phase = 0.0_f64;
        let mut remainder = 0.0_f64;
        let mut tone = |frequency: f64, duration_ms: f64, out: &mut Vec<i16>| {
            remainder += duration_ms * SAMPLE_RATE_HZ as f64 / 1000.0;
            let count = remainder.floor() as usize;
            remainder -= count as f64;
            let step = std::f64::consts::TAU * frequency / SAMPLE_RATE_HZ as f64;
            for _ in 0..count {
                out.push((phase.sin() * 18_000.0).round() as i16);
                phase = (phase + step) % std::f64::consts::TAU;
            }
        };
        append_vis_header(0x3c, &mut tone, &mut pcm);
        let audio: Vec<f32> = pcm
            .iter()
            .map(|sample| *sample as f32 / i16::MAX as f32)
            .collect();
        let mut receiver = MartinM1Receiver::default();
        assert!(receiver.push(&audio).is_none());
        assert_eq!(receiver.detected_vis(), Some(0x3c));
        assert_eq!(vis_mode_name(0x3c), "Scottie S1");
        assert!(receiver.progress().is_none());
    }

    #[test]
    fn martin_m1_afc_accepts_a_frequency_shifted_signal() {
        let source = vec![127_u8; WIDTH * HEIGHT * 3];
        let pcm = encode_martin_m1_with_offset(&source, 60.0).unwrap();
        let audio: Vec<f32> = pcm
            .iter()
            .map(|sample| *sample as f32 / i16::MAX as f32)
            .collect();
        let mut receiver = MartinM1Receiver::default();
        let mut result = None;
        for chunk in audio.chunks(4096) {
            result = receiver.push(chunk).or(result);
        }
        let decoded = result.expect("shifted Martin M1 should decode");
        let mean = decoded
            .rgb
            .iter()
            .map(|value| u64::from(*value))
            .sum::<u64>() as f64
            / decoded.rgb.len() as f64;
        assert!((mean - 127.0).abs() < 35.0, "decoded mean was {mean}");
    }
}
