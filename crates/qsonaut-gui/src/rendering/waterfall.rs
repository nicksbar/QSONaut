use super::super::*;

/// Approximate occupied signal width used by the audio reticle. These values
/// cover the protocol tone span rather than the 3 kHz receiver filter width.
pub(crate) fn native_channel_width_hz(
    mode: WorkspaceMode,
    fst4_submode: crate::modes::fst4::Submode,
) -> u32 {
    match mode {
        WorkspaceMode::Fst4 => match fst4_submode {
            crate::modes::fst4::Submode::S15 => 70,
            crate::modes::fst4::Submode::S30 => 30,
            crate::modes::fst4::Submode::S60 => 15,
            crate::modes::fst4::Submode::S120 => 8,
            crate::modes::fst4::Submode::S300 => 3,
        },
        WorkspaceMode::Jt9 => 16,
        // JT65A spans the 65-tone constellation at approximately 2.69 Hz.
        WorkspaceMode::Jt65 => 180,
        // QSONaut currently wires Q65-A30: 65 tones at approximately 3.33 Hz.
        WorkspaceMode::Q65 => 220,
        _ => 50,
    }
}

pub(crate) fn effective_visual_profile(
    tuning: &DisplayTuning,
    mode: &str,
    radio: bool,
) -> (u64, u8) {
    let auto_visual = if radio {
        tuning.radio_auto_visual
    } else {
        tuning.audio_auto_visual
    };
    let waterfall_speed = if radio {
        tuning.radio_waterfall_speed
    } else {
        tuning.audio_waterfall_speed
    };
    if !auto_visual {
        return match waterfall_speed {
            WaterfallSpeed::Slow => (220, 2),
            WaterfallSpeed::Mid => (120, 1),
            WaterfallSpeed::Fast => (50, 0),
        };
    }
    let m = mode.to_ascii_uppercase();
    if m.contains("DATA")
        || m.contains("-D")
        || m.contains("FT8")
        || m.contains("JS8")
        || m.contains("RTTY")
        || m.contains("CW")
    {
        (50, 0)
    } else if m.contains("FM") {
        (120, 1)
    } else {
        (90, 1)
    }
}

pub(crate) fn scope_projection_for_mode(mode: &str) -> ScopeProjection {
    let mode = mode.to_ascii_uppercase();
    if mode.contains("LSB") {
        ScopeProjection::LowerSideband
    } else if mode.contains("USB") || mode == "DATA" || mode.contains("DIG") {
        ScopeProjection::UpperSideband
    } else {
        ScopeProjection::Full
    }
}

pub(crate) fn sideband_scope_edges(
    frequency_hz: u64,
    visible_width_hz: u64,
    projection: ScopeProjection,
) -> Option<(u64, u64)> {
    let floor_khz = |hz: u64| hz / 1_000 * 1_000;
    let ceil_khz = |hz: u64| hz.div_ceil(1_000) * 1_000;
    match projection {
        ScopeProjection::LowerSideband => Some((
            floor_khz(frequency_hz.saturating_sub(visible_width_hz)),
            ceil_khz(frequency_hz),
        )),
        ScopeProjection::UpperSideband => Some((
            floor_khz(frequency_hz),
            ceil_khz(frequency_hz.saturating_add(visible_width_hz)),
        )),
        ScopeProjection::Full => None,
    }
}

pub(crate) fn scope_span_hz_for(
    metadata: Option<qsonaut_radio::ScopeMetadata>,
    span_code: u8,
) -> u64 {
    metadata
        .and_then(|metadata| {
            metadata
                .span_options_hz
                .get(usize::from(span_code))
                .copied()
        })
        .unwrap_or_else(|| scope_span_hz(span_code))
}

pub(crate) fn scope_span_label_for(
    metadata: Option<qsonaut_radio::ScopeMetadata>,
    span_code: u8,
) -> String {
    let span_hz = scope_span_hz_for(metadata, span_code);
    if span_hz.is_multiple_of(1_000) {
        format!("±{} kHz", span_hz / 1_000)
    } else {
        format!("±{:.1} kHz", span_hz as f32 / 1_000.0)
    }
}

pub(crate) fn scope_span_hz(span_code: u8) -> u64 {
    match span_code.min(7) {
        0 => 2_500,
        1 => 5_000,
        2 => 10_000,
        3 => 25_000,
        4 => 50_000,
        5 => 100_000,
        6 => 250_000,
        _ => 500_000,
    }
}

pub(crate) fn scope_span_for_filter(mode: &str, filter_width_hz: Option<u32>) -> u8 {
    // A driver may not document filter geometry. The neutral fallback keeps
    // the scope usable without pretending to know a model's filter table.
    let filter_width_hz = filter_width_hz.unwrap_or(3_000);
    let required_half_span_hz = match scope_projection_for_mode(mode) {
        ScopeProjection::Full => filter_width_hz.div_ceil(2),
        ScopeProjection::LowerSideband | ScopeProjection::UpperSideband => filter_width_hz,
    };
    match required_half_span_hz {
        ..=2_500 => 0,
        2_501..=5_000 => 1,
        5_001..=10_000 => 2,
        10_001..=25_000 => 3,
        25_001..=50_000 => 4,
        50_001..=100_000 => 5,
        100_001..=250_000 => 6,
        _ => 7,
    }
}

pub(crate) fn scope_span_for_filter_with_options(
    mode: &str,
    filter_width_hz: Option<u32>,
    span_options_hz: &[u64],
) -> u8 {
    if span_options_hz.is_empty() {
        return scope_span_for_filter(mode, filter_width_hz);
    }
    let filter_width_hz = u64::from(filter_width_hz.unwrap_or(3_000));
    let required_half_span_hz = match scope_projection_for_mode(mode) {
        ScopeProjection::Full => filter_width_hz.div_ceil(2),
        ScopeProjection::LowerSideband | ScopeProjection::UpperSideband => filter_width_hz,
    };
    span_options_hz
        .iter()
        .position(|span| *span >= required_half_span_hz)
        .unwrap_or(span_options_hz.len().saturating_sub(1)) as u8
}

pub(crate) fn band_edges_for_frequency(
    frequency_hz: Option<u64>,
) -> Option<(u64, u64, &'static str)> {
    match frequency_hz? {
        1_800_000..=2_000_000 => Some((1_800_000, 2_000_000, "160m")),
        3_500_000..=4_000_000 => Some((3_500_000, 4_000_000, "80m")),
        5_000_000..=5_500_000 => Some((5_000_000, 5_500_000, "60m")),
        7_000_000..=7_300_000 => Some((7_000_000, 7_300_000, "40m")),
        10_100_000..=10_150_000 => Some((10_100_000, 10_150_000, "30m")),
        14_000_000..=14_350_000 => Some((14_000_000, 14_350_000, "20m")),
        18_068_000..=18_168_000 => Some((18_068_000, 18_168_000, "17m")),
        21_000_000..=21_450_000 => Some((21_000_000, 21_450_000, "15m")),
        24_890_000..=24_990_000 => Some((24_890_000, 24_990_000, "12m")),
        28_000_000..=29_700_000 => Some((28_000_000, 29_700_000, "10m")),
        50_000_000..=54_000_000 => Some((50_000_000, 54_000_000, "6m")),
        144_000_000..=148_000_000 => Some((144_000_000, 148_000_000, "2m")),
        420_000_000..=450_000_000 => Some((420_000_000, 450_000_000, "70cm")),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::native_channel_width_hz;
    use crate::modes::fst4::Submode;
    use crate::WorkspaceMode;

    #[test]
    fn native_reticle_widths_match_protocol_tone_spans() {
        assert_eq!(
            native_channel_width_hz(WorkspaceMode::Fst4, Submode::S15),
            70
        );
        assert_eq!(
            native_channel_width_hz(WorkspaceMode::Fst4, Submode::S30),
            30
        );
        assert_eq!(
            native_channel_width_hz(WorkspaceMode::Fst4, Submode::S60),
            15
        );
        assert_eq!(
            native_channel_width_hz(WorkspaceMode::Fst4, Submode::S120),
            8
        );
        assert_eq!(
            native_channel_width_hz(WorkspaceMode::Fst4, Submode::S300),
            3
        );
        assert_eq!(
            native_channel_width_hz(WorkspaceMode::Jt9, Submode::S15),
            16
        );
        assert_eq!(
            native_channel_width_hz(WorkspaceMode::Jt65, Submode::S15),
            180
        );
        assert_eq!(
            native_channel_width_hz(WorkspaceMode::Q65, Submode::S15),
            220
        );
    }
}
