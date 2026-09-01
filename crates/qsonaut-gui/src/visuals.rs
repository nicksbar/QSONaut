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

pub(super) fn build_audio_waterfall_image_with_theme(
    rows: &VecDeque<Vec<u8>>,
    visible_bandwidth_hz: u32,
    width: usize,
    height: usize,
    theme: WaterfallTheme,
) -> ColorImage {
    let fraction = (visible_bandwidth_hz.min(AUDIO_MAX_FREQ_HZ) as f32 / AUDIO_MAX_FREQ_HZ as f32)
        .clamp(0.0, 1.0);
    let pixels = (0..height)
        .flat_map(|y| {
            let missing = height.saturating_sub(rows.len());
            let row = rows.get(y.saturating_sub(missing));
            let end = row.filter(|row| !row.is_empty()).map(|row| {
                (((row.len() - 1) as f32 * fraction).round() as usize).clamp(1, row.len() - 1)
            });
            (0..width).map(move |x| {
                let value = match (row, end) {
                    (Some(row), Some(end)) => sample_row_linear(row, end, x, width),
                    _ => 0,
                };
                waterfall_color(value, theme)
            })
        })
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

fn sample_row_linear(row: &[u8], source_last: usize, x: usize, width: usize) -> u8 {
    if width <= 1 || source_last == 0 {
        return row[0];
    }
    let position = x as f32 / (width - 1) as f32 * source_last as f32;
    let lower = position.floor() as usize;
    let upper = position.ceil() as usize;
    if lower == upper {
        row[lower]
    } else {
        let fraction = position - lower as f32;
        (row[lower] as f32 + (row[upper] as f32 - row[lower] as f32) * fraction).round() as u8
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downsampling_handles_empty_equal_singleton_and_resize_shapes() {
        assert_eq!(downsample_bins(&[], 4), vec![0; 4]);
        assert_eq!(downsample_bins(&[1, 2, 3], 3), vec![1, 2, 3]);
        assert_eq!(downsample_bins(&[7], 4), vec![7; 4]);
        assert_eq!(downsample_bins(&[1, 8, 3, 9], 2), vec![8, 9]);
        assert_eq!(downsample_bins(&[0, 100], 5), vec![0, 25, 50, 75, 100]);
        assert_eq!(resample_row_linear(&[], 3), vec![0; 3]);
        assert_eq!(resample_row_linear(&[9], 3), vec![9; 3]);
        assert_eq!(resample_row_linear(&[1, 2, 3], 3), vec![1, 2, 3]);
        assert!(resample_row_linear(&[1, 2], 0).is_empty());
    }

    #[test]
    fn cursor_and_fft_projection_handle_empty_and_clamped_inputs() {
        assert_eq!(audio_cursor_level(&VecDeque::new(), 1_000), 0);
        let mut rows = VecDeque::from([Vec::new(), vec![1, 4, 2]]);
        assert_eq!(audio_cursor_level(&rows, 4_000), 4);
        rows.pop_back();
        assert_eq!(audio_cursor_level(&rows, 1_000), 0);
        assert_eq!(fft_buffer_to_display_bins(&[], 3, 48_000), vec![0; 3]);
        assert!(fft_buffer_to_display_bins(&[Complex::new(1.0, 0.0); 8], 0, 48_000).is_empty());
        let bins = fft_buffer_to_display_bins(&[Complex::new(1.0, 0.0); 8], 4, 48_000);
        assert_eq!(bins.len(), 4);
        assert!(bins.iter().any(|value| *value > 0));
    }

    #[test]
    fn waterfall_images_cover_padding_resampling_and_all_themes() {
        let rows = VecDeque::from([vec![0, 128, 255], vec![255, 0, 128]]);
        for theme in [
            WaterfallTheme::RadioBlue,
            WaterfallTheme::Inferno,
            WaterfallTheme::Phosphor,
            WaterfallTheme::Monochrome,
        ] {
            let image = build_waterfall_image_with_theme(&rows, 4, 3, theme);
            assert_eq!(image.size, [4, 3]);
            assert_eq!(image.pixels.len(), 12);
            let audio = build_audio_waterfall_image_with_theme(&rows, 8_000, 4, 3, theme);
            assert_eq!(audio.size, [4, 3]);
        }
        let empty = build_scope_waterfall_image(&VecDeque::new(), 2, 2, WaterfallTheme::RadioBlue);
        assert_eq!(empty.pixels, vec![Color32::from_rgb(0, 0, 80); 4]);
        assert_eq!(
            waterfall_color(0, WaterfallTheme::RadioBlue),
            Color32::from_rgb(0, 0, 80)
        );
        assert_ne!(
            waterfall_color(255, WaterfallTheme::RadioBlue),
            waterfall_color(0, WaterfallTheme::RadioBlue)
        );
    }

    #[test]
    fn linear_sampling_and_scope_scaling_preserve_expected_bounds() {
        assert_eq!(sample_row_linear(&[10, 20, 30], 0, 0, 3), 10);
        assert_eq!(sample_row_linear(&[10, 20, 30], 2, 2, 3), 30);
        assert_eq!(sample_row_linear(&[10, 30], 1, 1, 1), 10);
        assert_eq!(scale_scope_levels(&[0, 160, 255], 0.1), vec![0, 255, 255]);
        assert_eq!(scale_scope_levels(&[0, 160, 255], 9.0), vec![0, 255, 255]);
    }
}
