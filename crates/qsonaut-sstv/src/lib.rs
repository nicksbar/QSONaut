//! Reusable analog SSTV streaming, VIS, and codec integration.
//!
//! Live audio is mono 12 kHz PCM. Automatic VIS selection and explicit receive
//! filtering cover the pinned backend's Martin, Scottie, Robot, and PD modes.

use std::f32::consts::TAU;

use image::{DynamicImage, ImageBuffer, Rgb};

pub use komitoto_sstv::SstvMode;

pub const SAMPLE_RATE_HZ: u32 = 12_000;
pub const WIDTH: usize = 320;
pub const HEIGHT: usize = 256;
pub const VIS_CODE_MARTIN_M1: u8 = 0x2c;
pub const MULTIMODE_SAMPLE_RATE_HZ: u32 = 48_000;
pub const AUTO_TARGET_MIN_OFFSET_HZ: i32 = -900;
pub const AUTO_TARGET_MAX_OFFSET_HZ: i32 = 700;
pub const AUTO_TARGET_CANDIDATES_PER_WINDOW: usize = 16;

const LEADER_MS: f64 = 300.0;
const VIS_BREAK_MS: f64 = 10.0;
const VIS_BIT_MS: f64 = 30.0;
const SYNC_MS: f64 = 4.862;
const GAP_MS: f64 = 0.572;
const CHANNEL_MS: f64 = 146.432;
const LINE_MS: f64 = SYNC_MS + 4.0 * GAP_MS + 3.0 * CHANNEL_MS;
const HEADER_MS: f64 = LEADER_MS * 2.0 + VIS_BREAK_MS + VIS_BIT_MS * 10.0;
const IMAGE_MS: f64 = LINE_MS * HEIGHT as f64;
const STREAM_DECODE_GUARD_MS: f64 = 20.0;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SstvError {
    #[error("Martin M1 needs exactly 320 x 256 RGB pixels")]
    InvalidImage,
    #[error("audio does not contain a complete Martin M1 image")]
    IncompleteAudio,
    #[error("VIS header is not Martin M1")]
    UnsupportedVis,
    #[error("SSTV image dimensions do not match its RGB buffer")]
    InvalidDimensions,
    #[error("SSTV codec failed: {0}")]
    Codec(String),
}

#[derive(Debug, Clone)]
pub struct DecodedImage {
    pub width: usize,
    pub height: usize,
    pub rgb: Vec<u8>,
}

/// Modes supplied by the pinned multi-mode codec backend.
pub fn supported_modes() -> &'static [SstvMode] {
    SstvMode::all()
}

pub fn mode_duration_seconds(mode: SstvMode) -> f32 {
    komitoto_sstv::spec::from_mode(mode).total_samples() as f32 / MULTIMODE_SAMPLE_RATE_HZ as f32
}

/// Map QSONaut's parity-stripped VIS value to a codec mode.
pub fn mode_from_vis(vis: u8) -> Option<SstvMode> {
    match vis {
        0x2c => Some(SstvMode::MartinM1),
        0x28 => Some(SstvMode::MartinM2),
        0x3c => Some(SstvMode::ScottieS1),
        0x38 => Some(SstvMode::ScottieS2),
        0x08 => Some(SstvMode::Robot36),
        0x0c => Some(SstvMode::Robot72),
        0x5d => Some(SstvMode::Pd50),
        0x63 => Some(SstvMode::Pd90),
        0x5f => Some(SstvMode::Pd120),
        0x62 => Some(SstvMode::Pd160),
        0x60 => Some(SstvMode::Pd180),
        0x61 => Some(SstvMode::Pd240),
        0x5e => Some(SstvMode::Pd290),
        _ => None,
    }
}

/// Encode arbitrary RGB pixels with the pinned multi-mode codec.
///
/// The input is resized to the selected mode's native dimensions. Returned
/// PCM is normalized to signed 16-bit samples at 48 kHz.
pub fn encode_rgb_mode(
    mode: SstvMode,
    width: u32,
    height: u32,
    rgb: &[u8],
) -> Result<Vec<i16>, SstvError> {
    let source = ImageBuffer::<Rgb<u8>, _>::from_raw(width, height, rgb.to_vec())
        .ok_or(SstvError::InvalidDimensions)?;
    let (target_width, target_height) = mode.resolution();
    let prepared = komitoto_sstv::image_proc::prepare_image(
        &DynamicImage::ImageRgb8(source),
        target_width,
        target_height,
        komitoto_sstv::image_proc::ResizeStrategy::Crop,
    );
    komitoto_sstv::SstvEncoder::new(mode)
        .encode(&prepared)
        .map(|samples| {
            samples
                .into_iter()
                .map(|sample| (sample.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16)
                .collect()
        })
        .map_err(|error| SstvError::Codec(error.to_string()))
}

/// Encode a selected mode for QSONaut's native 12 kHz transmit path.
pub fn encode_rgb_mode_12k(
    mode: SstvMode,
    width: u32,
    height: u32,
    rgb: &[u8],
) -> Result<Vec<i16>, SstvError> {
    Ok(encode_rgb_mode(mode, width, height, rgb)?
        .into_iter()
        .step_by((MULTIMODE_SAMPLE_RATE_HZ / SAMPLE_RATE_HZ) as usize)
        .collect())
}

/// Decode a complete, already mode-selected 48 kHz SSTV recording.
pub fn decode_mode(mode: SstvMode, audio: &[f32]) -> Result<DecodedImage, SstvError> {
    let image = komitoto_sstv::SstvDecoder::new(mode)
        .decode(audio)
        .map_err(|error| SstvError::Codec(error.to_string()))?
        .to_rgb8();
    Ok(DecodedImage {
        width: image.width() as usize,
        height: image.height() as usize,
        rgb: image.into_raw(),
    })
}

fn mode_sample_count_12k(mode: SstvMode) -> usize {
    komitoto_sstv::spec::from_mode(mode)
        .total_samples()
        .div_ceil((MULTIMODE_SAMPLE_RATE_HZ / SAMPLE_RATE_HZ) as usize)
}

fn decode_mode_12k(
    mode: SstvMode,
    audio: &[f32],
    frequency_offset_hz: f32,
) -> Result<DecodedImage, SstvError> {
    let corrected = if frequency_offset_hz.abs() >= 1.0 {
        let frequencies = komitoto_sstv::dsp::fm_demodulate(audio, SAMPLE_RATE_HZ);
        let mut phase = 0.0_f64;
        frequencies
            .into_iter()
            .map(|frequency| {
                phase +=
                    TAU as f64 * (frequency - frequency_offset_hz as f64) / SAMPLE_RATE_HZ as f64;
                phase.sin() as f32
            })
            .collect::<Vec<_>>()
    } else {
        audio.to_vec()
    };
    let factor = (MULTIMODE_SAMPLE_RATE_HZ / SAMPLE_RATE_HZ) as usize;
    let mut upsampled = Vec::with_capacity(corrected.len() * factor);
    for (index, &sample) in corrected.iter().enumerate() {
        let next = corrected.get(index + 1).copied().unwrap_or(sample);
        for step in 0..factor {
            let fraction = step as f32 / factor as f32;
            upsampled.push(sample + (next - sample) * fraction);
        }
    }
    decode_mode(mode, &upsampled)
}

/// Streaming, VIS-aware receiver for every mode supplied by the codec backend.
///
/// `None` selects the mode from a valid VIS header. `Some(mode)` accepts only
/// that mode's VIS header, which provides an explicit receive-mode filter.
#[derive(Debug, Default)]
pub struct MultiModeReceiver {
    buffer: Vec<f32>,
    transmission_start: Option<usize>,
    active_mode: Option<SstvMode>,
    selected_mode: Option<SstvMode>,
    auto_target: bool,
    locked_offset_hz: Option<f32>,
    search_from: usize,
    last_vis: Option<u8>,
    frequency_offset_hz: Option<f32>,
    tuning_offset_hz: f32,
    last_decode_error: Option<String>,
    last_completed_mode: Option<SstvMode>,
    scan_candidate_offset_hz: Option<f32>,
    scan_prominence_db: Option<f32>,
}

impl MultiModeReceiver {
    pub fn reset(&mut self) {
        self.buffer.clear();
        self.transmission_start = None;
        self.active_mode = None;
        self.search_from = 0;
        self.last_vis = None;
        self.frequency_offset_hz = None;
        self.locked_offset_hz = None;
        self.last_decode_error = None;
        self.last_completed_mode = None;
        self.scan_candidate_offset_hz = None;
        self.scan_prominence_db = None;
    }

    pub fn set_selected_mode(&mut self, mode: Option<SstvMode>) {
        if self.selected_mode != mode {
            self.reset();
            self.selected_mode = mode;
        }
    }

    pub fn set_auto_target(&mut self, enabled: bool) {
        if self.auto_target != enabled {
            self.reset();
            self.auto_target = enabled;
        }
    }

    pub fn auto_target(&self) -> bool {
        self.auto_target
    }

    pub fn locked_offset_hz(&self) -> Option<f32> {
        self.locked_offset_hz
    }

    pub fn scan_candidate_offset_hz(&self) -> Option<f32> {
        self.scan_candidate_offset_hz
    }

    pub fn scan_prominence_db(&self) -> Option<f32> {
        self.scan_prominence_db
    }

    pub fn selected_mode(&self) -> Option<SstvMode> {
        self.selected_mode
    }

    pub fn active_mode(&self) -> Option<SstvMode> {
        self.active_mode
    }

    pub fn progress(&self) -> Option<f32> {
        let start = self.transmission_start?;
        let mode = self.active_mode?;
        Some(
            (self.buffer.len().saturating_sub(start) as f32 / mode_sample_count_12k(mode) as f32)
                .clamp(0.0, 1.0),
        )
    }

    pub fn detected_vis(&self) -> Option<u8> {
        self.last_vis
    }

    pub fn frequency_offset_hz(&self) -> Option<f32> {
        self.frequency_offset_hz
    }

    pub fn take_decode_error(&mut self) -> Option<String> {
        self.last_decode_error.take()
    }

    pub fn take_completed_mode(&mut self) -> Option<SstvMode> {
        self.last_completed_mode.take()
    }

    pub fn set_tuning_offset_hz(&mut self, offset_hz: f32) {
        let offset_hz = offset_hz.clamp(-1_000.0, 1_000.0);
        if (self.tuning_offset_hz - offset_hz).abs() >= 1.0 {
            self.reset();
            self.tuning_offset_hz = offset_hz;
        }
    }

    pub fn push(&mut self, samples: &[f32]) -> Option<DecodedImage> {
        self.buffer.extend_from_slice(samples);
        if self.transmission_start.is_none() {
            self.find_header();
        }
        if let (Some(start), Some(mode)) = (self.transmission_start, self.active_mode) {
            let needed = start + mode_sample_count_12k(mode) + ms_samples(STREAM_DECODE_GUARD_MS);
            if self.buffer.len() >= needed {
                let result = decode_mode_12k(
                    mode,
                    &self.buffer[start..needed],
                    self.frequency_offset_hz.unwrap_or_default(),
                );
                self.reset();
                return match result {
                    Ok(image) => {
                        self.last_completed_mode = Some(mode);
                        Some(image)
                    }
                    Err(error) => {
                        self.last_decode_error = Some(error.to_string());
                        None
                    }
                };
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
        // Five milliseconds keeps the narrow VIS break/start windows aligned
        // even when a transmission begins between audio callback boundaries.
        let step = ms_samples(5.0);
        while self.search_from + header <= self.buffer.len() {
            let header_audio = &self.buffer[self.search_from..self.search_from + header];
            let detection = if self.auto_target {
                let scan = decode_vis_header_auto(header_audio);
                self.scan_candidate_offset_hz = scan.strongest_offset_hz;
                self.scan_prominence_db = scan.prominence_db;
                scan.detection
            } else {
                decode_vis_header(header_audio, self.tuning_offset_hz)
            };
            if let Some((vis, frequency_offset_hz)) = detection {
                self.last_vis = Some(vis);
                self.frequency_offset_hz = Some(frequency_offset_hz);
                self.locked_offset_hz = Some(frequency_offset_hz);
                if let Some(detected_mode) = mode_from_vis(vis) {
                    if self.selected_mode.is_none() || self.selected_mode == Some(detected_mode) {
                        self.transmission_start = Some(self.search_from);
                        self.active_mode = Some(detected_mode);
                        return;
                    }
                }
                self.search_from += header;
                continue;
            }
            self.search_from += step;
        }
    }
}

/// Streaming receiver with VIS detection and bounded buffering.
#[derive(Debug, Default)]
pub struct MartinM1Receiver {
    buffer: Vec<f32>,
    image_start: Option<usize>,
    search_from: usize,
    last_vis: Option<u8>,
    frequency_offset_hz: Option<f32>,
    tuning_offset_hz: f32,
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

    /// Move the expected SSTV tone plan within the captured audio channel.
    pub fn set_tuning_offset_hz(&mut self, offset_hz: f32) {
        let offset_hz = offset_hz.clamp(-1_000.0, 1_000.0);
        if (self.tuning_offset_hz - offset_hz).abs() >= 1.0 {
            self.reset();
            self.tuning_offset_hz = offset_hz;
        }
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
            if let Some((vis, frequency_offset_hz)) = decode_vis_header(
                &self.buffer[self.search_from..self.search_from + header],
                self.tuning_offset_hz,
            ) {
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
    let Some((vis, frequency_offset_hz)) = decode_vis_header(&audio[..header], 0.0) else {
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

fn decode_vis_header(audio: &[f32], tuning_offset_hz: f32) -> Option<(u8, f32)> {
    let leader1_audio = slice_ms(audio, 40.0, 260.0);
    if dominant_frequency(leader1_audio, tuning_offset_hz) != 1900.0 {
        return None;
    }
    let search_center_hz = (1_900.0 + tuning_offset_hz).round() as i32;
    let actual_leader_hz = peak_frequency(
        leader1_audio,
        (search_center_hz - 150).max(100) as u32,
        (search_center_hz + 150).max(100) as u32,
        10,
    );
    let frequency_offset_hz = actual_leader_hz - 1900.0;
    if (frequency_offset_hz - tuning_offset_hz).abs() > 150.0 {
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

struct AutoTargetScan {
    detection: Option<(u8, f32)>,
    strongest_offset_hz: Option<f32>,
    prominence_db: Option<f32>,
}

fn decode_vis_header_auto(audio: &[f32]) -> AutoTargetScan {
    let leader = slice_ms(audio, 40.0, 260.0);
    let mut candidates = ((1_900 + AUTO_TARGET_MIN_OFFSET_HZ)
        ..=(1_900 + AUTO_TARGET_MAX_OFFSET_HZ))
        .step_by(25)
        .map(|frequency_hz| {
            (
                frequency_hz as f32 - 1_900.0,
                tone_power(leader, frequency_hz as f32),
            )
        })
        .collect::<Vec<_>>();
    let mut powers = candidates
        .iter()
        .map(|(_, power)| *power)
        .collect::<Vec<_>>();
    powers.sort_by(f32::total_cmp);
    let median_power = powers.get(powers.len() / 2).copied().unwrap_or_default();
    candidates.sort_by(|left, right| right.1.total_cmp(&left.1));
    let strongest = candidates.first().copied();
    let prominence_db = strongest
        .map(|(_, power)| 10.0 * ((power + f32::EPSILON) / (median_power + f32::EPSILON)).log10());

    let mut tested_offsets = Vec::with_capacity(AUTO_TARGET_CANDIDATES_PER_WINDOW);
    let mut detection = None;
    for (offset_hz, _) in candidates {
        if tested_offsets
            .iter()
            .any(|tested: &f32| (*tested - offset_hz).abs() < 50.0)
        {
            continue;
        }
        tested_offsets.push(offset_hz);
        if let Some(found) = decode_vis_header(audio, offset_hz) {
            detection = Some(found);
            break;
        }
        if tested_offsets.len() == AUTO_TARGET_CANDIDATES_PER_WINDOW {
            break;
        }
    }

    AutoTargetScan {
        detection,
        strongest_offset_hz: strongest.map(|(offset, _)| offset),
        prominence_db,
    }
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

    #[test]
    fn streaming_receiver_manual_tuning_accepts_a_shift_beyond_afc() {
        let source = vec![96_u8; WIDTH * HEIGHT * 3];
        let pcm = encode_martin_m1_with_offset(&source, 420.0).unwrap();
        let audio: Vec<f32> = pcm
            .iter()
            .map(|sample| *sample as f32 / i16::MAX as f32)
            .collect();
        let mut receiver = MartinM1Receiver::default();
        receiver.set_tuning_offset_hz(400.0);
        let mut result = None;
        for chunk in audio.chunks(4096) {
            result = receiver.push(chunk).or(result);
        }
        assert!(result.is_some());
        assert_eq!(receiver.tuning_offset_hz, 400.0);
    }

    #[test]
    fn multimode_backend_vis_mapping_covers_every_supported_mode() {
        assert_eq!(supported_modes().len(), 13);
        for &mode in supported_modes() {
            let vis = komitoto_sstv::spec::from_mode(mode).vis_code() & 0x7f;
            assert_eq!(mode_from_vis(vis), Some(mode), "missing {}", mode.name());
        }
    }

    #[test]
    fn multimode_adapter_round_trips_martin_m2() {
        let source = vec![112_u8; WIDTH * HEIGHT * 3];
        let pcm = encode_rgb_mode(SstvMode::MartinM2, WIDTH as u32, HEIGHT as u32, &source)
            .expect("Martin M2 encode should succeed");
        let audio: Vec<f32> = pcm
            .iter()
            .map(|sample| *sample as f32 / i16::MAX as f32)
            .collect();
        let decoded =
            decode_mode(SstvMode::MartinM2, &audio).expect("Martin M2 decode should succeed");
        assert_eq!((decoded.width, decoded.height), (WIDTH, HEIGHT));
        let mean = decoded
            .rgb
            .iter()
            .map(|value| u64::from(*value))
            .sum::<u64>() as f64
            / decoded.rgb.len() as f64;
        assert!((mean - 112.0).abs() < 25.0, "decoded mean was {mean}");
    }

    #[test]
    fn streaming_multimode_receiver_auto_decodes_every_mode_at_12k() {
        let source = vec![112_u8; WIDTH * HEIGHT * 3];
        for &mode in supported_modes() {
            let pcm = encode_rgb_mode_12k(mode, WIDTH as u32, HEIGHT as u32, &source)
                .unwrap_or_else(|error| panic!("{} encode failed: {error}", mode.name()));
            let mut audio: Vec<f32> = pcm
                .iter()
                .map(|sample| *sample as f32 / i16::MAX as f32)
                .collect();
            audio.resize(audio.len() + ms_samples(STREAM_DECODE_GUARD_MS), 0.0);
            let mut receiver = MultiModeReceiver::default();
            let mut result = None;
            for chunk in audio.chunks(4096) {
                result = receiver.push(chunk).or(result);
            }
            let decoded = result.unwrap_or_else(|| panic!("{} did not decode", mode.name()));
            let (width, height) = mode.resolution();
            assert_eq!(
                (decoded.width, decoded.height),
                (width as usize, height as usize),
                "{} dimensions",
                mode.name()
            );
            assert_eq!(receiver.take_completed_mode(), Some(mode));
        }
    }

    #[test]
    fn auto_target_acquires_a_shifted_unaligned_transmission() {
        let source = vec![96_u8; WIDTH * HEIGHT * 3];
        let pcm = encode_martin_m1_with_offset(&source, 420.0).unwrap();
        let mut audio = vec![0.0_f32; ms_samples(7.0)];
        audio.extend(pcm.iter().map(|sample| *sample as f32 / i16::MAX as f32));
        audio.resize(audio.len() + ms_samples(STREAM_DECODE_GUARD_MS), 0.0);
        let mut receiver = MultiModeReceiver::default();
        receiver.set_auto_target(true);
        let mut result = None;
        for chunk in audio.chunks(4096) {
            result = receiver.push(chunk).or(result);
        }
        assert!(result.is_some(), "shifted auto-target image should decode");
        assert_eq!(receiver.take_completed_mode(), Some(SstvMode::MartinM1));
    }

    #[test]
    fn auto_target_rejects_silence_and_keeps_its_buffer_bounded() {
        let audio = vec![0.0_f32; SAMPLE_RATE_HZ as usize * 2];
        let mut receiver = MultiModeReceiver::default();
        receiver.set_auto_target(true);
        for chunk in audio.chunks(2048) {
            assert!(receiver.push(chunk).is_none());
        }
        assert!(receiver.detected_vis().is_none());
        assert!(receiver.buffer.len() <= ms_samples(HEADER_MS + 400.0));
    }

    #[test]
    fn auto_target_validates_ranked_candidates_beyond_the_strongest_tone() {
        let mut pcm = Vec::new();
        let mut phase = 0.0_f64;
        let mut remainder = 0.0_f64;
        let mut shifted_tone = |frequency: f64, duration_ms: f64, out: &mut Vec<i16>| {
            remainder += duration_ms * SAMPLE_RATE_HZ as f64 / 1000.0;
            let count = remainder.floor() as usize;
            remainder -= count as f64;
            let step = std::f64::consts::TAU * (frequency + 420.0) / SAMPLE_RATE_HZ as f64;
            for _ in 0..count {
                out.push((phase.sin() * 12_000.0).round() as i16);
                phase = (phase + step) % std::f64::consts::TAU;
            }
        };
        append_vis_header(VIS_CODE_MARTIN_M1, &mut shifted_tone, &mut pcm);
        let mut audio: Vec<f32> = pcm
            .iter()
            .map(|sample| *sample as f32 / i16::MAX as f32)
            .collect();
        let start = ms_samples(40.0);
        let end = ms_samples(260.0);
        for (index, sample) in audio[start..end].iter_mut().enumerate() {
            *sample += (TAU * 1_100.0 * index as f32 / SAMPLE_RATE_HZ as f32).sin() * 0.8;
        }

        let scan = decode_vis_header_auto(&audio);
        assert_eq!(scan.detection.map(|(vis, _)| vis), Some(VIS_CODE_MARTIN_M1));
        assert!(
            scan.strongest_offset_hz.unwrap_or_default() < -700.0,
            "the interference should be the strongest leader candidate"
        );
    }

    #[test]
    fn manual_receive_mode_filters_a_different_vis_header() {
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
        let mut receiver = MultiModeReceiver::default();
        receiver.set_selected_mode(Some(SstvMode::MartinM1));
        assert!(receiver.push(&audio).is_none());
        assert_eq!(receiver.detected_vis(), Some(0x3c));
        assert_eq!(receiver.active_mode(), None);
    }
}
