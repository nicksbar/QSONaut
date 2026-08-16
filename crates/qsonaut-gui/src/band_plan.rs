use qsonaut_radio::BaseMode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
            Self::Cw | Self::Fldigi => None,
        }
    }

    pub(super) fn has_native_decoder(self) -> bool {
        !matches!(self, Self::Cw | Self::Fldigi)
    }
}

pub(super) const WORKSPACE_MODES: [WorkspaceMode; 10] = [
    WorkspaceMode::Ft8,
    WorkspaceMode::Ft4,
    WorkspaceMode::Fst4,
    WorkspaceMode::Wspr,
    WorkspaceMode::Jt9,
    WorkspaceMode::Jt65,
    WorkspaceMode::Q65,
    WorkspaceMode::Msk144,
    WorkspaceMode::Cw,
    WorkspaceMode::Fldigi,
];

static FT8_BANDS: &[(&str, u64)] = &[
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
];

static FT4_BANDS: &[(&str, u64)] = &[
    ("80m", 3_575_000),
    ("40m", 7_047_500),
    ("30m", 10_140_000),
    ("20m", 14_080_000),
    ("17m", 18_104_000),
    ("15m", 21_140_000),
    ("12m", 24_919_000),
    ("10m", 28_180_000),
    ("6m", 50_318_000),
];

static FST4_BANDS: &[(&str, u64)] = &[
    ("80m", 3_573_000),
    ("40m", 7_047_500),
    ("30m", 10_140_000),
    ("20m", 14_080_000),
    ("17m", 18_104_000),
    ("15m", 21_140_000),
    ("12m", 24_919_000),
    ("10m", 28_180_000),
];

static WSPR_BANDS: &[(&str, u64)] = &[
    ("160m", 1_836_600),
    ("80m", 3_568_600),
    ("60m", 5_287_200),
    ("40m", 7_038_600),
    ("30m", 10_138_700),
    ("20m", 14_095_600),
    ("17m", 18_104_600),
    ("15m", 21_094_600),
    ("12m", 24_924_600),
    ("10m", 28_124_600),
    ("6m", 50_294_400),
];

static JT9_BANDS: &[(&str, u64)] = &[
    ("160m", 1_839_000),
    ("80m", 3_578_000),
    ("40m", 7_078_000),
    ("30m", 10_140_000),
    ("20m", 14_078_000),
    ("17m", 18_104_000),
    ("15m", 21_078_000),
    ("12m", 24_919_000),
    ("10m", 28_078_000),
    ("6m", 50_312_000),
];

static JT65_BANDS: &[(&str, u64)] = &[
    ("160m", 1_838_000),
    ("80m", 3_576_000),
    ("40m", 7_076_000),
    ("30m", 10_138_000),
    ("20m", 14_076_000),
    ("17m", 18_102_000),
    ("15m", 21_076_000),
    ("12m", 24_917_000),
    ("10m", 28_076_000),
    ("6m", 50_310_000),
];

static Q65_BANDS: &[(&str, u64)] = &[
    ("160m", 1_838_000),
    ("80m", 3_576_000),
    ("40m", 7_076_000),
    ("30m", 10_138_000),
    ("20m", 14_076_000),
    ("17m", 18_102_000),
    ("15m", 21_076_000),
    ("12m", 24_917_000),
    ("10m", 28_076_000),
    ("6m", 50_313_000),
];

static MSK144_BANDS: &[(&str, u64)] = &[
    ("6m", 50_280_000),
    ("2m", 144_360_000),
    ("70cm", 432_360_000),
];

static CW_BANDS: &[(&str, u64)] = &[
    ("80m", 3_560_000),
    ("40m", 7_030_000),
    ("30m", 10_106_000),
    ("20m", 14_060_000),
    ("17m", 18_096_000),
    ("15m", 21_060_000),
    ("12m", 24_906_000),
    ("10m", 28_060_000),
];

static FLDIGI_BANDS: &[(&str, u64)] = &[
    ("80m", 3_580_000),
    ("40m", 7_080_000),
    ("30m", 10_140_000),
    ("20m", 14_080_000),
    ("17m", 18_100_000),
    ("15m", 21_080_000),
    ("12m", 24_920_000),
    ("10m", 28_080_000),
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
        WorkspaceMode::Ft8 => FT8_BANDS,
        WorkspaceMode::Ft4 => FT4_BANDS,
        WorkspaceMode::Fst4 => FST4_BANDS,
        WorkspaceMode::Wspr => WSPR_BANDS,
        WorkspaceMode::Jt9 => JT9_BANDS,
        WorkspaceMode::Jt65 => JT65_BANDS,
        WorkspaceMode::Q65 => Q65_BANDS,
        WorkspaceMode::Msk144 => MSK144_BANDS,
        WorkspaceMode::Cw => CW_BANDS,
        WorkspaceMode::Fldigi => FLDIGI_BANDS,
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct WorkspaceRadioPreset {
    pub(super) base_mode: BaseMode,
    pub(super) data_mode: bool,
    pub(super) filter: u8,
}

pub(super) fn workspace_radio_preset(mode: WorkspaceMode) -> WorkspaceRadioPreset {
    match mode {
        WorkspaceMode::Cw => WorkspaceRadioPreset {
            base_mode: BaseMode::Cw,
            data_mode: false,
            filter: 2,
        },
        _ => WorkspaceRadioPreset {
            base_mode: BaseMode::Usb,
            data_mode: true,
            filter: 1,
        },
    }
}
