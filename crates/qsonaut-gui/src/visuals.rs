use std::collections::VecDeque;

use eframe::egui::{Color32, ColorImage};
use rustfft::num_complex::Complex;

use super::WaterfallTheme;

const AUDIO_MAX_FREQ_HZ: u32 = 4_000;

pub(super) fn downsample_bins(bins: &[u8], width: usize) -> Vec<u8> {
    if bins.is_empty() {
        return vec![0; width];
    }
    if bins.len() == width {
        return bins.to_vec();
    }
    if bins.len() == 1 {
        return vec![bins[0]; width];
    }
    if bins.len() > width {
        return (0..width)
            .map(|x| {
                let start = x * bins.len() / width;
                let end = ((x + 1) * bins.len() / width).max(start + 1);
                bins[start..end].iter().copied().max().unwrap_or(0)
            })
            .collect();
    }
    resample_row_linear(bins, width)
}

pub(super) fn audio_cursor_level(rows: &VecDeque<Vec<u8>>, frequency_hz: u32) -> u8 {
    let Some(row) = rows.back() else {
        return 0;
    };
    if row.is_empty() {
        return 0;
    }
    let position = frequency_hz.min(AUDIO_MAX_FREQ_HZ) as f32 / AUDIO_MAX_FREQ_HZ as f32;
    let center = (position * (row.len() - 1) as f32).round() as usize;
    let start = center.saturating_sub(1);
    let end = (center + 1).min(row.len() - 1);
    row[start..=end].iter().copied().max().unwrap_or(0)
}

pub(super) fn fft_buffer_to_display_bins(
    buffer: &[Complex<f32>],
    bins: usize,
    sample_rate_hz: u32,
) -> Vec<u8> {
    let sample_count = buffer.len();
    if sample_count == 0 || bins == 0 {
        return vec![0; bins];
    }
    let max_index =
        (sample_count as f32 * AUDIO_MAX_FREQ_HZ as f32 / sample_rate_hz as f32).round() as usize;
    let max_index = max_index.clamp(2, sample_count / 2);
    (0..bins)
        .map(|index| {
            let position = 1.0 + (index as f32 / (bins.max(2) - 1) as f32) * (max_index - 1) as f32;
            let lower = position.floor() as usize;
            let upper = (lower + 1).min(max_index);
            let fraction = position - lower as f32;
            let lower_magnitude = buffer[lower].norm() / sample_count as f32;
            let upper_magnitude = buffer[upper].norm() / sample_count as f32;
            let magnitude = lower_magnitude + (upper_magnitude - lower_magnitude) * fraction;
            let db = (20.0 * magnitude.max(1e-9_f32).log10()).clamp(-65.0, 0.0);
            ((db + 65.0) / 65.0 * 255.0).round().clamp(0.0, 255.0) as u8
        })
        .collect()
}

pub(super) fn build_waterfall_image_with_theme(
    rows: &VecDeque<Vec<u8>>,
    width: usize,
    height: usize,
    theme: WaterfallTheme,
) -> ColorImage {
    let mut scalar_grid = vec![0u8; width * height];
    let empty_row = vec![0u8; width];
    let missing = height.saturating_sub(rows.len());
    for y in 0..height {
        let rendered_row = if y < missing {
            empty_row.clone()
        } else {
            resample_row_linear(rows.get(y - missing).unwrap_or(&empty_row), width)
        };
        scalar_grid[y * width..(y + 1) * width].copy_from_slice(&rendered_row[..width]);
    }

    let pixels = scalar_grid
        .into_iter()
        .map(|value| waterfall_color(value, theme))
        .collect();
    ColorImage::new([width, height], pixels)
}

pub(super) fn build_scope_waterfall_image(
    rows: &VecDeque<Vec<u8>>,
    width: usize,
    height: usize,
    theme: WaterfallTheme,
) -> ColorImage {
    build_waterfall_image_with_theme(rows, width, height, theme)
}

fn resample_row_linear(row: &[u8], width: usize) -> Vec<u8> {
    if width == 0 {
        return Vec::new();
    }
    if row.is_empty() {
        return vec![0; width];
    }
    if row.len() == width {
        return row.to_vec();
    }
    if row.len() == 1 {
        return vec![row[0]; width];
    }

    let source_last = (row.len() - 1) as f32;
    let destination_last = (width - 1).max(1) as f32;
    (0..width)
        .map(|x| {
            let position = x as f32 / destination_last * source_last;
            let lower = position.floor() as usize;
            let upper = position.ceil() as usize;
            if lower == upper {
                row[lower]
            } else {
                let fraction = position - lower as f32;
                (row[lower] as f32 + (row[upper] as f32 - row[lower] as f32) * fraction).round()
                    as u8
            }
        })
        .collect()
}

fn waterfall_color(value: u8, theme: WaterfallTheme) -> Color32 {
    let normalized = value as f32 / 255.0;
    let (red, green, blue) = match theme {
        WaterfallTheme::RadioBlue if normalized < 0.33 => {
            let scale = normalized / 0.33;
            (0.0, scale * 180.0, 80.0 + scale * 175.0)
        }
        WaterfallTheme::RadioBlue if normalized < 0.66 => {
            let scale = (normalized - 0.33) / 0.33;
            (scale * 220.0, 180.0 + scale * 60.0, 255.0 - scale * 220.0)
        }
        WaterfallTheme::RadioBlue => {
            let scale = (normalized - 0.66) / 0.34;
            (
                220.0 + scale * 35.0,
                240.0 + scale * 15.0,
                35.0 + scale * 220.0,
            )
        }
        WaterfallTheme::Inferno => (
            (255.0 * normalized.powf(0.65)).min(255.0),
            (255.0 * normalized.powf(2.0)).min(255.0),
            (120.0 * (1.0 - normalized) + 135.0 * normalized.powf(4.0)).min(255.0),
        ),
        WaterfallTheme::Phosphor => (
            35.0 * normalized,
            255.0 * normalized.powf(0.75),
            100.0 * normalized.powf(1.3),
        ),
        WaterfallTheme::Monochrome => {
            let level = 255.0 * normalized;
            (level, level, level)
        }
    };
    Color32::from_rgb(red as u8, green as u8, blue as u8)
}

pub(super) fn scale_scope_levels(row: &[u8], intensity: f32) -> Vec<u8> {
    let gamma = 1.0 / intensity.clamp(0.7, 3.0);
    row.iter()
        .map(|&value| {
            let normalized = value.min(160) as f32 / 160.0;
            (normalized.powf(gamma) * 255.0).round().clamp(0.0, 255.0) as u8
        })
        .collect()
}
