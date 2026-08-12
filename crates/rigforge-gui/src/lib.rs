mod ft8_ops;

use anyhow::{anyhow, Context, Result};
use eframe::egui::{self, Color32, ColorImage, RichText, TextureHandle, TextureOptions};
use mfsk_core::{
    ft8::decode::WsjtxDepth,
    ft8::wave_gen::{message_to_tones as ft8_message_to_tones, tones_to_i16 as ft8_tones_to_i16},
    ft8::Ft8,
    msg::{
        decode_request::DecodeRequest,
        wsjt77::{pack77, unpack77},
    },
};
use rigforge_audio::{play_pcm_blocking, AudioService};
use rigforge_accelerate::{
    AccelerationReport, ActiveBackend, ComputePreference, DecodeTelemetry, DecodeTrace,
};
use rigforge_core::AppConfig;
use rigforge_dsp::resample::Decimator;
use rigforge_log::{QsoLog, QsoRecord};
use rigforge_radio::{
    enumerate_serial_ports, BaseMode, ControlId, ControlValue, IcomCiVRadio, Mode, Radio, RadioHal,
};
use rustfft::{num_complex::Complex, FftPlanner};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet, VecDeque};
use std::f32::consts::PI;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tracing::{debug, info};

use ft8_ops::{
    next_reply_period, next_tx_period, parse_message, select_candidate, should_retry_after_decode,
    AutoReplyPolicy, Ft8Session, QsoStage, ReplyCandidate, MAX_ATTEMPTS_PER_EXCHANGE,
};

const RADIO_WF_WIDTH: usize = 360;
const RADIO_WF_HEIGHT: usize = 180;
const AUDIO_BINS: usize = 512;
const AUDIO_WF_HEIGHT: usize = 120;
const AUDIO_MAX_FREQ_HZ: u32 = 4_000;
// 8192 samples @ 48 kHz = 170 ms window, ~5.9 Hz/bin, ~683 useful bins for 0-4 kHz.
const FFT_SIZE: usize = 8192;
const OPERATOR_PROFILE_FILE: &str = "profile.toml";
const OPERATOR_PROFILE_VERSION: u32 = 4;
const GUI_SCALE_BASE: f32 = 1.6;
const GUI_SCALE_MIN: f32 = 1.2;
const GUI_SCALE_MAX: f32 = 2.0;
const QSO_LOG_FILE: &str = "log.toml";
const QSO_ADIF_FILE: &str = "log.adi";
const LEGACY_OPERATOR_PROFILE_FILE: &str = ".rigforge_profile.toml";
const FT8_SLOT_MS: u128 = 15_000;
// The generated waveform starts at +0.5 s and ends at about +13.14 s.
const FT8_EARLY_DECODE_S: f64 = 13.2;
const FT8_SLOT_SAMPLES: usize = 12_000 * 15;
const FT8_DEEP_RUNTIME_BUDGET_MS: u128 = 12_000;
// mfsk-core's WSJT-X depth/recall ladder is calibrated at 1.3. In particular,
// D2 scales this to WSJT-X's 2.0 early-pass threshold; using 1.9 here had
// unintentionally raised the early gate to ~2.92 and discarded weak signals.
const FT8_FAST_SYNC_MIN: f32 = 1.3;
const FT8_FAST_MAX_CAND: usize = 96;
const FT8_DEEP_SYNC_MIN: f32 = 1.3;
const FT8_DEEP_MAX_CAND: usize = 120;
const FT8_TX_AMPLITUDE_I16: i16 = 18_000;
const FT8_TX_SAMPLE_RATE_HZ: u32 = 12_000;
const FT8_TX_MONITOR_FFT_SIZE: usize = 2_048;
const FT8_TX_MONITOR_HOP_SAMPLES: usize = 500;
const FT8_TX_AUDIO_START_S: f64 = ft8_ops::AUDIO_START_SECONDS;
const FT8_MAX_AUDIO_LATE_S: f64 = 1.75;
const FT8_ADAPTIVE_OFFSET_LIMIT_S: f32 = 2.5;
const FT4_SLOT_SECONDS: f64 = 7.5;
const FT4_SLOT_SAMPLES: usize = 12_000 * 15 / 2;
// FT4 occupies 103 x 48 ms after its nominal +0.5 s start.
const FT4_EARLY_DECODE_S: f64 = 6.6;
const FT4_ADAPTIVE_OFFSET_LIMIT_S: f32 = 1.0;

#[derive(Debug, Default)]
struct Ft8SlotGate {
    observed_period: Option<u64>,
    ready_after_boundary: bool,
    decoded_period: Option<u64>,
}

impl Ft8SlotGate {
    #[cfg(test)]
    fn observe(&mut self, period: u64, slot_position_s: f64, buffer_ready: bool) -> bool {
        self.observe_at(period, slot_position_s, FT8_EARLY_DECODE_S, buffer_ready)
    }

    fn observe_at(
        &mut self,
        period: u64,
        slot_position_s: f64,
        decode_at_s: f64,
        buffer_ready: bool,
    ) -> bool {
        match self.observed_period {
            None => {
                self.observed_period = Some(period);
                false
            }
            Some(observed) if observed != period => {
                self.observed_period = Some(period);
                self.ready_after_boundary = true;
                self.decoded_period = None;
                false
            }
            Some(_)
                if self.ready_after_boundary
                    && self.decoded_period != Some(period)
                    && slot_position_s >= decode_at_s
                    && buffer_ready =>
            {
                self.decoded_period = Some(period);
                true
            }
            Some(_) => false,
        }
    }

    fn reset(&mut self) {
        self.observed_period = None;
        self.ready_after_boundary = false;
        self.decoded_period = None;
    }

    fn skip(&mut self, period: u64) {
        if self.observed_period == Some(period) && self.ready_after_boundary {
            self.decoded_period = Some(period);
        }
    }
}

#[derive(Debug, Default)]
struct DigitalSlotGate {
    observed_period: Option<u64>,
}

impl DigitalSlotGate {
    fn boundary(&mut self, period: u64, buffer_ready: bool) -> bool {
        match self.observed_period {
            None => {
                self.observed_period = Some(period);
                false
            }
            Some(observed) if observed != period => {
                self.observed_period = Some(period);
                buffer_ready
            }
            Some(_) => false,
        }
    }

    fn reset(&mut self) {
        self.observed_period = None;
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OperatorProfile {
    #[serde(default)]
    profile_version: u32,
    callsign: String,
    grid: String,
    qth: String,
    follow_log: bool,
    max_log_entries: usize,
    deep_decode: bool,
    #[serde(default)]
    ft4_deep_decode: bool,
    #[serde(default)]
    ft4_autoseq: bool,
    #[serde(default)]
    ft4_auto_reply_policy: AutoReplyPolicy,
    #[serde(default)]
    ft4_cq_only_view: bool,
    #[serde(default = "default_follow_log")]
    ft4_follow_log: bool,
    #[serde(default = "default_max_log_entries")]
    ft4_max_log_entries: usize,
    #[serde(default)]
    autoseq: bool,
    #[serde(default)]
    auto_reply_policy: AutoReplyPolicy,
    #[serde(default)]
    auto_answer_cq: bool,
    #[serde(default)]
    cq_only_view: bool,
    #[serde(default)]
    civ_spectrum_on: bool,
    #[serde(default = "default_halt_after_tx")]
    halt_after_tx: bool,
    #[serde(default = "default_hold_tx_freq")]
    hold_tx_freq: bool,
    #[serde(default = "default_rx_tone_hz")]
    rx_tone_hz: u32,
    #[serde(default = "default_tx_tone_hz")]
    tx_tone_hz: u32,
    #[serde(default = "default_ptt_lead_ms")]
    ptt_lead_ms: u64,
    #[serde(default = "default_ptt_tail_ms")]
    ptt_tail_ms: u64,
    #[serde(default)]
    audio_input_device: Option<String>,
    #[serde(default)]
    audio_output_device: Option<String>,
    #[serde(default)]
    radio_serial_port: Option<String>,
    #[serde(default = "default_gui_scale")]
    gui_scale: f32,
    #[serde(default)]
    compute_preference: ComputePreference,
}

fn default_gui_scale() -> f32 {
    GUI_SCALE_BASE
}
fn default_follow_log() -> bool {
    true
}
fn default_max_log_entries() -> usize {
    300
}

fn default_rx_tone_hz() -> u32 {
    1500
}
fn default_tx_tone_hz() -> u32 {
    1500
}
fn default_halt_after_tx() -> bool {
    false
}
fn default_hold_tx_freq() -> bool {
    false
}
fn default_ptt_lead_ms() -> u64 {
    (ft8_ops::DEFAULT_PTT_LEAD_SECONDS * 1_000.0) as u64
}
fn default_ptt_tail_ms() -> u64 {
    100
}

fn should_move_tx_to_decode(message: &ft8_ops::ParsedMessage, continuing_exchange: bool) -> bool {
    !continuing_exchange && message.is_cq
}

fn operator_profile_path() -> PathBuf {
    rigforge_data_dir().join(OPERATOR_PROFILE_FILE)
}

fn qso_log_path() -> PathBuf {
    rigforge_data_dir().join(QSO_LOG_FILE)
}

fn qso_adif_path() -> PathBuf {
    rigforge_data_dir().join(QSO_ADIF_FILE)
}

fn rigforge_data_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    if let Some(root) = std::env::var_os("APPDATA") {
        return PathBuf::from(root).join("RigForge");
    }
    #[cfg(target_os = "linux")]
    {
        if let Some(root) = std::env::var_os("XDG_CONFIG_HOME") {
            return PathBuf::from(root).join("rigforge");
        }
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(".config").join("rigforge");
        }
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

fn load_operator_profile() -> Option<OperatorProfile> {
    let preferred = operator_profile_path();
    let legacy = std::env::current_dir()
        .ok()
        .map(|dir| dir.join(LEGACY_OPERATOR_PROFILE_FILE));
    let src = fs::read_to_string(&preferred)
        .ok()
        .or_else(|| legacy.and_then(|path| fs::read_to_string(path).ok()))?;
    toml::from_str::<OperatorProfile>(&src).ok()
}

fn save_operator_profile(profile: &OperatorProfile) -> Result<()> {
    let path = operator_profile_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let body = toml::to_string_pretty(profile)?;
    fs::write(&path, body)?;
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WaterfallSpeed {
    Slow,
    Mid,
    Fast,
}

#[derive(Debug, Clone)]
struct DisplayTuning {
    auto_visual: bool,
    waterfall_speed: WaterfallSpeed,
}

impl Default for DisplayTuning {
    fn default() -> Self {
        Self {
            auto_visual: true,
            waterfall_speed: WaterfallSpeed::Mid,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkspaceMode {
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
    fn label(self) -> &'static str {
        match self {
            WorkspaceMode::Ft8 => "FT8",
            WorkspaceMode::Ft4 => "FT4",
            WorkspaceMode::Fst4 => "FST4",
            WorkspaceMode::Wspr => "WSPR",
            WorkspaceMode::Jt9 => "JT9",
            WorkspaceMode::Jt65 => "JT65",
            WorkspaceMode::Q65 => "Q65",
            WorkspaceMode::Msk144 => "MSK144",
            WorkspaceMode::Cw => "CW",
            WorkspaceMode::Fldigi => "FLDIGI",
        }
    }

    fn core_slot_seconds(self) -> Option<f64> {
        match self {
            WorkspaceMode::Ft8 => Some(15.0),
            WorkspaceMode::Ft4 => Some(7.5),
            WorkspaceMode::Fst4 => Some(60.0),
            WorkspaceMode::Wspr => Some(120.0),
            WorkspaceMode::Jt9 | WorkspaceMode::Jt65 => Some(60.0),
            WorkspaceMode::Q65 => Some(30.0),
            WorkspaceMode::Msk144 => Some(15.0),
            WorkspaceMode::Cw | WorkspaceMode::Fldigi => None,
        }
    }

    fn has_native_decoder(self) -> bool {
        !matches!(self, WorkspaceMode::Cw | WorkspaceMode::Fldigi)
    }
}

const WORKSPACE_MODES: [WorkspaceMode; 10] = [
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

/// IC-7300 FT8 band table: label, FT8 dial frequency.
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

fn band_for_frequency(frequency_hz: u64) -> &'static str {
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

fn format_signal_report(report: i8) -> String {
    format!("{:+03}", report.clamp(-50, 49))
}

fn workspace_band_plan(mode: WorkspaceMode) -> &'static [(&'static str, u64)] {
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
struct WorkspaceRadioPreset {
    base_mode: BaseMode,
    data_mode: bool,
    filter: u8,
}

fn workspace_radio_preset(mode: WorkspaceMode) -> WorkspaceRadioPreset {
    match mode {
        WorkspaceMode::Cw => WorkspaceRadioPreset {
            base_mode: BaseMode::Cw,
            data_mode: false,
            filter: 2,
        },
        WorkspaceMode::Ft8
        | WorkspaceMode::Ft4
        | WorkspaceMode::Fst4
        | WorkspaceMode::Wspr
        | WorkspaceMode::Jt9
        | WorkspaceMode::Jt65
        | WorkspaceMode::Q65
        | WorkspaceMode::Msk144
        | WorkspaceMode::Fldigi => WorkspaceRadioPreset {
            base_mode: BaseMode::Usb,
            data_mode: true,
            filter: 1,
        },
    }
}

fn build_ft8_tx_pcm(compose: &str, tx_tone_hz: u32) -> Result<Vec<i16>> {
    let tokens: Vec<&str> = compose.split_whitespace().collect();
    if tokens.len() != 3 {
        anyhow::bail!("standard FT8 TX needs exactly 3 fields (DESTINATION SOURCE GRID/REPORT)");
    }
    let msg77 = if tokens[0].eq_ignore_ascii_case("CQ") {
        pack77("CQ", tokens[1], tokens[2])
            .ok_or_else(|| anyhow!("unable to pack FT8 CQ message: {compose}"))?
    } else {
        pack77(tokens[0], tokens[1], tokens[2])
            .ok_or_else(|| anyhow!("unable to pack FT8 standard message: {compose}"))?
    };
    let tones = ft8_message_to_tones(&msg77);
    Ok(ft8_tones_to_i16(
        &tones,
        tx_tone_hz as f32,
        FT8_TX_AMPLITUDE_I16,
    ))
}

fn build_native_digital_tx_pcm(
    mode: WorkspaceMode,
    compose: &str,
    tx_tone_hz: u32,
) -> Result<(Vec<i16>, f64)> {
    let tokens: Vec<&str> = compose.split_whitespace().collect();
    if tokens.len() != 3 {
        anyhow::bail!("{} TX needs exactly 3 message fields", mode.label());
    }
    let to_i16 = |audio: Vec<f32>| {
        audio
            .into_iter()
            .map(|sample| (sample.clamp(-1.0, 1.0) * FT8_TX_AMPLITUDE_I16 as f32).round() as i16)
            .collect::<Vec<_>>()
    };
    let tone = tx_tone_hz as f32;
    match mode {
        WorkspaceMode::Ft4 | WorkspaceMode::Fst4 => {
            let bits = pack77(tokens[0], tokens[1], tokens[2])
                .ok_or_else(|| anyhow!("unable to pack {} message", mode.label()))?;
            if mode == WorkspaceMode::Ft4 {
                let tones = mfsk_core::ft4::encode::message_to_tones(&bits);
                Ok((
                    mfsk_core::ft4::encode::tones_to_i16(&tones, tone, FT8_TX_AMPLITUDE_I16),
                    0.5,
                ))
            } else {
                let tones = mfsk_core::fst4::encode::message_to_tones(&bits);
                Ok((
                    mfsk_core::fst4::encode::tones_to_i16(&tones, tone, FT8_TX_AMPLITUDE_I16),
                    1.0,
                ))
            }
        }
        WorkspaceMode::Jt9 => {
            mfsk_core::jt9::synthesize_standard(tokens[0], tokens[1], tokens[2], 12_000, tone, 1.0)
                .map(|audio| (to_i16(audio), 0.0))
                .ok_or_else(|| anyhow!("unable to pack JT9 message"))
        }
        WorkspaceMode::Jt65 => {
            mfsk_core::jt65::synthesize_standard(tokens[0], tokens[1], tokens[2], 12_000, tone, 1.0)
                .map(|audio| (to_i16(audio), 0.0))
                .ok_or_else(|| anyhow!("unable to pack JT65 message"))
        }
        WorkspaceMode::Q65 => {
            mfsk_core::q65::synthesize_standard(tokens[0], tokens[1], tokens[2], 12_000, tone, 1.0)
                .map(|audio| (to_i16(audio), 1.0))
                .ok_or_else(|| anyhow!("unable to pack Q65 message"))
        }
        _ => anyhow::bail!("{} transmit synthesis is not available", mode.label()),
    }
}

fn play_ft8_tx_pcm(
    pcm: &[i16],
    abort: Arc<AtomicBool>,
    output_device: Option<&str>,
) -> Result<()> {
    play_pcm_blocking(
        pcm,
        FT8_TX_SAMPLE_RATE_HZ,
        output_device,
        abort,
    )
    .context("native audio output failed")
}

fn wait_until_epoch(target_s: f64, abort: &AtomicBool) -> Result<()> {
    loop {
        if abort.load(Ordering::Relaxed) {
            anyhow::bail!("TX aborted by operator");
        }
        let now_s = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs_f64())
            .unwrap_or(0.0);
        let remaining = target_s - now_s;
        if remaining <= 0.0 {
            return Ok(());
        }
        thread::sleep(Duration::from_secs_f64(remaining.min(0.025)));
    }
}

fn request_ptt(
    command_tx: &mpsc::Sender<GuiCommand>,
    enabled: bool,
    timeout: Duration,
) -> Result<()> {
    let (ack_tx, ack_rx) = mpsc::channel();
    command_tx
        .send(GuiCommand::SetPttWithAck(enabled, ack_tx))
        .context("radio command worker is unavailable")?;
    match ack_rx.recv_timeout(timeout) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(anyhow!(error)),
        Err(_) => anyhow::bail!(
            "radio did not confirm PTT {}",
            if enabled { "ON" } else { "OFF" }
        ),
    }
}

struct Ft8TxJob {
    period: u64,
    pcm: Arc<Vec<i16>>,
    ptt_lead: Duration,
    ptt_tail: Duration,
    output_device: Option<String>,
    abort: Arc<AtomicBool>,
    active: Arc<AtomicBool>,
    command_tx: mpsc::Sender<GuiCommand>,
    event_tx: mpsc::Sender<Ft8TxEvent>,
    state: Arc<Mutex<GuiState>>,
    repaint_ctx: Arc<OnceLock<egui::Context>>,
}

fn run_ft8_tx_job(job: Ft8TxJob) {
    let slot_start_s = job.period as f64 * ft8_ops::SLOT_SECONDS;
    let audio_start_s = slot_start_s + FT8_TX_AUDIO_START_S;
    let ptt_start_s = audio_start_s - job.ptt_lead.as_secs_f64();

    let result = (|| -> Result<()> {
        wait_until_epoch(ptt_start_s, &job.abort)?;
        request_ptt(&job.command_tx, true, Duration::from_secs(2))?;
        let _ = job.event_tx.send(Ft8TxEvent::PttConfirmed);

        wait_until_epoch(audio_start_s, &job.abort)?;
        let now_s = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs_f64())
            .unwrap_or(audio_start_s);
        let audio_late_s = now_s - audio_start_s;
        if audio_late_s > FT8_MAX_AUDIO_LATE_S {
            anyhow::bail!("PTT confirmation arrived too late for a valid FT8 frame");
        }
        info!(
            period = job.period,
            audio_late_ms = (audio_late_s.max(0.0) * 1_000.0).round() as u64,
            "FT8 TX audio starting"
        );

        let _ = job.event_tx.send(Ft8TxEvent::AudioStarted);
        let monitor_stop = Arc::new(AtomicBool::new(false));
        let monitor_handle = {
            let pcm = job.pcm.clone();
            let stop = monitor_stop.clone();
            let abort = job.abort.clone();
            let state = job.state.clone();
            let repaint_ctx = job.repaint_ctx.clone();
            thread::spawn(move || monitor_ft8_tx_waterfall(pcm, stop, abort, state, repaint_ctx))
        };
        let playback_result = play_ft8_tx_pcm(
            &job.pcm,
            job.abort.clone(),
            job.output_device.as_deref(),
        );
        monitor_stop.store(true, Ordering::Release);
        let _ = monitor_handle.join();
        playback_result?;
        if !job.ptt_tail.is_zero() {
            thread::sleep(job.ptt_tail);
        }
        Ok(())
    })();

    // PTT release is unconditional, including playback, timeout, and abort failures.
    let unkey_result = request_ptt(&job.command_tx, false, Duration::from_secs(2));
    job.active.store(false, Ordering::Release);

    match (result, unkey_result) {
        (Ok(()), Ok(())) => {
            let _ = job.event_tx.send(Ft8TxEvent::Complete);
        }
        (Err(error), Ok(())) => {
            let _ = job.event_tx.send(Ft8TxEvent::Failed(error.to_string()));
        }
        (Ok(()), Err(error)) => {
            let _ = job.event_tx.send(Ft8TxEvent::Failed(format!(
                "TX audio completed but PTT release failed: {error}"
            )));
        }
        (Err(error), Err(unkey_error)) => {
            let _ = job.event_tx.send(Ft8TxEvent::Failed(format!(
                "{error}; PTT release also failed: {unkey_error}"
            )));
        }
    }
}

fn monitor_ft8_tx_waterfall(
    pcm: Arc<Vec<i16>>,
    stop: Arc<AtomicBool>,
    abort: Arc<AtomicBool>,
    state: Arc<Mutex<GuiState>>,
    repaint_ctx: Arc<OnceLock<egui::Context>>,
) {
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(FT8_TX_MONITOR_FFT_SIZE);
    let mut fft_buf = vec![Complex::<f32>::new(0.0, 0.0); FT8_TX_MONITOR_FFT_SIZE];
    let started = Instant::now();

    for start in (0..pcm.len()).step_by(FT8_TX_MONITOR_HOP_SAMPLES) {
        let target = Duration::from_secs_f64(start as f64 / FT8_TX_SAMPLE_RATE_HZ as f64);
        while started.elapsed() < target {
            if stop.load(Ordering::Acquire) || abort.load(Ordering::Relaxed) {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        if stop.load(Ordering::Acquire) || abort.load(Ordering::Relaxed) {
            return;
        }

        for (offset, sample) in fft_buf.iter_mut().enumerate() {
            let value =
                pcm.get(start + offset).copied().unwrap_or_default() as f32 / i16::MAX as f32;
            let window =
                0.5 - 0.5 * (2.0 * PI * offset as f32 / (FT8_TX_MONITOR_FFT_SIZE - 1) as f32).cos();
            *sample = Complex::new(value * window, 0.0);
        }
        fft.process(&mut fft_buf);
        let bins = fft_buffer_to_display_bins(&fft_buf, AUDIO_BINS, FT8_TX_SAMPLE_RATE_HZ);
        let mut snapshot = state.lock().expect("ui state lock poisoned");
        if snapshot.audio_waterfall_rows.len() >= AUDIO_WF_HEIGHT {
            snapshot.audio_waterfall_rows.pop_front();
        }
        snapshot.audio_waterfall_rows.push_back(bins);
        snapshot.audio_waterfall_revision = snapshot.audio_waterfall_revision.wrapping_add(1);
        snapshot.audio_spectrum_status = "TX OUTPUT".to_string();
        drop(snapshot);
        if let Some(ctx) = repaint_ctx.get() {
            ctx.request_repaint();
        }
    }
}

struct DigitalTxJob {
    mode: WorkspaceMode,
    period: u64,
    slot_seconds: f64,
    audio_offset_s: f64,
    pcm: Arc<Vec<i16>>,
    ptt_lead: Duration,
    ptt_tail: Duration,
    output_device: Option<String>,
    abort: Arc<AtomicBool>,
    active: Arc<AtomicBool>,
    command_tx: mpsc::Sender<GuiCommand>,
    event_tx: mpsc::Sender<DigitalTxEvent>,
    state: Arc<Mutex<GuiState>>,
    repaint_ctx: Arc<OnceLock<egui::Context>>,
}

fn run_digital_tx_job(job: DigitalTxJob) {
    let slot_start_s = job.period as f64 * job.slot_seconds;
    let audio_start_s = slot_start_s + job.audio_offset_s;
    let ptt_start_s = audio_start_s - job.ptt_lead.as_secs_f64();
    let result = (|| -> Result<()> {
        wait_until_epoch(ptt_start_s, &job.abort)?;
        request_ptt(&job.command_tx, true, Duration::from_secs(2))?;
        wait_until_epoch(audio_start_s, &job.abort)?;
        info!(
            mode = job.mode.label(),
            period = job.period,
            "digital TX audio starting"
        );
        let _ = job
            .event_tx
            .send(DigitalTxEvent::AudioStarted(job.mode, job.period));
        let monitor_stop = Arc::new(AtomicBool::new(false));
        let monitor_handle = {
            let pcm = job.pcm.clone();
            let stop = monitor_stop.clone();
            let abort = job.abort.clone();
            let state = job.state.clone();
            let repaint_ctx = job.repaint_ctx.clone();
            thread::spawn(move || monitor_ft8_tx_waterfall(pcm, stop, abort, state, repaint_ctx))
        };
        let playback_result = play_ft8_tx_pcm(
            &job.pcm,
            job.abort.clone(),
            job.output_device.as_deref(),
        );
        monitor_stop.store(true, Ordering::Release);
        let _ = monitor_handle.join();
        playback_result?;
        if !job.ptt_tail.is_zero() {
            thread::sleep(job.ptt_tail);
        }
        Ok(())
    })();

    let unkey_result = request_ptt(&job.command_tx, false, Duration::from_secs(2));
    job.active.store(false, Ordering::Release);
    match (result, unkey_result) {
        (Ok(()), Ok(())) => {
            let _ = job.event_tx.send(DigitalTxEvent::Complete);
        }
        (Err(error), Ok(())) => {
            let _ = job.event_tx.send(DigitalTxEvent::Failed(error.to_string()));
        }
        (Ok(()), Err(error)) => {
            let _ = job.event_tx.send(DigitalTxEvent::Failed(format!(
                "audio completed but PTT release failed: {error}"
            )));
        }
        (Err(error), Err(unkey_error)) => {
            let _ = job.event_tx.send(DigitalTxEvent::Failed(format!(
                "{error}; PTT release also failed: {unkey_error}"
            )));
        }
    }
}

/// One decoded FT8 frame in the log.
#[derive(Debug, Clone)]
struct Ft8DecodeEntry {
    period: u64,
    utc: String,
    snr_db: i8,
    dt_s: f32,
    freq_hz: u32,
    message: String,
    is_cq: bool,
}

#[derive(Debug, Clone)]
struct Ft8TxChatEntry {
    period: u64,
    utc: String,
    message: String,
}

#[derive(Debug, Default)]
struct Ft8ActivityStats {
    latest_cycle: usize,
    average_per_cycle: f32,
    cq_this_cycle: usize,
    unique_stations: usize,
    most_heard: Option<(String, usize)>,
    median_snr: Option<i8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ft8ChatDirection {
    Rx,
    Tx,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OperatorCallHit {
    DirectedToMe,
    Mentioned,
}

fn operator_call_hit(message: &str, callsign: &str) -> Option<OperatorCallHit> {
    let callsign = callsign
        .trim_matches(|c| c == '<' || c == '>')
        .trim()
        .to_ascii_uppercase();
    if callsign.is_empty() || callsign == "N0CALL" {
        return None;
    }
    if parse_message(message).is_some_and(|parsed| parsed.directed_to(&callsign)) {
        return Some(OperatorCallHit::DirectedToMe);
    }
    message
        .split_whitespace()
        .map(|token| {
            token
                .trim_matches(|c: char| c == '<' || c == '>' || c.is_ascii_punctuation())
                .to_ascii_uppercase()
        })
        .any(|token| token == callsign)
        .then_some(OperatorCallHit::Mentioned)
}

fn call_hit_badge(hit: OperatorCallHit) -> (&'static str, Color32, Color32) {
    match hit {
        OperatorCallHit::DirectedToMe => (
            "📡 YOU!",
            Color32::from_rgb(255, 66, 196),
            Color32::from_rgb(73, 18, 69),
        ),
        OperatorCallHit::Mentioned => (
            "✨ YOUR CALL",
            Color32::from_rgb(87, 226, 255),
            Color32::from_rgb(18, 56, 73),
        ),
    }
}

fn draw_operator_call_banner(
    ui: &mut egui::Ui,
    mode: &str,
    callsign: &str,
    message: &str,
    hit: OperatorCallHit,
) {
    let (badge, accent, fill) = call_hit_badge(hit);
    egui::Frame::group(ui.style())
        .fill(fill)
        .stroke(egui::Stroke::new(2.0, accent))
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(badge).strong().size(16.0).color(accent));
                ui.label(
                    RichText::new(format!("{callsign} lit up the {mode} receiver"))
                        .strong()
                        .color(Color32::WHITE),
                );
                ui.separator();
                ui.label(RichText::new(message).monospace().strong().color(accent));
            });
        });
}

#[derive(Debug, Clone)]
struct Ft8ChatLine {
    period: u64,
    utc: String,
    message: String,
    detail: String,
    direction: Ft8ChatDirection,
}

#[derive(Debug, Clone)]
struct DigitalDecodeEntry {
    mode: WorkspaceMode,
    period: u64,
    utc: String,
    snr_db: f32,
    dt_s: f32,
    freq_hz: u32,
    message: String,
}

#[derive(Debug, Clone)]
struct DigitalTxChatEntry {
    mode: WorkspaceMode,
    period: u64,
    utc: String,
    message: String,
}

#[derive(Debug)]
struct PendingFt8Decode {
    samples: Vec<f32>,
    utc: String,
    period: u64,
    deep_decode: bool,
    alignment_s: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ft8SeqState {
    Idle,
    CqArmed,
    ReplyArmed,
    TxQueued,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ft8TxQueuePolicy {
    Standard,
    ReplyAsap,
    NextSlotOnly,
}

#[derive(Debug)]
enum Ft8TxEvent {
    PttConfirmed,
    AudioStarted,
    Complete,
    Failed(String),
}

#[derive(Debug)]
enum DigitalTxEvent {
    AudioStarted(WorkspaceMode, u64),
    Complete,
    Failed(String),
}

#[derive(Debug)]
struct PendingManualFt8Reply {
    compose: String,
    target: String,
    session: Ft8Session,
    freq_hz: u32,
    source_period: u64,
    move_tx_to_remote: bool,
}

impl Ft8SeqState {
    fn label(self) -> &'static str {
        match self {
            Ft8SeqState::Idle => "IDLE",
            Ft8SeqState::CqArmed => "CQ ARMED",
            Ft8SeqState::ReplyArmed => "REPLY ARMED",
            Ft8SeqState::TxQueued => "TX QUEUED",
        }
    }
}

#[derive(Debug, Clone)]
struct GuiState {
    frequency_hz: Option<u64>,
    mode: String,
    data_mode: Option<bool>,
    filter: Option<u8>,
    af_gain: Option<u8>,
    rf_gain: Option<u8>,
    rf_power: Option<u8>,
    ptt_on: bool,
    radio_spectrum_desired: bool,
    radio_spectrum_enabled: bool,
    radio_waterfall_status: String,
    radio_waterfall_rows: VecDeque<Vec<u8>>,
    radio_waterfall_revision: u64,
    audio_spectrum_status: String,
    audio_waterfall_rows: VecDeque<Vec<u8>>,
    audio_waterfall_revision: u64,
    audio_level_dbfs: Option<f32>,
    audio_clip_percent: f32,
    ft8_decode_status: String,
    ft8_clock_offset_s: Option<f32>,
    ft4_clock_offset_s: Option<f32>,
    workspace_mode: WorkspaceMode,
    ft8_deep_decode: bool,
    ft4_deep_decode: bool,
    ft8_pending: Vec<Ft8DecodeEntry>,
    ft8_last_decode_period: Option<u64>,
    digital_decode_status: String,
    digital_decodes: VecDeque<DigitalDecodeEntry>,
    ft4_last_decode_period: Option<u64>,
    digital_tx_period: Option<(WorkspaceMode, u64)>,
    selected_audio_hz: u32,
    compute_backend: ActiveBackend,
    ft8_compute_telemetry: Option<DecodeTelemetry>,
    digital_compute_telemetry: Option<DecodeTelemetry>,
    last_error: Option<String>,
    last_update: Option<Instant>,
}

impl Default for GuiState {
    fn default() -> Self {
        Self {
            frequency_hz: None,
            mode: "(unknown)".to_string(),
            data_mode: None,
            filter: None,
            af_gain: None,
            rf_gain: None,
            rf_power: None,
            ptt_on: false,
            radio_spectrum_desired: false,
            radio_spectrum_enabled: false,
            radio_waterfall_status: "OFF".to_string(),
            radio_waterfall_rows: VecDeque::with_capacity(RADIO_WF_HEIGHT),
            radio_waterfall_revision: 0,
            audio_spectrum_status: "INIT".to_string(),
            audio_waterfall_rows: VecDeque::with_capacity(AUDIO_WF_HEIGHT),
            audio_waterfall_revision: 0,
            audio_level_dbfs: None,
            audio_clip_percent: 0.0,
            ft8_decode_status: "STARTING".to_string(),
            ft8_clock_offset_s: None,
            ft4_clock_offset_s: None,
            workspace_mode: WorkspaceMode::Ft8,
            ft8_deep_decode: false,
            ft4_deep_decode: false,
            ft8_pending: Vec::new(),
            ft8_last_decode_period: None,
            digital_decode_status: "Select a native digital mode".to_string(),
            digital_decodes: VecDeque::with_capacity(300),
            ft4_last_decode_period: None,
            digital_tx_period: None,
            selected_audio_hz: default_rx_tone_hz(),
            compute_backend: ActiveBackend::CpuSimd,
            ft8_compute_telemetry: None,
            digital_compute_telemetry: None,
            last_error: None,
            last_update: None,
        }
    }
}

#[derive(Debug, Clone)]
enum GuiCommand {
    TuneDelta(i64),
    CycleMode,
    TogglePtt,
    AfGainDelta(i16),
    TuneWorkspaceBand(u64),
    SetFilter(u8),
    SetPtt(bool),
    SetPttWithAck(bool, mpsc::Sender<std::result::Result<(), String>>),
    Quit,
}

pub fn run_gui(config: AppConfig) -> Result<()> {
    let build_profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    info!(build_profile, "RigForge GUI startup");
    if cfg!(debug_assertions) {
        info!(
            "Running DEBUG build: FT8 decode latency will be significantly higher than release; use --release for real-time ops"
        );
    }

    info!(
        display_value = ?std::env::var("DISPLAY").ok(),
        wayland_value = ?std::env::var("WAYLAND_DISPLAY").ok(),
        winit_backend_value = ?std::env::var("WINIT_UNIX_BACKEND").ok(),
        wgpu_backend_value = ?std::env::var("WGPU_BACKEND").ok(),
        "Preparing native GUI launch"
    );

    configure_unix_gui_environment();
    // Match WSJT-X's acquisition range for the first decode. Once valid FT8
    // frames establish a median dT, the rolling capture window is aligned in
    // software on subsequent slots.
    if std::env::var_os("MFSK_SYNC_LAG_S").is_none() {
        std::env::set_var("MFSK_SYNC_LAG_S", "2.5");
    }

    info!(
        display_value = ?std::env::var("DISPLAY").ok(),
        wayland_value = ?std::env::var("WAYLAND_DISPLAY").ok(),
        winit_backend_value = ?std::env::var("WINIT_UNIX_BACKEND").ok(),
        wgpu_backend_value = ?std::env::var("WGPU_BACKEND").ok(),
        gallium_driver = ?std::env::var("GALLIUM_DRIVER").ok(),
        mesa_d3d12_adapter = ?std::env::var("MESA_D3D12_DEFAULT_ADAPTER_NAME").ok(),
        "GUI environment after configuration"
    );

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 860.0])
            .with_min_inner_size([980.0, 680.0])
            .with_title("RigForge — Radio Console")
            .with_resizable(true),
        ..Default::default()
    };

    let app_config = config.clone();
    info!(title = "RigForge", "Calling eframe::run_native");
    let result = eframe::run_native(
        "RigForge",
        options,
        Box::new(move |_cc| Ok(Box::new(RigforgeGuiApp::new(app_config.clone())))),
    );

    match result {
        Ok(_) => {
            info!("eframe run_native completed normally");
        }
        Err(err) => {
            info!(error = %err, "eframe run_native failed");
            return Err(anyhow!("eframe launch failed: {err}"));
        }
    }

    Ok(())
}

fn configure_unix_gui_environment() {
    #[cfg(target_os = "linux")]
    {
        if std::env::var_os("NO_AT_BRIDGE").is_none() {
            std::env::set_var("NO_AT_BRIDGE", "1");
        }

    }
}

struct RigforgeGuiApp {
    config: AppConfig,
    state: Arc<Mutex<GuiState>>,
    command_tx: Option<mpsc::Sender<GuiCommand>>,
    worker_stop: Arc<AtomicBool>,
    radio_worker_handle: Option<std::thread::JoinHandle<()>>,
    audio_worker_handle: Option<std::thread::JoinHandle<()>>,
    radio_waterfall_texture: Option<TextureHandle>,
    radio_waterfall_texture_revision: u64,
    audio_waterfall_texture: Option<TextureHandle>,
    audio_waterfall_texture_revision: u64,
    audio_waterfall_texture_bins: usize,
    workspace_mode: WorkspaceMode,
    display_tuning: Arc<Mutex<DisplayTuning>>,
    repaint_ctx: Arc<OnceLock<egui::Context>>,
    // FT8 workspace UX state (app-local, not shared with workers)
    ft8_log: Vec<Ft8DecodeEntry>,
    ft8_tx_chat: VecDeque<Ft8TxChatEntry>,
    ft8_seen_decode_period: Option<u64>,
    qso_log: QsoLog,
    qso_selected: Option<usize>,
    qso_log_status: String,
    qso_log_dirty: bool,
    ft8_compose: String,
    ft8_selected: Option<usize>,
    ft8_autoseq: bool,
    ft8_auto_reply_policy: AutoReplyPolicy,
    ft8_auto_answer_cq: bool,
    ft8_session: Option<Ft8Session>,
    ft8_seq_state: Ft8SeqState,
    ft8_seq_target: Option<String>,
    ft8_seq_status: String,
    ft8_last_click: Option<(usize, Instant)>,
    ft8_tx_queued_period: Option<u64>,
    ft8_tx_pcm: Option<Arc<Vec<i16>>>,
    ft8_queued_tx_message: Option<String>,
    ft8_last_tx_message: Option<String>,
    ft8_tx_started_period: Option<u64>,
    ft8_last_tx_period: Option<u64>,
    ft8_suppress_canceled_tx_events: bool,
    ft8_pending_manual_reply: Option<PendingManualFt8Reply>,
    ft8_tx_abort: Arc<AtomicBool>,
    ft8_tx_active: Arc<AtomicBool>,
    ft8_tx_event_tx: mpsc::Sender<Ft8TxEvent>,
    ft8_tx_event_rx: mpsc::Receiver<Ft8TxEvent>,
    ft8_last_tx_was_cq: bool,
    digital_compose: String,
    digital_selected: Option<DigitalDecodeEntry>,
    digital_seq_target: Option<String>,
    ft4_session: Option<Ft8Session>,
    ft4_seen_decodes: HashSet<(u64, u32, String)>,
    digital_tx_chat: VecDeque<DigitalTxChatEntry>,
    digital_queued_tx_message: Option<String>,
    digital_last_tx_message: Option<String>,
    digital_tx_status: String,
    digital_tx_started: Option<(WorkspaceMode, u64)>,
    ft4_last_tx_period: Option<u64>,
    ft4_seen_decode_period: Option<u64>,
    digital_tx_abort: Arc<AtomicBool>,
    digital_tx_active: Arc<AtomicBool>,
    digital_suppress_canceled_tx_events: bool,
    digital_tx_event_tx: mpsc::Sender<DigitalTxEvent>,
    digital_tx_event_rx: mpsc::Receiver<DigitalTxEvent>,
    ft8_halt_after_tx: bool,
    ft4_halt_after_tx: bool,
    ft8_hold_tx_freq: bool,
    ft8_deep_decode: bool,
    ft4_deep_decode: bool,
    ft4_autoseq: bool,
    ft4_auto_reply_policy: AutoReplyPolicy,
    ft4_cq_only_view: bool,
    ft4_follow_log: bool,
    ft4_max_log_entries: usize,
    ft8_cq_only_view: bool,
    ft8_follow_log: bool,
    ft8_max_log_entries: usize,
    station_callsign: String,
    station_grid: String,
    station_qth: String,
    civ_spectrum_on: bool,
    rx_tone_hz: u32,
    tx_tone_hz: u32,
    ptt_lead_ms: u64,
    ptt_tail_ms: u64,
    profile_io_status: String,
    profile_dirty: bool,
    audio_input_devices: Vec<String>,
    audio_output_devices: Vec<String>,
    radio_serial_ports: Vec<String>,
    show_signal_panel: bool,
    show_device_settings: bool,
    device_restart_required: bool,
    gui_scale: f32,
    compute_preference: ComputePreference,
    acceleration_report: AccelerationReport,
}

impl RigforgeGuiApp {
    fn new(mut config: AppConfig) -> Self {
        if let Some(profile) = load_operator_profile() {
            if profile.profile_version >= 3 {
                config.audio.input_device = profile.audio_input_device;
                config.audio.output_device = profile.audio_output_device;
                config.radio.serial_port = profile.radio_serial_port;
            }
        }

        let audio_input_devices = AudioService::input_devices().unwrap_or_default();
        let audio_output_devices = AudioService::output_devices().unwrap_or_default();
        let radio_serial_ports = enumerate_serial_ports().unwrap_or_default();
        let state = Arc::new(Mutex::new(GuiState::default()));
        let worker_stop = Arc::new(AtomicBool::new(false));
        let display_tuning = Arc::new(Mutex::new(DisplayTuning::default()));

        let repaint_ctx: Arc<OnceLock<egui::Context>> = Arc::new(OnceLock::new());

        let (command_tx, radio_worker_handle) = if config.radio.enabled {
            let port = config.radio.serial_port.clone().unwrap_or_default();
            let radio = IcomCiVRadio::new(
                port.clone(),
                config.radio.baud_rate,
                config.radio.controller_civ_address,
            )
            .with_radio_address(config.radio.civ_address);
            let (tx, rx) = mpsc::channel::<GuiCommand>();
            let display_port = if port.is_empty() {
                "auto".to_string()
            } else {
                port.clone()
            };
            info!(port = %display_port, baud = config.radio.baud_rate, "Starting GUI radio worker");
            let handle = spawn_radio_worker(
                radio,
                state.clone(),
                worker_stop.clone(),
                display_tuning.clone(),
                rx,
                repaint_ctx.clone(),
            );
            (Some(tx), Some(handle))
        } else {
            {
                let mut s = state.lock().expect("ui state lock poisoned");
                s.last_error = Some(
                    "Radio is disabled in config; UI running in monitor-only mode".to_string(),
                );
                s.radio_waterfall_status = "UNAVAILABLE (radio disabled)".to_string();
            }
            (None, None)
        };

        let ft8_tx_active = Arc::new(AtomicBool::new(false));
        let digital_tx_active = Arc::new(AtomicBool::new(false));
        let audio_worker_handle = Some(spawn_audio_spectrum_worker(
            state.clone(),
            worker_stop.clone(),
            ft8_tx_active.clone(),
            digital_tx_active.clone(),
            config.audio.enabled,
            config.audio.sample_rate_hz,
            config.audio.channels,
            config.audio.input_device.clone(),
            repaint_ctx.clone(),
            display_tuning.clone(),
        ));

        let mut station_callsign = config
            .station
            .callsign
            .clone()
            .unwrap_or_else(|| "N0CALL".to_string());
        let mut station_grid = config
            .station
            .grid
            .clone()
            .unwrap_or_else(|| "AA00".to_string());

        let mut station_qth = String::new();
        let mut ft8_follow_log = true;
        let mut ft8_max_log_entries = 300usize;
        let mut ft8_deep_decode = false;
        let mut ft4_deep_decode = false;
        let mut ft4_autoseq = false;
        let mut ft4_auto_reply_policy = AutoReplyPolicy::default();
        let mut ft4_cq_only_view = false;
        let mut ft4_follow_log = true;
        let mut ft4_max_log_entries = 300usize;
        let mut ft8_autoseq = false;
        let mut ft8_auto_reply_policy = AutoReplyPolicy::default();
        let mut ft8_auto_answer_cq = false;
        let mut ft8_cq_only_view = false;
        let mut civ_spectrum_on = false;
        let mut ft8_halt_after_tx = false;
        let mut ft8_hold_tx_freq = false;
        let mut rx_tone_hz = default_rx_tone_hz();
        let mut tx_tone_hz = default_tx_tone_hz();
        let mut ptt_lead_ms = default_ptt_lead_ms();
        let mut ptt_tail_ms = default_ptt_tail_ms();
        let mut gui_scale = default_gui_scale();
        let mut compute_preference = ComputePreference::Auto;
        let profile_io_status: String;

        if let Some(p) = load_operator_profile() {
            station_callsign = p.callsign;
            station_grid = p.grid;
            station_qth = p.qth;
            ft8_follow_log = p.follow_log;
            ft8_max_log_entries = p.max_log_entries.clamp(80, 1000);
            ft8_deep_decode = p.deep_decode;
            ft4_deep_decode = p.ft4_deep_decode;
            ft4_autoseq = p.ft4_autoseq;
            ft4_auto_reply_policy = p.ft4_auto_reply_policy;
            ft4_cq_only_view = p.ft4_cq_only_view;
            ft4_follow_log = p.ft4_follow_log;
            ft4_max_log_entries = p.ft4_max_log_entries.clamp(80, 300);
            ft8_autoseq = p.autoseq;
            ft8_auto_reply_policy = p.auto_reply_policy;
            ft8_auto_answer_cq = p.auto_answer_cq;
            ft8_cq_only_view = p.cq_only_view;
            civ_spectrum_on = p.civ_spectrum_on;
            // Stop-after-TX is a one-shot runtime request, never a startup mode.
            ft8_halt_after_tx = false;
            ft8_hold_tx_freq = if p.profile_version >= 3 {
                p.hold_tx_freq
            } else {
                false
            };
            rx_tone_hz = p.rx_tone_hz;
            tx_tone_hz = p.tx_tone_hz;
            if !ft8_hold_tx_freq {
                tx_tone_hz = rx_tone_hz;
            }
            ptt_lead_ms = p.ptt_lead_ms.clamp(100, 1_500);
            ptt_tail_ms = p.ptt_tail_ms.clamp(0, 1_000);
            gui_scale = if p.profile_version >= OPERATOR_PROFILE_VERSION {
                p.gui_scale.clamp(GUI_SCALE_MIN, GUI_SCALE_MAX)
            } else {
                // v3 called this physical size 160%; it is the v4 100% baseline.
                default_gui_scale()
            };
            compute_preference = p.compute_preference;
            config.station.callsign = Some(station_callsign.clone());
            config.station.grid = Some(station_grid.clone());
            profile_io_status = format!("Loaded {}", OPERATOR_PROFILE_FILE);
        } else {
            let bootstrap = OperatorProfile {
                profile_version: OPERATOR_PROFILE_VERSION,
                callsign: station_callsign.clone(),
                grid: station_grid.clone(),
                qth: station_qth.clone(),
                follow_log: ft8_follow_log,
                max_log_entries: ft8_max_log_entries,
                deep_decode: ft8_deep_decode,
                ft4_deep_decode,
                ft4_autoseq,
                ft4_auto_reply_policy,
                ft4_cq_only_view,
                ft4_follow_log,
                ft4_max_log_entries,
                autoseq: ft8_autoseq,
                auto_reply_policy: ft8_auto_reply_policy,
                auto_answer_cq: ft8_auto_answer_cq,
                cq_only_view: ft8_cq_only_view,
                civ_spectrum_on,
                halt_after_tx: ft8_halt_after_tx,
                hold_tx_freq: ft8_hold_tx_freq,
                rx_tone_hz,
                tx_tone_hz,
                ptt_lead_ms,
                ptt_tail_ms,
                audio_input_device: config.audio.input_device.clone(),
                audio_output_device: config.audio.output_device.clone(),
                radio_serial_port: config.radio.serial_port.clone(),
                gui_scale,
                compute_preference,
            };
            match save_operator_profile(&bootstrap) {
                Ok(_) => {
                    profile_io_status = format!("Created {}", OPERATOR_PROFILE_FILE);
                }
                Err(err) => {
                    profile_io_status = format!("Profile init failed: {err}");
                }
            }
        }

        let (ft8_tx_event_tx, ft8_tx_event_rx) = mpsc::channel();
        let (digital_tx_event_tx, digital_tx_event_rx) = mpsc::channel();
        let (qso_log, qso_log_status) = match QsoLog::load(&qso_log_path()) {
            Ok(log) => {
                let count = log.contacts.len();
                (log, format!("Loaded {count} contacts"))
            }
            Err(error) => (QsoLog::default(), format!("Log load failed: {error}")),
        };

        let acceleration_report = AccelerationReport::probe(compute_preference);

        Self {
            config,
            state,
            command_tx,
            worker_stop,
            radio_worker_handle,
            audio_worker_handle,
            radio_waterfall_texture: None,
            radio_waterfall_texture_revision: 0,
            audio_waterfall_texture: None,
            audio_waterfall_texture_revision: 0,
            audio_waterfall_texture_bins: 0,
            workspace_mode: WorkspaceMode::Ft8,
            display_tuning,
            repaint_ctx,
            ft8_log: Vec::new(),
            ft8_tx_chat: VecDeque::new(),
            ft8_seen_decode_period: None,
            qso_log,
            qso_selected: None,
            qso_log_status,
            qso_log_dirty: false,
            ft8_compose: String::new(),
            ft8_selected: None,
            ft8_autoseq,
            ft8_auto_reply_policy,
            ft8_auto_answer_cq,
            ft8_session: None,
            ft8_seq_state: Ft8SeqState::Idle,
            ft8_seq_target: None,
            ft8_seq_status: "🌙 RX deck ready · listening for signals".to_string(),
            ft8_last_click: None,
            ft8_tx_queued_period: None,
            ft8_tx_pcm: None,
            ft8_queued_tx_message: None,
            ft8_last_tx_message: None,
            ft8_tx_started_period: None,
            ft8_last_tx_period: None,
            ft8_suppress_canceled_tx_events: false,
            ft8_pending_manual_reply: None,
            ft8_tx_abort: Arc::new(AtomicBool::new(false)),
            ft8_tx_active,
            ft8_tx_event_tx,
            ft8_tx_event_rx,
            ft8_last_tx_was_cq: false,
            digital_compose: String::new(),
            digital_selected: None,
            digital_seq_target: None,
            ft4_session: None,
            ft4_seen_decodes: HashSet::new(),
            digital_tx_chat: VecDeque::new(),
            digital_queued_tx_message: None,
            digital_last_tx_message: None,
            digital_tx_status: "🌊 RX deck ready · listening for signals".to_string(),
            digital_tx_started: None,
            ft4_last_tx_period: None,
            ft4_seen_decode_period: None,
            digital_tx_abort: Arc::new(AtomicBool::new(false)),
            digital_tx_active,
            digital_suppress_canceled_tx_events: false,
            digital_tx_event_tx,
            digital_tx_event_rx,
            ft8_halt_after_tx,
            ft4_halt_after_tx: false,
            ft8_hold_tx_freq,
            ft8_deep_decode,
            ft4_deep_decode,
            ft4_autoseq,
            ft4_auto_reply_policy,
            ft4_cq_only_view,
            ft4_follow_log,
            ft4_max_log_entries,
            ft8_cq_only_view,
            ft8_follow_log,
            ft8_max_log_entries,
            station_callsign,
            station_grid,
            station_qth,
            civ_spectrum_on,
            rx_tone_hz,
            tx_tone_hz,
            ptt_lead_ms,
            ptt_tail_ms,
            profile_io_status,
            profile_dirty: false,
            audio_input_devices,
            audio_output_devices,
            radio_serial_ports,
            show_signal_panel: true,
            show_device_settings: false,
            device_restart_required: false,
            gui_scale,
            compute_preference,
            acceleration_report,
        }
    }

    fn persist_profile(&mut self, status_prefix: &str) {
        match save_operator_profile(&self.current_operator_profile()) {
            Ok(_) => {
                self.profile_io_status = format!("{status_prefix} {}", OPERATOR_PROFILE_FILE);
                self.profile_dirty = false;
            }
            Err(err) => {
                self.profile_io_status = format!("Save failed: {err}");
            }
        }
    }

    fn persist_qso_log(&mut self, status_prefix: &str) {
        match self.qso_log.save(&qso_log_path()) {
            Ok(()) => {
                self.qso_log_status = format!("{status_prefix} {}", QSO_LOG_FILE);
                self.qso_log_dirty = false;
            }
            Err(error) => self.qso_log_status = format!("Log save failed: {error}"),
        }
    }

    fn append_qso(&mut self, mut record: QsoRecord, status: &str) {
        if self
            .qso_log
            .contacts
            .iter()
            .any(|contact| contact.id == record.id)
        {
            record.id = self
                .qso_log
                .contacts
                .iter()
                .map(|contact| contact.id)
                .max()
                .unwrap_or_default()
                .saturating_add(1);
        }
        self.qso_log.contacts.push(record);
        self.qso_selected = Some(self.qso_log.contacts.len() - 1);
        self.qso_log_dirty = true;
        self.persist_qso_log(status);
    }

    fn log_completed_ft8_session(&mut self, session: &Ft8Session) {
        let frequency_hz = self
            .state
            .lock()
            .expect("ui state lock poisoned")
            .frequency_hz
            .unwrap_or_default();
        let started_at = session.started_period.saturating_mul(15);
        let ended_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or_else(|_| session.last_rx_period.saturating_add(1).saturating_mul(15));
        let mut record = QsoRecord::new(
            &session.target,
            "FT8",
            band_for_frequency(frequency_hz),
            frequency_hz,
            started_at,
            ended_at,
        );
        record.grid = session.remote_grid.clone().unwrap_or_default();
        record.report_sent = session
            .report_sent
            .map(format_signal_report)
            .unwrap_or_default();
        record.report_received = session
            .report_received
            .map(format_signal_report)
            .unwrap_or_default();
        self.append_qso(record, "Auto-logged");
    }

    fn log_completed_ft4_session(&mut self, session: &Ft8Session) {
        let frequency_hz = self
            .state
            .lock()
            .expect("ui state lock poisoned")
            .frequency_hz
            .unwrap_or_default();
        let started_at = (session.started_period as f64 * FT4_SLOT_SECONDS) as u64;
        let ended_at = (session.last_rx_period.saturating_add(1) as f64 * FT4_SLOT_SECONDS) as u64;
        let mut record = QsoRecord::new(
            &session.target,
            "FT4",
            band_for_frequency(frequency_hz),
            frequency_hz,
            started_at,
            ended_at,
        );
        record.grid = session.remote_grid.clone().unwrap_or_default();
        record.report_sent = session
            .report_sent
            .map(format_signal_report)
            .unwrap_or_default();
        record.report_received = session
            .report_received
            .map(format_signal_report)
            .unwrap_or_default();
        self.append_qso(record, "Auto-logged FT4");
    }

    fn handle_ft4_decodes(
        &mut self,
        decodes: &[DigitalDecodeEntry],
        completed_period: Option<u64>,
    ) {
        let fresh: Vec<DigitalDecodeEntry> = decodes
            .iter()
            .filter(|entry| {
                entry.mode == WorkspaceMode::Ft4
                    && self.ft4_seen_decodes.insert((
                        entry.period,
                        entry.freq_hz,
                        entry.message.clone(),
                    ))
            })
            .cloned()
            .collect();
        if self.ft4_seen_decodes.len() > 1_000 {
            let latest = fresh
                .iter()
                .map(|entry| entry.period)
                .max()
                .unwrap_or_default();
            self.ft4_seen_decodes
                .retain(|(period, _, _)| *period + 100 >= latest);
        }
        if !self.ft4_autoseq || self.digital_tx_active.load(Ordering::Acquire) {
            return;
        }

        let my_call = self.station_callsign_or_default().to_string();
        let my_grid = self.station_grid_or_default().to_string();
        let awaiting_cq_caller = self
            .digital_last_tx_message
            .as_deref()
            .is_some_and(|message| message.starts_with("CQ "));
        if self.ft4_session.is_none() {
            let candidates = fresh.iter().enumerate().filter_map(|(index, entry)| {
                let parsed = parse_message(&entry.message)?;
                let eligible = (awaiting_cq_caller && parsed.directed_to(&my_call))
                    || (self.ft8_auto_answer_cq && parsed.is_cq);
                if !eligible || ft8_ops::callsign_eq(&parsed.from, &my_call) {
                    return None;
                }
                Some(ReplyCandidate {
                    index,
                    snr_db: entry.snr_db.round() as i8,
                    freq_hz: entry.freq_hz,
                    parsed,
                })
            });
            if let Some(chosen) =
                select_candidate(candidates, self.ft4_auto_reply_policy, self.rx_tone_hz)
            {
                self.ft4_session = Some(Ft8Session::start(
                    chosen.parsed.from.clone(),
                    fresh[chosen.index].period,
                ));
                self.digital_seq_target = Some(chosen.parsed.from.clone());
                self.digital_tx_status = format!(
                    "🎯 {} selected by {} priority",
                    chosen.parsed.from,
                    self.ft4_auto_reply_policy.label()
                );
            }
        }
        let mut queued_response = false;
        for entry in fresh {
            let Some(parsed) = parse_message(&entry.message) else {
                continue;
            };
            let Some(session) = self.ft4_session.as_mut() else {
                continue;
            };
            let response = session.response_to(
                &parsed,
                &my_call,
                &my_grid,
                entry.snr_db.round() as i8,
                entry.period,
            );
            if session.stage == QsoStage::Complete {
                let completed = session.clone();
                self.log_completed_ft4_session(&completed);
                self.digital_tx_status =
                    format!("🏁 FT4 QSO with {} complete · nice contact!", completed.target);
                self.ft4_session = None;
                self.digital_seq_target = None;
                break;
            }
            if let Some(response) = response {
                self.digital_compose = response;
                self.rx_tone_hz = entry.freq_hz;
                if !self.ft8_hold_tx_freq {
                    self.tx_tone_hz = entry.freq_hz;
                }
                self.queue_native_digital_tx(WorkspaceMode::Ft4);
                queued_response = true;
                break;
            }
        }
        if queued_response || self.ft4_session.is_none() {
            return;
        }
        if completed_period.is_some_and(|period| {
            self.ft4_last_tx_period
                .is_some_and(|last_tx| period == last_tx.saturating_add(1))
        }) {
            let attempts = self
                .ft4_session
                .as_ref()
                .map(|session| session.tx_attempts)
                .unwrap_or_default();
            if attempts >= MAX_ATTEMPTS_PER_EXCHANGE {
                self.ft4_autoseq = false;
                self.ft4_session = None;
                self.digital_tx_status = format!(
                    "FT4 stopped after {MAX_ATTEMPTS_PER_EXCHANGE} unanswered attempts"
                );
            } else if !self.digital_compose.trim().is_empty() {
                self.digital_tx_status =
                    "🔁 No FT4 reply yet · repeating the last exchange".to_string();
                self.queue_native_digital_tx(WorkspaceMode::Ft4);
            }
        }
    }

    fn queue_ft8_tx_from_compose(&mut self, policy: Ft8TxQueuePolicy, rx_period: Option<u64>) {
        if self.ft8_compose.trim().is_empty() {
            self.ft8_seq_status = "TX not queued: compose is empty".to_string();
            return;
        }
        let Some(command_tx) = self.command_tx.clone() else {
            self.ft8_seq_status = "TX unavailable: radio control is disabled".to_string();
            return;
        };
        if self.ft8_tx_active.load(Ordering::Acquire) || self.ft8_tx_queued_period.is_some() {
            self.ft8_seq_status =
                "TX not queued: another transmission is already scheduled".to_string();
            return;
        }
        if self.digital_tx_active.load(Ordering::Acquire) {
            self.ft8_seq_status = "TX not queued: another digital mode is transmitting".to_string();
            return;
        }
        if self.ft8_suppress_canceled_tx_events {
            self.ft8_seq_status = "TX cancellation is still settling; try again".to_string();
            return;
        }
        if self
            .ft8_session
            .as_ref()
            .is_some_and(|session| session.tx_attempts >= MAX_ATTEMPTS_PER_EXCHANGE)
        {
            self.cancel_ft8_sequence(format!(
                "Stopped after {MAX_ATTEMPTS_PER_EXCHANGE} unanswered attempts"
            ));
            return;
        }
        self.ft8_tx_abort.store(false, Ordering::Relaxed);
        match build_ft8_tx_pcm(&self.ft8_compose, self.tx_tone_hz) {
            Ok(pcm) => {
                let now_s = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs_f64())
                    .unwrap_or(0.0);
                let period = (now_s / ft8_ops::SLOT_SECONDS) as u64;
                let target_period = match policy {
                    Ft8TxQueuePolicy::Standard => {
                        next_tx_period(now_s, None, self.ptt_lead_ms as f64 / 1_000.0)
                    }
                    Ft8TxQueuePolicy::ReplyAsap => rx_period.map_or_else(
                        || next_tx_period(now_s, None, self.ptt_lead_ms as f64 / 1_000.0),
                        |source_period| {
                            next_reply_period(
                                now_s,
                                source_period,
                                self.ptt_lead_ms as f64 / 1_000.0,
                            )
                        },
                    ),
                    Ft8TxQueuePolicy::NextSlotOnly => next_tx_period(
                        now_s,
                        Some(((period + 1) % 2) as u8),
                        self.ptt_lead_ms as f64 / 1_000.0,
                    ),
                };
                info!(
                    ?policy,
                    source_period = ?rx_period,
                    current_period = period,
                    target_period,
                    slot_position_s = now_s % ft8_ops::SLOT_SECONDS,
                    "FT8 TX scheduled"
                );
                let pcm = Arc::new(pcm);
                self.ft8_tx_pcm = Some(pcm.clone());
                self.ft8_queued_tx_message = Some(self.ft8_compose.trim().to_string());
                self.ft8_tx_queued_period = Some(target_period);
                self.ft8_tx_started_period = None;
                self.ft8_last_tx_was_cq =
                    parse_message(&self.ft8_compose).is_some_and(|message| message.is_cq);
                self.ft8_seq_state = Ft8SeqState::TxQueued;
                self.ft8_seq_status = match policy {
                    Ft8TxQueuePolicy::ReplyAsap if target_period == period => format!(
                        "Reply STARTING NOW at slot +{:.2}s",
                        now_s % ft8_ops::SLOT_SECONDS
                    ),
                    Ft8TxQueuePolicy::ReplyAsap => format!(
                        "Reply queued for future slot {} (period {})",
                        utc_hhmmss_millis(target_period as f64 * 15.0),
                        target_period
                    ),
                    Ft8TxQueuePolicy::NextSlotOnly => format!(
                        "CQ queued for next slot {} (period {})",
                        utc_hhmmss_millis(target_period as f64 * 15.0),
                        target_period
                    ),
                    Ft8TxQueuePolicy::Standard => format!(
                        "TX queued for {} (period {})",
                        utc_hhmmss_millis(target_period as f64 * 15.0),
                        target_period
                    ),
                };

                self.ft8_tx_active.store(true, Ordering::Release);
                let job = Ft8TxJob {
                    period: target_period,
                    pcm,
                    ptt_lead: Duration::from_millis(self.ptt_lead_ms),
                    ptt_tail: Duration::from_millis(self.ptt_tail_ms),
                    output_device: self.config.audio.output_device.clone(),
                    abort: self.ft8_tx_abort.clone(),
                    active: self.ft8_tx_active.clone(),
                    command_tx,
                    event_tx: self.ft8_tx_event_tx.clone(),
                    state: self.state.clone(),
                    repaint_ctx: self.repaint_ctx.clone(),
                };
                thread::spawn(move || run_ft8_tx_job(job));
                if let Some(session) = self.ft8_session.as_mut() {
                    session.tx_attempts = session.tx_attempts.saturating_add(1);
                    self.ft8_seq_status.push_str(&format!(
                        " | attempt {}/{}",
                        session.tx_attempts, MAX_ATTEMPTS_PER_EXCHANGE
                    ));
                }
            }
            Err(err) => {
                self.ft8_seq_status = format!("TX encode failed: {err}");
            }
        }
    }

    fn retune_from_decode_pick(&mut self, freq_hz: u32, move_tx_to_remote: bool) -> bool {
        let picked = freq_hz.clamp(100, 3_500);
        self.rx_tone_hz = picked;
        let tx_moved = move_tx_to_remote && !self.ft8_hold_tx_freq;
        if tx_moved {
            self.tx_tone_hz = picked;
        }
        self.profile_dirty = true;
        self.persist_profile("Auto-saved");
        tx_moved
    }

    fn force_stop_tx(&mut self) {
        let had_scheduled_tx =
            self.ft8_tx_active.load(Ordering::Acquire) || self.ft8_tx_queued_period.is_some();
        if had_scheduled_tx {
            self.ft8_suppress_canceled_tx_events = true;
        }
        self.ft8_tx_abort.store(true, Ordering::Relaxed);
        self.ft8_tx_active.store(false, Ordering::Relaxed);

        self.ft8_tx_queued_period = None;
        self.ft8_tx_started_period = None;
        self.ft8_tx_pcm = None;
        self.ft8_queued_tx_message = None;
        self.ft8_pending_manual_reply = None;
        self.ft8_seq_target = None;
        self.ft8_session = None;
        self.ft8_seq_state = Ft8SeqState::Idle;
        self.ft8_seq_status = "TX force-stopped".to_string();

        if let Some(tx) = &self.command_tx {
            let _ = tx.send(GuiCommand::SetPtt(false));
        }

    }

    fn cancel_ft8_sequence(&mut self, reason: String) {
        self.force_stop_tx();
        if self.digital_tx_active.load(Ordering::Acquire) {
            self.stop_native_digital_tx();
        }
        self.ft8_autoseq = false;
        self.ft8_halt_after_tx = false;
        self.ft8_seq_status = reason;
        self.profile_dirty = true;
        self.persist_profile("Automatic operation stopped");
    }

    fn any_tx_armed(&self, snapshot: &GuiState) -> bool {
        snapshot.ptt_on
            || self.ft8_autoseq
            || self.ft4_autoseq
            || self.ft8_tx_active.load(Ordering::Acquire)
            || self.ft8_tx_queued_period.is_some()
            || self.digital_tx_active.load(Ordering::Acquire)
    }

    fn disarm_all_tx(&mut self, reason: &str) {
        self.force_stop_tx();
        self.stop_native_digital_tx();
        self.ft8_autoseq = false;
        self.ft4_autoseq = false;
        self.ft8_halt_after_tx = false;
        self.ft4_halt_after_tx = false;
        self.ft4_session = None;
        self.digital_seq_target = None;
        self.digital_tx_started = None;
        self.digital_last_tx_message = None;
        self.ft8_seq_status = reason.to_string();
        self.digital_tx_status = reason.to_string();
        self.profile_dirty = true;
        self.persist_profile("All TX disarmed");
    }

    fn arm_manual_ft8_reply(&mut self, reply: PendingManualFt8Reply) {
        self.ft8_compose = reply.compose;
        let tx_moved = self.retune_from_decode_pick(reply.freq_hz, reply.move_tx_to_remote);
        self.ft8_autoseq = true;
        self.ft8_seq_state = Ft8SeqState::ReplyArmed;
        self.ft8_seq_target = Some(reply.target.clone());
        self.ft8_session = Some(reply.session);
        self.ft8_seq_status = if self.ft8_hold_tx_freq {
            format!(
                "Reply armed for {}; RX moved to {} Hz (TX held)",
                reply.target, self.rx_tone_hz
            )
        } else if tx_moved {
            format!(
                "Reply armed for {}; RX/TX set to {} Hz",
                reply.target, self.rx_tone_hz
            )
        } else {
            format!(
                "Reply armed for {}; RX moved to {} Hz (TX stays at {} Hz)",
                reply.target, self.rx_tone_hz, self.tx_tone_hz
            )
        };
        self.profile_dirty = true;
        self.persist_profile("Auto-saved");
        self.profile_io_status = "Auto-seq armed from decode selection".to_string();
        self.queue_ft8_tx_from_compose(Ft8TxQueuePolicy::ReplyAsap, Some(reply.source_period));
    }

    fn process_ft8_tx_pipeline(&mut self) {
        while let Ok(event) = self.ft8_tx_event_rx.try_recv() {
            if self.ft8_suppress_canceled_tx_events {
                let terminal = matches!(&event, Ft8TxEvent::Complete | Ft8TxEvent::Failed(_));
                if terminal {
                    self.ft8_suppress_canceled_tx_events = false;
                    if let Some(reply) = self.ft8_pending_manual_reply.take() {
                        self.arm_manual_ft8_reply(reply);
                    }
                }
                continue;
            }
            match event {
                Ft8TxEvent::PttConfirmed => {
                    self.ft8_seq_status =
                        "⚡ PTT confirmed · waveform launch is locked in".to_string();
                }
                Ft8TxEvent::AudioStarted => {
                    self.ft8_tx_started_period = self.ft8_tx_queued_period;
                    self.ft8_last_tx_message = self.ft8_queued_tx_message.clone();
                    if let Some(message) = self.ft8_queued_tx_message.clone() {
                        let now_s = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .map(|duration| duration.as_secs_f64())
                            .unwrap_or_default();
                        let period = self
                            .ft8_tx_queued_period
                            .unwrap_or_else(|| (now_s / 15.0).floor() as u64);
                        let duplicate = self
                            .ft8_tx_chat
                            .back()
                            .is_some_and(|entry| entry.period == period && entry.message == message);
                        if !duplicate {
                            self.ft8_tx_chat.push_back(Ft8TxChatEntry {
                                period,
                                utc: utc_hhmmss_millis(now_s),
                                message,
                            });
                            while self.ft8_tx_chat.len() > 100 {
                                self.ft8_tx_chat.pop_front();
                            }
                        }
                    }
                    self.ft8_seq_status = "🔥 FT8 waveform on the air".to_string();
                }
                Ft8TxEvent::Complete => {
                    let completed_session = self
                        .ft8_session
                        .as_ref()
                        .filter(|session| session.stage == QsoStage::FinalSent)
                        .cloned();
                    let stop_after_tx = self.ft8_halt_after_tx;
                    self.ft8_halt_after_tx = false;
                    self.ft8_seq_status = if stop_after_tx {
                        self.ft8_autoseq = false;
                        "🔒 TX complete · automatic TX is paused".to_string()
                    } else if self.ft8_last_tx_was_cq {
                        "📣 CQ away · listening for callers".to_string()
                    } else {
                        "📡 TX complete · ears open for the reply".to_string()
                    };
                    self.ft8_last_tx_period = self.ft8_tx_started_period;
                    self.ft8_seq_state = if self.ft8_autoseq {
                        if self.ft8_last_tx_was_cq {
                            Ft8SeqState::CqArmed
                        } else {
                            Ft8SeqState::ReplyArmed
                        }
                    } else {
                        Ft8SeqState::Idle
                    };
                    self.ft8_tx_queued_period = None;
                    self.ft8_tx_started_period = None;
                    self.ft8_tx_pcm = None;
                    self.ft8_queued_tx_message = None;
                    self.ft8_tx_abort.store(false, Ordering::Relaxed);
                    if stop_after_tx {
                        self.profile_dirty = true;
                        self.persist_profile("Automatic TX paused and saved");
                    }
                    if let Some(session) = completed_session {
                        let target = session.target.clone();
                        self.log_completed_ft8_session(&session);
                        self.ft8_seq_status =
                            format!("🏁 QSO with {target} complete and logged · beautiful!");
                        self.ft8_seq_target = None;
                        self.ft8_session = None;
                    }
                }
                Ft8TxEvent::Failed(error) => {
                    self.ft8_seq_status = format!("⚠ TX failed · {error}");
                    self.ft8_seq_state = Ft8SeqState::Idle;
                    self.ft8_tx_queued_period = None;
                    self.ft8_tx_started_period = None;
                    self.ft8_tx_pcm = None;
                    self.ft8_queued_tx_message = None;
                    self.ft8_autoseq = false;
                    self.ft8_seq_target = None;
                    self.ft8_session = None;
                }
            }
        }
    }

    fn queue_native_digital_tx(&mut self, mode: WorkspaceMode) {
        if self.digital_suppress_canceled_tx_events {
            self.digital_tx_status = "TX cancellation is still settling; try again".to_string();
            return;
        }
        if self.ft8_tx_active.load(Ordering::Acquire)
            || self.digital_tx_active.load(Ordering::Acquire)
        {
            self.digital_tx_status = "TX not queued: another transmission is active".to_string();
            return;
        }
        let Some(command_tx) = self.command_tx.clone() else {
            self.digital_tx_status = "TX unavailable: radio control is disabled".to_string();
            return;
        };
        let Some(slot_seconds) = mode.core_slot_seconds() else {
            self.digital_tx_status = format!("{} TX backend is not available", mode.label());
            return;
        };
        match build_native_digital_tx_pcm(mode, &self.digital_compose, self.tx_tone_hz) {
            Ok((pcm, audio_offset_s)) => {
                let now_s = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|duration| duration.as_secs_f64())
                    .unwrap_or(0.0);
                let period = (now_s / slot_seconds).floor() as u64 + 1;
                self.digital_tx_abort.store(false, Ordering::Release);
                self.digital_tx_active.store(true, Ordering::Release);
                self.digital_tx_status = format!(
                    "{} queued for {}",
                    mode.label(),
                    utc_hhmmss_millis(period as f64 * slot_seconds)
                );
                self.digital_queued_tx_message = Some(self.digital_compose.trim().to_string());
                self.state
                    .lock()
                    .expect("ui state lock poisoned")
                    .digital_tx_period = Some((mode, period));
                let job = DigitalTxJob {
                    mode,
                    period,
                    slot_seconds,
                    audio_offset_s,
                    pcm: Arc::new(pcm),
                    ptt_lead: Duration::from_millis(self.ptt_lead_ms),
                    ptt_tail: Duration::from_millis(self.ptt_tail_ms),
                    output_device: self.config.audio.output_device.clone(),
                    abort: self.digital_tx_abort.clone(),
                    active: self.digital_tx_active.clone(),
                    command_tx,
                    event_tx: self.digital_tx_event_tx.clone(),
                    state: self.state.clone(),
                    repaint_ctx: self.repaint_ctx.clone(),
                };
                thread::spawn(move || run_digital_tx_job(job));
            }
            Err(error) => {
                self.digital_tx_status = format!("TX encode failed: {error}");
            }
        }
    }

    fn stop_native_digital_tx(&mut self) {
        let had_scheduled_tx = self.digital_tx_active.load(Ordering::Acquire)
            || self.digital_tx_started.is_some()
            || self.digital_queued_tx_message.is_some();
        if had_scheduled_tx {
            self.digital_suppress_canceled_tx_events = true;
        }
        self.digital_tx_abort.store(true, Ordering::Release);
        self.digital_tx_active.store(false, Ordering::Release);
        if let Some(tx) = &self.command_tx {
            let _ = tx.send(GuiCommand::SetPtt(false));
        }
        self.state
            .lock()
            .expect("ui state lock poisoned")
            .digital_tx_period = None;
        self.digital_tx_status = "TX stopped".to_string();
        self.digital_queued_tx_message = None;
    }

    fn process_native_digital_tx_pipeline(&mut self) {
        while let Ok(event) = self.digital_tx_event_rx.try_recv() {
            if self.digital_suppress_canceled_tx_events {
                if matches!(event, DigitalTxEvent::Complete | DigitalTxEvent::Failed(_)) {
                    self.digital_suppress_canceled_tx_events = false;
                    self.digital_tx_abort.store(false, Ordering::Release);
                }
                continue;
            }
            self.digital_tx_status = match event {
                DigitalTxEvent::AudioStarted(mode, period) => {
                    self.digital_tx_started = Some((mode, period));
                    if mode == WorkspaceMode::Ft4 {
                        if let Some(session) = self.ft4_session.as_mut() {
                            session.tx_attempts = session.tx_attempts.saturating_add(1);
                        }
                    }
                    if let Some(message) = self.digital_queued_tx_message.clone() {
                        let utc = utc_hhmmss_millis(
                            period as f64 * mode.core_slot_seconds().unwrap_or(1.0),
                        );
                        self.digital_tx_chat.push_back(DigitalTxChatEntry {
                            mode,
                            period,
                            utc,
                            message,
                        });
                        while self.digital_tx_chat.len() > 100 {
                            self.digital_tx_chat.pop_front();
                        }
                    }
                    format!("🔥 {} waveform on the air", mode.label())
                }
                DigitalTxEvent::Complete => {
                    let completed_mode = self.digital_tx_started.take().map(|(mode, period)| {
                        if mode == WorkspaceMode::Ft4 {
                            self.ft4_last_tx_period = Some(period);
                        }
                        mode
                    });
                    self.digital_last_tx_message = self.digital_queued_tx_message.take();
                    if completed_mode == Some(WorkspaceMode::Ft4) && self.ft4_halt_after_tx {
                        self.ft4_halt_after_tx = false;
                        self.ft4_autoseq = false;
                        self.profile_dirty = true;
                        self.persist_profile("FT4 automatic TX paused");
                        "🔒 FT4 TX complete · automatic TX is paused".to_string()
                    } else {
                        "📡 TX complete · receiver back on watch".to_string()
                    }
                }
                DigitalTxEvent::Failed(error) => {
                    self.digital_tx_started = None;
                    self.digital_queued_tx_message = None;
                    format!("⚠ TX failed · {error}")
                }
            };
        }
    }

    fn handle_ft8_decodes(&mut self, decodes: &[Ft8DecodeEntry], completed_period: Option<u64>) {
        if decodes.is_empty() && completed_period.is_none() {
            return;
        }

        let my_call = self.station_callsign_or_default().to_ascii_uppercase();
        let my_grid = self.station_grid_or_default().to_ascii_uppercase();
        if my_call == "N0CALL" {
            self.ft8_seq_status = "Auto reply paused: set a valid operator callsign".to_string();
            return;
        }

        if let Some(target) = self.ft8_seq_target.clone() {
            let working_other = decodes.iter().find_map(|entry| {
                let parsed = parse_message(&entry.message)?;
                (ft8_ops::callsign_eq(&parsed.from, &target) && parsed.directed_away_from(&my_call))
                    .then(|| parsed.to.unwrap_or_else(|| "another station".to_string()))
            });
            if let Some(other) = working_other {
                self.cancel_ft8_sequence(format!("Canceled: {target} is responding to {other}"));
                return;
            }
        }

        if !self.ft8_autoseq
            || self.ft8_tx_active.load(Ordering::Acquire)
            || self.ft8_tx_queued_period.is_some()
        {
            return;
        }

        if let Some(session) = self.ft8_session.as_mut() {
            let target = session.target.clone();
            let response = decodes.iter().find_map(|entry| {
                let parsed = parse_message(&entry.message)?;
                let message =
                    session.response_to(&parsed, &my_call, &my_grid, entry.snr_db, entry.period)?;
                Some((message, entry.period, entry.freq_hz))
            });

            if session.stage == QsoStage::Complete {
                let completed_session = session.clone();
                self.ft8_seq_status = format!("QSO with {target} complete; ready for next caller");
                self.ft8_seq_state = Ft8SeqState::Idle;
                self.ft8_seq_target = None;
                self.ft8_session = None;
                self.log_completed_ft8_session(&completed_session);
                return;
            }

            if let Some((message, period, freq_hz)) = response {
                self.ft8_compose = message;
                self.retune_from_decode_pick(freq_hz, false);
                self.queue_ft8_tx_from_compose(Ft8TxQueuePolicy::ReplyAsap, Some(period));
            } else if completed_period
                .is_some_and(|period| should_retry_after_decode(self.ft8_last_tx_period, period))
            {
                let period = completed_period.expect("checked above");
                self.ft8_seq_status = format!("🔁 No reply from {target} yet · trying again");
                self.queue_ft8_tx_from_compose(Ft8TxQueuePolicy::ReplyAsap, Some(period));
            }
            return;
        }

        let candidates = decodes.iter().enumerate().filter_map(|(index, entry)| {
            let parsed = parse_message(&entry.message)?;
            let eligible =
                parsed.directed_to(&my_call) || (self.ft8_auto_answer_cq && parsed.is_cq);
            if !eligible || ft8_ops::callsign_eq(&parsed.from, &my_call) {
                return None;
            }
            Some(ReplyCandidate {
                index,
                snr_db: entry.snr_db,
                freq_hz: entry.freq_hz,
                parsed,
            })
        });
        let Some(selected) =
            select_candidate(candidates, self.ft8_auto_reply_policy, self.rx_tone_hz)
        else {
            return;
        };
        let entry = &decodes[selected.index];
        let mut session = Ft8Session::start(selected.parsed.from.clone(), entry.period);
        let Some(response) = session.response_to(
            &selected.parsed,
            &my_call,
            &my_grid,
            entry.snr_db,
            entry.period,
        ) else {
            return;
        };

        self.ft8_seq_target = Some(session.target.clone());
        self.ft8_seq_state = Ft8SeqState::ReplyArmed;
        self.ft8_seq_status = format!(
            "{} selected {} at {:+} dB",
            self.ft8_auto_reply_policy.label(),
            session.target,
            entry.snr_db
        );
        self.ft8_session = Some(session);
        self.ft8_compose = response;
        self.retune_from_decode_pick(
            entry.freq_hz,
            should_move_tx_to_decode(&selected.parsed, false),
        );
        self.queue_ft8_tx_from_compose(Ft8TxQueuePolicy::ReplyAsap, Some(entry.period));
    }

    fn current_operator_profile(&self) -> OperatorProfile {
        OperatorProfile {
            profile_version: OPERATOR_PROFILE_VERSION,
            callsign: self.station_callsign_or_default().to_string(),
            grid: self.station_grid_or_default().to_string(),
            qth: self.station_qth.trim().to_string(),
            follow_log: self.ft8_follow_log,
            max_log_entries: self.ft8_max_log_entries.clamp(80, 1000),
            deep_decode: self.ft8_deep_decode,
            ft4_deep_decode: self.ft4_deep_decode,
            ft4_autoseq: self.ft4_autoseq,
            ft4_auto_reply_policy: self.ft4_auto_reply_policy,
            ft4_cq_only_view: self.ft4_cq_only_view,
            ft4_follow_log: self.ft4_follow_log,
            ft4_max_log_entries: self.ft4_max_log_entries.clamp(80, 300),
            autoseq: self.ft8_autoseq,
            auto_reply_policy: self.ft8_auto_reply_policy,
            auto_answer_cq: self.ft8_auto_answer_cq,
            cq_only_view: self.ft8_cq_only_view,
            civ_spectrum_on: self.civ_spectrum_on,
            // This control is deliberately one-shot and is not restored on launch.
            halt_after_tx: false,
            hold_tx_freq: self.ft8_hold_tx_freq,
            rx_tone_hz: self.rx_tone_hz,
            tx_tone_hz: self.tx_tone_hz,
            ptt_lead_ms: self.ptt_lead_ms.clamp(100, 1_500),
            ptt_tail_ms: self.ptt_tail_ms.clamp(0, 1_000),
            audio_input_device: self.config.audio.input_device.clone(),
            audio_output_device: self.config.audio.output_device.clone(),
            radio_serial_port: self.config.radio.serial_port.clone(),
            gui_scale: self.gui_scale.clamp(GUI_SCALE_MIN, GUI_SCALE_MAX),
            compute_preference: self.compute_preference,
        }
    }

    fn station_callsign_or_default(&self) -> &str {
        let v = self.station_callsign.trim();
        if v.is_empty() {
            "N0CALL"
        } else {
            v
        }
    }

    fn station_grid_or_default(&self) -> &str {
        let v = self.station_grid.trim();
        if v.is_empty() {
            "AA00"
        } else {
            v
        }
    }

    fn draw_station_profile(&mut self, ui: &mut egui::Ui) {
        ui.heading("Operator Profile");
        ui.separator();

        ui.horizontal(|ui| {
            ui.label(RichText::new("Call").strong());
            let changed = ui
                .add(
                    egui::TextEdit::singleline(&mut self.station_callsign)
                        .desired_width(110.0)
                        .hint_text("N0CALL")
                        .font(egui::TextStyle::Monospace),
                )
                .changed();
            if changed {
                self.station_callsign = self.station_callsign.trim().to_ascii_uppercase();
                let val = self.station_callsign.trim();
                self.config.station.callsign = if val.is_empty() {
                    None
                } else {
                    Some(val.to_string())
                };
                self.profile_dirty = true;
                self.persist_profile("Auto-saved");
            }

            ui.label(RichText::new("Grid").strong());
            let changed = ui
                .add(
                    egui::TextEdit::singleline(&mut self.station_grid)
                        .desired_width(90.0)
                        .hint_text("AA00")
                        .font(egui::TextStyle::Monospace),
                )
                .changed();
            if changed {
                self.station_grid = self.station_grid.trim().to_ascii_uppercase();
                let val = self.station_grid.trim();
                self.config.station.grid = if val.is_empty() {
                    None
                } else {
                    Some(val.to_string())
                };
                self.profile_dirty = true;
                self.persist_profile("Auto-saved");
            }
        });

        ui.horizontal(|ui| {
            ui.label(RichText::new("QTH").strong());
            let qth_changed = ui
                .add(
                    egui::TextEdit::singleline(&mut self.station_qth)
                        .desired_width(ui.available_width())
                        .hint_text("City / locator notes")
                        .font(egui::TextStyle::Monospace),
                )
                .changed();
            if qth_changed {
                self.profile_dirty = true;
                self.persist_profile("Auto-saved");
            }
        });

        ui.add_space(4.0);
        ui.label(
            RichText::new(&self.profile_io_status)
                .small()
                .color(Color32::GRAY),
        );
    }

    fn send_command(&self, cmd: GuiCommand) {
        if let Some(tx) = &self.command_tx {
            let _ = tx.send(cmd);
        }
    }

    fn draw_contact_log(&mut self, ui: &mut egui::Ui, snapshot: &GuiState) {
        ui.horizontal(|ui| {
            ui.heading("Contact Log");
            ui.separator();
            if ui.small_button("+").on_hover_text("New contact").clicked() {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|duration| duration.as_secs())
                    .unwrap_or_default();
                let frequency_hz = snapshot.frequency_hz.unwrap_or_default();
                let record = QsoRecord::new(
                    "",
                    self.workspace_mode.label(),
                    band_for_frequency(frequency_hz),
                    frequency_hz,
                    now,
                    now,
                );
                self.qso_log.contacts.push(record);
                self.qso_selected = Some(self.qso_log.contacts.len() - 1);
                self.qso_log_dirty = true;
                self.qso_log_status = "New contact; edit and save".to_string();
            }
            if ui
                .add_enabled(self.qso_log_dirty, egui::Button::new("Save"))
                .clicked()
            {
                self.persist_qso_log("Saved");
            }
            if ui.small_button("Export ADIF").clicked() {
                match self.qso_log.export_adif(&qso_adif_path()) {
                    Ok(()) => self.qso_log_status = format!("Exported {}", QSO_ADIF_FILE),
                    Err(error) => self.qso_log_status = format!("ADIF export failed: {error}"),
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(format!("{} QSOs", self.qso_log.contacts.len()));
            });
        });
        ui.separator();

        let mut selected = self.qso_selected;
        egui::ScrollArea::vertical()
            .id_salt("qso_log_rows")
            .max_height(132.0)
            .show(ui, |ui| {
                egui::Grid::new("qso_log_grid")
                    .striped(true)
                    .min_col_width(38.0)
                    .show(ui, |ui| {
                        for heading in ["Date", "UTC", "Call", "Band", "Mode"] {
                            ui.label(RichText::new(heading).strong().small());
                        }
                        ui.end_row();

                        for (index, contact) in self.qso_log.contacts.iter().enumerate().rev() {
                            let is_selected = selected == Some(index);
                            if ui
                                .selectable_label(is_selected, &contact.qso_date)
                                .clicked()
                            {
                                selected = Some(index);
                            }
                            ui.label(&contact.time_on);
                            ui.label(RichText::new(&contact.callsign).monospace().strong());
                            ui.label(&contact.band);
                            ui.label(&contact.mode);
                            ui.end_row();
                        }
                    });
            });
        self.qso_selected = selected;

        let mut changed = false;
        let mut delete_selected = false;
        if let Some(index) = self.qso_selected {
            if let Some(contact) = self.qso_log.contacts.get_mut(index) {
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label("Call");
                    changed |= ui
                        .add(
                            egui::TextEdit::singleline(&mut contact.callsign)
                                .desired_width(95.0)
                                .font(egui::TextStyle::Monospace),
                        )
                        .changed();
                    ui.label("Grid");
                    changed |= ui
                        .add(
                            egui::TextEdit::singleline(&mut contact.grid)
                                .desired_width(72.0)
                                .font(egui::TextStyle::Monospace),
                        )
                        .changed();
                    ui.label("Mode");
                    changed |= ui
                        .add(egui::TextEdit::singleline(&mut contact.mode).desired_width(62.0))
                        .changed();
                    ui.label("Band");
                    changed |= ui
                        .add(egui::TextEdit::singleline(&mut contact.band).desired_width(48.0))
                        .changed();
                });
                ui.horizontal(|ui| {
                    ui.label("Date");
                    changed |= ui
                        .add(
                            egui::TextEdit::singleline(&mut contact.qso_date)
                                .desired_width(74.0)
                                .font(egui::TextStyle::Monospace),
                        )
                        .changed();
                    ui.label("On");
                    changed |= ui
                        .add(
                            egui::TextEdit::singleline(&mut contact.time_on)
                                .desired_width(58.0)
                                .font(egui::TextStyle::Monospace),
                        )
                        .changed();
                    ui.label("Off");
                    changed |= ui
                        .add(
                            egui::TextEdit::singleline(&mut contact.time_off)
                                .desired_width(58.0)
                                .font(egui::TextStyle::Monospace),
                        )
                        .changed();
                    let mut frequency_mhz = contact.frequency_hz as f64 / 1_000_000.0;
                    ui.label("MHz");
                    if ui
                        .add(
                            egui::DragValue::new(&mut frequency_mhz)
                                .range(0.0..=10_000.0)
                                .speed(0.001)
                                .max_decimals(6),
                        )
                        .changed()
                    {
                        contact.frequency_hz = (frequency_mhz * 1_000_000.0).round() as u64;
                        changed = true;
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Sent");
                    changed |= ui
                        .add(
                            egui::TextEdit::singleline(&mut contact.report_sent)
                                .desired_width(48.0)
                                .font(egui::TextStyle::Monospace),
                        )
                        .changed();
                    ui.label("Rcvd");
                    changed |= ui
                        .add(
                            egui::TextEdit::singleline(&mut contact.report_received)
                                .desired_width(48.0)
                                .font(egui::TextStyle::Monospace),
                        )
                        .changed();
                    ui.label("Notes");
                    changed |= ui
                        .add(
                            egui::TextEdit::singleline(&mut contact.notes)
                                .desired_width(ui.available_width().max(80.0)),
                        )
                        .changed();
                });
                ui.horizontal(|ui| {
                    if ui.small_button("Delete").clicked() {
                        delete_selected = true;
                    }
                    ui.label(RichText::new(&self.qso_log_status).small().color(
                        if self.qso_log_dirty {
                            Color32::YELLOW
                        } else {
                            Color32::GRAY
                        },
                    ));
                });
            }
        } else {
            ui.label(
                RichText::new(&self.qso_log_status)
                    .small()
                    .color(Color32::GRAY),
            );
        }

        if changed {
            if let Some(index) = self.qso_selected {
                if let Some(contact) = self.qso_log.contacts.get_mut(index) {
                    contact.callsign = contact.callsign.trim().to_ascii_uppercase();
                    contact.grid = contact.grid.trim().to_ascii_uppercase();
                    contact.mode = contact.mode.trim().to_ascii_uppercase();
                }
            }
            self.qso_log_dirty = true;
            self.qso_log_status = "Unsaved changes".to_string();
        }
        if delete_selected {
            if let Some(index) = self.qso_selected.take() {
                self.qso_log.contacts.remove(index);
                self.qso_log_dirty = true;
                self.persist_qso_log("Deleted contact from");
            }
        }
    }

    fn refresh_device_lists(&mut self) {
        self.audio_input_devices = AudioService::input_devices().unwrap_or_default();
        self.audio_output_devices = AudioService::output_devices().unwrap_or_default();
        self.radio_serial_ports = enumerate_serial_ports().unwrap_or_default();
    }

    fn draw_device_settings(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("Devices");
            if ui.small_button("Refresh").clicked() {
                self.refresh_device_lists();
            }
            ui.label(
                RichText::new(format!("{} / {}", std::env::consts::OS, std::env::consts::ARCH))
                    .small()
                    .color(Color32::GRAY),
            );
        });
        ui.separator();

        let input_devices = self.audio_input_devices.clone();
        let output_devices = self.audio_output_devices.clone();
        let serial_ports = self.radio_serial_ports.clone();
        let old_input = self.config.audio.input_device.clone();
        let old_output = self.config.audio.output_device.clone();
        let old_port = self.config.radio.serial_port.clone();

        egui::Grid::new("device_settings_grid")
            .num_columns(2)
            .spacing([10.0, 6.0])
            .show(ui, |ui| {
                ui.label("Audio input");
                egui::ComboBox::from_id_salt("audio_input_device")
                    .selected_text(
                        self.config
                            .audio
                            .input_device
                            .as_deref()
                            .unwrap_or("System default"),
                    )
                    .width(ui.available_width().max(180.0))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.config.audio.input_device,
                            None,
                            "System default",
                        );
                        for name in &input_devices {
                            ui.selectable_value(
                                &mut self.config.audio.input_device,
                                Some(name.clone()),
                                name,
                            );
                        }
                    });
                ui.end_row();

                ui.label("Audio output");
                egui::ComboBox::from_id_salt("audio_output_device")
                    .selected_text(
                        self.config
                            .audio
                            .output_device
                            .as_deref()
                            .unwrap_or("System default"),
                    )
                    .width(ui.available_width().max(180.0))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.config.audio.output_device,
                            None,
                            "System default",
                        );
                        for name in &output_devices {
                            ui.selectable_value(
                                &mut self.config.audio.output_device,
                                Some(name.clone()),
                                name,
                            );
                        }
                    });
                ui.end_row();

                ui.label("Radio / USB serial");
                egui::ComboBox::from_id_salt("radio_serial_port")
                    .selected_text(
                        self.config
                            .radio
                            .serial_port
                            .as_deref()
                            .unwrap_or("Auto detect"),
                    )
                    .width(ui.available_width().max(180.0))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.config.radio.serial_port,
                            None,
                            "Auto detect",
                        );
                        for name in &serial_ports {
                            ui.selectable_value(
                                &mut self.config.radio.serial_port,
                                Some(name.clone()),
                                name,
                            );
                        }
                    });
                ui.end_row();
            });

        if old_input != self.config.audio.input_device
            || old_output != self.config.audio.output_device
            || old_port != self.config.radio.serial_port
        {
            self.device_restart_required = true;
            self.profile_dirty = true;
            self.persist_profile("Saved devices to");
        }

        if self.device_restart_required {
            ui.add_space(4.0);
            ui.label(
                RichText::new(
                    "Output changes apply to the next transmission. Restart RigForge to reconnect input or radio devices.",
                )
                .small()
                .color(Color32::YELLOW),
            );
        } else if input_devices.is_empty() && output_devices.is_empty() {
            ui.label(
                RichText::new("No audio devices were reported by the operating system.")
                    .small()
                    .color(Color32::YELLOW),
            );
        }
    }

    fn draw_status(&mut self, ui: &mut egui::Ui, snapshot: &GuiState) {
        ui.heading("📡 Station Health");
        ui.label(
            RichText::new("What matters right now—not a wall of driver diagnostics.")
                .small()
                .color(Color32::GRAY),
        );
        ui.add_space(4.0);

        let update_age = snapshot.last_update.map(|last| last.elapsed().as_secs_f32());
        let (radio_value, radio_detail, radio_color) = match update_age {
            Some(age) if age < 3.0 => (
                "CONNECTED".to_string(),
                format!("Radio answered {:.1}s ago", age),
                Color32::LIGHT_GREEN,
            ),
            Some(age) => (
                "STALE".to_string(),
                format!("No fresh radio state for {:.1}s", age),
                Color32::YELLOW,
            ),
            None => (
                "WAITING".to_string(),
                "Waiting for the first radio update".to_string(),
                Color32::GRAY,
            ),
        };
        operator_status_card(ui, "📻 Radio link", &radio_value, &radio_detail, radio_color);

        let (audio_value, audio_detail, audio_color) = snapshot.audio_level_dbfs.map_or_else(
            || {
                (
                    "NO LEVEL".to_string(),
                    snapshot.audio_spectrum_status.clone(),
                    Color32::YELLOW,
                )
            },
            |level| {
                let clipped = snapshot.audio_clip_percent > 0.1;
                (
                    if clipped {
                        "CLIPPING".to_string()
                    } else {
                        format!("{level:.0} dBFS")
                    },
                    format!(
                        "{} · {:.1}% clipped",
                        snapshot.audio_spectrum_status, snapshot.audio_clip_percent
                    ),
                    if clipped {
                        Color32::from_rgb(255, 110, 100)
                    } else if level < -45.0 {
                        Color32::YELLOW
                    } else {
                        Color32::LIGHT_GREEN
                    },
                )
            },
        );
        operator_status_card(ui, "🎧 Audio input", &audio_value, &audio_detail, audio_color);

        let decode_status = match self.workspace_mode {
            WorkspaceMode::Ft8 => snapshot.ft8_decode_status.as_str(),
            _ => snapshot.digital_decode_status.as_str(),
        };
        operator_status_card(
            ui,
            "🔬 Decode engine",
            self.workspace_mode.label(),
            decode_status,
            if decode_status.contains("failed") || decode_status.contains("NO INPUT") {
                Color32::YELLOW
            } else {
                Color32::LIGHT_BLUE
            },
        );

        ui.horizontal(|ui| {
            ui.label(RichText::new("⚙ Compute policy").strong());
            let previous = self.compute_preference;
            egui::ComboBox::from_id_salt("compute_preference")
                .selected_text(self.compute_preference.label())
                .show_ui(ui, |ui| {
                    for preference in ComputePreference::ALL {
                        ui.selectable_value(
                            &mut self.compute_preference,
                            preference,
                            preference.label(),
                        );
                    }
                });
            if self.compute_preference != previous {
                self.acceleration_report = AccelerationReport::probe(self.compute_preference);
                self.profile_dirty = true;
                self.persist_profile("Compute policy saved to");
            }
        });
        let compute_detail = self
            .acceleration_report
            .fallback_reason
            .as_deref()
            .map(|reason| format!("{} · {reason}", self.acceleration_report.hardware_detail()))
            .unwrap_or_else(|| self.acceleration_report.hardware_detail());
        operator_status_card(
            ui,
            "🚀 Compute backend",
            &self.acceleration_report.summary(),
            &compute_detail,
            if self.acceleration_report.active == ActiveBackend::GpuCompute {
                Color32::from_rgb(210, 120, 255)
            } else {
                Color32::from_rgb(120, 210, 255)
            },
        );

        let gui_driver = std::env::var("GALLIUM_DRIVER").unwrap_or_default();
        let gui_adapter = std::env::var("MESA_D3D12_DEFAULT_ADAPTER_NAME")
            .unwrap_or_else(|_| "automatic adapter".to_string());
        let gui_renderer_detail = if gui_driver.eq_ignore_ascii_case("d3d12") {
            format!("{gui_adapter} preference · Mesa/WSLg")
        } else {
            "Override with GALLIUM_DRIVER; software rendering raises CPU load".to_string()
        };
        operator_status_card(
            ui,
            "🎨 GUI renderer",
            if gui_driver.eq_ignore_ascii_case("d3d12") {
                "D3D12 HARDWARE"
            } else if gui_driver.is_empty() {
                "SYSTEM DEFAULT"
            } else {
                &gui_driver
            },
            &gui_renderer_detail,
            if gui_driver.eq_ignore_ascii_case("d3d12") {
                Color32::LIGHT_GREEN
            } else {
                Color32::YELLOW
            },
        );

        let telemetry = if self.workspace_mode == WorkspaceMode::Ft8 {
            snapshot.ft8_compute_telemetry.as_ref()
        } else {
            snapshot.digital_compute_telemetry.as_ref()
        };
        if let Some(telemetry) = telemetry {
            operator_status_card(
                ui,
                "⏱ Last decode",
                &telemetry.concise(),
                &telemetry.stage_detail(),
                if telemetry.realtime_percent() > 80.0 {
                    Color32::YELLOW
                } else {
                    Color32::LIGHT_GREEN
                },
            );
        }

        operator_status_card(
            ui,
            "📊 Radio levels",
            &format!("AF {} · RF {} · PWR {}", fmt_opt_u8(snapshot.af_gain), fmt_opt_u8(snapshot.rf_gain), fmt_opt_u8(snapshot.rf_power)),
            &format!(
                "{} · {}",
                snapshot
                    .filter
                    .map(|value| format!("FIL{value}"))
                    .unwrap_or_else(|| "filter unknown".to_string()),
                if snapshot.data_mode == Some(true) {
                    "data mode"
                } else {
                    "voice/CW mode"
                }
            ),
            Color32::from_rgb(210, 190, 110),
        );

        ui.add_space(4.0);
        if ui
            .checkbox(&mut self.civ_spectrum_on, "📈 Radio scope waterfall")
            .changed()
        {
            self.profile_dirty = true;
            self.persist_profile("Auto-saved");
        }

        let wf_color = if !snapshot.radio_spectrum_desired {
            Color32::GRAY
        } else if snapshot.radio_spectrum_enabled {
            Color32::LIGHT_GREEN
        } else {
            Color32::YELLOW
        };
        ui.label(RichText::new(&snapshot.radio_waterfall_status).small().color(wf_color));

        if let Some(err) = &snapshot.last_error {
            ui.add_space(6.0);
            egui::Frame::group(ui.style())
                .fill(Color32::from_rgb(70, 42, 20))
                .stroke(egui::Stroke::new(1.5, Color32::YELLOW))
                .show(ui, |ui| {
                    ui.label(RichText::new("⚠ NEEDS ATTENTION").strong().color(Color32::YELLOW));
                    ui.label(err);
                });
        }
    }

    fn draw_radio_waterfall(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        snapshot: &GuiState,
    ) {
        ui.heading("Radio Waterfall (CI-V Scope)");
        ui.separator();

        if !snapshot.radio_spectrum_desired {
            ui.label(
                RichText::new("CI-V spectrum disabled (toggle it on in Live Status)")
                    .color(Color32::GRAY),
            );
            return;
        }

        let display_size = egui::vec2(ui.available_width(), RADIO_WF_HEIGHT as f32 * 1.9);

        if self.radio_waterfall_texture.is_none()
            || self.radio_waterfall_texture_revision != snapshot.radio_waterfall_revision
        {
            let image = build_waterfall_image(
                &snapshot.radio_waterfall_rows,
                RADIO_WF_WIDTH,
                RADIO_WF_HEIGHT,
                0.7,
            );
            if let Some(tex) = &mut self.radio_waterfall_texture {
                tex.set(image, TextureOptions::LINEAR);
            } else {
                self.radio_waterfall_texture = Some(ctx.load_texture(
                    "rigforge-radio-waterfall",
                    image,
                    TextureOptions::LINEAR,
                ));
            }
            self.radio_waterfall_texture_revision = snapshot.radio_waterfall_revision;
        }

        if let Some(tex) = &self.radio_waterfall_texture {
            ui.image((tex.id(), display_size));
        }

        ui.label(
            "Toggleable CI-V scope stream. Palette: blue\u{2192}cyan\u{2192}yellow\u{2192}white",
        );
    }

    fn draw_audio_waterfall(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        snapshot: &GuiState,
    ) {
        ui.heading("Audio Waterfall (RX Input / TX Output)");
        ui.separator();

        let bw_hz = filter_bandwidth_hz(&snapshot.mode, snapshot.filter);
        let display_bins = ((bw_hz.min(AUDIO_MAX_FREQ_HZ) as f32 / AUDIO_MAX_FREQ_HZ as f32)
            * AUDIO_BINS as f32)
            .round() as usize;
        let display_bins = display_bins.clamp(16, AUDIO_BINS);

        // Capture layout geometry before texture ops — available_width() can change mid-frame.
        let display_size = egui::vec2(ui.available_width(), AUDIO_WF_HEIGHT as f32 * 1.9);

        if self.audio_waterfall_texture.is_none()
            || self.audio_waterfall_texture_revision != snapshot.audio_waterfall_revision
            || self.audio_waterfall_texture_bins != display_bins
        {
            let image = build_waterfall_image(
                &snapshot.audio_waterfall_rows,
                display_bins,
                AUDIO_WF_HEIGHT,
                1.0,
            );
            if let Some(tex) = &mut self.audio_waterfall_texture {
                tex.set(image, TextureOptions::LINEAR);
            } else {
                self.audio_waterfall_texture = Some(ctx.load_texture(
                    "rigforge-audio-waterfall",
                    image,
                    TextureOptions::LINEAR,
                ));
            }
            self.audio_waterfall_texture_revision = snapshot.audio_waterfall_revision;
            self.audio_waterfall_texture_bins = display_bins;
        }
        if let Some(tex) = &self.audio_waterfall_texture {
            let image_widget =
                egui::Image::new((tex.id(), display_size)).sense(egui::Sense::click());
            let response = ui.add(image_widget);

            if let Some(pos) = response.interact_pointer_pos() {
                let rel = ((pos.x - response.rect.left()) / response.rect.width()).clamp(0.0, 1.0);
                let pick_hz = ((rel * bw_hz.min(AUDIO_MAX_FREQ_HZ) as f32).round() as u32)
                    .clamp(100, bw_hz.min(AUDIO_MAX_FREQ_HZ).max(100));

                if response.clicked() {
                    self.rx_tone_hz = pick_hz;
                    if !self.ft8_hold_tx_freq {
                        self.tx_tone_hz = pick_hz;
                    }
                    self.profile_dirty = true;
                    self.persist_profile("Auto-saved");
                    self.profile_io_status = format!("RX tone set: {} Hz", self.rx_tone_hz);
                }
                if response.secondary_clicked() {
                    self.tx_tone_hz = pick_hz;
                    self.profile_dirty = true;
                    self.persist_profile("Auto-saved");
                    self.profile_io_status = format!("TX tone set: {} Hz", self.tx_tone_hz);
                }
            }

            let bw = bw_hz.min(AUDIO_MAX_FREQ_HZ).max(1) as f32;
            let rx_x = response.rect.left()
                + (self.rx_tone_hz.min(bw as u32) as f32 / bw) * response.rect.width();
            let tx_x = response.rect.left()
                + (self.tx_tone_hz.min(bw as u32) as f32 / bw) * response.rect.width();

            let channel_half_width = (12.5 / bw * response.rect.width()).max(2.0);
            let rx_band = egui::Rect::from_min_max(
                egui::pos2(rx_x - channel_half_width, response.rect.top()),
                egui::pos2(rx_x + channel_half_width, response.rect.bottom()),
            );
            ui.painter().rect_filled(
                rx_band,
                0.0,
                Color32::from_rgba_unmultiplied(80, 220, 110, 32),
            );
            if self.tx_tone_hz.abs_diff(self.rx_tone_hz) > 12 {
                let tx_band = egui::Rect::from_min_max(
                    egui::pos2(tx_x - channel_half_width, response.rect.top()),
                    egui::pos2(tx_x + channel_half_width, response.rect.bottom()),
                );
                ui.painter().rect_filled(
                    tx_band,
                    0.0,
                    Color32::from_rgba_unmultiplied(240, 150, 60, 32),
                );
            }

            ui.painter().line_segment(
                [
                    egui::pos2(rx_x, response.rect.top()),
                    egui::pos2(rx_x, response.rect.bottom()),
                ],
                egui::Stroke::new(1.5, Color32::from_rgb(120, 220, 120)),
            );
            ui.painter().line_segment(
                [
                    egui::pos2(tx_x, response.rect.top()),
                    egui::pos2(tx_x, response.rect.bottom()),
                ],
                egui::Stroke::new(1.5, Color32::from_rgb(220, 160, 80)),
            );

            ui.painter().text(
                egui::pos2(response.rect.left() + 6.0, response.rect.top() + 4.0),
                egui::Align2::LEFT_TOP,
                format!("RX {} Hz", self.rx_tone_hz),
                egui::TextStyle::Small.resolve(ui.style()),
                Color32::from_rgb(120, 220, 120),
            );
            ui.painter().text(
                egui::pos2(response.rect.left() + 6.0, response.rect.top() + 20.0),
                egui::Align2::LEFT_TOP,
                format!("TX {} Hz", self.tx_tone_hz),
                egui::TextStyle::Small.resolve(ui.style()),
                Color32::from_rgb(220, 160, 80),
            );
        }
        ui.label(format!(
            "Audio: {}  |  0\u{2013}{} Hz ({} {})  |  L-click RX / R-click TX",
            snapshot.audio_spectrum_status,
            bw_hz.min(AUDIO_MAX_FREQ_HZ),
            snapshot.mode,
            snapshot
                .filter
                .map(|f| format!("FIL{f}"))
                .unwrap_or_default(),
        ));
    }

    fn draw_band_controls(&self, ui: &mut egui::Ui, snapshot: &GuiState) {
        ui.heading(format!("Band / Filter ({})", self.workspace_mode.label()));
        ui.separator();

        let current_hz = snapshot.frequency_hz.unwrap_or(0);
        let band_plan = workspace_band_plan(self.workspace_mode);

        // Band buttons — 3 per row
        ui.horizontal(|ui| {
            for &(label, freq_hz) in band_plan {
                let on_band = (current_hz as i64 - freq_hz as i64).unsigned_abs() < 200_000;
                if ui
                    .add(
                        egui::Button::new(RichText::new(label).monospace().strong()).fill(
                            if on_band {
                                Color32::from_rgb(30, 80, 30)
                            } else {
                                Color32::from_gray(40)
                            },
                        ),
                    )
                    .on_hover_text(format!(
                        "{:.3} MHz  {}",
                        freq_hz as f64 / 1_000_000.0,
                        self.workspace_mode.label()
                    ))
                    .clicked()
                {
                    self.send_command(GuiCommand::TuneWorkspaceBand(freq_hz));
                }
            }
        });

        ui.add_space(4.0);

        // Filter selector
        ui.horizontal(|ui| {
            ui.label(RichText::new("BW").strong());
            for fil in 1u8..=3 {
                let active = snapshot.filter == Some(fil);
                let label = format!("FIL{fil}");
                if ui
                    .add(
                        egui::Button::new(RichText::new(&label).monospace()).fill(if active {
                            Color32::from_rgb(20, 60, 120)
                        } else {
                            Color32::from_gray(40)
                        }),
                    )
                    .clicked()
                {
                    self.send_command(GuiCommand::SetFilter(fil));
                }
            }
        });
    }

    fn ft8_conversation_target(&self) -> Option<String> {
        self.ft8_seq_target
            .clone()
            .or_else(|| self.ft8_session.as_ref().map(|session| session.target.clone()))
            .or_else(|| {
                self.ft8_selected
                    .and_then(|index| self.ft8_log.get(index))
                    .and_then(|entry| parse_message(&entry.message))
                    .map(|message| message.from)
            })
    }

    fn draw_ft8_activity_stats(&self, ui: &mut egui::Ui) {
        let stats = ft8_activity_stats(&self.ft8_log);
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new("📊 BAND PULSE")
                    .strong()
                    .color(Color32::LIGHT_BLUE),
            );
            ft8_stat_chip(
                ui,
                "This cycle",
                stats.latest_cycle.to_string(),
                format!("{} CQ", stats.cq_this_cycle),
            );
            ft8_stat_chip(
                ui,
                "Average",
                format!("{:.1}/cycle", stats.average_per_cycle),
                "rolling log".to_string(),
            );
            ft8_stat_chip(
                ui,
                "Stations",
                stats.unique_stations.to_string(),
                "unique heard".to_string(),
            );
            let (most_heard, detail) = stats
                .most_heard
                .map(|(call, count)| (call, format!("{count} decodes")))
                .unwrap_or_else(|| ("—".to_string(), "waiting".to_string()));
            ft8_stat_chip(ui, "Most heard", most_heard, detail);
            ft8_stat_chip(
                ui,
                "Median SNR",
                stats
                    .median_snr
                    .map(|snr| format!("{snr:+} dB"))
                    .unwrap_or_else(|| "—".to_string()),
                "rolling log".to_string(),
            );
        });
    }

    fn draw_ft8_conversation(
        &self,
        ui: &mut egui::Ui,
        snapshot: &GuiState,
        height: f32,
    ) {
        let target = self.ft8_conversation_target();
        let operator_call = self.station_callsign_or_default().to_string();
        let rx_level = channel_waterfall_level(&snapshot.audio_waterfall_rows, self.rx_tone_hz);
        let tx_level = channel_waterfall_level(&snapshot.audio_waterfall_rows, self.tx_tone_hz);
        let mut lines = Vec::new();

        for entry in &self.ft8_log {
            let belongs = if let Some(target) = target.as_deref() {
                parse_message(&entry.message).is_some_and(|message| {
                    // RX chat contains transmissions from the station we are
                    // working, not unrelated callers transmitting to them.
                    ft8_ops::callsign_eq(&message.from, target)
                })
            } else {
                entry.freq_hz.abs_diff(self.rx_tone_hz) <= 15
            };
            if belongs {
                lines.push(Ft8ChatLine {
                    period: entry.period,
                    utc: entry.utc.clone(),
                    message: entry.message.clone(),
                    detail: format!("RX {:+} dB · {:.1}s · {} Hz", entry.snr_db, entry.dt_s, entry.freq_hz),
                    direction: Ft8ChatDirection::Rx,
                });
            }
        }
        for entry in &self.ft8_tx_chat {
            let belongs = if let Some(target) = target.as_deref() {
                parse_message(&entry.message).is_some_and(|message| {
                    ft8_ops::callsign_eq(&message.from, target)
                        || message
                            .to
                            .as_deref()
                            .is_some_and(|to| ft8_ops::callsign_eq(to, target))
                })
            } else {
                true
            };
            if belongs {
                lines.push(Ft8ChatLine {
                    period: entry.period,
                    utc: entry.utc.clone(),
                    message: entry.message.clone(),
                    detail: format!("TX · {} Hz", self.tx_tone_hz),
                    direction: Ft8ChatDirection::Tx,
                });
            }
        }
        lines.sort_by_key(|line| (line.period, line.direction == Ft8ChatDirection::Tx));
        if lines.len() > 30 {
            lines.drain(..lines.len() - 30);
        }

        egui::Frame::group(ui.style())
            .fill(Color32::from_rgb(20, 23, 28))
            .show(ui, |ui| {
                ui.set_min_height(height);
                ui.set_max_height(height);
                ui.horizontal_wrapped(|ui| {
                    let title = target
                        .as_deref()
                        .map(|call| format!("💬 ACTIVE QSO · {call}"))
                        .unwrap_or_else(|| format!("🎧 CHANNEL CHAT · {} Hz", self.rx_tone_hz));
                    ui.label(RichText::new(title).strong().color(Color32::LIGHT_BLUE));
                    if let Some(session) = &self.ft8_session {
                        ui.label(
                            RichText::new(qso_stage_label(session.stage))
                                .small()
                                .color(Color32::from_rgb(220, 180, 90)),
                        );
                    }
                    ui.separator();
                    ui.label(
                        RichText::new(format!("RX {} Hz", self.rx_tone_hz))
                            .monospace()
                            .color(Color32::from_rgb(120, 220, 120)),
                    );
                    ui.add(
                        egui::ProgressBar::new(rx_level as f32 / 255.0)
                            .desired_width(70.0),
                    );
                    ui.label(
                        RichText::new(format!("TX {} Hz", self.tx_tone_hz))
                            .monospace()
                            .color(Color32::from_rgb(220, 160, 80)),
                    );
                    ui.add(
                        egui::ProgressBar::new(tx_level as f32 / 255.0)
                            .desired_width(70.0),
                    );
                });
                ui.separator();

                egui::ScrollArea::vertical()
                    .id_salt("ft8_conversation")
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        if lines.is_empty() {
                            ui.centered_and_justified(|ui| {
                                ui.label(
                                    RichText::new(if target.is_some() {
                                        "📡 Listening for this station’s next move…"
                                    } else {
                                        "✨ Select a decode or tune a channel to open a conversation."
                                    })
                                    .color(Color32::GRAY),
                                );
                            });
                        }
                        for line in lines {
                            let is_tx = line.direction == Ft8ChatDirection::Tx;
                            let call_hit = (!is_tx)
                                .then(|| operator_call_hit(&line.message, &operator_call))
                                .flatten();
                            let layout = if is_tx {
                                egui::Layout::right_to_left(egui::Align::Min)
                            } else {
                                egui::Layout::left_to_right(egui::Align::Min)
                            };
                            ui.with_layout(layout, |ui| {
                                let (fill, stroke) = call_hit.map_or_else(
                                    || {
                                        (
                                            if is_tx {
                                                Color32::from_rgb(53, 43, 25)
                                            } else {
                                                Color32::from_rgb(25, 49, 38)
                                            },
                                            egui::Stroke::NONE,
                                        )
                                    },
                                    |hit| {
                                        let (_, accent, fill) = call_hit_badge(hit);
                                        (fill, egui::Stroke::new(2.0, accent))
                                    },
                                );
                                egui::Frame::group(ui.style())
                                    .fill(fill)
                                    .stroke(stroke)
                                    .show(ui, |ui| {
                                        if let Some(hit) = call_hit {
                                            let (badge, accent, _) = call_hit_badge(hit);
                                            ui.label(RichText::new(badge).strong().color(accent));
                                        }
                                        ui.label(
                                            RichText::new(&line.message).monospace().strong(),
                                        );
                                        ui.label(
                                            RichText::new(format!(
                                                "{} · {}",
                                                line.utc, line.detail
                                            ))
                                            .small()
                                            .color(Color32::GRAY),
                                        );
                                    });
                            });
                            ui.add_space(2.0);
                        }
                    });
            });
    }

    fn draw_ft8_workspace(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context, snapshot: &GuiState) {
        // ── Header row ──────────────────────────────────────────────────────
        ui.horizontal(|ui| {
            ui.heading("FT8");
            ui.separator();

            // 15-second period progress
            let progress = ft8_period_progress();
            let tx_active = snapshot.ptt_on || self.ft8_tx_active.load(Ordering::Acquire);
            let phase_label = if snapshot.ptt_on {
                "TX NOW"
            } else if self.ft8_tx_queued_period.is_some() {
                "TX QUEUED"
            } else if self.ft8_autoseq && self.ft8_session.is_some() {
                "AUTO TX ARMED"
            } else {
                "RX"
            };
            let period_color = if tx_active || self.ft8_tx_queued_period.is_some() {
                Color32::from_rgb(190, 70, 35)
            } else if self.ft8_autoseq && self.ft8_session.is_some() {
                Color32::from_rgb(220, 160, 80)
            } else {
                Color32::from_rgb(30, 130, 30)
            };
            ui.label(RichText::new(phase_label).strong().color(period_color));
            let bar_w = 140.0;
            let bar_h = 14.0;
            let (rect, _) = ui.allocate_exact_size(egui::vec2(bar_w, bar_h), egui::Sense::hover());
            ui.painter().rect_filled(rect, 2.0, Color32::from_gray(30));
            let fill = egui::Rect::from_min_size(rect.min, egui::vec2(bar_w * progress, bar_h));
            ui.painter().rect_filled(fill, 2.0, period_color);
            let remaining = 15.0 * (1.0 - progress);
            ui.label(format!("{remaining:.1}s"));

            ui.separator();
            // Freq display
            if let Some(hz) = snapshot.frequency_hz {
                ui.label(
                    RichText::new(format!("{:.3} MHz", hz as f64 / 1_000_000.0))
                        .monospace()
                        .strong(),
                );
            }
            ui.label(RichText::new(&snapshot.mode).monospace());
            ui.separator();
            ui.label(
                RichText::new(format!("RX {} Hz", self.rx_tone_hz))
                    .monospace()
                    .color(Color32::from_rgb(120, 220, 120)),
            );
            ui.label(
                RichText::new(format!("TX {} Hz", self.tx_tone_hz))
                    .monospace()
                    .color(Color32::from_rgb(220, 160, 80)),
            );
            ui.separator();
            ui.label(
                RichText::new(format!("SEQ {}", self.ft8_seq_state.label()))
                    .monospace()
                    .color(Color32::LIGHT_BLUE),
            );

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let halt_label = if self.ft8_halt_after_tx {
                    "STOP AFTER THIS TX"
                } else {
                    "STOP AFTER NEXT TX"
                };
                let halt_color = if self.ft8_halt_after_tx {
                    Color32::from_rgb(220, 180, 80)
                } else {
                    Color32::GRAY
                };
                if ui
                    .button(RichText::new(halt_label).color(halt_color))
                    .on_hover_text(
                        "Pause automatic transmissions after the next TX completes; click again to cancel",
                    )
                    .clicked()
                {
                    self.ft8_halt_after_tx = !self.ft8_halt_after_tx;
                    self.ft8_seq_status = if self.ft8_halt_after_tx {
                        "Automatic TX will pause after the next transmission".to_string()
                    } else {
                        "Stop-after-TX canceled".to_string()
                    };
                }

                let hold_label = if self.ft8_hold_tx_freq {
                    "HOLD TX FREQ"
                } else {
                    "TX TRACKS RX"
                };
                let hold_color = if self.ft8_hold_tx_freq {
                    Color32::from_rgb(120, 200, 220)
                } else {
                    Color32::from_rgb(120, 220, 120)
                };
                if ui
                    .button(RichText::new(hold_label).color(hold_color))
                    .clicked()
                {
                    self.ft8_hold_tx_freq = !self.ft8_hold_tx_freq;
                    if !self.ft8_hold_tx_freq {
                        self.tx_tone_hz = self.rx_tone_hz;
                    }
                    self.profile_dirty = true;
                    self.persist_profile("Auto-saved");
                }

                let deep_label = if self.ft8_deep_decode {
                    "DECODE: DEEP"
                } else {
                    "DECODE: FAST"
                };
                let deep_color = if self.ft8_deep_decode {
                    Color32::YELLOW
                } else {
                    Color32::LIGHT_GREEN
                };
                if ui
                    .button(RichText::new(deep_label).color(deep_color))
                    .clicked()
                {
                    self.ft8_deep_decode = !self.ft8_deep_decode;
                    self.profile_dirty = true;
                    self.persist_profile("Auto-saved");
                }
            });
        });

        ui.horizontal_wrapped(|ui| {
            let (auto_label, auto_fill, auto_stroke) = if self.ft8_autoseq {
                (
                    "🔥 FT8 TX ARMED · CLICK TO DISARM ALL",
                    Color32::from_rgb(92, 43, 25),
                    Color32::from_rgb(255, 151, 72),
                )
            } else {
                (
                    "🔒 FT8 AUTO DISARMED · CLICK TO ARM",
                    Color32::from_rgb(28, 52, 70),
                    Color32::from_rgb(92, 174, 220),
                )
            };
            if ui
                .add(
                    egui::Button::new(RichText::new(auto_label).strong().color(Color32::WHITE))
                        .fill(auto_fill)
                        .stroke(egui::Stroke::new(1.5, auto_stroke)),
                )
                .on_hover_text(if self.ft8_autoseq {
                    "Cancel queued/active TX, drop PTT, and disarm FT8 and FT4"
                } else {
                    "Arm FT8 automatic sequencing; no transmission is sent until an exchange is started"
                })
                .clicked()
            {
                if self.ft8_autoseq {
                    self.disarm_all_tx("All TX stopped and disarmed by operator");
                } else {
                    self.ft8_autoseq = true;
                    if self.ft8_session.is_some() {
                        self.ft8_seq_state = Ft8SeqState::ReplyArmed;
                        self.ft8_seq_status = "Automatic TX resumed; waiting for reply".to_string();
                    } else {
                        self.ft8_seq_status =
                            "FT8 automatic operation armed; waiting for an exchange".to_string();
                    }
                    self.profile_dirty = true;
                    self.persist_profile("FT8 TX armed");
                }
            }
            ui.label("Select caller:");
            let previous_policy = self.ft8_auto_reply_policy;
            egui::ComboBox::from_id_salt("ft8_auto_reply_policy")
                .selected_text(self.ft8_auto_reply_policy.label())
                .show_ui(ui, |ui| {
                    for policy in AutoReplyPolicy::ALL {
                        ui.selectable_value(
                            &mut self.ft8_auto_reply_policy,
                            policy,
                            policy.label(),
                        );
                    }
                });
            if self.ft8_auto_reply_policy != previous_policy {
                self.profile_dirty = true;
                self.persist_profile("Auto-saved");
            }
            if ui
                .checkbox(&mut self.ft8_auto_answer_cq, "Answer unattended CQs")
                .changed()
            {
                self.profile_dirty = true;
                self.persist_profile("Auto-saved");
            }
            ui.separator();
            ui.label(
                RichText::new(&snapshot.ft8_decode_status)
                    .small()
                    .color(Color32::GRAY),
            );
            if let Some(level) = snapshot.audio_level_dbfs {
                let color = if snapshot.audio_clip_percent > 0.1 || level < -45.0 {
                    Color32::YELLOW
                } else {
                    Color32::LIGHT_GREEN
                };
                ui.label(
                    RichText::new(format!(
                        "Input {level:.0} dBFS / clip {:.1}%",
                        snapshot.audio_clip_percent
                    ))
                    .small()
                    .color(color),
                );
            }
            if let Some(offset) = snapshot.ft8_clock_offset_s {
                let color = if offset.abs() > 1.0 {
                    Color32::YELLOW
                } else {
                    Color32::LIGHT_GREEN
                };
                ui.label(
                    RichText::new(format!("Clock dT {offset:+.2}s"))
                        .small()
                        .color(color),
                );
            }
        });

        ui.separator();
        self.draw_ft8_activity_stats(ui);
        ui.add_space(4.0);

        let panel_h = ui.available_height();
        let conversation_h = (panel_h * 0.28).clamp(150.0, 260.0);
        let decode_h = (panel_h * 0.38).max(170.0);
        let tx_h = (panel_h * 0.20).max(120.0);
        let operator_call = self.station_callsign_or_default().to_string();

        if let Some((entry, hit)) = self.ft8_log.iter().rev().find_map(|entry| {
            operator_call_hit(&entry.message, &operator_call).map(|hit| (entry, hit))
        }) {
            draw_operator_call_banner(ui, "FT8", &operator_call, &entry.message, hit);
            ui.add_space(4.0);
        }

        // ── Decode log ───────────────────────────────────────────────────────
        egui::Frame::dark_canvas(ui.style()).show(ui, |ui| {
            ui.set_min_height(decode_h);
            ui.set_max_height(decode_h);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("📡 LIVE DECODES")
                        .strong()
                        .color(Color32::LIGHT_BLUE),
                );
                ui.separator();
                if ui.checkbox(&mut self.ft8_cq_only_view, "CQ only").changed() {
                    self.profile_dirty = true;
                    self.persist_profile("Auto-saved");
                }
                if ui.checkbox(&mut self.ft8_follow_log, "Follow").changed() {
                    self.profile_dirty = true;
                    self.persist_profile("Auto-saved");
                }
                ui.label("Keep");
                if ui
                    .add(
                        egui::DragValue::new(&mut self.ft8_max_log_entries)
                            .range(80..=1000)
                            .speed(5),
                    )
                    .changed()
                {
                    self.profile_dirty = true;
                    self.persist_profile("Auto-saved");
                }
                ui.label("rows");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("Clear").clicked() {
                        self.ft8_log.clear();
                        self.ft8_tx_chat.clear();
                        self.ft8_selected = None;
                    }
                    ui.label(format!("{} msgs", self.ft8_log.len()));
                });
            });
            ui.separator();

            ui.horizontal(|ui| {
                ui.label(RichText::new("UTC").monospace().strong());
                ui.add_space(8.0);
                ui.label(RichText::new("SNR").monospace().strong());
                ui.add_space(8.0);
                ui.label(RichText::new("dT").monospace().strong());
                ui.add_space(8.0);
                ui.label(RichText::new("Hz").monospace().strong());
                ui.add_space(8.0);
                ui.label(RichText::new("Message").monospace().strong());
            });
            ui.separator();

            egui::ScrollArea::vertical()
                .id_salt("ft8_log")
                .stick_to_bottom(self.ft8_follow_log)
                .show(ui, |ui| {
                    if self.ft8_log.is_empty() {
                        ui.add_space(10.0);
                        ui.centered_and_justified(|ui| {
                            ui.label(
                                RichText::new("🌌 Listening… the band is quiet for the moment.")
                                    .color(Color32::from_gray(100)),
                            );
                        });
                        return;
                    }

                    let selected = self.ft8_selected;
                    let mut new_sel = selected;
                    let mut prev_utc: Option<&str> = None;
                    let mut reply_target_from_double_click: Option<String> = None;
                    let mut compose_from_double_click: Option<String> = None;
                    let mut picked_freq_from_double_click: Option<u32> = None;
                    let mut picked_period_from_double_click: Option<u64> = None;
                    let mut move_tx_from_double_click: Option<bool> = None;
                    let mut session_from_double_click: Option<Ft8Session> = None;
                    for (i, entry) in self.ft8_log.iter().enumerate() {
                        if self.ft8_cq_only_view && !entry.is_cq {
                            continue;
                        }
                        if let Some(prev) = prev_utc {
                            if prev != entry.utc {
                                ui.separator();
                            }
                        }
                        prev_utc = Some(&entry.utc);

                        let is_sel = selected == Some(i);
                        let call_hit = operator_call_hit(&entry.message, &operator_call);
                        let text_color = if let Some(hit) = call_hit {
                            call_hit_badge(hit).1
                        } else if entry.is_cq {
                            Color32::from_rgb(100, 220, 100)
                        } else if entry.snr_db >= -5 {
                            Color32::from_rgb(220, 220, 140)
                        } else {
                            Color32::LIGHT_GRAY
                        };
                        let row = RichText::new(format!(
                            "{:12}  {:+3}  {:5.1}  {:>5}  {}",
                            entry.utc, entry.snr_db, entry.dt_s, entry.freq_hz, entry.message
                        ))
                        .monospace()
                        .color(text_color);

                        let resp = if let Some(hit) = call_hit {
                            let (badge, accent, fill) = call_hit_badge(hit);
                            egui::Frame::group(ui.style())
                                .fill(fill)
                                .stroke(egui::Stroke::new(1.5, accent))
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(RichText::new(badge).strong().color(accent));
                                        ui.selectable_label(is_sel, row)
                                    })
                                    .inner
                                })
                                .inner
                        } else {
                            ui.selectable_label(is_sel, row)
                        };
                        if resp.clicked() {
                            let now = Instant::now();
                            let synthetic_double = self
                                .ft8_last_click
                                .map(|(idx, t)| {
                                    idx == i && now.duration_since(t) <= Duration::from_millis(500)
                                })
                                .unwrap_or(false);
                            self.ft8_last_click = Some((i, now));

                            new_sel = if is_sel { None } else { Some(i) };

                            if synthetic_double || resp.double_clicked() {
                                // Pre-fill compose with a reply to this call.
                                if let Some(parsed) = parse_message(&entry.message) {
                                    let call = parsed.from.clone();
                                    let my = self.station_callsign_or_default();
                                    let grid = self.station_grid_or_default();
                                    let mut session = Ft8Session::start(call.clone(), entry.period);
                                    if let Some(response) = session.response_to(
                                        &parsed,
                                        my,
                                        grid,
                                        entry.snr_db,
                                        entry.period,
                                    ) {
                                        compose_from_double_click = Some(response);
                                        reply_target_from_double_click = Some(call);
                                        session_from_double_click = Some(session);
                                        picked_period_from_double_click = Some(entry.period);
                                        picked_freq_from_double_click = Some(entry.freq_hz);
                                        // Only answering a CQ deliberately moves TX. A caller
                                        // answering our CQ is received on their offset while we
                                        // keep transmitting where we called.
                                        move_tx_from_double_click =
                                            Some(should_move_tx_to_decode(&parsed, false));
                                    }
                                }
                            }
                        }
                    }
                    self.ft8_selected = new_sel;
                    if let (
                        Some(compose),
                        Some(target),
                        Some(session),
                        Some(freq_hz),
                        Some(period),
                        Some(move_tx_to_remote),
                    ) = (
                        compose_from_double_click,
                        reply_target_from_double_click,
                        session_from_double_click,
                        picked_freq_from_double_click,
                        picked_period_from_double_click,
                        move_tx_from_double_click,
                    ) {
                        let reply = PendingManualFt8Reply {
                            compose,
                            target: target.clone(),
                            session,
                            freq_hz,
                            source_period: period,
                            move_tx_to_remote,
                        };
                        let tx_scheduled = self.ft8_tx_active.load(Ordering::Acquire)
                            || self.ft8_tx_queued_period.is_some();
                        let same_target = self
                            .ft8_seq_target
                            .as_deref()
                            .is_some_and(|current| ft8_ops::callsign_eq(current, &target));
                        if tx_scheduled && same_target {
                            self.ft8_seq_status = format!("Reply to {target} is already queued");
                        } else if tx_scheduled
                            && (snapshot.ptt_on || self.ft8_tx_started_period.is_some())
                        {
                            self.ft8_seq_status =
                                "Current TX is already on air; target was not changed".to_string();
                        } else if tx_scheduled {
                            self.cancel_ft8_sequence(format!(
                                "Canceling prior reply; switching to {target}"
                            ));
                            self.ft8_pending_manual_reply = Some(reply);
                        } else {
                            self.arm_manual_ft8_reply(reply);
                        }
                    }
                });
        });

        ui.add_space(4.0);

        // QSO and selected-channel traffic belongs below global band activity,
        // in the mode workspace rather than the universal monitoring rail.
        self.draw_ft8_conversation(ui, snapshot, conversation_h);
        ui.add_space(4.0);

        // ── TX compose ───────────────────────────────────────────────────────
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_min_height(tx_h);
            ui.horizontal(|ui| {
                ui.label(RichText::new("📣 TX DECK").strong());
                ui.separator();
                let ptt_color = if snapshot.ptt_on {
                    Color32::from_rgb(200, 60, 60)
                } else {
                    Color32::from_gray(80)
                };
                if ui
                    .button(
                        RichText::new(if snapshot.ptt_on {
                            "● PTT ON"
                        } else {
                            "○ PTT"
                        })
                        .color(ptt_color),
                    )
                    .clicked()
                {
                    if snapshot.ptt_on
                        || self.ft8_tx_active.load(Ordering::Acquire)
                        || self.digital_tx_active.load(Ordering::Acquire)
                    {
                        self.disarm_all_tx("TX/PTT stopped and all modes disarmed");
                    } else {
                        self.send_command(GuiCommand::TogglePtt);
                    }
                }
                let tx_active = self.ft8_tx_active.load(Ordering::Relaxed);
                if ui
                    .button(RichText::new("⛔ STOP + DISARM ALL").strong().color(if tx_active {
                        Color32::from_rgb(255, 130, 130)
                    } else {
                        Color32::from_gray(120)
                    }))
                    .on_hover_text(
                        "Drop PTT, cancel queued TX, and disarm FT8/FT4 automatic operation",
                    )
                    .clicked()
                {
                    self.disarm_all_tx("All TX stopped and disarmed by operator");
                }
            });

            ui.horizontal(|ui| {
                let available = ui.available_width() - 70.0;
                ui.add(
                    egui::TextEdit::singleline(&mut self.ft8_compose)
                        .desired_width(available)
                        .hint_text("CQ W1AW FN20")
                        .font(egui::TextStyle::Monospace),
                );
                if ui
                    .button(
                        RichText::new("SEND")
                            .strong()
                            .color(Color32::from_rgb(80, 180, 80)),
                    )
                    .clicked()
                    && !self.ft8_compose.is_empty()
                {
                    self.queue_ft8_tx_from_compose(Ft8TxQueuePolicy::Standard, None);
                }
            });

            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                let my = self.station_callsign_or_default().to_string();
                let grid = self.station_grid_or_default().to_string();
                let target = self.ft8_seq_target.clone();
                if ui.small_button("CALL CQ").clicked() {
                    self.ft8_compose = format!("CQ {my} {grid}");
                    self.ft8_autoseq = true;
                    self.ft8_seq_state = Ft8SeqState::CqArmed;
                    self.ft8_seq_target = None;
                    self.ft8_seq_status = "CQ armed (waiting for next slot)".to_string();
                    self.ft8_session = None;
                    self.queue_ft8_tx_from_compose(Ft8TxQueuePolicy::NextSlotOnly, None);
                    self.profile_dirty = true;
                    self.persist_profile("Auto-saved");
                }
                for (label, exchange) in [("Grid", grid.as_str()), ("RR73", "RR73"), ("73", "73")] {
                    if ui
                        .add_enabled(target.is_some(), egui::Button::new(label))
                        .on_hover_text("Requires an active or selected QSO target")
                        .clicked()
                    {
                        self.ft8_compose =
                            format!("{} {my} {exchange}", target.as_deref().unwrap_or_default());
                    }
                }
            });

            ui.label(
                RichText::new(&self.ft8_seq_status)
                    .small()
                    .color(Color32::GRAY),
            );
            ui.horizontal(|ui| {
                ui.label("PTT lead");
                if ui
                    .add(
                        egui::DragValue::new(&mut self.ptt_lead_ms)
                            .range(100..=1500)
                            .suffix(" ms"),
                    )
                    .changed()
                {
                    self.profile_dirty = true;
                    self.persist_profile("Auto-saved");
                }
                ui.label("tail");
                if ui
                    .add(
                        egui::DragValue::new(&mut self.ptt_tail_ms)
                            .range(0..=1000)
                            .suffix(" ms"),
                    )
                    .changed()
                {
                    self.profile_dirty = true;
                    self.persist_profile("Auto-saved");
                }
            });
        });

        ui.add_space(4.0);

        // ── Selected decode detail ───────────────────────────────────────────
        if let Some(idx) = self.ft8_selected {
            if let Some(e) = self.ft8_log.get(idx) {
                let e = e.clone();
                ui.group(|ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(&e.message).monospace().strong());
                        ui.separator();
                        ui.label(format!(
                            "{}  {:+}dB  {}Hz  Δt{:.1}s",
                            e.utc, e.snr_db, e.freq_hz, e.dt_s
                        ));
                        if e.is_cq {
                            ui.label(RichText::new("CQ").color(Color32::LIGHT_GREEN).strong());
                        }
                    });
                    ui.horizontal_wrapped(|ui| {
                        let my = self.station_callsign_or_default();
                        let grid = self.station_grid_or_default();
                        if let Some(call) = parse_message(&e.message).map(|message| message.from) {
                            if ui.small_button(format!("Reply → {call}")).clicked() {
                                self.ft8_compose = format!("{call} {my} {grid}");
                                self.ft8_seq_target = Some(call.clone());
                            }
                        }
                    });
                });
            }
        }
    }

    fn draw_ft4_conversation(
        &self,
        ui: &mut egui::Ui,
        snapshot: &GuiState,
        height: f32,
    ) {
        let operator_call = self.station_callsign_or_default().to_string();
        let target = self
            .digital_seq_target
            .clone()
            .or_else(|| {
                self.digital_selected
                    .as_ref()
                    .and_then(|entry| parse_message(&entry.message))
                    .map(|message| message.from)
            });
        let mut lines = Vec::new();
        for entry in snapshot
            .digital_decodes
            .iter()
            .filter(|entry| entry.mode == WorkspaceMode::Ft4)
        {
            let belongs = target.as_deref().map_or_else(
                || entry.freq_hz.abs_diff(self.rx_tone_hz) <= 15,
                |target| {
                    parse_message(&entry.message)
                        .is_some_and(|message| ft8_ops::callsign_eq(&message.from, target))
                },
            );
            if belongs {
                lines.push(Ft8ChatLine {
                    period: entry.period,
                    utc: entry.utc.clone(),
                    message: entry.message.clone(),
                    detail: format!(
                        "RX {:+.1} dB · {:+.2}s · {} Hz",
                        entry.snr_db, entry.dt_s, entry.freq_hz
                    ),
                    direction: Ft8ChatDirection::Rx,
                });
            }
        }
        for entry in self
            .digital_tx_chat
            .iter()
            .filter(|entry| entry.mode == WorkspaceMode::Ft4)
        {
            let belongs = target.as_deref().map_or(true, |target| {
                parse_message(&entry.message).is_some_and(|message| {
                    message
                        .to
                        .as_deref()
                        .is_some_and(|to| ft8_ops::callsign_eq(to, target))
                })
            });
            if belongs {
                lines.push(Ft8ChatLine {
                    period: entry.period,
                    utc: entry.utc.clone(),
                    message: entry.message.clone(),
                    detail: format!("TX · {} Hz", self.tx_tone_hz),
                    direction: Ft8ChatDirection::Tx,
                });
            }
        }
        lines.sort_by_key(|line| (line.period, line.direction == Ft8ChatDirection::Tx));
        if lines.len() > 30 {
            lines.drain(..lines.len() - 30);
        }
        let rx_level = channel_waterfall_level(&snapshot.audio_waterfall_rows, self.rx_tone_hz);
        let tx_level = channel_waterfall_level(&snapshot.audio_waterfall_rows, self.tx_tone_hz);

        egui::Frame::group(ui.style())
            .fill(Color32::from_rgb(20, 23, 28))
            .show(ui, |ui| {
                ui.set_min_height(height);
                ui.set_max_height(height);
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new(
                            target
                                .as_deref()
                                .map(|call| format!("💬 ACTIVE FT4 QSO · {call}"))
                                .unwrap_or_else(|| {
                                    format!("🎧 FT4 CHANNEL CHAT · {} Hz", self.rx_tone_hz)
                                }),
                        )
                        .strong()
                        .color(Color32::LIGHT_BLUE),
                    );
                    if let Some(session) = &self.ft4_session {
                        ui.label(
                            RichText::new(qso_stage_label(session.stage))
                                .small()
                                .color(Color32::from_rgb(220, 180, 90)),
                        );
                    }
                    ui.separator();
                    ui.label(
                        RichText::new(format!("RX {} Hz", self.rx_tone_hz))
                            .monospace()
                            .color(Color32::from_rgb(120, 220, 120)),
                    );
                    ui.add(egui::ProgressBar::new(rx_level as f32 / 255.0).desired_width(70.0));
                    ui.label(
                        RichText::new(format!("TX {} Hz", self.tx_tone_hz))
                            .monospace()
                            .color(Color32::from_rgb(220, 160, 80)),
                    );
                    ui.add(egui::ProgressBar::new(tx_level as f32 / 255.0).desired_width(70.0));
                });
                ui.separator();
                egui::ScrollArea::vertical()
                    .id_salt("ft4_conversation")
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        if lines.is_empty() {
                            ui.centered_and_justified(|ui| {
                                ui.label(
                                    RichText::new(
                                        "✨ Select a decode or tune FT4 to open a conversation.",
                                    )
                                        .color(Color32::GRAY),
                                );
                            });
                        }
                        for line in lines {
                            let is_tx = line.direction == Ft8ChatDirection::Tx;
                            let call_hit = (!is_tx)
                                .then(|| operator_call_hit(&line.message, &operator_call))
                                .flatten();
                            let layout = if is_tx {
                                egui::Layout::right_to_left(egui::Align::Min)
                            } else {
                                egui::Layout::left_to_right(egui::Align::Min)
                            };
                            ui.with_layout(layout, |ui| {
                                let (fill, stroke) = call_hit.map_or_else(
                                    || {
                                        (
                                            if is_tx {
                                                Color32::from_rgb(53, 43, 25)
                                            } else {
                                                Color32::from_rgb(25, 49, 38)
                                            },
                                            egui::Stroke::NONE,
                                        )
                                    },
                                    |hit| {
                                        let (_, accent, fill) = call_hit_badge(hit);
                                        (fill, egui::Stroke::new(2.0, accent))
                                    },
                                );
                                egui::Frame::group(ui.style())
                                    .fill(fill)
                                    .stroke(stroke)
                                    .show(ui, |ui| {
                                        if let Some(hit) = call_hit {
                                            let (badge, accent, _) = call_hit_badge(hit);
                                            ui.label(RichText::new(badge).strong().color(accent));
                                        }
                                        ui.label(
                                            RichText::new(&line.message).monospace().strong(),
                                        );
                                        ui.label(
                                            RichText::new(format!(
                                                "{} · {}",
                                                line.utc, line.detail
                                            ))
                                            .small()
                                            .color(Color32::GRAY),
                                        );
                                    });
                            });
                            ui.add_space(2.0);
                        }
                    });
            });
    }

    fn draw_ft4_workspace(&mut self, ui: &mut egui::Ui, snapshot: &GuiState) {
        let now_s = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs_f64())
            .unwrap_or_default();
        let progress = (now_s % FT4_SLOT_SECONDS) / FT4_SLOT_SECONDS;
        let tx_active = self.digital_tx_active.load(Ordering::Acquire);
        ui.horizontal_wrapped(|ui| {
            ui.heading("FT4");
            ui.separator();
            let phase_label = if snapshot.ptt_on {
                "TX NOW"
            } else if tx_active {
                "TX QUEUED"
            } else if self.ft4_autoseq {
                "AUTO TX ARMED"
            } else {
                "RX · DISARMED"
            };
            ui.label(
                RichText::new(phase_label)
                    .strong()
                    .color(if snapshot.ptt_on || tx_active {
                        Color32::from_rgb(210, 90, 60)
                    } else if self.ft4_autoseq {
                        Color32::from_rgb(255, 170, 75)
                    } else {
                        Color32::from_rgb(105, 190, 225)
                    }),
            );
            ui.add(
                egui::ProgressBar::new(progress as f32)
                    .desired_width(150.0)
                    .text(format!("{:.1}s", FT4_SLOT_SECONDS * (1.0 - progress))),
            );
            if let Some(hz) = snapshot.frequency_hz {
                ui.label(
                    RichText::new(format!("{:.3} MHz", hz as f64 / 1_000_000.0))
                        .monospace()
                        .strong(),
                );
            }
            ui.label(RichText::new(&snapshot.mode).monospace());
            ui.separator();
            ui.label(
                RichText::new(format!("RX {} Hz", self.rx_tone_hz))
                    .monospace()
                    .color(Color32::from_rgb(120, 220, 120)),
            );
            ui.label(
                RichText::new(format!("TX {} Hz", self.tx_tone_hz))
                    .monospace()
                    .color(Color32::from_rgb(220, 160, 80)),
            );
            ui.separator();
            let seq_label = self
                .ft4_session
                .as_ref()
                .map(|session| qso_stage_label(session.stage))
                .unwrap_or(if self.ft4_autoseq { "ARMED" } else { "IDLE" });
            ui.label(
                RichText::new(format!("SEQ {seq_label}"))
                    .monospace()
                    .color(Color32::LIGHT_BLUE),
            );

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let halt_label = if self.ft4_halt_after_tx {
                    "STOP AFTER THIS TX"
                } else {
                    "STOP AFTER NEXT TX"
                };
                if ui
                    .button(RichText::new(halt_label).color(if self.ft4_halt_after_tx {
                        Color32::from_rgb(220, 180, 80)
                    } else {
                        Color32::GRAY
                    }))
                    .on_hover_text("Pause FT4 automatic transmissions after the next TX completes")
                    .clicked()
                {
                    self.ft4_halt_after_tx = !self.ft4_halt_after_tx;
                    self.digital_tx_status = if self.ft4_halt_after_tx {
                        "🔒 FT4 will pause after the next transmission".to_string()
                    } else {
                        "Stop-after-TX canceled".to_string()
                    };
                }

                let hold_label = if self.ft8_hold_tx_freq {
                    "HOLD TX FREQ"
                } else {
                    "TX TRACKS RX"
                };
                if ui
                    .button(RichText::new(hold_label).color(if self.ft8_hold_tx_freq {
                        Color32::from_rgb(120, 200, 220)
                    } else {
                        Color32::from_rgb(120, 220, 120)
                    }))
                    .clicked()
                {
                    self.ft8_hold_tx_freq = !self.ft8_hold_tx_freq;
                    if !self.ft8_hold_tx_freq {
                        self.tx_tone_hz = self.rx_tone_hz;
                    }
                    self.profile_dirty = true;
                    self.persist_profile("Auto-saved");
                }

                let deep_label = if self.ft4_deep_decode {
                    "DECODE: DEEP"
                } else {
                    "DECODE: FAST"
                };
                if ui
                    .button(RichText::new(deep_label).color(if self.ft4_deep_decode {
                        Color32::YELLOW
                    } else {
                        Color32::LIGHT_GREEN
                    }))
                    .clicked()
                {
                    self.ft4_deep_decode = !self.ft4_deep_decode;
                    self.profile_dirty = true;
                    self.persist_profile("Auto-saved");
                }
            });
        });
        ui.horizontal_wrapped(|ui| {
            let (auto_label, auto_fill, auto_stroke) = if self.ft4_autoseq {
                (
                    "🔥 FT4 TX ARMED · CLICK TO DISARM ALL",
                    Color32::from_rgb(92, 43, 25),
                    Color32::from_rgb(255, 151, 72),
                )
            } else {
                (
                    "🔒 FT4 AUTO DISARMED · CLICK TO ARM",
                    Color32::from_rgb(28, 52, 70),
                    Color32::from_rgb(92, 174, 220),
                )
            };
            if ui
                .add(
                    egui::Button::new(RichText::new(auto_label).strong().color(Color32::WHITE))
                        .fill(auto_fill)
                        .stroke(egui::Stroke::new(1.5, auto_stroke)),
                )
                .clicked()
            {
                if self.ft4_autoseq {
                    self.disarm_all_tx("All TX stopped and disarmed by operator");
                } else {
                    self.ft4_autoseq = true;
                    self.digital_tx_status =
                        "FT4 automatic operation armed; waiting for an exchange".to_string();
                    self.profile_dirty = true;
                    self.persist_profile("FT4 TX armed");
                }
            }
            ui.label("Select caller:");
            let previous_policy = self.ft4_auto_reply_policy;
            egui::ComboBox::from_id_salt("ft4_auto_reply_policy")
                .selected_text(self.ft4_auto_reply_policy.label())
                .show_ui(ui, |ui| {
                    for policy in AutoReplyPolicy::ALL {
                        ui.selectable_value(
                            &mut self.ft4_auto_reply_policy,
                            policy,
                            policy.label(),
                        );
                    }
                });
            if self.ft4_auto_reply_policy != previous_policy {
                self.profile_dirty = true;
                self.persist_profile("Auto-saved");
            }
            if ui
                .checkbox(&mut self.ft8_auto_answer_cq, "Answer unattended CQs")
                .on_hover_text("Shared FT8/FT4 policy; only active while this mode is armed")
                .changed()
            {
                self.profile_dirty = true;
                self.persist_profile("Auto-saved");
            }
            ui.separator();
            ui.label(
                RichText::new(&snapshot.digital_decode_status)
                    .small()
                    .color(Color32::GRAY),
            );
            if let Some(offset) = snapshot.ft4_clock_offset_s {
                ui.label(
                    RichText::new(format!("Adaptive clock dT {offset:+.2}s"))
                        .small()
                        .color(if offset.abs() > 0.5 {
                            Color32::YELLOW
                        } else {
                            Color32::LIGHT_GREEN
                        }),
                );
            }
            if let Some(level) = snapshot.audio_level_dbfs {
                ui.label(
                    RichText::new(format!(
                        "Input {level:.0} dBFS / clip {:.1}%",
                        snapshot.audio_clip_percent
                    ))
                    .small()
                    .color(Color32::GRAY),
                );
            }
        });
        ui.separator();

        let stats = digital_activity_stats(&snapshot.digital_decodes, WorkspaceMode::Ft4);
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new("📊 BAND PULSE")
                    .strong()
                    .color(Color32::LIGHT_BLUE),
            );
            ft8_stat_chip(
                ui,
                "This cycle",
                stats.latest_cycle.to_string(),
                format!("{} CQ", stats.cq_this_cycle),
            );
            ft8_stat_chip(
                ui,
                "Average",
                format!("{:.1}/cycle", stats.average_per_cycle),
                "rolling log".to_string(),
            );
            ft8_stat_chip(
                ui,
                "Stations",
                stats.unique_stations.to_string(),
                "unique heard".to_string(),
            );
            let (heard, detail) = stats
                .most_heard
                .map(|(call, count)| (call, format!("{count} decodes")))
                .unwrap_or_else(|| ("—".to_string(), "waiting".to_string()));
            ft8_stat_chip(ui, "Most heard", heard, detail);
            ft8_stat_chip(
                ui,
                "Median SNR",
                stats
                    .median_snr
                    .map(|snr| format!("{snr:+} dB"))
                    .unwrap_or_else(|| "—".to_string()),
                "rolling log".to_string(),
            );
        });
        ui.add_space(4.0);

        let panel_h = ui.available_height();
        let decode_h = (panel_h * 0.38).max(170.0);
        let conversation_h = (panel_h * 0.28).clamp(150.0, 260.0);
        let tx_h = (panel_h * 0.20).max(120.0);
        let mut entries: Vec<DigitalDecodeEntry> = snapshot
            .digital_decodes
            .iter()
            .filter(|entry| {
                entry.mode == WorkspaceMode::Ft4
                    && (!self.ft4_cq_only_view || entry.message.starts_with("CQ "))
            })
            .cloned()
            .collect();
        if entries.len() > self.ft4_max_log_entries {
            entries.drain(..entries.len() - self.ft4_max_log_entries);
        }
        let operator_call = self.station_callsign_or_default().to_string();

        if let Some((entry, hit)) = entries.iter().rev().find_map(|entry| {
            operator_call_hit(&entry.message, &operator_call).map(|hit| (entry, hit))
        }) {
            draw_operator_call_banner(ui, "FT4", &operator_call, &entry.message, hit);
            ui.add_space(4.0);
        }

        egui::Frame::dark_canvas(ui.style()).show(ui, |ui| {
            ui.set_min_height(decode_h);
            ui.set_max_height(decode_h);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("⚡ FT4 LIVE DECODES")
                        .strong()
                        .color(Color32::LIGHT_BLUE),
                );
                ui.separator();
                if ui.checkbox(&mut self.ft4_cq_only_view, "CQ only").changed() {
                    self.profile_dirty = true;
                    self.persist_profile("Auto-saved");
                }
                if ui.checkbox(&mut self.ft4_follow_log, "Follow").changed() {
                    self.profile_dirty = true;
                    self.persist_profile("Auto-saved");
                }
                ui.label("Keep");
                if ui
                    .add(
                        egui::DragValue::new(&mut self.ft4_max_log_entries)
                            .range(80..=300)
                            .speed(5),
                    )
                    .changed()
                {
                    self.profile_dirty = true;
                    self.persist_profile("Auto-saved");
                }
                ui.label("rows");
                ui.label(format!("{} msgs", entries.len()));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("Clear").clicked() {
                        self.state
                            .lock()
                            .expect("ui state lock poisoned")
                            .digital_decodes
                            .retain(|entry| entry.mode != WorkspaceMode::Ft4);
                        self.digital_selected = None;
                        self.digital_seq_target = None;
                        self.ft4_session = None;
                        self.digital_tx_chat
                            .retain(|entry| entry.mode != WorkspaceMode::Ft4);
                    }
                });
            });
            ui.separator();
            ui.label(
                RichText::new("UTC          SNR     dT      Hz  Message")
                    .monospace()
                    .strong(),
            );
            ui.separator();
            egui::ScrollArea::vertical()
                .id_salt("ft4_global_decodes")
                .stick_to_bottom(self.ft4_follow_log)
                .show(ui, |ui| {
                    if entries.is_empty() {
                        ui.label(
                            RichText::new("🌊 Listening hard… collecting the next FT4 waveform.")
                                .color(Color32::GRAY),
                        );
                    }
                    for entry in &entries {
                        let selected = self.digital_selected.as_ref().is_some_and(|selected| {
                            selected.period == entry.period
                                && selected.freq_hz == entry.freq_hz
                                && selected.message == entry.message
                        });
                        let call_hit = operator_call_hit(&entry.message, &operator_call);
                        let row = RichText::new(format!(
                            "{:12}  {:+5.1}  {:+6.2}  {:>5}  {}",
                            entry.utc, entry.snr_db, entry.dt_s, entry.freq_hz, entry.message
                        ))
                        .monospace()
                        .color(if let Some(hit) = call_hit {
                            call_hit_badge(hit).1
                        } else if entry.message.starts_with("CQ ") {
                            Color32::LIGHT_GREEN
                        } else {
                            Color32::LIGHT_GRAY
                        });
                        let response = if let Some(hit) = call_hit {
                            let (badge, accent, fill) = call_hit_badge(hit);
                            egui::Frame::group(ui.style())
                                .fill(fill)
                                .stroke(egui::Stroke::new(1.5, accent))
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(RichText::new(badge).strong().color(accent));
                                        ui.selectable_label(selected, row)
                                    })
                                    .inner
                                })
                                .inner
                        } else {
                            ui.selectable_label(selected, row)
                        };
                        if response.clicked() {
                            self.digital_selected = Some(entry.clone());
                            self.digital_seq_target =
                                parse_message(&entry.message).map(|message| message.from);
                            self.rx_tone_hz = entry.freq_hz;
                            if !self.ft8_hold_tx_freq {
                                self.tx_tone_hz = entry.freq_hz;
                            }
                        }
                        if response.double_clicked() {
                            if let Some(message) = parse_message(&entry.message) {
                                let my_call = self.station_callsign_or_default().to_string();
                                let my_grid = self.station_grid_or_default().to_string();
                                let mut session =
                                    Ft8Session::start(message.from.clone(), entry.period);
                                if let Some(reply) = session.response_to(
                                    &message,
                                    &my_call,
                                    &my_grid,
                                    entry.snr_db.round() as i8,
                                    entry.period,
                                ) {
                                    self.digital_compose = reply;
                                    self.digital_seq_target = Some(message.from);
                                    self.ft4_session = Some(session);
                                    self.ft4_autoseq = true;
                                    self.queue_native_digital_tx(WorkspaceMode::Ft4);
                                    self.profile_dirty = true;
                                    self.persist_profile("Auto-saved");
                                }
                            }
                        }
                    }
                });
        });
        ui.add_space(4.0);
        self.draw_ft4_conversation(ui, snapshot, conversation_h);
        ui.add_space(4.0);

        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_min_height(tx_h);
            ui.horizontal(|ui| {
                ui.label(RichText::new("📣 FT4 TX DECK").strong());
                ui.add_enabled(
                    !tx_active,
                    egui::TextEdit::singleline(&mut self.digital_compose)
                        .desired_width((ui.available_width() - 260.0).max(180.0))
                        .hint_text("CQ W1AW FN20")
                        .font(egui::TextStyle::Monospace),
                );
                if ui.button("CALL CQ").clicked() {
                    self.digital_compose = format!(
                        "CQ {} {}",
                        self.station_callsign_or_default(),
                        self.station_grid_or_default()
                    );
                    self.digital_seq_target = None;
                    self.ft4_session = None;
                    self.ft4_autoseq = true;
                    self.queue_native_digital_tx(WorkspaceMode::Ft4);
                }
                if ui
                    .add_enabled(
                        !tx_active && !self.digital_compose.trim().is_empty(),
                        egui::Button::new("SEND NEXT SLOT"),
                    )
                    .clicked()
                {
                    self.queue_native_digital_tx(WorkspaceMode::Ft4);
                }
                if ui
                    .add(
                        egui::Button::new(
                            RichText::new("⛔ STOP + DISARM ALL")
                                .strong()
                                .color(Color32::WHITE),
                        )
                        .fill(Color32::from_rgb(112, 30, 38))
                        .stroke(egui::Stroke::new(1.5, Color32::from_rgb(255, 100, 110))),
                    )
                    .on_hover_text("Drop PTT and permanently cancel FT8/FT4 automatic TX until re-armed")
                    .clicked()
                {
                    self.disarm_all_tx("All TX stopped and disarmed by operator");
                }
            });
            ui.horizontal_wrapped(|ui| {
                if let Some(target) = self.digital_seq_target.clone() {
                    let my = self.station_callsign_or_default().to_string();
                    let grid = self.station_grid_or_default().to_string();
                    for (label, exchange) in
                        [("Grid", grid.as_str()), ("RR73", "RR73"), ("73", "73")]
                    {
                        if ui.small_button(label).clicked() {
                            self.digital_compose = format!("{target} {my} {exchange}");
                        }
                    }
                }
                ui.label(RichText::new(&self.digital_tx_status).small().color(Color32::GRAY));
            });
            ui.horizontal(|ui| {
                ui.label("PTT lead");
                if ui
                    .add(
                        egui::DragValue::new(&mut self.ptt_lead_ms)
                            .range(100..=1500)
                            .suffix(" ms"),
                    )
                    .changed()
                {
                    self.profile_dirty = true;
                    self.persist_profile("Auto-saved");
                }
                ui.label("tail");
                if ui
                    .add(
                        egui::DragValue::new(&mut self.ptt_tail_ms)
                            .range(0..=1000)
                            .suffix(" ms"),
                    )
                    .changed()
                {
                    self.profile_dirty = true;
                    self.persist_profile("Auto-saved");
                }
            });
        });
    }

    fn draw_mfsk_mode_workspace(
        &mut self,
        ui: &mut egui::Ui,
        snapshot: &GuiState,
        mode: WorkspaceMode,
    ) {
        let preset = workspace_radio_preset(mode);
        let preset_label = match (preset.base_mode, preset.data_mode) {
            (BaseMode::Usb, true) => "USB-D",
            (BaseMode::Usb, false) => "USB",
            (BaseMode::Lsb, true) => "LSB-D",
            (BaseMode::Lsb, false) => "LSB",
            (BaseMode::Cw | BaseMode::CwR, _) => "CW",
            (BaseMode::Rtty | BaseMode::RttyR, true) => "RTTY-D",
            (BaseMode::Rtty | BaseMode::RttyR, false) => "RTTY",
            _ => "DIGITAL",
        };
        let slot_s = mode.core_slot_seconds().map_or_else(
            || "Continuous".to_string(),
            |seconds| {
                if seconds.fract() == 0.0 {
                    format!("{seconds:.0} s")
                } else {
                    format!("{seconds:.1} s")
                }
            },
        );

        ui.heading(mode.label());
        ui.separator();

        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Backend:").strong());
            let backend = if mode.has_native_decoder() {
                "mfsk-core"
            } else if mode == WorkspaceMode::Fldigi {
                "external FLDIGI bridge"
            } else {
                "CW backend pending"
            };
            ui.label(
                RichText::new(backend)
                    .monospace()
                    .color(if mode.has_native_decoder() {
                        Color32::LIGHT_GREEN
                    } else {
                        Color32::YELLOW
                    }),
            );
            ui.separator();
            ui.label(format!("Slot: {slot_s}"));
            ui.separator();
            ui.label(format!("Radio preset: {preset_label} FIL{}", preset.filter));
            ui.separator();
            ui.label(format!(
                "Radio: {:.3} MHz  {}",
                snapshot.frequency_hz.unwrap_or_default() as f64 / 1_000_000.0,
                snapshot.mode
            ));
        });

        ui.add_space(8.0);
        if let Some(slot_seconds) = mode.core_slot_seconds() {
            let now_s = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_secs_f64())
                .unwrap_or(0.0);
            let progress = (now_s % slot_seconds) / slot_seconds;
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(&snapshot.digital_decode_status)
                        .monospace()
                        .color(Color32::LIGHT_GREEN),
                );
                ui.add(
                    egui::ProgressBar::new(progress as f32)
                        .desired_width(180.0)
                        .text(format!("{:.1}s", slot_seconds * (1.0 - progress))),
                );
            });

            ui.separator();
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("UTC          SNR     dT      Hz  Message")
                        .monospace()
                        .strong(),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("Clear").clicked() {
                        self.state
                            .lock()
                            .expect("ui state lock poisoned")
                            .digital_decodes
                            .retain(|entry| entry.mode != mode);
                    }
                });
            });
            ui.separator();
            egui::ScrollArea::vertical()
                .id_salt(("digital-decodes", mode.label()))
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    let mut shown = 0usize;
                    for entry in snapshot
                        .digital_decodes
                        .iter()
                        .filter(|entry| entry.mode == mode)
                    {
                        shown += 1;
                        ui.label(
                            RichText::new(format!(
                                "{:12}  {:+5.1}  {:+6.2}  {:>5}  {}",
                                entry.utc, entry.snr_db, entry.dt_s, entry.freq_hz, entry.message
                            ))
                            .monospace(),
                        )
                        .on_hover_text(format!("decode period {}", entry.period));
                    }
                    if shown == 0 {
                        ui.label(
                            RichText::new(format!(
                                "Collecting the first complete {} slot",
                                mode.label()
                            ))
                            .color(Color32::GRAY),
                        );
                    }
                });

            ui.separator();
            let can_transmit = matches!(
                mode,
                WorkspaceMode::Ft4
                    | WorkspaceMode::Fst4
                    | WorkspaceMode::Jt9
                    | WorkspaceMode::Jt65
                    | WorkspaceMode::Q65
            );
            ui.horizontal(|ui| {
                ui.label(RichText::new("TX").strong());
                ui.add_enabled(
                    can_transmit && !self.digital_tx_active.load(Ordering::Acquire),
                    egui::TextEdit::singleline(&mut self.digital_compose)
                        .desired_width((ui.available_width() - 250.0).max(180.0))
                        .hint_text("CQ W1AW FN20")
                        .font(egui::TextStyle::Monospace),
                );
                if ui
                    .add_enabled(can_transmit, egui::Button::new("CQ"))
                    .clicked()
                {
                    self.digital_compose = format!(
                        "CQ {} {}",
                        self.station_callsign_or_default(),
                        self.station_grid_or_default()
                    );
                }
                if ui
                    .add_enabled(
                        can_transmit
                            && !self.digital_compose.trim().is_empty()
                            && !self.digital_tx_active.load(Ordering::Acquire),
                        egui::Button::new("SEND NEXT SLOT"),
                    )
                    .clicked()
                {
                    self.queue_native_digital_tx(mode);
                }
                if ui
                    .add_enabled(
                        self.digital_tx_active.load(Ordering::Acquire),
                        egui::Button::new("STOP TX"),
                    )
                    .clicked()
                {
                    self.stop_native_digital_tx();
                }
            });
            ui.label(
                RichText::new(if can_transmit {
                    self.digital_tx_status.as_str()
                } else if mode == WorkspaceMode::Wspr {
                    "WSPR transmit setup requires callsign, locator, power, and beacon duty-cycle controls"
                } else {
                    "MSK144 receive is active; transmit framing is not yet exposed by the core audio API"
                })
                .color(if can_transmit {
                    Color32::GRAY
                } else {
                    Color32::YELLOW
                }),
            );
        } else {
            ui.separator();
            let status = if mode == WorkspaceMode::Cw {
                "CW is currently a radio preset and waterfall view. A decoder, keyer timing engine, sidetone controls, and macro workflow are still required."
            } else {
                "FLDIGI is currently a radio preset and waterfall view. No XML-RPC modem connection is active yet."
            };
            ui.label(RichText::new(status).color(Color32::YELLOW));
        }
    }

    fn draw_tx_safety_card(&mut self, ui: &mut egui::Ui, snapshot: &GuiState) {
        let armed = self.any_tx_armed(snapshot);
        let (fill, border, status, detail) = if armed {
            (
                Color32::from_rgb(73, 35, 24),
                Color32::from_rgb(255, 137, 61),
                "🔥 TRANSMIT ARMED",
                "FT8/FT4 automation, queued audio, or PTT can transmit",
            )
        } else {
            (
                Color32::from_rgb(22, 48, 59),
                Color32::from_rgb(77, 184, 211),
                "🔒 ALL TX DISARMED",
                "Safe state · arm from the FT8 or FT4 workspace",
            )
        };

        egui::Frame::group(ui.style())
            .fill(fill)
            .stroke(egui::Stroke::new(2.0, border))
            .show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new(status).strong().size(17.0).color(Color32::WHITE));
                    ui.label(RichText::new(detail).small().color(Color32::LIGHT_GRAY));
                    ui.add_space(3.0);
                    if ui
                        .add_sized(
                            [ui.available_width(), 34.0],
                            egui::Button::new(
                                RichText::new("⛔ STOP + DISARM ALL TX")
                                    .strong()
                                    .color(Color32::WHITE),
                            )
                            .fill(Color32::from_rgb(126, 25, 39))
                            .stroke(egui::Stroke::new(
                                1.5,
                                Color32::from_rgb(255, 105, 115),
                            )),
                        )
                        .on_hover_text(
                            "Drop PTT, abort queued/active audio, and disarm every automatic sequence",
                        )
                        .clicked()
                    {
                        self.disarm_all_tx("All TX stopped and disarmed by global safety control");
                    }
                });
            });
    }

    fn draw_workspace(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, snapshot: &GuiState) {
        ui.horizontal_wrapped(|ui| {
            for mode in WORKSPACE_MODES {
                ui.selectable_value(&mut self.workspace_mode, mode, mode.label());
            }
        });
        ui.separator();

        match self.workspace_mode {
            WorkspaceMode::Ft8 => self.draw_ft8_workspace(ui, ctx, snapshot),
            WorkspaceMode::Ft4 => self.draw_ft4_workspace(ui, snapshot),
            WorkspaceMode::Fst4
            | WorkspaceMode::Wspr
            | WorkspaceMode::Jt9
            | WorkspaceMode::Jt65
            | WorkspaceMode::Q65
            | WorkspaceMode::Msk144
            | WorkspaceMode::Cw
            | WorkspaceMode::Fldigi => {
                self.draw_mfsk_mode_workspace(ui, snapshot, self.workspace_mode)
            }
        }
    }

    fn draw_radio_control_strip(&mut self, ui: &mut egui::Ui, snapshot: &GuiState) {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Radio").strong());

            if ui.small_button("-1 kHz").clicked() {
                self.send_command(GuiCommand::TuneDelta(-1_000));
            }
            if ui.small_button("+1 kHz").clicked() {
                self.send_command(GuiCommand::TuneDelta(1_000));
            }
            if ui.small_button("Mode").clicked() {
                self.send_command(GuiCommand::CycleMode);
            }
            if ui
                .small_button(if snapshot.ptt_on { "PTT OFF" } else { "PTT ON" })
                .clicked()
            {
                if snapshot.ptt_on
                    || self.ft8_tx_active.load(Ordering::Acquire)
                    || self.digital_tx_active.load(Ordering::Acquire)
                {
                    self.disarm_all_tx("TX/PTT stopped and all modes disarmed");
                } else {
                    self.send_command(GuiCommand::TogglePtt);
                }
            }
            if ui.small_button("AF-").clicked() {
                self.send_command(GuiCommand::AfGainDelta(-5));
            }
            if ui.small_button("AF+").clicked() {
                self.send_command(GuiCommand::AfGainDelta(5));
            }

            ui.separator();

            let mut tuning = self.display_tuning.lock().expect("tuning lock poisoned");
            let auto_clicked = ui
                .selectable_label(tuning.auto_visual, "AUTO")
                .on_hover_text("Auto-select waterfall speed and audio detail for current mode")
                .clicked();
            if auto_clicked {
                tuning.auto_visual = !tuning.auto_visual;
            }

            ui.label("WF");
            if ui
                .selectable_label(
                    !tuning.auto_visual && tuning.waterfall_speed == WaterfallSpeed::Slow,
                    "Slow",
                )
                .clicked()
            {
                tuning.auto_visual = false;
                tuning.waterfall_speed = WaterfallSpeed::Slow;
            }
            if ui
                .selectable_label(
                    !tuning.auto_visual && tuning.waterfall_speed == WaterfallSpeed::Mid,
                    "Mid",
                )
                .clicked()
            {
                tuning.auto_visual = false;
                tuning.waterfall_speed = WaterfallSpeed::Mid;
            }
            if ui
                .selectable_label(
                    !tuning.auto_visual && tuning.waterfall_speed == WaterfallSpeed::Fast,
                    "Fast",
                )
                .clicked()
            {
                tuning.auto_visual = false;
                tuning.waterfall_speed = WaterfallSpeed::Fast;
            }

            drop(tuning);

            ui.separator();
            ui.label(format!(
                "{} | {} | {}",
                snapshot.mode,
                snapshot
                    .frequency_hz
                    .map(|v| format!("{v} Hz"))
                    .unwrap_or_else(|| "freq ?".to_string()),
                if !snapshot.radio_spectrum_desired {
                    "WF OFF"
                } else if snapshot.radio_spectrum_enabled {
                    "WF LIVE"
                } else {
                    "WF ARMING"
                },
            ));
        });
    }
}

impl eframe::App for RigforgeGuiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Zoom is layered on top of the OS DPI scale, so text, controls,
        // spacing, hit targets, and custom drawings stay in proportion.
        if (ctx.zoom_factor() - self.gui_scale).abs() > 0.001 {
            ctx.set_zoom_factor(self.gui_scale);
        }
        // Give background workers a handle so they can trigger repaints directly.
        let _ = self.repaint_ctx.get_or_init(|| ctx.clone());
        // Safety-net repaint in case no worker data arrives for a long time.
        ctx.request_repaint_after(Duration::from_secs(1));

        // Drain FT8 decodes from the shared pending queue into app-local log.
        let (new_decodes, latest_decode_period) = {
            let mut s = self.state.lock().expect("ui state lock poisoned");
            s.workspace_mode = self.workspace_mode;
            s.ft8_deep_decode = self.ft8_deep_decode;
            s.ft4_deep_decode = self.ft4_deep_decode;
            s.selected_audio_hz = self.rx_tone_hz;
            s.compute_backend = self.acceleration_report.active;
            s.radio_spectrum_desired = self.civ_spectrum_on;
            (
                s.ft8_pending.drain(..).collect::<Vec<_>>(),
                s.ft8_last_decode_period,
            )
        };
        let completed_decode_period =
            latest_decode_period.filter(|period| self.ft8_seen_decode_period != Some(*period));
        if completed_decode_period.is_some() {
            self.ft8_seen_decode_period = completed_decode_period;
        }
        self.process_ft8_tx_pipeline();
        self.process_native_digital_tx_pipeline();
        let (ft4_decodes, latest_ft4_period) = {
            let shared = self.state.lock().expect("ui state lock poisoned");
            (
                shared
                    .digital_decodes
                    .iter()
                    .filter(|entry| entry.mode == WorkspaceMode::Ft4)
                    .cloned()
                    .collect::<Vec<_>>(),
                shared.ft4_last_decode_period,
            )
        };
        let completed_ft4_period = latest_ft4_period
            .filter(|period| self.ft4_seen_decode_period != Some(*period));
        if completed_ft4_period.is_some() {
            self.ft4_seen_decode_period = completed_ft4_period;
        }
        self.handle_ft4_decodes(&ft4_decodes, completed_ft4_period);
        self.handle_ft8_decodes(&new_decodes, completed_decode_period);
        self.ft8_log.extend(new_decodes);
        // Keep the log bounded.
        let max_entries = self.ft8_max_log_entries.max(80);
        if self.ft8_log.len() > max_entries {
            let removed = self.ft8_log.len() - max_entries;
            self.ft8_log.drain(..removed);
            if let Some(sel) = self.ft8_selected {
                self.ft8_selected = sel.checked_sub(removed);
            }
        }

        let snapshot = self.state.lock().expect("ui state lock poisoned").clone();

        egui::TopBottomPanel::top("header")
            .resizable(true)
            .min_height(72.0)
            .max_height(160.0)
            .show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.heading("⚡ RigForge");
                ui.separator();
                let frequency = snapshot
                    .frequency_hz
                    .map(|hz| format!("{:.6} MHz", hz as f64 / 1_000_000.0))
                    .unwrap_or_else(|| "RADIO OFFLINE".to_string());
                ui.label(
                    RichText::new(frequency)
                        .monospace()
                        .strong()
                        .size(19.0)
                        .color(if snapshot.frequency_hz.is_some() {
                            Color32::from_rgb(120, 225, 255)
                        } else {
                            Color32::YELLOW
                        }),
                );
                if let Some(hz) = snapshot.frequency_hz {
                    ui.label(
                        RichText::new(band_for_frequency(hz))
                            .strong()
                            .color(Color32::from_rgb(220, 190, 100)),
                    );
                }
                ui.separator();
                ui.label(
                    RichText::new(&snapshot.mode)
                        .monospace()
                        .strong()
                        .color(Color32::WHITE),
                );
                ui.label(
                    RichText::new(
                        snapshot
                            .filter
                            .map(|filter| format!("FIL{filter}"))
                            .unwrap_or_else(|| "FIL?".to_string()),
                    )
                    .monospace()
                    .color(Color32::GRAY),
                );
                ui.label(
                    RichText::new(self.workspace_mode.label())
                        .strong()
                        .color(Color32::LIGHT_BLUE),
                );
                ui.separator();
                ui.label(
                    RichText::new(format!("RX {} · TX {} Hz", self.rx_tone_hz, self.tx_tone_hz))
                        .monospace()
                        .color(Color32::from_rgb(135, 220, 145)),
                );
                let armed = self.any_tx_armed(&snapshot);
                ui.label(
                    RichText::new(if snapshot.ptt_on {
                        "🔥 ON AIR"
                    } else if armed {
                        "⚠ TX ARMED"
                    } else {
                        "🔒 TX SAFE"
                    })
                    .strong()
                    .color(if snapshot.ptt_on {
                        Color32::from_rgb(255, 95, 85)
                    } else if armed {
                        Color32::from_rgb(255, 170, 75)
                    } else {
                        Color32::from_rgb(100, 205, 225)
                    }),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!(
                            "📍 {} · {}",
                            self.station_callsign_or_default(),
                            self.station_grid_or_default()
                        ))
                        .strong()
                        .color(Color32::from_rgb(255, 210, 110)),
                    );
                });
            });
            ui.separator();
            ui.horizontal_wrapped(|ui| {
                if ui
                    .selectable_label(self.show_signal_panel, "Signals")
                    .on_hover_text("Show or hide the resizable monitoring panel")
                    .clicked()
                {
                    self.show_signal_panel = !self.show_signal_panel;
                }
                if ui
                    .selectable_label(self.show_device_settings, "Devices")
                    .on_hover_text("Choose OS audio and USB/serial devices")
                    .clicked()
                {
                    self.show_device_settings = !self.show_device_settings;
                    self.show_signal_panel = true;
                }
                ui.separator();
                let previous_scale = self.gui_scale;
                egui::ComboBox::from_id_salt("gui_scale")
                    .selected_text(format!(
                        "UI {:.0}%",
                        self.gui_scale / GUI_SCALE_BASE * 100.0
                    ))
                    .width(82.0)
                    .show_ui(ui, |ui| {
                        for percent in [75_u32, 85, 100, 110, 125] {
                            let scale = GUI_SCALE_BASE * percent as f32 / 100.0;
                            ui.selectable_value(
                                &mut self.gui_scale,
                                scale,
                                format!("{percent}%"),
                            );
                        }
                    });
                if (previous_scale - self.gui_scale).abs() > 0.001 {
                    ctx.set_zoom_factor(self.gui_scale);
                    self.profile_dirty = true;
                    self.persist_profile("Auto-saved");
                }
                ui.separator();
                if ui.small_button("Save Profile").clicked() {
                    self.persist_profile("Saved");
                }
                if ui.small_button("Reload Profile").clicked() {
                    match load_operator_profile() {
                        Some(p) => {
                            self.station_callsign = p.callsign;
                            self.station_grid = p.grid;
                            self.station_qth = p.qth;
                            self.ft8_follow_log = p.follow_log;
                            self.ft8_max_log_entries = p.max_log_entries.clamp(80, 1000);
                            self.ft8_deep_decode = p.deep_decode;
                            self.ft4_deep_decode = p.ft4_deep_decode;
                            self.ft4_autoseq = p.ft4_autoseq;
                            self.ft4_auto_reply_policy = p.ft4_auto_reply_policy;
                            self.ft4_cq_only_view = p.ft4_cq_only_view;
                            self.ft4_follow_log = p.ft4_follow_log;
                            self.ft4_max_log_entries = p.ft4_max_log_entries.clamp(80, 300);
                            self.ft8_autoseq = p.autoseq;
                            self.ft8_auto_reply_policy = p.auto_reply_policy;
                            self.ft8_auto_answer_cq = p.auto_answer_cq;
                            self.ft8_cq_only_view = p.cq_only_view;
                            self.civ_spectrum_on = p.civ_spectrum_on;
                            self.ft8_halt_after_tx = false;
                            self.ft8_hold_tx_freq = if p.profile_version >= 3 {
                                p.hold_tx_freq
                            } else {
                                false
                            };
                            self.rx_tone_hz = p.rx_tone_hz;
                            self.tx_tone_hz = p.tx_tone_hz;
                            if !self.ft8_hold_tx_freq {
                                self.tx_tone_hz = self.rx_tone_hz;
                            }
                            self.ptt_lead_ms = p.ptt_lead_ms.clamp(100, 1_500);
                            self.ptt_tail_ms = p.ptt_tail_ms.clamp(0, 1_000);
                            self.gui_scale = if p.profile_version >= OPERATOR_PROFILE_VERSION {
                                p.gui_scale.clamp(GUI_SCALE_MIN, GUI_SCALE_MAX)
                            } else {
                                default_gui_scale()
                            };
                            self.compute_preference = p.compute_preference;
                            self.acceleration_report =
                                AccelerationReport::probe(self.compute_preference);
                            if p.profile_version >= 3 {
                                self.config.audio.input_device = p.audio_input_device;
                                self.config.audio.output_device = p.audio_output_device;
                                self.config.radio.serial_port = p.radio_serial_port;
                            }
                            self.config.station.callsign = Some(self.station_callsign.clone());
                            self.config.station.grid = Some(self.station_grid.clone());
                            self.profile_io_status = format!("Loaded {}", OPERATOR_PROFILE_FILE);
                            self.profile_dirty = false;
                        }
                        None => {
                            self.profile_io_status = format!("No {} found", OPERATOR_PROFILE_FILE);
                        }
                    }
                }
                let status_color = if self.profile_dirty {
                    Color32::YELLOW
                } else {
                    Color32::GRAY
                };
                ui.label(
                    RichText::new(&self.profile_io_status)
                        .small()
                        .color(status_color),
                );
            });
        });

        egui::TopBottomPanel::bottom("radio_strip")
            .resizable(true)
            .default_height(38.0)
            .height_range(32.0..=120.0)
            .show(ctx, |ui| {
                self.draw_radio_control_strip(ui, &snapshot);
            });

        if self.show_signal_panel {
            egui::SidePanel::left("signals")
                .resizable(true)
                .default_width(430.0)
                .min_width(300.0)
                .max_width(ctx.content_rect().width() * 0.72)
                .show(ctx, |ui| {
                    self.draw_tx_safety_card(ui, &snapshot);
                    ui.add_space(6.0);
                    egui::ScrollArea::vertical()
                        .id_salt("signals_scroll")
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            if self.show_device_settings {
                                ui.group(|ui| self.draw_device_settings(ui));
                                ui.add_space(4.0);
                            }
                            egui::CollapsingHeader::new("📡 Station health")
                                .default_open(true)
                                .show(ui, |ui| self.draw_status(ui, &snapshot));
                            egui::CollapsingHeader::new("Radio waterfall")
                                .default_open(true)
                                .show(ui, |ui| self.draw_radio_waterfall(ui, ctx, &snapshot));
                            egui::CollapsingHeader::new("Audio waterfall")
                                .default_open(true)
                                .show(ui, |ui| self.draw_audio_waterfall(ui, ctx, &snapshot));
                            egui::CollapsingHeader::new("Station profile")
                                .default_open(false)
                                .show(ui, |ui| self.draw_station_profile(ui));
                            egui::CollapsingHeader::new("Band controls")
                                .default_open(true)
                                .show(ui, |ui| self.draw_band_controls(ui, &snapshot));
                            egui::CollapsingHeader::new("Contact log")
                                .default_open(false)
                                .show(ui, |ui| self.draw_contact_log(ui, &snapshot));
                        });
                });
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::both()
                .id_salt("workspace_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| self.draw_workspace(ui, ctx, &snapshot));
        });
    }
}

impl Drop for RigforgeGuiApp {
    fn drop(&mut self) {
        self.force_stop_tx();
        self.stop_native_digital_tx();
        self.persist_profile("Saved on exit");
        if self.qso_log_dirty {
            self.persist_qso_log("Saved on exit");
        }
        self.worker_stop.store(true, Ordering::Relaxed);
        if let Some(tx) = &self.command_tx {
            let _ = tx.send(GuiCommand::Quit);
        }
        if let Some(handle) = self.radio_worker_handle.take() {
            let _ = handle.join();
        }
        if let Some(handle) = self.audio_worker_handle.take() {
            let _ = handle.join();
        }
    }
}

fn spawn_radio_worker(
    radio: IcomCiVRadio,
    state: Arc<Mutex<GuiState>>,
    stop: Arc<AtomicBool>,
    display_tuning: Arc<Mutex<DisplayTuning>>,
    rx: mpsc::Receiver<GuiCommand>,
    repaint_ctx: Arc<OnceLock<egui::Context>>,
) -> std::thread::JoinHandle<()> {
    thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(err) => {
                let mut s = state.lock().expect("ui state lock poisoned");
                s.last_error = Some(format!("failed to start GUI runtime: {err}"));
                return;
            }
        };

        // Shared cell: the frame reader writes here, the display ticker reads from here.
        let latest_scope: Arc<Mutex<Option<Vec<u8>>>> = Arc::new(Mutex::new(None));

        let stream_state = state.clone();
        let stream_radio = radio.clone();
        let stream_stop = stop.clone();
        let scope_writer = latest_scope.clone();
        let _stream_handle = thread::spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(err) => {
                    let mut s = stream_state.lock().expect("ui state lock poisoned");
                    s.last_error = Some(format!("failed to start streaming runtime: {err}"));
                    return;
                }
            };

            while !stream_stop.load(Ordering::Relaxed) {
                let (spectrum_desired, spectrum_enabled) = {
                    let s = stream_state.lock().expect("ui state lock poisoned");
                    (s.radio_spectrum_desired, s.radio_spectrum_enabled)
                };

                if !spectrum_desired {
                    if spectrum_enabled {
                        let _ = rt.block_on(stream_radio.disable_spectrum_stream());
                        let mut s = stream_state.lock().expect("ui state lock poisoned");
                        s.radio_spectrum_enabled = false;
                        s.radio_waterfall_status = "OFF".to_string();
                        s.radio_waterfall_rows.clear();
                        s.radio_waterfall_revision = s.radio_waterfall_revision.wrapping_add(1);
                    }
                    thread::sleep(Duration::from_millis(250));
                    continue;
                }

                if !spectrum_enabled {
                    match rt
                        .block_on(stream_radio.enable_spectrum_stream(Duration::from_millis(2_500)))
                    {
                        Ok(frame) => {
                            let mut s = stream_state.lock().expect("ui state lock poisoned");
                            s.radio_spectrum_enabled = true;
                            s.radio_waterfall_status = if frame.len() >= 6 && frame[4] == 0xFB {
                                "ARMED (ACK)".to_string()
                            } else {
                                "READY".to_string()
                            };
                            s.last_error = None;
                        }
                        Err(err) => {
                            let mut s = stream_state.lock().expect("ui state lock poisoned");
                            s.radio_spectrum_enabled = false;
                            s.radio_waterfall_status = "ENABLE RETRY".to_string();
                            s.last_error = Some(err.to_string());
                            thread::sleep(Duration::from_millis(1_000));
                        }
                    }
                    continue;
                }

                // 300 ms gives enough headroom for two-segment frame assembly.
                match rt.block_on(
                    stream_radio.try_scope_waveform_bins_stream(Duration::from_millis(300)),
                ) {
                    Ok(Some(bins)) if !bins.is_empty() => {
                        *scope_writer.lock().expect("scope lock") = Some(bins);
                    }
                    Err(err) => {
                        let msg = err.to_string();
                        let mut s = stream_state.lock().expect("ui state lock poisoned");
                        if is_transient_civ_read_error(&msg) {
                            s.radio_waterfall_status = "WAITING FRAME".to_string();
                        } else {
                            s.radio_spectrum_enabled = false;
                            s.radio_waterfall_status = "NO FRAME".to_string();
                            s.last_error = Some(msg);
                        }
                    }
                    _ => {}
                }
            }
        });

        // Display ticker: pushes one row at the target interval, matching the audio worker rate.
        let ticker_state = state.clone();
        let ticker_stop = stop.clone();
        let ticker_repaint = repaint_ctx.clone();
        let ticker_tuning = display_tuning.clone();
        let scope_reader = latest_scope;
        let _ticker_handle = thread::spawn(move || {
            while !ticker_stop.load(Ordering::Relaxed) {
                let interval_ms: u64 = {
                    let t = ticker_tuning.lock().expect("tuning lock poisoned");
                    let mode = ticker_state
                        .lock()
                        .expect("ui state lock poisoned")
                        .mode
                        .clone();
                    effective_visual_profile(&t, &mode).0
                };
                thread::sleep(Duration::from_millis(interval_ms));

                let bins = scope_reader.lock().expect("scope lock").clone();
                if let Some(bins) = bins {
                    let mut s = ticker_state.lock().expect("ui state lock poisoned");
                    apply_waterfall_bins(&mut s, &bins);
                    s.radio_waterfall_status = "READY".to_string();
                    drop(s);
                    if let Some(ctx) = ticker_repaint.get() {
                        ctx.request_repaint();
                    }
                }
            }
        });

        poll_radio_core_state(&rt, &radio, &state);

        while !stop.load(Ordering::Relaxed) {
            let cmd = match rx.recv() {
                Ok(cmd) => Some(cmd),
                Err(_) => break,
            };

            if let Some(cmd) = cmd {
                match cmd {
                    GuiCommand::Quit => return,
                    GuiCommand::TuneDelta(delta) => {
                        let freq = rt.block_on(radio.frequency()).ok();
                        if let Some(freq) = freq {
                            let target = if delta.is_negative() {
                                freq.saturating_sub(delta.unsigned_abs())
                            } else {
                                freq.saturating_add(delta as u64)
                            };
                            if let Err(err) = rt.block_on(radio.set_frequency(target)) {
                                let mut s = state.lock().expect("ui state lock poisoned");
                                s.last_error = Some(err.to_string());
                            }
                        }
                        poll_radio_core_state(&rt, &radio, &state);
                    }
                    GuiCommand::CycleMode => {
                        let current = rt.block_on(radio.mode()).unwrap_or(Mode::Usb);
                        let next = match current {
                            Mode::Usb => Mode::Lsb,
                            Mode::Lsb => Mode::Cw,
                            Mode::Cw => Mode::Data,
                            Mode::Data => Mode::Usb,
                        };
                        if let Err(err) = rt.block_on(Radio::set_mode(&radio, next)) {
                            let mut s = state.lock().expect("ui state lock poisoned");
                            s.last_error = Some(err.to_string());
                        }
                        poll_radio_core_state(&rt, &radio, &state);
                    }
                    GuiCommand::TogglePtt => {
                        let ptt_target = {
                            let s = state.lock().expect("ui state lock poisoned");
                            !s.ptt_on
                        };
                        let result = rt
                            .block_on(radio.set_ptt(ptt_target))
                            .map_err(|error| error.to_string());
                        let mut s = state.lock().expect("ui state lock poisoned");
                        match result {
                            Ok(()) => {
                                s.ptt_on = ptt_target;
                                s.last_error = None;
                            }
                            Err(error) => s.last_error = Some(error),
                        }
                        drop(s);
                        poll_radio_core_state(&rt, &radio, &state);
                    }
                    GuiCommand::SetPtt(target) => {
                        let result = rt
                            .block_on(radio.set_ptt(target))
                            .map_err(|error| error.to_string());
                        let mut s = state.lock().expect("ui state lock poisoned");
                        match result {
                            Ok(()) => {
                                s.ptt_on = target;
                                s.last_error = None;
                            }
                            Err(error) => s.last_error = Some(error),
                        }
                        drop(s);
                        poll_radio_core_state(&rt, &radio, &state);
                    }
                    GuiCommand::SetPttWithAck(target, ack_tx) => {
                        let result = rt
                            .block_on(radio.set_ptt(target))
                            .map_err(|error| error.to_string());
                        {
                            let mut s = state.lock().expect("ui state lock poisoned");
                            match &result {
                                Ok(()) => {
                                    s.ptt_on = target;
                                    s.last_error = None;
                                }
                                Err(error) => s.last_error = Some(error.clone()),
                            }
                        }
                        let _ = ack_tx.send(result);
                        poll_radio_core_state(&rt, &radio, &state);
                    }
                    GuiCommand::TuneWorkspaceBand(freq_hz) => {
                        let (workspace_mode, current_filter) = {
                            let s = state.lock().expect("ui state lock poisoned");
                            (s.workspace_mode, s.filter)
                        };
                        let preset = workspace_radio_preset(workspace_mode);
                        let filter_to_keep = current_filter.unwrap_or(preset.filter).clamp(1, 3);
                        let _ = rt.block_on(radio.set_frequency(freq_hz));
                        let _ = rt.block_on(radio.set_operating_mode_details(
                            preset.base_mode,
                            preset.data_mode,
                            filter_to_keep,
                        ));
                        poll_radio_core_state(&rt, &radio, &state);
                    }
                    GuiCommand::SetFilter(n) => {
                        let workspace_mode =
                            state.lock().expect("ui state lock poisoned").workspace_mode;
                        let preset = workspace_radio_preset(workspace_mode);
                        let target_filter = n.clamp(1, 3);
                        if let Err(err) = rt.block_on(radio.set_operating_mode_details(
                            preset.base_mode,
                            preset.data_mode,
                            target_filter,
                        )) {
                            let mut s = state.lock().expect("ui state lock poisoned");
                            s.last_error = Some(err.to_string());
                        }
                        poll_radio_core_state(&rt, &radio, &state);
                    }
                    GuiCommand::AfGainDelta(delta) => {
                        let current = match rt
                            .block_on(radio.get_control(ControlId::AfGain))
                            .ok()
                            .flatten()
                        {
                            Some(ControlValue::U8(v)) => v,
                            _ => 100,
                        };
                        let target = if delta.is_negative() {
                            current.saturating_sub(delta.unsigned_abs() as u8)
                        } else {
                            current.saturating_add(delta as u8).min(255)
                        };
                        if let Err(err) = rt.block_on(
                            radio.set_control(ControlId::AfGain, ControlValue::U8(target)),
                        ) {
                            let mut s = state.lock().expect("ui state lock poisoned");
                            s.last_error = Some(err.to_string());
                        }
                        poll_radio_core_state(&rt, &radio, &state);
                    }
                }
            }

            if let Ok(mut s) = state.lock() {
                let t = display_tuning.lock().expect("tuning lock poisoned").clone();
                let (_, sweep_code) = effective_visual_profile(&t, &s.mode);
                if let Err(err) = rt.block_on(radio.set_scope_sweep_speed(sweep_code)) {
                    s.last_error = Some(err.to_string());
                }
            }
        }
    })
}

fn poll_radio_core_state(
    rt: &tokio::runtime::Runtime,
    radio: &IcomCiVRadio,
    state: &Arc<Mutex<GuiState>>,
) {
    // Read what we need under a brief lock, then do all I/O outside it.
    let spectrum_enabled = state
        .lock()
        .expect("ui state lock poisoned")
        .radio_spectrum_enabled;
    let status_result = if spectrum_enabled {
        rt.block_on(radio.probe_stream_status())
    } else {
        radio.probe()
    };
    let af = read_u8_control(rt, radio, ControlId::AfGain);
    let rf = read_u8_control(rt, radio, ControlId::RfGain);
    let pwr = read_u8_control(rt, radio, ControlId::RfPower);
    let filt = read_u8_control(rt, radio, ControlId::Filter);

    let mut s = state.lock().expect("ui state lock poisoned");
    if let Ok(status) = status_result {
        if let Some(freq) = status.frequency_hz {
            s.frequency_hz = Some(freq);
        }
        if let Some(mode) = status.mode {
            s.mode = mode;
        }
        if let Some(details) = status.mode_details {
            s.data_mode = Some(details.data_mode);
            s.filter = details.filter;
        }
        s.last_update = Some(Instant::now());
    }
    if let Some(v) = af {
        s.af_gain = Some(v);
    }
    if let Some(v) = rf {
        s.rf_gain = Some(v);
    }
    if let Some(v) = pwr {
        s.rf_power = Some(v);
    }
    if let Some(v) = filt {
        s.filter = Some(v);
    }
}

fn apply_waterfall_bins(next: &mut GuiState, bins: &[u8]) {
    let row = downsample_bins(bins, RADIO_WF_WIDTH);
    if next.radio_waterfall_rows.len() >= RADIO_WF_HEIGHT {
        next.radio_waterfall_rows.pop_front();
    }
    next.radio_waterfall_rows.push_back(row);
    next.radio_waterfall_revision = next.radio_waterfall_revision.wrapping_add(1);
}

fn read_u8_control(
    rt: &tokio::runtime::Runtime,
    radio: &IcomCiVRadio,
    id: ControlId,
) -> Option<u8> {
    match rt.block_on(radio.get_control(id)).ok().flatten() {
        Some(ControlValue::U8(v)) => Some(v),
        _ => None,
    }
}

fn downsample_bins(bins: &[u8], width: usize) -> Vec<u8> {
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
        let mut out = Vec::with_capacity(width);
        for x in 0..width {
            let start = x * bins.len() / width;
            let end = ((x + 1) * bins.len() / width).max(start + 1);
            let mut peak = 0u8;
            for &v in &bins[start..end] {
                if v > peak {
                    peak = v;
                }
            }
            out.push(peak);
        }
        return out;
    }

    // If source is narrower than target, upsample with linear interpolation to reduce blockiness.
    let mut out = Vec::with_capacity(width);
    let src_last = (bins.len() - 1) as f32;
    let dst_last = (width - 1).max(1) as f32;
    for x in 0..width {
        let pos = (x as f32 / dst_last) * src_last;
        let i0 = pos.floor() as usize;
        let i1 = pos.ceil() as usize;
        if i0 == i1 {
            out.push(bins[i0]);
        } else {
            let t = pos - i0 as f32;
            let a = bins[i0] as f32;
            let b = bins[i1] as f32;
            out.push((a + (b - a) * t).round() as u8);
        }
    }
    out
}

fn ft8_activity_stats(log: &[Ft8DecodeEntry]) -> Ft8ActivityStats {
    let mut per_cycle: HashMap<u64, usize> = HashMap::new();
    let mut station_counts: HashMap<String, usize> = HashMap::new();
    let mut stations = HashSet::new();
    let mut snrs = Vec::with_capacity(log.len());

    for entry in log {
        *per_cycle.entry(entry.period).or_default() += 1;
        snrs.push(entry.snr_db);
        if let Some(message) = parse_message(&entry.message) {
            stations.insert(message.from.clone());
            *station_counts.entry(message.from).or_default() += 1;
        }
    }

    let latest_period = log.iter().map(|entry| entry.period).max();
    let latest_cycle = latest_period
        .and_then(|period| per_cycle.get(&period).copied())
        .unwrap_or_default();
    let cq_this_cycle = latest_period
        .map(|period| {
            log.iter()
                .filter(|entry| entry.period == period && entry.is_cq)
                .count()
        })
        .unwrap_or_default();
    let average_per_cycle = if per_cycle.is_empty() {
        0.0
    } else {
        log.len() as f32 / per_cycle.len() as f32
    };
    let most_heard = station_counts.into_iter().max_by(|left, right| {
        left.1
            .cmp(&right.1)
            .then_with(|| right.0.cmp(&left.0))
    });
    snrs.sort_unstable();
    let median_snr = snrs.get(snrs.len() / 2).copied();

    Ft8ActivityStats {
        latest_cycle,
        average_per_cycle,
        cq_this_cycle,
        unique_stations: stations.len(),
        most_heard,
        median_snr,
    }
}

fn digital_activity_stats(
    log: &VecDeque<DigitalDecodeEntry>,
    mode: WorkspaceMode,
) -> Ft8ActivityStats {
    let entries: Vec<&DigitalDecodeEntry> =
        log.iter().filter(|entry| entry.mode == mode).collect();
    let mut per_cycle: HashMap<u64, usize> = HashMap::new();
    let mut station_counts: HashMap<String, usize> = HashMap::new();
    let mut stations = HashSet::new();
    let mut snrs = Vec::with_capacity(entries.len());
    for entry in &entries {
        *per_cycle.entry(entry.period).or_default() += 1;
        snrs.push(entry.snr_db.round() as i8);
        if let Some(message) = parse_message(&entry.message) {
            stations.insert(message.from.clone());
            *station_counts.entry(message.from).or_default() += 1;
        }
    }
    let latest_period = entries.iter().map(|entry| entry.period).max();
    let latest_cycle = latest_period
        .and_then(|period| per_cycle.get(&period).copied())
        .unwrap_or_default();
    let cq_this_cycle = latest_period.map_or(0, |period| {
        entries
            .iter()
            .filter(|entry| entry.period == period && entry.message.starts_with("CQ "))
            .count()
    });
    let average_per_cycle = if per_cycle.is_empty() {
        0.0
    } else {
        entries.len() as f32 / per_cycle.len() as f32
    };
    let most_heard = station_counts.into_iter().max_by(|left, right| {
        left.1
            .cmp(&right.1)
            .then_with(|| right.0.cmp(&left.0))
    });
    snrs.sort_unstable();
    let median_snr = snrs.get(snrs.len() / 2).copied();
    Ft8ActivityStats {
        latest_cycle,
        average_per_cycle,
        cq_this_cycle,
        unique_stations: stations.len(),
        most_heard,
        median_snr,
    }
}

fn ft8_stat_chip(ui: &mut egui::Ui, label: &str, value: String, detail: String) {
    egui::Frame::group(ui.style())
        .fill(Color32::from_rgb(28, 32, 38))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(label).small().color(Color32::GRAY));
                ui.label(RichText::new(value).strong());
                ui.label(RichText::new(detail).small().color(Color32::DARK_GRAY));
            });
        });
}

fn operator_status_card(
    ui: &mut egui::Ui,
    label: &str,
    value: &str,
    detail: &str,
    accent: Color32,
) {
    egui::Frame::group(ui.style())
        .fill(Color32::from_rgb(24, 29, 36))
        .stroke(egui::Stroke::new(1.0, accent.gamma_multiply(0.7)))
        .show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(label).strong().color(Color32::LIGHT_GRAY));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(value).strong().monospace().color(accent));
                });
            });
            ui.label(RichText::new(detail).small().color(Color32::GRAY));
        });
    ui.add_space(3.0);
}

fn qso_stage_label(stage: QsoStage) -> &'static str {
    match stage {
        QsoStage::Calling => "Calling",
        QsoStage::GridSent => "Grid sent",
        QsoStage::ReportSent => "Report sent",
        QsoStage::RogerReportSent => "Roger/report sent",
        QsoStage::FinalSent => "Final sent",
        QsoStage::Complete => "Complete",
    }
}

fn channel_waterfall_level(rows: &VecDeque<Vec<u8>>, frequency_hz: u32) -> u8 {
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

fn spawn_audio_spectrum_worker(
    state: Arc<Mutex<GuiState>>,
    stop: Arc<AtomicBool>,
    tx_active: Arc<AtomicBool>,
    digital_tx_active: Arc<AtomicBool>,
    enabled: bool,
    sample_rate_hz: u32,
    channels: u8,
    preferred_device: Option<String>,
    repaint_ctx: Arc<OnceLock<egui::Context>>,
    display_tuning: Arc<Mutex<DisplayTuning>>,
) -> std::thread::JoinHandle<()> {
    thread::spawn(move || {
        if !enabled {
            let mut s = state.lock().expect("ui state lock poisoned");
            s.audio_spectrum_status = "DISABLED".to_string();
            return;
        }

        let audio_service = AudioService::new(preferred_device, true);
        let mut stream = match audio_service.open_stream(sample_rate_hz, channels as u16) {
            Ok(stream) => stream,
            Err(err) => {
                let mut s = state.lock().expect("ui state lock poisoned");
                s.audio_spectrum_status = format!("NO INPUT ({err})");
                return;
            }
        };

        let mut fft_planner = FftPlanner::<f32>::new();
        let audio_fft = fft_planner.plan_fft_forward(FFT_SIZE);
        let mut fft_buf = vec![Complex::<f32>::new(0.0, 0.0); FFT_SIZE];
        let mut ring: VecDeque<f32> = VecDeque::with_capacity(FFT_SIZE);
        let decode_in_progress = Arc::new(AtomicBool::new(false));
        let deferred_decode: Arc<Mutex<Option<PendingFt8Decode>>> = Arc::new(Mutex::new(None));
        let digital_decode_in_progress = Arc::new(AtomicBool::new(false));

        // 12 kHz decimation pipeline for FT8 decode
        let can_decode = sample_rate_hz == 48_000;
        let mut decimator = if can_decode {
            Some(Decimator::new(sample_rate_hz))
        } else {
            None
        };
        if can_decode {
            state
                .lock()
                .expect("ui state lock poisoned")
                .ft8_decode_status = "WARMING DECODER".to_string();
            warm_ft8_decoder();
            state
                .lock()
                .expect("ui state lock poisoned")
                .ft8_decode_status = "READY: waiting for full slot".to_string();
        } else {
            state
                .lock()
                .expect("ui state lock poisoned")
                .ft8_decode_status =
                format!("UNAVAILABLE: FT8 requires 48 kHz input (configured {sample_rate_hz} Hz)");
        }
        // 15-second accumulation buffer at 12 kHz (180 000 samples)
        // Retain pre-boundary audio so adaptive timing can compensate for a
        // clock that is behind UTC without discarding the start of a frame.
        let mut ft8_buf: Vec<f32> = Vec::with_capacity(12_000 * 18);
        let mut ft8_slot_gate = Ft8SlotGate::default();
        let mut digital_buf: Vec<f32> = Vec::with_capacity(12_000 * 120);
        let mut digital_slot_gate = DigitalSlotGate::default();
        let mut ft4_slot_gate = Ft8SlotGate::default();
        let mut decode_workspace_last: Option<WorkspaceMode> = None;

        while !stop.load(Ordering::Relaxed) {
            let chunk_samples = {
                let t = display_tuning.lock().expect("tuning lock poisoned");
                let mode = {
                    let s = state.lock().expect("ui state lock poisoned");
                    s.mode.clone()
                };
                let interval_ms = effective_visual_profile(&t, &mode).0;
                ((sample_rate_hz as u64 * interval_ms / 1_000) as usize).max(256)
            };
            let chunk_bytes = (chunk_samples * 2).max(512);
            match stream.read_chunk(chunk_bytes) {
                Ok(samples) => {
                    let samples_f32: Vec<f32> = samples
                        .iter()
                        .map(|&s| s as f32 / i16::MAX as f32)
                        .collect();
                    let rms = if samples_f32.is_empty() {
                        0.0
                    } else {
                        (samples_f32
                            .iter()
                            .map(|sample| sample * sample)
                            .sum::<f32>()
                            / samples_f32.len() as f32)
                            .sqrt()
                    };
                    let clip_percent = if samples_f32.is_empty() {
                        0.0
                    } else {
                        samples_f32
                            .iter()
                            .filter(|sample| sample.abs() >= 0.99)
                            .count() as f32
                            * 100.0
                            / samples_f32.len() as f32
                    };
                    // ── Display ring buffer + FFT ──────────────────────────
                    for &x in &samples_f32 {
                        ring.push_back(x);
                    }
                    while ring.len() > FFT_SIZE {
                        ring.pop_front();
                    }
                    let nfill = ring.len();
                    for (i, b) in fft_buf.iter_mut().enumerate() {
                        *b = if i < nfill {
                            let w =
                                0.5 - 0.5 * (2.0 * PI * i as f32 / (nfill.max(2) - 1) as f32).cos();
                            Complex::new(ring[i] * w, 0.0)
                        } else {
                            Complex::new(0.0, 0.0)
                        };
                    }
                    audio_fft.process(&mut fft_buf);
                    let bins = fft_buffer_to_display_bins(&fft_buf, AUDIO_BINS, sample_rate_hz);
                    {
                        let mut s = state.lock().expect("ui state lock poisoned");
                        if !s.ptt_on {
                            if s.audio_waterfall_rows.len() >= AUDIO_WF_HEIGHT {
                                s.audio_waterfall_rows.pop_front();
                            }
                            s.audio_waterfall_rows.push_back(bins);
                            s.audio_waterfall_revision =
                                s.audio_waterfall_revision.wrapping_add(1);
                            s.audio_spectrum_status = "LIVE RX".to_string();
                        }
                        s.audio_level_dbfs = Some(20.0 * rms.max(1e-9).log10());
                        s.audio_clip_percent = clip_percent;
                    }

                    // ── Slot-aligned native digital decoders ──────────────
                    if let Some(ref mut dec) = decimator {
                        let active_workspace_mode =
                            state.lock().expect("ui state lock poisoned").workspace_mode;
                        if decode_workspace_last != Some(active_workspace_mode) {
                            decode_workspace_last = Some(active_workspace_mode);
                            ft8_buf.clear();
                            digital_buf.clear();
                            ft8_slot_gate.reset();
                            ft4_slot_gate.reset();
                            digital_slot_gate.reset();
                            *dec = Decimator::new(sample_rate_hz);
                            let mut s = state.lock().expect("ui state lock poisoned");
                            if active_workspace_mode == WorkspaceMode::Ft8 {
                                s.ft8_decode_status =
                                    "READY: collecting a fresh FT8 slot".to_string();
                            } else if active_workspace_mode.has_native_decoder() {
                                s.digital_decode_status = format!(
                                    "READY: collecting a fresh {} slot",
                                    active_workspace_mode.label()
                                );
                            }
                        }
                        let ds = dec.process(&samples_f32);
                        if active_workspace_mode == WorkspaceMode::Ft8 {
                            ft8_buf.extend_from_slice(&ds);
                            // Keep a full slot plus ±2.5 s timing headroom.
                            let max_buf = 12_000 * 18;
                            if ft8_buf.len() > max_buf {
                                ft8_buf.drain(..ft8_buf.len() - max_buf);
                            }
                            // Arm at a UTC boundary, then decode as soon as the FT8 waveform has
                            // ended. Startup mid-slot is intentionally ignored.
                            let now_s = SystemTime::now()
                                .duration_since(UNIX_EPOCH)
                                .map(|d| d.as_secs_f64())
                                .unwrap_or(0.0);
                            let current_period = (now_s / 15.0) as u64;
                            let slot_position_s = now_s % 15.0;
                            let captured_samples = (slot_position_s * 12_000.0).round() as usize;
                            let alignment_s = state
                                .lock()
                                .expect("ui state lock poisoned")
                                .ft8_clock_offset_s
                                .unwrap_or(0.0)
                                .clamp(
                                    -FT8_ADAPTIVE_OFFSET_LIMIT_S,
                                    FT8_ADAPTIVE_OFFSET_LIMIT_S,
                                );
                            let adaptive_decode_s =
                                (FT8_EARLY_DECODE_S + alignment_s.max(0.0) as f64).min(14.6);
                            let buffer_ready = captured_samples >= (12_000 * 12)
                                && ft8_buf.len() >= captured_samples;
                            if (tx_active.load(Ordering::Acquire)
                                || digital_tx_active.load(Ordering::Acquire))
                                && slot_position_s >= FT8_EARLY_DECODE_S
                            {
                                ft8_slot_gate.skip(current_period);
                                state
                                    .lock()
                                    .expect("ui state lock poisoned")
                                    .ft8_decode_status = "TX SLOT: decode skipped".to_string();
                            } else if ft8_slot_gate.observe_at(
                                current_period,
                                slot_position_s,
                                adaptive_decode_s,
                                buffer_ready,
                            ) {
                                let decoded_period = current_period;
                                let slot_start_s = decoded_period as f64 * 15.0;
                                let utc = utc_hhmmss_millis(slot_start_s);
                                let deep_decode = state
                                    .lock()
                                    .expect("ui state lock poisoned")
                                    .ft8_deep_decode;
                                let pending = PendingFt8Decode {
                                    samples: prepare_early_ft8_slot(
                                        &ft8_buf,
                                        captured_samples,
                                        alignment_s,
                                    ),
                                    utc,
                                    period: decoded_period,
                                    deep_decode,
                                    alignment_s,
                                };
                                let in_progress = decode_in_progress.clone();
                                if in_progress
                                    .compare_exchange(
                                        false,
                                        true,
                                        Ordering::AcqRel,
                                        Ordering::Relaxed,
                                    )
                                    .is_ok()
                                {
                                    let rms = (pending
                                        .samples
                                        .iter()
                                        .map(|sample| sample * sample)
                                        .sum::<f32>()
                                        / pending.samples.len().max(1) as f32)
                                        .sqrt();
                                    let peak = pending
                                        .samples
                                        .iter()
                                        .map(|sample| sample.abs())
                                        .fold(0.0_f32, f32::max);
                                    info!(
                                        buf_samples = pending.samples.len(),
                                        utc = %pending.utc,
                                        slot_position_ms = (slot_position_s * 1_000.0).round() as u64,
                                        captured_samples,
                                        slot_rms_dbfs = 20.0 * rms.max(1e-9).log10(),
                                        slot_peak_dbfs = 20.0 * peak.max(1e-9).log10(),
                                        deep_decode = pending.deep_decode,
                                        "FT8 decode triggered"
                                    );
                                    let state_d = state.clone();
                                    let deferred_decode_d = deferred_decode.clone();
                                    thread::spawn(move || {
                                        run_ft8_decode_worker(pending, state_d, deferred_decode_d);
                                        in_progress.store(false, Ordering::Release);
                                    });
                                } else {
                                    *deferred_decode
                                        .lock()
                                        .expect("deferred decode lock poisoned") = Some(pending);
                                    info!(
                                        "FT8 decode deferred: previous decode pass still running"
                                    );
                                }
                            }
                        } else if active_workspace_mode == WorkspaceMode::Ft4 {
                            digital_buf.extend_from_slice(&ds);
                            let max_buf = 12_000 * 10;
                            if digital_buf.len() > max_buf {
                                digital_buf.drain(..digital_buf.len() - max_buf);
                            }
                            let now_s = SystemTime::now()
                                .duration_since(UNIX_EPOCH)
                                .map(|duration| duration.as_secs_f64())
                                .unwrap_or(0.0);
                            let current_period = (now_s / FT4_SLOT_SECONDS) as u64;
                            let slot_position_s = now_s % FT4_SLOT_SECONDS;
                            let captured_samples = (slot_position_s * 12_000.0).round() as usize;
                            let alignment_s = state
                                .lock()
                                .expect("ui state lock poisoned")
                                .ft4_clock_offset_s
                                .unwrap_or(0.0)
                                .clamp(
                                    -FT4_ADAPTIVE_OFFSET_LIMIT_S,
                                    FT4_ADAPTIVE_OFFSET_LIMIT_S,
                                );
                            let decode_at_s = (FT4_EARLY_DECODE_S
                                + alignment_s.max(0.0) as f64)
                                .min(7.1);
                            let buffer_ready = captured_samples >= 12_000 * 5
                                && digital_buf.len() >= captured_samples;
                            if (tx_active.load(Ordering::Acquire)
                                || digital_tx_active.load(Ordering::Acquire))
                                && slot_position_s >= FT4_EARLY_DECODE_S
                            {
                                ft4_slot_gate.skip(current_period);
                                state
                                    .lock()
                                    .expect("ui state lock poisoned")
                                    .digital_decode_status =
                                    "FT4 TX SLOT: decode skipped".to_string();
                            } else if ft4_slot_gate.observe_at(
                                current_period,
                                slot_position_s,
                                decode_at_s,
                                buffer_ready,
                            ) {
                                let decoded_period = current_period;
                                let skip_own_tx = {
                                    let mut shared =
                                        state.lock().expect("ui state lock poisoned");
                                    let skip = shared.digital_tx_period
                                        == Some((WorkspaceMode::Ft4, decoded_period));
                                    if skip {
                                        shared.digital_tx_period = None;
                                        shared.digital_decode_status =
                                            "FT4 TX slot complete; receiving".to_string();
                                    }
                                    skip
                                };
                                if skip_own_tx {
                                    continue;
                                }
                                let samples = prepare_early_digital_slot(
                                    &digital_buf,
                                    captured_samples,
                                    FT4_SLOT_SAMPLES,
                                    alignment_s,
                                );
                                let in_progress = digital_decode_in_progress.clone();
                                if in_progress
                                    .compare_exchange(
                                        false,
                                        true,
                                        Ordering::AcqRel,
                                        Ordering::Relaxed,
                                    )
                                    .is_ok()
                                {
                                    let state_d = state.clone();
                                    let utc = utc_hhmmss_millis(
                                        decoded_period as f64 * FT4_SLOT_SECONDS,
                                    );
                                    let selected_audio_hz = state
                                        .lock()
                                        .expect("ui state lock poisoned")
                                        .selected_audio_hz;
                                    let deep_decode = state
                                        .lock()
                                        .expect("ui state lock poisoned")
                                        .ft4_deep_decode;
                                    let rms = (samples
                                        .iter()
                                        .map(|sample| sample * sample)
                                        .sum::<f32>()
                                        / samples.len().max(1) as f32)
                                        .sqrt();
                                    let peak = samples
                                        .iter()
                                        .map(|sample| sample.abs())
                                        .fold(0.0_f32, f32::max);
                                    info!(
                                        mode = "FT4",
                                        period = decoded_period,
                                        slot_position_ms =
                                            (slot_position_s * 1_000.0).round() as u64,
                                        alignment_s,
                                        deep_decode,
                                        slot_rms_dbfs = 20.0 * rms.max(1e-9).log10(),
                                        slot_peak_dbfs = 20.0 * peak.max(1e-9).log10(),
                                        "FT4 early decode triggered"
                                    );
                                    thread::spawn(move || {
                                        run_native_digital_decode(
                                            WorkspaceMode::Ft4,
                                            samples,
                                            decoded_period,
                                            utc,
                                            selected_audio_hz,
                                            deep_decode,
                                            state_d,
                                        );
                                        in_progress.store(false, Ordering::Release);
                                    });
                                } else {
                                    state
                                        .lock()
                                        .expect("ui state lock poisoned")
                                        .digital_decode_status =
                                        "FT4 decode deferred: previous pass still running"
                                            .to_string();
                                }
                            }
                        } else if let Some(slot_seconds) = active_workspace_mode.core_slot_seconds()
                        {
                            digital_buf.extend_from_slice(&ds);
                            let slot_samples = (slot_seconds * 12_000.0).round() as usize;
                            if digital_buf.len() > slot_samples {
                                digital_buf.drain(..digital_buf.len() - slot_samples);
                            }
                            let now_s = SystemTime::now()
                                .duration_since(UNIX_EPOCH)
                                .map(|duration| duration.as_secs_f64())
                                .unwrap_or(0.0);
                            let current_period = (now_s / slot_seconds) as u64;
                            let buffer_ready =
                                digital_buf.len() >= slot_samples.saturating_sub(12_000 / 2);
                            if digital_slot_gate.boundary(current_period, buffer_ready) {
                                let decoded_period = current_period.saturating_sub(1);
                                let skip_own_tx = {
                                    let mut shared = state.lock().expect("ui state lock poisoned");
                                    let skip = shared.digital_tx_period
                                        == Some((active_workspace_mode, decoded_period));
                                    if skip {
                                        shared.digital_tx_period = None;
                                        shared.digital_decode_status = format!(
                                            "{} TX slot complete; receiving",
                                            active_workspace_mode.label()
                                        );
                                    }
                                    skip
                                };
                                if skip_own_tx {
                                    continue;
                                }
                                let mut samples = vec![0.0f32; slot_samples];
                                let copy_len = digital_buf.len().min(slot_samples);
                                samples[slot_samples - copy_len..]
                                    .copy_from_slice(&digital_buf[digital_buf.len() - copy_len..]);
                                let state_d = state.clone();
                                let in_progress = digital_decode_in_progress.clone();
                                if in_progress
                                    .compare_exchange(
                                        false,
                                        true,
                                        Ordering::AcqRel,
                                        Ordering::Relaxed,
                                    )
                                    .is_ok()
                                {
                                    let utc =
                                        utc_hhmmss_millis(decoded_period as f64 * slot_seconds);
                                    let selected_audio_hz = state
                                        .lock()
                                        .expect("ui state lock poisoned")
                                        .selected_audio_hz;
                                    info!(
                                        mode = active_workspace_mode.label(),
                                        period = decoded_period,
                                        buf_samples = samples.len(),
                                        utc,
                                        "digital decode triggered"
                                    );
                                    thread::spawn(move || {
                                        run_native_digital_decode(
                                            active_workspace_mode,
                                            samples,
                                            decoded_period,
                                            utc,
                                            selected_audio_hz,
                                            false,
                                            state_d,
                                        );
                                        in_progress.store(false, Ordering::Release);
                                    });
                                } else {
                                    state
                                        .lock()
                                        .expect("ui state lock poisoned")
                                        .digital_decode_status = format!(
                                        "{} decode skipped: previous pass still running",
                                        active_workspace_mode.label()
                                    );
                                }
                            }
                        }
                    }

                    if let Some(ctx) = repaint_ctx.get() {
                        ctx.request_repaint();
                    }
                }
                Err(err) => {
                    state
                        .lock()
                        .expect("ui state lock poisoned")
                        .audio_spectrum_status = format!("NO INPUT ({err})");
                    thread::sleep(Duration::from_millis(100));
                }
            }
        }
    })
}

fn warm_ft8_decoder() {
    let warmup_audio = vec![0i16; FT8_SLOT_SAMPLES];
    let started = Instant::now();
    let _ = DecodeRequest::<Ft8>::wsjtx_depth(
        &warmup_audio,
        100.0,
        3_000.0,
        1.6,
        16,
        WsjtxDepth::D1,
        None,
    )
    .decode();
    info!(
        elapsed_ms = started.elapsed().as_millis() as u64,
        "FT8 decoder warmup complete"
    );
}

fn prepare_early_ft8_slot(
    rolling: &[f32],
    captured_samples: usize,
    alignment_s: f32,
) -> Vec<f32> {
    let mut slot = vec![0.0; FT8_SLOT_SAMPLES];
    let local_boundary = rolling.len() as isize - captured_samples.min(rolling.len()) as isize;
    let alignment_samples = (alignment_s * 12_000.0).round() as isize;
    let requested_start = local_boundary + alignment_samples;
    let source_start = requested_start.max(0) as usize;
    let destination_start = requested_start.min(0).unsigned_abs().min(FT8_SLOT_SAMPLES);
    let copy_len = rolling
        .len()
        .saturating_sub(source_start)
        .min(FT8_SLOT_SAMPLES.saturating_sub(destination_start));
    if copy_len > 0 {
        slot[destination_start..destination_start + copy_len]
            .copy_from_slice(&rolling[source_start..source_start + copy_len]);
    }
    slot
}

fn prepare_early_digital_slot(
    rolling: &[f32],
    captured_samples: usize,
    slot_samples: usize,
    alignment_s: f32,
) -> Vec<f32> {
    let mut slot = vec![0.0; slot_samples];
    let local_boundary = rolling.len() as isize - captured_samples.min(rolling.len()) as isize;
    let alignment_samples = (alignment_s * 12_000.0).round() as isize;
    let requested_start = local_boundary + alignment_samples;
    let source_start = requested_start.max(0) as usize;
    let destination_start = requested_start.min(0).unsigned_abs().min(slot_samples);
    let copy_len = rolling
        .len()
        .saturating_sub(source_start)
        .min(slot_samples.saturating_sub(destination_start));
    if copy_len > 0 {
        slot[destination_start..destination_start + copy_len]
            .copy_from_slice(&rolling[source_start..source_start + copy_len]);
    }
    slot
}

fn run_native_digital_decode(
    mode: WorkspaceMode,
    samples: Vec<f32>,
    period: u64,
    utc: String,
    selected_audio_hz: u32,
    deep_decode: bool,
    state: Arc<Mutex<GuiState>>,
) {
    let backend = state
        .lock()
        .expect("ui state lock poisoned")
        .compute_backend;
    let budget = Duration::from_secs_f64(mode.core_slot_seconds().unwrap_or(15.0));
    let mut trace = DecodeTrace::new(mode.label(), backend, samples.len(), budget);
    let mut decoded = Vec::new();
    let mut push = |snr_db: f32, dt_s: f32, freq_hz: f32, message: String| {
        decoded.push(DigitalDecodeEntry {
            mode,
            period,
            utc: utc.clone(),
            snr_db,
            dt_s,
            freq_hz: freq_hz.max(0.0).round() as u32,
            message,
        });
    };

    trace.measure("protocol decode", || match mode {
        WorkspaceMode::Ft4 | WorkspaceMode::Fst4 => {
            let audio: Vec<i16> = samples
                .iter()
                .map(|sample| (sample.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16)
                .collect();
            if mode == WorkspaceMode::Ft4 {
                let sync_min = if deep_decode { 0.45 } else { 0.6 };
                let request = DecodeRequest::<mfsk_core::ft4::Ft4>::new(
                    &audio,
                    100.0,
                    3_000.0,
                    sync_min,
                    160,
                )
                .freq_hint(selected_audio_hz as f32);
                let outcome = if deep_decode {
                    request.sic_rounds(3).decode()
                } else {
                    request.decode()
                };
                for result in outcome.results {
                    if let Some(message) = unpack77(result.message77()) {
                        push(result.snr_db, result.dt_sec, result.freq_hz, message);
                    }
                }
            } else {
                let outcome =
                    DecodeRequest::<mfsk_core::fst4::Fst4s60>::new(&audio, 100.0, 3_000.0, 0.8, 50)
                        .decode();
                for result in outcome.results {
                    if let Some(message) = unpack77(result.message77()) {
                        push(result.snr_db, result.dt_sec, result.freq_hz, message);
                    }
                }
            }
        }
        WorkspaceMode::Wspr => {
            for result in mfsk_core::wspr::decode::decode_scan_default(&samples, 12_000) {
                push(
                    result.snr_db,
                    result.dt_sec,
                    result.freq_hz,
                    result.message.to_string(),
                );
            }
        }
        WorkspaceMode::Jt9 => {
            for result in mfsk_core::jt9::decode_scan_default(&samples, 12_000) {
                push(
                    result.snr_db,
                    result.start_sample as f32 / 12_000.0,
                    result.freq_hz,
                    result.message.to_string(),
                );
            }
        }
        WorkspaceMode::Jt65 => {
            for result in mfsk_core::jt65::decode_scan_chase_default(&samples, 12_000) {
                push(
                    result.snr_db,
                    result.start_sample as f32 / 12_000.0,
                    result.freq_hz,
                    result.message.to_string(),
                );
            }
        }
        WorkspaceMode::Q65 => {
            let request = mfsk_core::q65::DecodeRequest::<mfsk_core::q65::Q65a30>::new(
                &samples,
                12_000,
                0,
                mfsk_core::q65::SearchParams::default(),
            );
            for result in request.decode() {
                push(
                    result.snr_db,
                    result.start_sample as f32 / 12_000.0,
                    result.freq_hz,
                    result.message,
                );
            }
        }
        WorkspaceMode::Msk144 => {
            let audio: Vec<i16> = samples
                .iter()
                .map(|sample| (sample.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16)
                .collect();
            for result in mfsk_core::msk144::decode::decode_slot(
                &audio,
                selected_audio_hz as f32,
                200.0,
                mfsk_core::msk144::decode::Depth::Normal,
            ) {
                push(
                    result.snr_db as f32,
                    result.tsec,
                    result.freq_hz,
                    result.message,
                );
            }
        }
        WorkspaceMode::Ft8 | WorkspaceMode::Cw | WorkspaceMode::Fldigi => {}
    });

    let telemetry = trace.finish(decoded.len());
    let elapsed_ms = telemetry.total.as_millis();
    info!(
        mode = mode.label(),
        decoded = decoded.len(),
        elapsed_ms = elapsed_ms as u64,
        "digital decode pass complete"
    );
    let mut shared = state.lock().expect("ui state lock poisoned");
    shared.digital_compute_telemetry = Some(telemetry);
    if mode == WorkspaceMode::Ft4 {
        shared.ft4_last_decode_period = Some(period);
    }
    if mode == WorkspaceMode::Ft4 && !decoded.is_empty() {
        let mut offsets: Vec<f32> = decoded.iter().map(|result| result.dt_s).collect();
        offsets.sort_by(f32::total_cmp);
        let measured = offsets[offsets.len() / 2].clamp(
            -FT4_ADAPTIVE_OFFSET_LIMIT_S,
            FT4_ADAPTIVE_OFFSET_LIMIT_S,
        );
        shared.ft4_clock_offset_s = Some(shared.ft4_clock_offset_s.map_or(measured, |previous| {
            previous + 0.35 * (measured - previous)
        }));
    }
    shared.digital_decode_status = if decoded.is_empty() {
        format!("LIVE: no {} decodes in {elapsed_ms} ms", mode.label())
    } else {
        let timing = if mode == WorkspaceMode::Ft4 {
            shared
                .ft4_clock_offset_s
                .map(|offset| format!(" | adaptive dT {offset:+.2}s"))
                .unwrap_or_default()
        } else {
            String::new()
        };
        format!(
            "LIVE: {} {} decoded in {elapsed_ms} ms{timing}",
            decoded.len(), mode.label()
        )
    };
    shared.digital_decodes.extend(decoded);
    while shared.digital_decodes.len() > 300 {
        shared.digital_decodes.pop_front();
    }
}

fn run_ft8_decode_worker(
    mut pending: PendingFt8Decode,
    state: Arc<Mutex<GuiState>>,
    deferred_decode: Arc<Mutex<Option<PendingFt8Decode>>>,
) {
    loop {
        let elapsed_ms = run_ft8_decode(
            pending.samples,
            state.clone(),
            pending.utc,
            pending.period,
            pending.deep_decode,
            pending.alignment_s,
        );
        let next = deferred_decode
            .lock()
            .expect("deferred decode lock poisoned")
            .take();
        if let Some(mut next_pending) = next {
            if next_pending.deep_decode && elapsed_ms > FT8_DEEP_RUNTIME_BUDGET_MS {
                next_pending.deep_decode = false;
                info!(
                    elapsed_ms = elapsed_ms as u64,
                    budget_ms = FT8_DEEP_RUNTIME_BUDGET_MS as u64,
                    "FT8 deep decode exceeded realtime budget; switching deferred pass to FAST"
                );
            }
            info!(
                buf_samples = next_pending.samples.len(),
                utc = %next_pending.utc,
                deep_decode = next_pending.deep_decode,
                "FT8 running deferred decode"
            );
            pending = next_pending;
        } else {
            break;
        }
    }
}

fn fft_buffer_to_display_bins(buf: &[Complex<f32>], bins: usize, sample_rate_hz: u32) -> Vec<u8> {
    let n = buf.len();
    if n == 0 || bins == 0 {
        return vec![0; bins];
    }
    let max_k = (n as f32 * AUDIO_MAX_FREQ_HZ as f32 / sample_rate_hz as f32).round() as usize;
    let max_k = max_k.clamp(2, n / 2);
    (0..bins)
        .map(|i| {
            // Fractional bin position with linear magnitude interpolation.
            let pos = 1.0 + (i as f32 / (bins.max(2) - 1) as f32) * (max_k - 1) as f32;
            let k0 = pos.floor() as usize;
            let k1 = (k0 + 1).min(max_k);
            let t = pos - k0 as f32;
            let m0 = buf[k0].norm() / n as f32;
            let m1 = buf[k1].norm() / n as f32;
            let mag = m0 + (m1 - m0) * t;
            let db = (20.0 * mag.max(1e-9_f32).log10()).clamp(-65.0, 0.0);
            ((db + 65.0) / 65.0 * 255.0).round().clamp(0.0, 255.0) as u8
        })
        .collect()
}

fn utc_hhmmss_millis(epoch_s: f64) -> String {
    let day_s = epoch_s.max(0.0).rem_euclid(86_400.0);
    let h = (day_s / 3600.0).floor() as u64;
    let m = ((day_s % 3600.0) / 60.0).floor() as u64;
    let sec_f = day_s % 60.0;
    let s = sec_f.floor() as u64;
    let mut ms = ((sec_f - s as f64) * 1000.0).round() as u64;
    let mut sec = s;
    let mut min = m;
    let mut hour = h;

    if ms == 1000 {
        ms = 0;
        sec += 1;
        if sec == 60 {
            sec = 0;
            min += 1;
            if min == 60 {
                min = 0;
                hour = (hour + 1) % 24;
            }
        }
    }

    format!("{:02}:{:02}:{:02}.{:03}", hour, min, sec, ms)
}

/// Background FT8 decode — runs in its own thread, one per period.
fn run_ft8_decode(
    samples: Vec<f32>,
    state: Arc<Mutex<GuiState>>,
    utc: String,
    period: u64,
    deep_decode: bool,
    alignment_s: f32,
) -> u128 {
    let backend = state
        .lock()
        .expect("ui state lock poisoned")
        .compute_backend;
    let mut trace = DecodeTrace::new(
        "FT8",
        backend,
        samples.len(),
        Duration::from_millis(FT8_SLOT_MS as u64),
    );
    let audio_i16: Vec<i16> = trace.measure("prepare PCM", || {
        samples
            .iter()
            .map(|&x| {
                let s = x.clamp(-1.0, 1.0);
                (s * i16::MAX as f32).round() as i16
            })
            .collect()
    });

    // mfsk-core FT8 decode (12 kHz slot-aligned audio), mapped to the
    // library's WSJT-X depth presets for clearer latency/recall behavior.
    let outcome = trace.measure("protocol decode", || if deep_decode {
        // D2: staged early decode (`sic_early`) with WSJT-X-style profile.
        DecodeRequest::<Ft8>::wsjtx_depth(
            &audio_i16,
            100.0,
            3_000.0,
            FT8_DEEP_SYNC_MIN,
            FT8_DEEP_MAX_CAND,
            WsjtxDepth::D2,
            None,
        )
        .decode()
    } else {
        // D1: non-early SIC (`sic_rounds(2)`) for lower latency.
        DecodeRequest::<Ft8>::wsjtx_depth(
            &audio_i16,
            100.0,
            3_000.0,
            FT8_FAST_SYNC_MIN,
            FT8_FAST_MAX_CAND,
            WsjtxDepth::D1,
            None,
        )
        .decode()
    });

    let results: Vec<Ft8DecodeEntry> = trace.measure("unpack results", || {
        let mut results = Vec::new();
        for r in &outcome.results {
            if let Some(msg) = unpack77(r.message77()) {
                let is_cq = msg.starts_with("CQ");
                let snr = r.snr_db.round() as i8;
                let absolute_dt_s = alignment_s + r.dt_sec;
                debug!(freq = r.freq_hz, dt_s = absolute_dt_s, snr, msg, "FT8 decode OK");
                results.push(Ft8DecodeEntry {
                    period,
                    utc: utc.clone(),
                    snr_db: snr,
                    dt_s: absolute_dt_s,
                    freq_hz: r.freq_hz.max(0.0).round() as u32,
                    message: msg,
                    is_cq,
                });
            }
        }
        results
    });

    let telemetry = trace.finish(results.len());
    let elapsed_ms = telemetry.total.as_millis();
    info!(
        deep_decode,
        decoded = results.len(),
        elapsed_ms = elapsed_ms as u64,
        over_slot = elapsed_ms > FT8_SLOT_MS,
        "FT8 decode pass complete"
    );

    let mut s = state.lock().expect("ui state lock poisoned");
    s.ft8_compute_telemetry = Some(telemetry);
    s.ft8_last_decode_period = Some(period);
    if !results.is_empty() {
        let mut offsets: Vec<f32> = results.iter().map(|result| result.dt_s).collect();
        offsets.sort_by(f32::total_cmp);
        let measured_offset = offsets[offsets.len() / 2]
            .clamp(-FT8_ADAPTIVE_OFFSET_LIMIT_S, FT8_ADAPTIVE_OFFSET_LIMIT_S);
        let adaptive_offset = s.ft8_clock_offset_s.map_or(measured_offset, |previous| {
            previous + 0.35 * (measured_offset - previous)
        });
        s.ft8_clock_offset_s = Some(adaptive_offset);
        s.ft8_decode_status = format!(
            "LIVE: {} decoded in {} ms | adaptive dT {adaptive_offset:+.2}s",
            results.len(), elapsed_ms
        );
        s.ft8_pending.extend(results);
    } else {
        s.ft8_decode_status = format!("LIVE: no decodes in {elapsed_ms} ms");
    }

    elapsed_ms
}

fn ft8_period_progress() -> f32 {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);
    let pos = secs % 15.0;
    (pos / 15.0) as f32
}

fn filter_bandwidth_hz(mode: &str, filter: Option<u8>) -> u32 {
    let f = filter.unwrap_or(1);
    let m = mode.to_ascii_uppercase();
    if m.contains("CW") {
        match f {
            1 => 500,
            2 => 250,
            3 => 100,
            _ => 500,
        }
    } else if m.contains("FM") {
        match f {
            1 => 15_000,
            2 => 10_000,
            3 => 7_000,
            _ => 15_000,
        }
    } else if m.contains("RTTY") {
        match f {
            1 => 500,
            2 => 350,
            3 => 250,
            _ => 500,
        }
    } else {
        // USB / LSB / Data — IC-7300 defaults
        match f {
            1 => 3_000,
            2 => 2_400,
            3 => 1_800,
            _ => 3_000,
        }
    }
}

fn effective_visual_profile(tuning: &DisplayTuning, mode: &str) -> (u64, u8) {
    if !tuning.auto_visual {
        return match tuning.waterfall_speed {
            WaterfallSpeed::Slow => (220, 2),
            WaterfallSpeed::Mid => (120, 1),
            WaterfallSpeed::Fast => (35, 0),
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
        // Timed digital modes do not benefit from a video-rate waterfall.
        // Ten rows per second keeps tuning responsive while sharply reducing
        // FFT, texture upload, and repaint work.
        (100, 1)
    } else if m.contains("FM") {
        (120, 1)
    } else {
        (90, 1)
    }
}

fn is_transient_civ_read_error(message: &str) -> bool {
    let m = message.to_ascii_lowercase();
    m.contains("failed to read ci-v response") || m.contains("timed out") || m.contains("timeout")
}

fn build_waterfall_image(
    rows: &VecDeque<Vec<u8>>,
    width: usize,
    height: usize,
    gamma: f32,
) -> ColorImage {
    let mut pixels = vec![Color32::BLACK; width * height];
    let empty_row = vec![0u8; width];
    let missing = height.saturating_sub(rows.len());
    for y in 0..height {
        let src_row = if y < missing {
            &empty_row
        } else {
            rows.get(y - missing).unwrap_or(&empty_row)
        };
        for x in 0..width {
            let value = src_row.get(x).copied().unwrap_or(0);
            pixels[y * width + x] = waterfall_color(value, gamma);
        }
    }
    ColorImage::new([width, height], pixels)
}

fn waterfall_color(v: u8, gamma: f32) -> Color32 {
    let t = (v as f32 / 255.0).powf(gamma);
    let (r, g, b) = if t < 0.33 {
        let k = t / 0.33;
        (0.0, k * 180.0, 80.0 + k * 175.0)
    } else if t < 0.66 {
        let k = (t - 0.33) / 0.33;
        (k * 220.0, 180.0 + k * 60.0, 255.0 - k * 220.0)
    } else {
        let k = (t - 0.66) / 0.34;
        (220.0 + k * 35.0, 240.0 + k * 15.0, 35.0 + k * 220.0)
    };
    Color32::from_rgb(r as u8, g as u8, b as u8)
}

fn fmt_opt_u8(v: Option<u8>) -> String {
    v.map(|n| n.to_string()).unwrap_or_else(|| "?".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode_pcm_samples(bytes: &[u8]) -> Vec<i16> {
        bytes
            .chunks_exact(2)
            .map(|c| i16::from_le_bytes([c[0], c[1]]))
            .collect()
    }

    #[test]
    fn decode_pcm_samples_handles_little_endian_i16() {
        let bytes = [0x00, 0x00, 0x01, 0x00, 0xFF, 0xFF, 0xFE, 0xFF];
        let samples = decode_pcm_samples(&bytes);
        assert_eq!(samples, vec![0i16, 1, -1, -2]);
    }

    #[test]
    fn operator_callsign_highlight_distinguishes_calls_from_mentions() {
        assert_eq!(
            operator_call_hit("W1AW N7XYZ -12", "W1AW"),
            Some(OperatorCallHit::DirectedToMe)
        );
        assert_eq!(
            operator_call_hit("CQ TEST N7XYZ W1AW", "W1AW"),
            Some(OperatorCallHit::Mentioned)
        );
        assert_eq!(operator_call_hit("CQ W1A FN42", "W1AW"), None);
        assert_eq!(operator_call_hit("CQ N7XYZ FN42", "N0CALL"), None);
    }

    #[test]
    fn apply_waterfall_bins_caps_rows_and_preserves_latest() {
        let mut state = GuiState::default();
        for i in 0..RADIO_WF_HEIGHT + 3 {
            apply_waterfall_bins(&mut state, &[i as u8; 8]);
        }

        assert_eq!(state.radio_waterfall_rows.len(), RADIO_WF_HEIGHT);
        assert_eq!(
            state.radio_waterfall_rows.back().unwrap()[0],
            (RADIO_WF_HEIGHT + 2) as u8
        );
    }

    #[test]
    fn channel_waterfall_level_tracks_selected_frequency() {
        let mut rows = VecDeque::new();
        let mut row = vec![0u8; AUDIO_BINS];
        let selected_hz = 1_500;
        let selected_bin = ((selected_hz as f32 / AUDIO_MAX_FREQ_HZ as f32)
            * (AUDIO_BINS - 1) as f32)
            .round() as usize;
        row[selected_bin] = 210;
        rows.push_back(row);

        assert_eq!(channel_waterfall_level(&rows, selected_hz), 210);
        assert_eq!(channel_waterfall_level(&rows, 3_000), 0);
    }

    #[test]
    fn ft8_activity_stats_group_cycles_and_callsigns() {
        let entry = |period, snr_db, message: &str, is_cq| Ft8DecodeEntry {
            period,
            utc: "12:00:00".to_string(),
            snr_db,
            dt_s: 0.2,
            freq_hz: 1_500,
            message: message.to_string(),
            is_cq,
        };
        let log = vec![
            entry(10, -20, "CQ K1ABC FN42", true),
            entry(10, -12, "CQ W9XYZ EN50", true),
            entry(11, -8, "W1AW K1ABC -08", false),
            entry(11, 2, "CQ K1ABC FN42", true),
        ];

        let stats = ft8_activity_stats(&log);
        assert_eq!(stats.latest_cycle, 2);
        assert_eq!(stats.cq_this_cycle, 1);
        assert_eq!(stats.average_per_cycle, 2.0);
        assert_eq!(stats.unique_stations, 2);
        assert_eq!(stats.most_heard, Some(("K1ABC".to_string(), 3)));
        assert_eq!(stats.median_snr, Some(-8));
    }

    #[test]
    fn tx_moves_only_when_starting_contact_from_remote_cq() {
        let cq = parse_message("CQ K1ABC FN42").expect("CQ");
        let caller = parse_message("W1AW K1ABC FN42").expect("directed reply");

        assert!(should_move_tx_to_decode(&cq, false));
        assert!(!should_move_tx_to_decode(&caller, false));
        assert!(!should_move_tx_to_decode(&cq, true));
    }

    #[test]
    fn compute_audio_spectrum_bins_returns_expected_length() {
        fn compute_audio_spectrum_bins(
            samples: &[i16],
            bins: usize,
            sample_rate_hz: u32,
        ) -> Vec<u8> {
            let n = samples.len().min(FFT_SIZE).max(2);
            let mut planner = FftPlanner::<f32>::new();
            let fft = planner.plan_fft_forward(n);
            let mut buf: Vec<Complex<f32>> = samples
                .iter()
                .take(n)
                .enumerate()
                .map(|(i, &s)| {
                    let w = 0.5 - 0.5 * (2.0 * PI * i as f32 / (n - 1) as f32).cos();
                    Complex::new(s as f32 / i16::MAX as f32 * w, 0.0)
                })
                .collect();
            fft.process(&mut buf);
            fft_buffer_to_display_bins(&buf, bins, sample_rate_hz)
        }
        let bins = compute_audio_spectrum_bins(&[0i16; 256], AUDIO_BINS, 48_000);
        assert_eq!(bins.len(), AUDIO_BINS);
    }

    #[test]
    fn automatic_digital_visuals_use_an_efficient_cadence() {
        let tuning = DisplayTuning::default();
        assert_eq!(effective_visual_profile(&tuning, "USB-D"), (100, 1));
        assert_eq!(effective_visual_profile(&tuning, "FT8"), (100, 1));
    }

    #[test]
    fn ft8_slot_gate_never_decodes_mid_slot_after_startup() {
        let mut gate = Ft8SlotGate::default();

        assert!(!gate.observe(100, 13.5, true));
        assert!(!gate.observe(100, 14.5, true));
        assert!(!gate.observe(101, 0.1, false));
        assert!(!gate.observe(101, 12.9, true));
        assert!(gate.observe(101, FT8_EARLY_DECODE_S, true));
        assert!(!gate.observe(101, 14.0, true));
    }

    #[test]
    fn ft8_slot_gate_reset_requires_another_boundary() {
        let mut gate = Ft8SlotGate::default();

        assert!(!gate.observe(100, 14.0, true));
        assert!(!gate.observe(101, 0.0, false));
        assert!(gate.observe(101, FT8_EARLY_DECODE_S, true));
        gate.reset();
        assert!(!gate.observe(200, 14.0, true));
        assert!(!gate.observe(201, 0.0, false));
        assert!(gate.observe(201, FT8_EARLY_DECODE_S, true));
    }

    #[test]
    fn ft8_slot_gate_can_skip_our_tx_period() {
        let mut gate = Ft8SlotGate::default();
        assert!(!gate.observe(100, 14.0, true));
        assert!(!gate.observe(101, 0.0, false));
        gate.skip(101);
        assert!(!gate.observe(101, 14.0, true));
        assert!(!gate.observe(102, 0.0, false));
        assert!(gate.observe(102, FT8_EARLY_DECODE_S, true));
    }

    #[test]
    fn digital_slot_gate_requires_a_complete_period_after_startup() {
        let mut gate = DigitalSlotGate::default();
        assert!(!gate.boundary(10, true));
        assert!(!gate.boundary(10, true));
        assert!(!gate.boundary(11, false));
        assert!(gate.boundary(12, true));
        assert!(!gate.boundary(12, true));
    }

    #[test]
    fn native_digital_tx_builders_generate_audio() {
        for mode in [
            WorkspaceMode::Ft4,
            WorkspaceMode::Fst4,
            WorkspaceMode::Jt9,
            WorkspaceMode::Jt65,
            WorkspaceMode::Q65,
        ] {
            let (pcm, offset) = build_native_digital_tx_pcm(mode, "CQ W1AW AA00", 1_500)
                .unwrap_or_else(|error| panic!("{} synthesis failed: {error}", mode.label()));
            assert!(!pcm.is_empty(), "{} synthesis was empty", mode.label());
            assert!(pcm.iter().any(|sample| *sample != 0));
            assert!(offset >= 0.0);
        }
    }

    #[test]
    fn ft4_workspace_adapter_decodes_generated_audio() {
        let (pcm, offset_s) =
            build_native_digital_tx_pcm(WorkspaceMode::Ft4, "CQ W1AW AA00", 1_500)
                .expect("FT4 synthesis");
        let mut slot = vec![0.0f32; (7.5 * 12_000.0) as usize];
        let start = (offset_s * 12_000.0) as usize;
        for (dst, sample) in slot[start..].iter_mut().zip(pcm) {
            *dst = sample as f32 / i16::MAX as f32;
        }
        let state = Arc::new(Mutex::new(GuiState::default()));
        run_native_digital_decode(
            WorkspaceMode::Ft4,
            slot,
            10,
            "00:01:15.000".to_string(),
            1_500,
            false,
            state.clone(),
        );
        let shared = state.lock().expect("state");
        assert!(shared
            .digital_decodes
            .iter()
            .any(|entry| entry.message == "CQ W1AW AA00"));
    }

    #[test]
    fn early_ft4_capture_contains_a_deliberately_late_decodable_waveform() {
        let (pcm, _) = build_native_digital_tx_pcm(
            WorkspaceMode::Ft4,
            "CQ W1AW AA00",
            1_500,
        )
        .expect("FT4 synthesis");
        let captured = (FT4_EARLY_DECODE_S * 12_000.0).round() as usize;
        let prehistory = 12_000 * 2;
        let mut rolling = vec![0.0f32; prehistory + captured];
        // Start 0.7 s later than the nominal +0.5 s. The old 5.5 s
        // trigger truncated this frame; the guarded early window must not.
        let start = prehistory + (1.2_f32 * 12_000.0).round() as usize;
        for (dst, sample) in rolling[start..].iter_mut().zip(pcm) {
            *dst = sample as f32 / i16::MAX as f32;
        }
        let slot = prepare_early_digital_slot(
            &rolling,
            captured,
            FT4_SLOT_SAMPLES,
            0.0,
        );
        let state = Arc::new(Mutex::new(GuiState::default()));
        run_native_digital_decode(
            WorkspaceMode::Ft4,
            slot,
            10,
            "00:01:15.000".to_string(),
            1_500,
            false,
            state.clone(),
        );
        assert!(state
            .lock()
            .expect("state")
            .digital_decodes
            .iter()
            .any(|entry| entry.message == "CQ W1AW AA00"));
    }

    #[test]
    fn early_ft8_slot_keeps_current_audio_at_slot_start() {
        let rolling: Vec<f32> = (0..FT8_SLOT_SAMPLES).map(|sample| sample as f32).collect();
        let captured = 13 * 12_000;
        let slot = prepare_early_ft8_slot(&rolling, captured, 0.0);

        assert_eq!(slot.len(), FT8_SLOT_SAMPLES);
        assert_eq!(slot[0], (FT8_SLOT_SAMPLES - captured) as f32);
        assert_eq!(slot[captured - 1], (FT8_SLOT_SAMPLES - 1) as f32);
        assert_eq!(slot[captured], 0.0);
    }

    #[test]
    fn adaptive_ft8_slot_moves_the_capture_boundary_both_directions() {
        let rolling_len = 18 * 12_000;
        let captured = 13 * 12_000;
        let rolling: Vec<f32> = (0..rolling_len).map(|sample| sample as f32).collect();
        let local_boundary = rolling_len - captured;

        let late = prepare_early_ft8_slot(&rolling, captured, 0.5);
        assert_eq!(late[0], (local_boundary + 6_000) as f32);

        let early = prepare_early_ft8_slot(&rolling, captured, -0.5);
        assert_eq!(early[0], (local_boundary - 6_000) as f32);
    }

    #[test]
    fn early_ft8_slot_contains_a_decodable_complete_waveform() {
        let pcm = build_ft8_tx_pcm("CQ W1AW AA00", 1_500).expect("FT8 PCM");
        let captured = (FT8_EARLY_DECODE_S * 12_000.0).round() as usize;
        let mut rolling = vec![0.0f32; FT8_SLOT_SAMPLES];
        let current_slot_start = FT8_SLOT_SAMPLES - captured;
        let signal_start = current_slot_start + (FT8_TX_AUDIO_START_S * 12_000.0) as usize;
        for (dst, sample) in rolling[signal_start..].iter_mut().zip(pcm) {
            *dst = sample as f32 / i16::MAX as f32;
        }
        let slot = prepare_early_ft8_slot(&rolling, captured, 0.0);
        let audio: Vec<i16> = slot
            .iter()
            .map(|sample| (sample * i16::MAX as f32).round() as i16)
            .collect();
        let outcome = DecodeRequest::<Ft8>::wsjtx_depth(
            &audio,
            100.0,
            3_000.0,
            FT8_FAST_SYNC_MIN,
            FT8_FAST_MAX_CAND,
            WsjtxDepth::D1,
            None,
        )
        .decode();

        let messages: Vec<String> = outcome
            .results
            .iter()
            .filter_map(|result| unpack77(result.message77()))
            .collect();
        assert!(
            messages.iter().any(|message| message == "CQ W1AW AA00"),
            "early decode messages: {messages:?}"
        );
    }
}
