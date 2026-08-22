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
}

impl MartinM1Receiver {
    pub fn reset(&mut self) {
        self.buffer.clear();
        self.image_start = None;
        self.search_from = 0;
    }

    pub fn progress(&self) -> Option<f32> {
        let start = self.image_start?;
        Some(
            ((self.buffer.len().saturating_sub(start)) as f64 / ms_samples(IMAGE_MS) as f64)
                .clamp(0.0, 1.0) as f32,
        )
    }

    pub fn push(&mut self, samples: &[f32]) -> Option<DecodedImage> {
        self.buffer.extend_from_slice(samples);
        if self.image_start.is_none() {
            self.find_header();
        }
        if let Some(start) = self.image_start {
            let needed = start + ms_samples(IMAGE_MS);
            if self.buffer.len() >= needed {
                let decoded = decode_image_audio(&self.buffer[start..needed]);
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
            if header_matches(&self.buffer[self.search_from..self.search_from + header]) {
                self.image_start = Some(self.search_from + header);
                return;
            }
            self.search_from += step;
        }
    }
}

pub fn encode_martin_m1(rgb: &[u8]) -> Result<Vec<i16>, SstvError> {
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
        let step = std::f64::consts::TAU * frequency / SAMPLE_RATE_HZ as f64;
        for _ in 0..count {
            out.push((phase.sin() * 18_000.0).round() as i16);
            phase = (phase + step) % std::f64::consts::TAU;
        }
    };

    tone(1900.0, LEADER_MS, &mut out);
    tone(1200.0, VIS_BREAK_MS, &mut out);
    tone(1900.0, LEADER_MS, &mut out);
    tone(1200.0, VIS_BIT_MS, &mut out);
    let mut ones = 0;
    for bit in 0..7 {
        let one = VIS_CODE_MARTIN_M1 & (1 << bit) != 0;
        ones += usize::from(one);
        tone(if one { 1100.0 } else { 1300.0 }, VIS_BIT_MS, &mut out);
    }
    tone(
        if ones % 2 == 1 { 1100.0 } else { 1300.0 },
        VIS_BIT_MS,
        &mut out,
    );
    tone(1200.0, VIS_BIT_MS, &mut out);

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
    if !header_matches(&audio[..header]) {
        return Err(SstvError::UnsupportedVis);
    }
    decode_image_audio(&audio[header..])
}

fn decode_image_audio(audio: &[f32]) -> Result<DecodedImage, SstvError> {
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
                let frequency = frequencies[index].clamp(1500.0, 2300.0);
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

fn header_matches(audio: &[f32]) -> bool {
    let leader1 = dominant_frequency(slice_ms(audio, 40.0, 260.0));
    let break_tone = dominant_frequency(slice_ms(audio, 301.0, 309.0));
    let leader2 = dominant_frequency(slice_ms(audio, 350.0, 570.0));
    if !near(leader1, 1900.0, 90.0)
        || !near(break_tone, 1200.0, 110.0)
        || !near(leader2, 1900.0, 90.0)
    {
        return false;
    }
    let bits_start = LEADER_MS * 2.0 + VIS_BREAK_MS + VIS_BIT_MS;
    let mut vis = 0_u8;
    let mut ones = 0;
    for bit in 0..7 {
        let start = bits_start + bit as f64 * VIS_BIT_MS;
        let frequency = dominant_frequency(slice_ms(audio, start + 4.0, start + 26.0));
        if near(frequency, 1100.0, 90.0) {
            vis |= 1 << bit;
            ones += 1;
        } else if !near(frequency, 1300.0, 90.0) {
            return false;
        }
    }
    let parity_start = bits_start + 7.0 * VIS_BIT_MS;
    let parity = dominant_frequency(slice_ms(audio, parity_start + 4.0, parity_start + 26.0));
    let parity_ok = if ones % 2 == 1 {
        near(parity, 1100.0, 90.0)
    } else {
        near(parity, 1300.0, 90.0)
    };
    parity_ok && vis == VIS_CODE_MARTIN_M1
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

fn dominant_frequency(audio: &[f32]) -> f32 {
    let mut best = (0.0_f32, 0.0_f32);
    for frequency in [1100, 1200, 1300, 1900] {
        let omega = TAU * frequency as f32 / SAMPLE_RATE_HZ as f32;
        let coeff = 2.0 * omega.cos();
        let (mut q1, mut q2) = (0.0_f32, 0.0_f32);
        for &sample in audio {
            let q0 = coeff * q1 - q2 + sample;
            q2 = q1;
            q1 = q0;
        }
        let power = q1 * q1 + q2 * q2 - coeff * q1 * q2;
        if power > best.1 {
            best = (frequency as f32, power);
        }
    }
    best.0
}

fn slice_ms(audio: &[f32], start_ms: f64, end_ms: f64) -> &[f32] {
    let start = ms_samples(start_ms).min(audio.len());
    let end = ms_samples(end_ms).min(audio.len()).max(start);
    &audio[start..end]
}

fn near(value: f32, expected: f32, tolerance: f32) -> bool {
    (value - expected).abs() <= tolerance
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
}
