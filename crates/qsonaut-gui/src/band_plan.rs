use qsonaut_radio::BaseMode;

use crate::modes::{cw, fst4, ft4, ft8, jt65, jt9, native, q65, sstv, voice, wspr};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum WorkspaceMode {
    Ft8,
    Ft4,
    Fst4,
    Wspr,
    Jt9,
    Jt65,
    Q65,
    Msk144,
    Cw,
    Voice,
    Sstv,
    Fldigi,
}

impl WorkspaceMode {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Ft8 => "FT8",
            Self::Ft4 => "FT4",
            Self::Fst4 => "FST4",
            Self::Wspr => "WSPR",
            Self::Jt9 => "JT9",
            Self::Jt65 => "JT65",
            Self::Q65 => "Q65",
            Self::Msk144 => "MSK144",
            Self::Cw => "CW",
            Self::Voice => "VOICE",
            Self::Sstv => "SSTV",
            Self::Fldigi => "FLDIGI",
        }
    }

    pub(super) fn core_slot_seconds(self) -> Option<f64> {
        match self {
            Self::Ft8 => Some(15.0),
            Self::Ft4 => Some(7.5),
            Self::Fst4 => Some(60.0),
            Self::Wspr => Some(120.0),
            Self::Jt9 | Self::Jt65 => Some(60.0),
            Self::Q65 => Some(30.0),
            Self::Msk144 => Some(15.0),
            Self::Cw | Self::Voice | Self::Sstv | Self::Fldigi => None,
        }
    }

    pub(super) fn slot_seconds(self, fst4_submode: crate::modes::fst4::Submode) -> Option<f64> {
        if self == Self::Fst4 {
            Some(fst4_submode.seconds())
        } else {
            self.core_slot_seconds()
        }
    }

    pub(super) fn has_native_decoder(self) -> bool {
        !matches!(self, Self::Cw | Self::Voice | Self::Sstv | Self::Fldigi)
    }

    pub(super) fn is_uhf(self) -> bool {
        matches!(self, Self::Msk144)
    }
}

pub(super) const WORKSPACE_MODES: [WorkspaceMode; 12] = [
    WorkspaceMode::Ft8,
    WorkspaceMode::Ft4,
    WorkspaceMode::Fst4,
    WorkspaceMode::Wspr,
    WorkspaceMode::Jt9,
    WorkspaceMode::Jt65,
    WorkspaceMode::Q65,
    WorkspaceMode::Msk144,
    WorkspaceMode::Cw,
    WorkspaceMode::Voice,
    WorkspaceMode::Sstv,
    WorkspaceMode::Fldigi,
];

pub(super) const HF_WORKSPACE_MODES: [WorkspaceMode; 10] = [
    WorkspaceMode::Ft8,
    WorkspaceMode::Ft4,
    WorkspaceMode::Fst4,
    WorkspaceMode::Wspr,
    WorkspaceMode::Jt9,
    WorkspaceMode::Jt65,
    WorkspaceMode::Q65,
    WorkspaceMode::Cw,
    WorkspaceMode::Voice,
    WorkspaceMode::Sstv,
];

pub(super) const OTHER_WORKSPACE_MODES: [WorkspaceMode; 2] =
    [WorkspaceMode::Msk144, WorkspaceMode::Fldigi];

// Shared band vocabulary for higher-level activity profiles. Mode-specific
// center frequencies remain owned by each workspace band plan below.
pub(super) const CORE_BAND_LABELS: &[&str] = &[
    "160m", "80m", "60m", "40m", "30m", "20m", "17m", "15m", "12m", "10m", "6m", "2m", "70cm",
];
pub(super) const CORE_HF_BAND_LABELS: &[&str] = &[
    "160m", "80m", "60m", "40m", "30m", "20m", "17m", "15m", "12m", "10m", "6m",
];
pub(super) const CORE_VHF_BAND_LABELS: &[&str] = &["2m", "70cm"];
pub(super) const CORE_EMCOMM_BAND_LABELS: &[&str] = &["80m", "40m", "20m"];

// The workspace plans intentionally contain only the most useful calling
// frequencies for a mode.  The band picker is different: it is a radio
// control and must expose every band the connected radio can operate on.
// These are sensible mode-neutral fallback centers for bands that a mode's
// focused plan does not happen to include.
const BAND_PICKER_FALLBACKS: &[(&str, u64)] = &[
    ("160m", 1_840_000),
    ("80m", 3_573_000),
    ("60m", 5_357_000),
    ("40m", 7_074_000),
    ("30m", 10_136_000),
    ("20m", 14_074_000),
    ("17m", 18_100_000),
    ("15m", 21_074_000),
    ("12m", 24_915_000),
    ("10m", 28_074_000),
    ("6m", 50_313_000),
    ("2m", 144_174_000),
    ("70cm", 432_174_000),
];

pub(super) fn band_for_frequency(frequency_hz: u64) -> &'static str {
    match frequency_hz {
        1_800_000..=2_000_000 => "160m",
        3_500_000..=4_000_000 => "80m",
        5_000_000..=5_500_000 => "60m",
        7_000_000..=7_300_000 => "40m",
        10_100_000..=10_150_000 => "30m",
        14_000_000..=14_350_000 => "20m",
        18_068_000..=18_168_000 => "17m",
        21_000_000..=21_450_000 => "15m",
        24_890_000..=24_990_000 => "12m",
        28_000_000..=29_700_000 => "10m",
        50_000_000..=54_000_000 => "6m",
        144_000_000..=148_000_000 => "2m",
        420_000_000..=450_000_000 => "70cm",
        _ => "",
    }
}

pub(super) fn workspace_band_plan(mode: WorkspaceMode) -> &'static [(&'static str, u64)] {
    match mode {
        WorkspaceMode::Ft8 => ft8::BAND_PLAN,
        WorkspaceMode::Ft4 => ft4::BAND_PLAN,
        WorkspaceMode::Fst4 => fst4::BAND_PLAN,
        WorkspaceMode::Wspr => wspr::BAND_PLAN,
        WorkspaceMode::Jt9 => jt9::BAND_PLAN,
        WorkspaceMode::Jt65 => jt65::BAND_PLAN,
        WorkspaceMode::Q65 => q65::BAND_PLAN,
        WorkspaceMode::Msk144 => native::MSK144_BAND_PLAN,
        WorkspaceMode::Cw => cw::BAND_PLAN,
        WorkspaceMode::Voice => voice::BAND_PLAN,
        WorkspaceMode::Sstv => sstv::BAND_PLAN,
        WorkspaceMode::Fldigi => native::FLDIGI_BAND_PLAN,
    }
}

pub(super) fn band_picker_plan(mode: WorkspaceMode) -> Vec<(&'static str, u64)> {
    CORE_BAND_LABELS
        .iter()
        .filter_map(|label| {
            workspace_band_plan(mode)
                .iter()
                .find(|(plan_label, _)| plan_label == label)
                .copied()
                .or_else(|| {
                    BAND_PICKER_FALLBACKS
                        .iter()
                        .find(|(fallback, _)| fallback == label)
                        .copied()
                })
        })
        .collect()
}

#[derive(Debug, Clone, Copy)]
pub(super) struct WorkspaceRadioPreset {
    pub(super) base_mode: BaseMode,
    pub(super) data_mode: bool,
    pub(super) filter: u8,
}

pub(super) fn workspace_radio_preset(mode: WorkspaceMode) -> WorkspaceRadioPreset {
    workspace_radio_preset_for_frequency(mode, None)
}

pub(super) fn workspace_radio_preset_for_frequency(
    mode: WorkspaceMode,
    frequency_hz: Option<u64>,
) -> WorkspaceRadioPreset {
    if mode == WorkspaceMode::Sstv {
        return WorkspaceRadioPreset {
            base_mode: BaseMode::Usb,
            data_mode: false,
            filter: 1,
        };
    }
    if mode == WorkspaceMode::Voice {
        let base_mode = match frequency_hz.map(band_for_frequency) {
            Some("160m" | "80m" | "40m") => BaseMode::Lsb,
            Some("2m" | "70cm") => BaseMode::Fm,
            _ => BaseMode::Usb,
        };
        return WorkspaceRadioPreset {
            base_mode,
            data_mode: false,
            filter: 2,
        };
    }
    WorkspaceRadioPreset {
        // Software modes operate on generated/received audio tones. USB-D keeps
        // a wide receive passband and lets the mode place audio-tone cursors.
        base_mode: BaseMode::Usb,
        data_mode: true,
        filter: 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_band_plan_uses_mode_specific_center_frequencies() {
        let ft8 = workspace_band_plan(WorkspaceMode::Ft8);
        assert_eq!(
            ft8.iter()
                .find(|(label, _)| *label == "20m")
                .map(|(_, freq)| *freq),
            Some(14_074_000)
        );
        let cw = workspace_band_plan(WorkspaceMode::Cw);
        assert_eq!(
            cw.iter()
                .find(|(label, _)| *label == "40m")
                .map(|(_, freq)| *freq),
            Some(7_030_000)
        );
        let wspr = workspace_band_plan(WorkspaceMode::Wspr);
        assert_eq!(
            wspr.iter()
                .find(|(label, _)| *label == "20m")
                .map(|(_, freq)| *freq),
            Some(14_095_600)
        );
    }

    #[test]
    fn band_picker_includes_bands_missing_from_focused_mode_plan() {
        let picker = band_picker_plan(WorkspaceMode::Ft8);
        assert!(picker.iter().any(|(label, _)| *label == "160m"));
        assert!(picker.iter().any(|(label, _)| *label == "60m"));
        assert!(picker.iter().any(|(label, _)| *label == "17m"));
        assert_eq!(picker.len(), CORE_BAND_LABELS.len());
    }

    #[test]
    fn software_cw_uses_usb_data_for_audio_subband_operation() {
        let preset = workspace_radio_preset(WorkspaceMode::Cw);
        assert_eq!(preset.base_mode, BaseMode::Usb);
        assert!(preset.data_mode);
        assert_eq!(preset.filter, 1);
    }

    #[test]
    fn sstv_uses_arrl_calling_centers_and_voice_usb() {
        let sstv = workspace_band_plan(WorkspaceMode::Sstv);
        assert!(sstv.contains(&("80m", 3_845_000)));
        assert!(sstv.contains(&("20m", 14_230_000)));
        assert!(sstv.contains(&("10m", 28_680_000)));
        let preset = workspace_radio_preset(WorkspaceMode::Sstv);
        assert_eq!(preset.base_mode, BaseMode::Usb);
        assert!(!preset.data_mode);
        assert_eq!(preset.filter, 1);
    }

    #[test]
    fn voice_uses_plain_usb_instead_of_data_mode() {
        let preset = workspace_radio_preset(WorkspaceMode::Voice);
        assert_eq!(preset.base_mode, BaseMode::Usb);
        assert!(!preset.data_mode);
    }

    #[test]
    fn voice_uses_band_conventional_sidebands() {
        assert_eq!(
            workspace_radio_preset_for_frequency(WorkspaceMode::Voice, Some(7_200_000)).base_mode,
            BaseMode::Lsb
        );
        assert_eq!(
            workspace_radio_preset_for_frequency(WorkspaceMode::Voice, Some(14_300_000)).base_mode,
            BaseMode::Usb
        );
        assert_eq!(
            workspace_radio_preset_for_frequency(WorkspaceMode::Voice, Some(146_520_000)).base_mode,
            BaseMode::Fm
        );
    }
}
