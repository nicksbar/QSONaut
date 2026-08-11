use anyhow::{Context, anyhow, Result};
use eframe::egui::{self, Color32, ColorImage, RichText, TextureHandle, TextureOptions};
use mfsk_core::{
    ft8::decode::WsjtxDepth,
    ft8::wave_gen::{message_to_tones as ft8_message_to_tones, tones_to_i16 as ft8_tones_to_i16},
    ft8::Ft8,
    msg::{decode_request::DecodeRequest, wsjt77::{pack77, unpack77}},
};
use rigforge_audio::AudioService;
use rigforge_core::AppConfig;
use rigforge_dsp::resample::Decimator;
use rigforge_radio::{BaseMode, ControlId, ControlValue, IcomCiVRadio, Mode, Radio, RadioHal};
use rustfft::{FftPlanner, num_complex::Complex};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::f32::consts::PI;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::process::Stdio;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tracing::{debug, info};

const RADIO_WF_WIDTH: usize = 360;
const RADIO_WF_HEIGHT: usize = 180;
const AUDIO_BINS: usize = 512;
const AUDIO_WF_HEIGHT: usize = 120;
const AUDIO_MAX_FREQ_HZ: u32 = 4_000;
// 8192 samples @ 48 kHz = 170 ms window, ~5.9 Hz/bin, ~683 useful bins for 0-4 kHz.
const FFT_SIZE: usize = 8192;
const OPERATOR_PROFILE_FILE: &str = ".rigforge_profile.toml";
const FT8_SLOT_MS: u128 = 15_000;
const FT8_DEEP_RUNTIME_BUDGET_MS: u128 = 12_000;
const FT8_FAST_SYNC_MIN: f32 = 1.7;
const FT8_FAST_MAX_CAND: usize = 96;
const FT8_DEEP_SYNC_MIN: f32 = 1.9;
const FT8_DEEP_MAX_CAND: usize = 120;
const FT8_TX_AMPLITUDE_I16: i16 = 18_000;
const FT8_TX_SAMPLE_RATE_HZ: u32 = 12_000;
const FT8_TX_SLOT_START_POS_S: f64 = 0.50;
const FT8_TX_LAUNCH_WINDOW_S: f64 = 1.20;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OperatorProfile {
    callsign: String,
    grid: String,
    qth: String,
    follow_log: bool,
    max_log_entries: usize,
    deep_decode: bool,
    #[serde(default)]
    autoseq: bool,
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
}

fn default_rx_tone_hz() -> u32 { 1500 }
fn default_tx_tone_hz() -> u32 { 1500 }
fn default_halt_after_tx() -> bool { true }
fn default_hold_tx_freq() -> bool { true }

fn operator_profile_path() -> PathBuf {
    std::env::current_dir()
        .map(|d| d.join(OPERATOR_PROFILE_FILE))
        .unwrap_or_else(|_| PathBuf::from(OPERATOR_PROFILE_FILE))
}

fn load_operator_profile() -> Option<OperatorProfile> {
    let path = operator_profile_path();
    let src = fs::read_to_string(path).ok()?;
    toml::from_str::<OperatorProfile>(&src).ok()
}

fn save_operator_profile(profile: &OperatorProfile) -> Result<()> {
    let path = operator_profile_path();
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
    ("160m",  1_840_000),
    ("80m",   3_573_000),
    ("60m",   5_357_000),
    ("40m",   7_074_000),
    ("30m",  10_136_000),
    ("20m",  14_074_000),
    ("17m",  18_100_000),
    ("15m",  21_074_000),
    ("12m",  24_915_000),
    ("10m",  28_074_000),
    ("6m",   50_313_000),
];

static FT4_BANDS: &[(&str, u64)] = &[
    ("80m",   3_575_000),
    ("40m",   7_047_500),
    ("30m",  10_140_000),
    ("20m",  14_080_000),
    ("17m",  18_104_000),
    ("15m",  21_140_000),
    ("12m",  24_919_000),
    ("10m",  28_180_000),
    ("6m",   50_318_000),
];

static FST4_BANDS: &[(&str, u64)] = &[
    ("80m",   3_573_000),
    ("40m",   7_047_500),
    ("30m",  10_140_000),
    ("20m",  14_080_000),
    ("17m",  18_104_000),
    ("15m",  21_140_000),
    ("12m",  24_919_000),
    ("10m",  28_180_000),
];

static WSPR_BANDS: &[(&str, u64)] = &[
    ("160m",  1_836_600),
    ("80m",   3_568_600),
    ("60m",   5_287_200),
    ("40m",   7_038_600),
    ("30m",  10_138_700),
    ("20m",  14_095_600),
    ("17m",  18_104_600),
    ("15m",  21_094_600),
    ("12m",  24_924_600),
    ("10m",  28_124_600),
    ("6m",   50_294_400),
];

static JT9_BANDS: &[(&str, u64)] = &[
    ("160m",  1_839_000),
    ("80m",   3_578_000),
    ("40m",   7_078_000),
    ("30m",  10_140_000),
    ("20m",  14_078_000),
    ("17m",  18_104_000),
    ("15m",  21_078_000),
    ("12m",  24_919_000),
    ("10m",  28_078_000),
    ("6m",   50_312_000),
];

static JT65_BANDS: &[(&str, u64)] = &[
    ("160m",  1_838_000),
    ("80m",   3_576_000),
    ("40m",   7_076_000),
    ("30m",  10_138_000),
    ("20m",  14_076_000),
    ("17m",  18_102_000),
    ("15m",  21_076_000),
    ("12m",  24_917_000),
    ("10m",  28_076_000),
    ("6m",   50_310_000),
];

static Q65_BANDS: &[(&str, u64)] = &[
    ("160m",  1_838_000),
    ("80m",   3_576_000),
    ("40m",   7_076_000),
    ("30m",  10_138_000),
    ("20m",  14_076_000),
    ("17m",  18_102_000),
    ("15m",  21_076_000),
    ("12m",  24_917_000),
    ("10m",  28_076_000),
    ("6m",   50_313_000),
];

static MSK144_BANDS: &[(&str, u64)] = &[
    ("6m",   50_280_000),
    ("2m",  144_360_000),
    ("70cm",432_360_000),
];

static CW_BANDS: &[(&str, u64)] = &[
    ("80m",   3_560_000),
    ("40m",   7_030_000),
    ("30m",  10_106_000),
    ("20m",  14_060_000),
    ("17m",  18_096_000),
    ("15m",  21_060_000),
    ("12m",  24_906_000),
    ("10m",  28_060_000),
];

static FLDIGI_BANDS: &[(&str, u64)] = &[
    ("80m",   3_580_000),
    ("40m",   7_080_000),
    ("30m",  10_140_000),
    ("20m",  14_080_000),
    ("17m",  18_100_000),
    ("15m",  21_080_000),
    ("12m",  24_920_000),
    ("10m",  28_080_000),
];

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

fn ft8_reply_target(message: &str) -> Option<String> {
    let tokens: Vec<&str> = message.split_whitespace().collect();
    let first = *tokens.first()?;
    if first.eq_ignore_ascii_case("CQ") {
        // Handle CQ variants like: "CQ DX K1ABC FN42" by picking the first
        // token that looks like a callsign.
        for t in tokens.iter().skip(1) {
            if ft8_is_probable_callsign(t) {
                return Some(t.to_ascii_uppercase());
            }
        }
        None
    } else {
        Some(first.to_ascii_uppercase())
    }
}

fn ft8_is_probable_callsign(token: &str) -> bool {
    let t = token.trim().to_ascii_uppercase();
    if t.len() < 3 {
        return false;
    }
    match t.as_str() {
        "DX" | "TEST" | "QRZ" | "CQ" | "POTA" | "SOTA" | "NA" | "EU" | "AS" | "AF" | "SA" | "OC" => {
            return false;
        }
        _ => {}
    }
    let has_alpha = t.chars().any(|c| c.is_ascii_alphabetic());
    let has_digit = t.chars().any(|c| c.is_ascii_digit());
    has_alpha && has_digit
}

fn build_ft8_tx_pcm(compose: &str, tx_tone_hz: u32) -> Result<Vec<i16>> {
    let tokens: Vec<&str> = compose.split_whitespace().collect();
    if tokens.len() < 3 {
        anyhow::bail!("FT8 TX needs at least 3 fields (CALL1 CALL2 GRID/REPORT)");
    }
    let msg77 = if tokens[0].eq_ignore_ascii_case("CQ") {
        pack77("CQ", tokens[1], tokens[2])
            .ok_or_else(|| anyhow!("unable to pack FT8 CQ message: {compose}"))?
    } else {
        pack77(tokens[0], tokens[1], tokens[2])
            .ok_or_else(|| anyhow!("unable to pack FT8 standard message: {compose}"))?
    };
    let tones = ft8_message_to_tones(&msg77);
    Ok(ft8_tones_to_i16(&tones, tx_tone_hz as f32, FT8_TX_AMPLITUDE_I16))
}

fn play_ft8_tx_pcm(
    pcm: &[i16],
    abort: Arc<AtomicBool>,
    pid_slot: Arc<Mutex<Option<u32>>>,
) -> Result<()> {
    let mut cmd = Command::new("aplay");
    cmd.arg("-q")
        .arg("-f").arg("S16_LE")
        .arg("-r").arg(FT8_TX_SAMPLE_RATE_HZ.to_string())
        .arg("-c").arg("1")
        .arg("-t").arg("raw")
        .arg("-")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());

    if let Ok(dev) = std::env::var("RIGFORGE_AUDIO_OUTPUT_DEVICE") {
        if !dev.trim().is_empty() {
            cmd.arg("-D").arg(dev);
        }
    }

    let mut child = cmd.spawn().context("failed to spawn aplay for FT8 TX")?;
    {
        let mut slot = pid_slot.lock().expect("tx pid lock poisoned");
        *slot = Some(child.id());
    }
    let mut stdin = child.stdin.take().context("aplay stdin unavailable")?;
    let mut bytes = Vec::with_capacity(pcm.len() * 2);
    for &s in pcm {
        bytes.extend_from_slice(&s.to_le_bytes());
    }

    for chunk in bytes.chunks(4096) {
        if abort.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            *pid_slot.lock().expect("tx pid lock poisoned") = None;
            anyhow::bail!("TX aborted by operator");
        }
        stdin
            .write_all(chunk)
            .context("failed writing FT8 TX PCM to aplay")?;
    }
    drop(stdin);

    loop {
        if abort.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            *pid_slot.lock().expect("tx pid lock poisoned") = None;
            anyhow::bail!("TX aborted by operator");
        }
        if let Some(status) = child.try_wait().context("failed waiting for aplay")? {
            *pid_slot.lock().expect("tx pid lock poisoned") = None;
            if !status.success() {
                anyhow::bail!("aplay TX failed with status {status}");
            }
            return Ok(());
        }
        thread::sleep(Duration::from_millis(20));
    }
}

/// One decoded FT8 frame in the log.
#[derive(Debug, Clone)]
struct Ft8DecodeEntry {
    utc: String,
    snr_db: i8,
    dt_s: f32,
    freq_hz: u32,
    message: String,
    is_cq: bool,
}

#[derive(Debug)]
struct PendingFt8Decode {
    samples: Vec<f32>,
    utc: String,
    deep_decode: bool,
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
    audio_spectrum_status: String,
    audio_waterfall_rows: VecDeque<Vec<u8>>,
    workspace_mode: WorkspaceMode,
    ft8_deep_decode: bool,
    ft8_pending: Vec<Ft8DecodeEntry>,
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
            audio_spectrum_status: "INIT".to_string(),
            audio_waterfall_rows: VecDeque::with_capacity(AUDIO_WF_HEIGHT),
            workspace_mode: WorkspaceMode::Ft8,
            ft8_deep_decode: false,
            ft8_pending: Vec::new(),
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
    Quit,
}

pub fn run_gui(config: AppConfig) -> Result<()> {
    let build_profile = if cfg!(debug_assertions) { "debug" } else { "release" };
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

    info!(
        display_value = ?std::env::var("DISPLAY").ok(),
        wayland_value = ?std::env::var("WAYLAND_DISPLAY").ok(),
        winit_backend_value = ?std::env::var("WINIT_UNIX_BACKEND").ok(),
        wgpu_backend_value = ?std::env::var("WGPU_BACKEND").ok(),
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
    audio_waterfall_texture: Option<TextureHandle>,
    workspace_mode: WorkspaceMode,
    display_tuning: Arc<Mutex<DisplayTuning>>,
    repaint_ctx: Arc<OnceLock<egui::Context>>,
    // FT8 workspace UX state (app-local, not shared with workers)
    ft8_log: Vec<Ft8DecodeEntry>,
    ft8_compose: String,
    ft8_selected: Option<usize>,
    ft8_autoseq: bool,
    ft8_seq_state: Ft8SeqState,
    ft8_seq_target: Option<String>,
    ft8_seq_status: String,
    ft8_last_click: Option<(usize, Instant)>,
    ft8_tx_queued_period: Option<u64>,
    ft8_tx_pcm: Option<Arc<Vec<i16>>>,
    ft8_tx_started_period: Option<u64>,
    ft8_tx_abort: Arc<AtomicBool>,
    ft8_tx_active: Arc<AtomicBool>,
    ft8_tx_aplay_pid: Arc<Mutex<Option<u32>>>,
    ft8_halt_after_tx: bool,
    ft8_hold_tx_freq: bool,
    ft8_deep_decode: bool,
    ft8_cq_only_view: bool,
    ft8_follow_log: bool,
    ft8_max_log_entries: usize,
    station_callsign: String,
    station_grid: String,
    station_qth: String,
    civ_spectrum_on: bool,
    rx_tone_hz: u32,
    tx_tone_hz: u32,
    profile_io_status: String,
    profile_dirty: bool,
}

impl RigforgeGuiApp {
    fn new(mut config: AppConfig) -> Self {
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
                s.last_error = Some("Radio is disabled in config; UI running in monitor-only mode".to_string());
                s.radio_waterfall_status = "UNAVAILABLE (radio disabled)".to_string();
            }
            (None, None)
        };

        let audio_worker_handle = Some(spawn_audio_spectrum_worker(
            state.clone(),
            worker_stop.clone(),
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
        let mut ft8_autoseq = false;
        let mut ft8_cq_only_view = false;
        let mut civ_spectrum_on = false;
        let mut ft8_halt_after_tx = true;
        let mut ft8_hold_tx_freq = true;
        let mut rx_tone_hz = default_rx_tone_hz();
        let mut tx_tone_hz = default_tx_tone_hz();
        let profile_io_status: String;

        if let Some(p) = load_operator_profile() {
            station_callsign = p.callsign;
            station_grid = p.grid;
            station_qth = p.qth;
            ft8_follow_log = p.follow_log;
            ft8_max_log_entries = p.max_log_entries.clamp(80, 1000);
            ft8_deep_decode = p.deep_decode;
            ft8_autoseq = p.autoseq;
            ft8_cq_only_view = p.cq_only_view;
            civ_spectrum_on = p.civ_spectrum_on;
            ft8_halt_after_tx = p.halt_after_tx;
            ft8_hold_tx_freq = p.hold_tx_freq;
            rx_tone_hz = p.rx_tone_hz;
            tx_tone_hz = p.tx_tone_hz;
            config.station.callsign = Some(station_callsign.clone());
            config.station.grid = Some(station_grid.clone());
            profile_io_status = format!("Loaded {}", OPERATOR_PROFILE_FILE);
        } else {
            let bootstrap = OperatorProfile {
                callsign: station_callsign.clone(),
                grid: station_grid.clone(),
                qth: station_qth.clone(),
                follow_log: ft8_follow_log,
                max_log_entries: ft8_max_log_entries,
                deep_decode: ft8_deep_decode,
                autoseq: ft8_autoseq,
                cq_only_view: ft8_cq_only_view,
                civ_spectrum_on,
                halt_after_tx: ft8_halt_after_tx,
                hold_tx_freq: ft8_hold_tx_freq,
                rx_tone_hz,
                tx_tone_hz,
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

        Self {
            config,
            state,
            command_tx,
            worker_stop,
            radio_worker_handle,
            audio_worker_handle,
            radio_waterfall_texture: None,
            audio_waterfall_texture: None,
            workspace_mode: WorkspaceMode::Ft8,
            display_tuning,
            repaint_ctx,
            ft8_log: Vec::new(),
            ft8_compose: String::new(),
            ft8_selected: None,
            ft8_autoseq,
            ft8_seq_state: Ft8SeqState::Idle,
            ft8_seq_target: None,
            ft8_seq_status: "Idle".to_string(),
            ft8_last_click: None,
            ft8_tx_queued_period: None,
            ft8_tx_pcm: None,
            ft8_tx_started_period: None,
            ft8_tx_abort: Arc::new(AtomicBool::new(false)),
            ft8_tx_active: Arc::new(AtomicBool::new(false)),
            ft8_tx_aplay_pid: Arc::new(Mutex::new(None)),
            ft8_halt_after_tx,
            ft8_hold_tx_freq,
            ft8_deep_decode,
            ft8_cq_only_view,
            ft8_follow_log,
            ft8_max_log_entries,
            station_callsign,
            station_grid,
            station_qth,
            civ_spectrum_on,
            rx_tone_hz,
            tx_tone_hz,
            profile_io_status,
            profile_dirty: false,
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

    fn queue_ft8_tx_from_compose(&mut self, policy: Ft8TxQueuePolicy) {
        if self.ft8_compose.trim().is_empty() {
            self.ft8_seq_status = "TX not queued: compose is empty".to_string();
            return;
        }
        self.ft8_tx_abort.store(false, Ordering::Relaxed);
        match build_ft8_tx_pcm(&self.ft8_compose, self.tx_tone_hz) {
            Ok(pcm) => {
                let now_s = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs_f64())
                    .unwrap_or(0.0);
                let period = (now_s / 15.0) as u64;
                let pos = now_s % 15.0;
                let target_period = match policy {
                    Ft8TxQueuePolicy::Standard => {
                        if pos < FT8_TX_SLOT_START_POS_S { period } else { period + 1 }
                    }
                    Ft8TxQueuePolicy::ReplyAsap => {
                        if pos < (FT8_TX_SLOT_START_POS_S + FT8_TX_LAUNCH_WINDOW_S) {
                            period
                        } else {
                            period + 1
                        }
                    }
                    Ft8TxQueuePolicy::NextSlotOnly => period + 1,
                };
                self.ft8_tx_pcm = Some(Arc::new(pcm));
                self.ft8_tx_queued_period = Some(target_period);
                self.ft8_tx_started_period = None;
                self.ft8_seq_state = Ft8SeqState::TxQueued;
                self.ft8_seq_status = match policy {
                    Ft8TxQueuePolicy::ReplyAsap => format!(
                        "Reply queued ASAP for {} (period {})",
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
            }
            Err(err) => {
                self.ft8_seq_status = format!("TX encode failed: {err}");
            }
        }
    }

    fn retune_from_decode_pick(&mut self, freq_hz: u32) {
        let picked = freq_hz.clamp(100, 3_500);
        self.rx_tone_hz = picked;
        if !self.ft8_hold_tx_freq {
            self.tx_tone_hz = picked;
        }
        self.profile_dirty = true;
        self.persist_profile("Auto-saved");
    }

    fn force_stop_tx(&mut self) {
        self.ft8_tx_abort.store(true, Ordering::Relaxed);
        self.ft8_tx_active.store(false, Ordering::Relaxed);

        self.ft8_tx_queued_period = None;
        self.ft8_tx_started_period = None;
        self.ft8_tx_pcm = None;
        self.ft8_seq_target = None;
        self.ft8_seq_state = Ft8SeqState::Idle;
        self.ft8_seq_status = "TX force-stopped".to_string();

        if let Some(tx) = &self.command_tx {
            let _ = tx.send(GuiCommand::SetPtt(false));
        }

        if let Some(pid) = *self.ft8_tx_aplay_pid.lock().expect("tx pid lock poisoned") {
            let _ = Command::new("kill")
                .arg("-TERM")
                .arg(pid.to_string())
                .status();
        }
        *self.ft8_tx_aplay_pid.lock().expect("tx pid lock poisoned") = None;
    }

    fn process_ft8_tx_pipeline(&mut self, snapshot: &GuiState) {
        if self.workspace_mode != WorkspaceMode::Ft8 {
            return;
        }
        let queued_period = match self.ft8_tx_queued_period {
            Some(p) => p,
            None => return,
        };
        let pcm = match &self.ft8_tx_pcm {
            Some(p) => p.clone(),
            None => return,
        };

        let now_s = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);
        let period = (now_s / 15.0) as u64;
        let pos = now_s % 15.0;

        if period > queued_period {
            self.ft8_seq_status = "TX missed scheduled slot; re-queue needed".to_string();
            self.ft8_seq_state = Ft8SeqState::Idle;
            self.ft8_tx_queued_period = None;
            self.ft8_tx_started_period = None;
            self.ft8_tx_pcm = None;
            return;
        }

        if period == queued_period
            && pos >= FT8_TX_SLOT_START_POS_S
            && pos <= (FT8_TX_SLOT_START_POS_S + FT8_TX_LAUNCH_WINDOW_S)
            && self.ft8_tx_started_period != Some(period)
        {
            self.ft8_tx_started_period = Some(period);
            self.ft8_seq_status = "TX slot started".to_string();
            self.ft8_tx_abort.store(false, Ordering::Relaxed);
            self.ft8_tx_active.store(true, Ordering::Relaxed);

            if let Some(tx) = &self.command_tx {
                let txc = tx.clone();
                let p = pcm.clone();
                let abort = self.ft8_tx_abort.clone();
                let active = self.ft8_tx_active.clone();
                let pid_slot = self.ft8_tx_aplay_pid.clone();
                thread::spawn(move || {
                    let _ = txc.send(GuiCommand::SetPtt(true));
                    let _ = play_ft8_tx_pcm(&p, abort.clone(), pid_slot);
                    let _ = txc.send(GuiCommand::SetPtt(false));
                    active.store(false, Ordering::Relaxed);
                });
            }
        }

        if self.ft8_tx_started_period == Some(period)
            && !snapshot.ptt_on
            && !self.ft8_tx_active.load(Ordering::Relaxed)
        {
            if self.ft8_halt_after_tx {
                self.ft8_autoseq = false;
                self.ft8_seq_target = None;
                self.ft8_seq_status = "TX complete (halted)".to_string();
            } else {
                self.ft8_seq_status = "TX complete".to_string();
            }
            self.ft8_seq_state = Ft8SeqState::Idle;
            self.ft8_tx_queued_period = None;
            self.ft8_tx_started_period = None;
            self.ft8_tx_pcm = None;
            self.ft8_tx_abort.store(false, Ordering::Relaxed);
            *self.ft8_tx_aplay_pid.lock().expect("tx pid lock poisoned") = None;
        }
    }

    fn current_operator_profile(&self) -> OperatorProfile {
        OperatorProfile {
            callsign: self.station_callsign_or_default().to_string(),
            grid: self.station_grid_or_default().to_string(),
            qth: self.station_qth.trim().to_string(),
            follow_log: self.ft8_follow_log,
            max_log_entries: self.ft8_max_log_entries.clamp(80, 1000),
            deep_decode: self.ft8_deep_decode,
            autoseq: self.ft8_autoseq,
            cq_only_view: self.ft8_cq_only_view,
            civ_spectrum_on: self.civ_spectrum_on,
            halt_after_tx: self.ft8_halt_after_tx,
            hold_tx_freq: self.ft8_hold_tx_freq,
            rx_tone_hz: self.rx_tone_hz,
            tx_tone_hz: self.tx_tone_hz,
        }
    }

    fn station_callsign_or_default(&self) -> &str {
        let v = self.station_callsign.trim();
        if v.is_empty() { "N0CALL" } else { v }
    }

    fn station_grid_or_default(&self) -> &str {
        let v = self.station_grid.trim();
        if v.is_empty() { "AA00" } else { v }
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
                self.config.station.callsign = if val.is_empty() { None } else { Some(val.to_string()) };
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
                self.config.station.grid = if val.is_empty() { None } else { Some(val.to_string()) };
                self.profile_dirty = true;
                self.persist_profile("Auto-saved");
            }
        });

        ui.horizontal(|ui| {
            ui.label(RichText::new("QTH").strong());
            let qth_changed = ui.add(
                egui::TextEdit::singleline(&mut self.station_qth)
                    .desired_width(ui.available_width())
                    .hint_text("City / locator notes")
                    .font(egui::TextStyle::Monospace),
            ).changed();
            if qth_changed {
                self.profile_dirty = true;
                self.persist_profile("Auto-saved");
            }
        });

        ui.add_space(4.0);
        ui.horizontal(|ui| {
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
                        self.ft8_autoseq = p.autoseq;
                        self.ft8_cq_only_view = p.cq_only_view;
                        self.civ_spectrum_on = p.civ_spectrum_on;
                        self.ft8_halt_after_tx = p.halt_after_tx;
                        self.ft8_hold_tx_freq = p.hold_tx_freq;
                        self.rx_tone_hz = p.rx_tone_hz;
                        self.tx_tone_hz = p.tx_tone_hz;
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
        });
        ui.label(RichText::new(&self.profile_io_status).small().color(Color32::GRAY));
    }

    fn send_command(&self, cmd: GuiCommand) {
        if let Some(tx) = &self.command_tx {
            let _ = tx.send(cmd);
        }
    }

    fn draw_status(&mut self, ui: &mut egui::Ui, snapshot: &GuiState) {
        ui.heading("Live Status");
        ui.separator();

        let freq = snapshot
            .frequency_hz
            .map(|v| format!("{v} Hz"))
            .unwrap_or_else(|| "(unavailable)".to_string());

        ui.label(format!("Frequency: {freq}"));
        ui.label(format!("Mode: {}", snapshot.mode));
        ui.label(format!(
            "PTT: {}",
            if snapshot.ptt_on { "ON" } else { "OFF" }
        ));
        ui.label(format!(
            "Data mode: {}",
            snapshot
                .data_mode
                .map(|v| if v { "ON" } else { "OFF" })
                .unwrap_or("?")
        ));
        ui.label(format!(
            "Filter: {}",
            snapshot
                .filter
                .map(|v| format!("FIL{v}"))
                .unwrap_or_else(|| "?".to_string())
        ));

        ui.add_space(6.0);

        ui.label(format!(
            "AF: {}   RF: {}   Power: {}",
            fmt_opt_u8(snapshot.af_gain),
            fmt_opt_u8(snapshot.rf_gain),
            fmt_opt_u8(snapshot.rf_power)
        ));

        if ui
            .checkbox(&mut self.civ_spectrum_on, "CI-V spectrum waterfall")
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
        ui.label(RichText::new(format!(
            "Radio waterfall: {} ({})",
            snapshot.radio_waterfall_status,
            if snapshot.radio_spectrum_desired {
                if snapshot.radio_spectrum_enabled {
                    "enabled"
                } else {
                    "arming"
                }
            } else {
                "disabled"
            }
        ))
        .color(wf_color));

        ui.label(RichText::new(format!(
            "Audio spectrum: {}",
            snapshot.audio_spectrum_status
        ))
        .color(if snapshot.audio_spectrum_status.contains("LIVE") {
            Color32::LIGHT_GREEN
        } else {
            Color32::YELLOW
        }));

        if let Some(last) = snapshot.last_update {
            ui.label(format!("Last update: {:.1}s ago", last.elapsed().as_secs_f32()));
        }

        if let Some(err) = &snapshot.last_error {
            ui.add_space(6.0);
            ui.label(RichText::new(format!("Last error: {err}")).color(Color32::YELLOW));
        }
    }

    fn draw_radio_waterfall(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, snapshot: &GuiState) {
        ui.heading("Radio Waterfall (CI-V Scope)");
        ui.separator();

        if !snapshot.radio_spectrum_desired {
            ui.label(RichText::new("CI-V spectrum disabled (toggle it on in Live Status)").color(Color32::GRAY));
            return;
        }

        let display_size = egui::vec2(ui.available_width(), RADIO_WF_HEIGHT as f32 * 1.9);

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

        if let Some(tex) = &self.radio_waterfall_texture {
            ui.image((tex.id(), display_size));
        }

        ui.label("Toggleable CI-V scope stream. Palette: blue\u{2192}cyan\u{2192}yellow\u{2192}white");
    }

    fn draw_audio_waterfall(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, snapshot: &GuiState) {
        ui.heading("Audio Waterfall (DSP Input)");
        ui.separator();

        let bw_hz = filter_bandwidth_hz(&snapshot.mode, snapshot.filter);
        let display_bins = ((bw_hz.min(AUDIO_MAX_FREQ_HZ) as f32 / AUDIO_MAX_FREQ_HZ as f32)
            * AUDIO_BINS as f32)
            .round() as usize;
        let display_bins = display_bins.clamp(16, AUDIO_BINS);

        // Capture layout geometry before texture ops — available_width() can change mid-frame.
        let display_size = egui::vec2(ui.available_width(), AUDIO_WF_HEIGHT as f32 * 1.9);

        let image = build_waterfall_image(&snapshot.audio_waterfall_rows, display_bins, AUDIO_WF_HEIGHT, 1.0);
        if let Some(tex) = &mut self.audio_waterfall_texture {
            tex.set(image, TextureOptions::LINEAR);
        } else {
            self.audio_waterfall_texture = Some(ctx.load_texture(
                "rigforge-audio-waterfall",
                image,
                TextureOptions::LINEAR,
            ));
        }
        if let Some(tex) = &self.audio_waterfall_texture {
            let image_widget = egui::Image::new((tex.id(), display_size)).sense(egui::Sense::click());
            let response = ui.add(image_widget);

            if let Some(pos) = response.interact_pointer_pos() {
                let rel = ((pos.x - response.rect.left()) / response.rect.width()).clamp(0.0, 1.0);
                let pick_hz = ((rel * bw_hz.min(AUDIO_MAX_FREQ_HZ) as f32).round() as u32)
                    .clamp(100, bw_hz.min(AUDIO_MAX_FREQ_HZ).max(100));

                if response.clicked() {
                    self.rx_tone_hz = pick_hz;
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

            ui.painter().line_segment(
                [egui::pos2(rx_x, response.rect.top()), egui::pos2(rx_x, response.rect.bottom())],
                egui::Stroke::new(1.5, Color32::from_rgb(120, 220, 120)),
            );
            ui.painter().line_segment(
                [egui::pos2(tx_x, response.rect.top()), egui::pos2(tx_x, response.rect.bottom())],
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
            snapshot.filter.map(|f| format!("FIL{f}")).unwrap_or_default(),
        ));
    }

    fn draw_band_controls(&self, ui: &mut egui::Ui, snapshot: &GuiState) {
        ui.heading(format!("Band / Filter ({})", self.workspace_mode.label()));
        ui.separator();

        let current_hz = snapshot.frequency_hz.unwrap_or(0);
        let band_plan = workspace_band_plan(self.workspace_mode);

        // Band buttons — 3 per row
        ui.horizontal_wrapped(|ui| {
            for &(label, freq_hz) in band_plan {
                let on_band = (current_hz as i64 - freq_hz as i64).unsigned_abs() < 200_000;
                if ui
                    .add(egui::Button::new(
                        RichText::new(label).monospace().strong(),
                    )
                    .fill(if on_band {
                        Color32::from_rgb(30, 80, 30)
                    } else {
                        Color32::from_gray(40)
                    }))
                    .on_hover_text(format!("{:.3} MHz  {}", freq_hz as f64 / 1_000_000.0, self.workspace_mode.label()))
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
                    .add(egui::Button::new(
                        RichText::new(&label).monospace(),
                    )
                    .fill(if active {
                        Color32::from_rgb(20, 60, 120)
                    } else {
                        Color32::from_gray(40)
                    }))
                    .clicked()
                {
                    self.send_command(GuiCommand::SetFilter(fil));
                }
            }
        });
    }

    fn draw_ft8_workspace(&mut self, ui: &mut egui::Ui, _ctx: &egui::Context, snapshot: &GuiState) {
        // ── Header row ──────────────────────────────────────────────────────
        ui.horizontal(|ui| {
            ui.heading("FT8");
            ui.separator();

            // 15-second period progress
            let (progress, is_rx) = ft8_period_progress();
            let period_color = if is_rx {
                Color32::from_rgb(30, 130, 30)
            } else {
                Color32::from_rgb(160, 60, 20)
            };
            let phase_label = if is_rx { "RX" } else { "TX" };
            ui.label(RichText::new(phase_label).strong().color(period_color));
            let bar_w = 140.0;
            let bar_h = 14.0;
            let (rect, _) = ui.allocate_exact_size(egui::vec2(bar_w, bar_h), egui::Sense::hover());
            ui.painter().rect_filled(rect, 2.0, Color32::from_gray(30));
            let fill = egui::Rect::from_min_size(
                rect.min,
                egui::vec2(bar_w * progress, bar_h),
            );
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
            ui.label(RichText::new(format!("RX {} Hz", self.rx_tone_hz)).monospace().color(Color32::from_rgb(120, 220, 120)));
            ui.label(RichText::new(format!("TX {} Hz", self.tx_tone_hz)).monospace().color(Color32::from_rgb(220, 160, 80)));
            ui.separator();
            ui.label(RichText::new(format!("SEQ {}", self.ft8_seq_state.label())).monospace().color(Color32::LIGHT_BLUE));

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let halt_label = if self.ft8_halt_after_tx {
                    "HALT AFTER TX"
                } else {
                    "CONTINUE AFTER TX"
                };
                let halt_color = if self.ft8_halt_after_tx {
                    Color32::from_rgb(220, 180, 80)
                } else {
                    Color32::GRAY
                };
                if ui.button(RichText::new(halt_label).color(halt_color)).clicked() {
                    self.ft8_halt_after_tx = !self.ft8_halt_after_tx;
                    self.profile_dirty = true;
                    self.persist_profile("Auto-saved");
                }

                let hold_label = if self.ft8_hold_tx_freq {
                    "HOLD TX FREQ"
                } else {
                    "TRACK TX=RX"
                };
                let hold_color = if self.ft8_hold_tx_freq {
                    Color32::from_rgb(120, 200, 220)
                } else {
                    Color32::GRAY
                };
                if ui.button(RichText::new(hold_label).color(hold_color)).clicked() {
                    self.ft8_hold_tx_freq = !self.ft8_hold_tx_freq;
                    self.profile_dirty = true;
                    self.persist_profile("Auto-saved");
                }

                let auto_label = if self.ft8_autoseq { "AUTO-SEQ ON" } else { "AUTO-SEQ OFF" };
                let auto_color = if self.ft8_autoseq { Color32::LIGHT_GREEN } else { Color32::GRAY };
                if ui.button(RichText::new(auto_label).color(auto_color)).clicked() {
                    self.ft8_autoseq = !self.ft8_autoseq;
                    self.profile_dirty = true;
                    self.persist_profile("Auto-saved");
                }

                let deep_label = if self.ft8_deep_decode { "DECODE: DEEP" } else { "DECODE: FAST" };
                let deep_color = if self.ft8_deep_decode { Color32::YELLOW } else { Color32::LIGHT_GREEN };
                if ui.button(RichText::new(deep_label).color(deep_color)).clicked() {
                    self.ft8_deep_decode = !self.ft8_deep_decode;
                    self.profile_dirty = true;
                    self.persist_profile("Auto-saved");
                }
            });
        });

        ui.separator();

        let panel_h = ui.available_height();
        let decode_h = (panel_h * 0.62).max(80.0);
        let tx_h = (panel_h * 0.26).max(60.0);

        // ── Decode log ───────────────────────────────────────────────────────
        egui::Frame::dark_canvas(ui.style()).show(ui, |ui| {
            ui.set_min_height(decode_h);
            ui.set_max_height(decode_h);
            ui.horizontal(|ui| {
                ui.label(RichText::new("DECODES").strong());
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
                if ui.add(
                    egui::DragValue::new(&mut self.ft8_max_log_entries)
                        .range(80..=1000)
                        .speed(5),
                ).changed() {
                    self.profile_dirty = true;
                    self.persist_profile("Auto-saved");
                }
                ui.label("rows");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("Clear").clicked() {
                        self.ft8_log.clear();
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
                                RichText::new("Listening… decodes will appear here.")
                                    .color(Color32::from_gray(100)),
                            );
                        });
                        return;
                    }

                    let selected = self.ft8_selected;
                    let mut new_sel = selected;
                    let mut prev_utc: Option<&str> = None;
                    let mut arm_autoseq_from_double_click = false;
                    let mut reply_target_from_double_click: Option<String> = None;
                    let mut picked_freq_from_double_click: Option<u32> = None;
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
                        let text_color = if entry.is_cq {
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

                        let resp = ui.selectable_label(is_sel, row);
                        if resp.clicked() {
                            let now = Instant::now();
                            let synthetic_double = self
                                .ft8_last_click
                                .map(|(idx, t)| idx == i && now.duration_since(t) <= Duration::from_millis(500))
                                .unwrap_or(false);
                            self.ft8_last_click = Some((i, now));

                            new_sel = if is_sel { None } else { Some(i) };

                            if synthetic_double || resp.double_clicked() {
                                // Pre-fill compose with a reply to this call.
                                if let Some(call) = ft8_reply_target(&entry.message) {
                                    let my = self.station_callsign_or_default();
                                    let grid = self.station_grid_or_default();
                                    self.ft8_compose = format!("{call} {my} {grid}");
                                    reply_target_from_double_click = Some(call);
                                }
                                arm_autoseq_from_double_click = true;
                                picked_freq_from_double_click = Some(entry.freq_hz);
                            }
                        }
                    }
                    self.ft8_selected = new_sel;
                    if let Some(freq_hz) = picked_freq_from_double_click {
                        self.retune_from_decode_pick(freq_hz);
                    }
                    if arm_autoseq_from_double_click {
                        self.ft8_autoseq = true;
                        self.ft8_seq_state = Ft8SeqState::ReplyArmed;
                        self.ft8_seq_target = reply_target_from_double_click;
                        self.ft8_seq_status = if let Some(target) = &self.ft8_seq_target {
                            if self.ft8_hold_tx_freq {
                                format!("Reply armed for {target}; RX moved to {} Hz (TX held)", self.rx_tone_hz)
                            } else {
                                format!("Reply armed for {target}; RX/TX set to {} Hz", self.rx_tone_hz)
                            }
                        } else {
                            "Reply armed (no callsign parsed)".to_string()
                        };
                        self.profile_dirty = true;
                        self.persist_profile("Auto-saved");
                        self.profile_io_status = "Auto-seq armed from decode selection".to_string();
                        self.queue_ft8_tx_from_compose(Ft8TxQueuePolicy::ReplyAsap);
                    }
                });
        });

        ui.add_space(4.0);

        // ── TX compose ───────────────────────────────────────────────────────
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_min_height(tx_h);
            ui.horizontal(|ui| {
                ui.label(RichText::new("TX").strong());
                ui.separator();
                let ptt_color = if snapshot.ptt_on { Color32::from_rgb(200, 60, 60) } else { Color32::from_gray(80) };
                if ui.button(RichText::new(if snapshot.ptt_on { "● PTT ON" } else { "○ PTT" }).color(ptt_color)).clicked() {
                    self.send_command(GuiCommand::TogglePtt);
                }
                let tx_active = self.ft8_tx_active.load(Ordering::Relaxed);
                if ui
                    .button(
                        RichText::new("FORCE STOP TX").color(if tx_active {
                            Color32::from_rgb(255, 130, 130)
                        } else {
                            Color32::from_gray(120)
                        }),
                    )
                    .on_hover_text("Drop PTT, cancel queued TX, and stop active aplay output")
                    .clicked()
                {
                    self.force_stop_tx();
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
                if ui.button(RichText::new("SEND").strong().color(Color32::from_rgb(80, 180, 80))).clicked()
                    && !self.ft8_compose.is_empty()
                {
                    self.queue_ft8_tx_from_compose(Ft8TxQueuePolicy::Standard);
                }
            });

            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                let my = self.station_callsign_or_default();
                let grid = self.station_grid_or_default();
                let macros: &[(&str, String)] = &[
                    ("CQ", format!("CQ {my} {grid}")),
                    ("73", format!("73")),
                    ("RR73", format!("RR73")),
                    ("RST+Grid", format!("{my} {grid}")),
                ];
                if ui.small_button("CALL CQ").clicked() {
                    self.ft8_compose = format!("CQ {my} {grid}");
                    self.ft8_autoseq = true;
                    self.ft8_seq_state = Ft8SeqState::CqArmed;
                    self.ft8_seq_target = None;
                    self.ft8_seq_status = "CQ armed (waiting for next slot)".to_string();
                    self.queue_ft8_tx_from_compose(Ft8TxQueuePolicy::NextSlotOnly);
                    self.profile_dirty = true;
                    self.persist_profile("Auto-saved");
                }
                for (label, text) in macros {
                    if ui.small_button(*label).clicked() {
                        self.ft8_compose = text.clone();
                    }
                }
            });

            ui.label(RichText::new(&self.ft8_seq_status).small().color(Color32::GRAY));
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
                        ui.label(format!("{}  {:+}dB  {}Hz  Δt{:.1}s", e.utc, e.snr_db, e.freq_hz, e.dt_s));
                        if e.is_cq {
                            ui.label(RichText::new("CQ").color(Color32::LIGHT_GREEN).strong());
                        }
                    });
                    ui.horizontal_wrapped(|ui| {
                        let my = self.station_callsign_or_default();
                        let grid = self.station_grid_or_default();
                        if let Some(call) = e.message.split_whitespace().nth(1) {
                            if ui.small_button(format!("Reply → {call}")).clicked() {
                                self.ft8_compose = format!("{call} {my} {grid}");
                            }
                            if ui.small_button("Log QSO").clicked() {}
                        }
                    });
                });
            }
        }
    }

    fn draw_mfsk_mode_workspace(&self, ui: &mut egui::Ui, snapshot: &GuiState, mode: WorkspaceMode) {
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
        let (slot_s, status) = match mode {
            WorkspaceMode::Ft8 => ("15 s", "Active decode path"),
            WorkspaceMode::Ft4 => ("7.5 s", "Panel ready — decoder wiring next"),
            WorkspaceMode::Fst4 => ("15–300 s", "Panel ready — decoder wiring next"),
            WorkspaceMode::Wspr => ("120 s", "Panel ready — decoder wiring next"),
            WorkspaceMode::Jt9 => ("60 s", "Panel ready — decoder wiring next"),
            WorkspaceMode::Jt65 => ("60 s", "Panel ready — decoder wiring next"),
            WorkspaceMode::Q65 => ("30/60 s", "Panel ready — decoder wiring next"),
            WorkspaceMode::Msk144 => ("15 s bursts", "Panel ready — decoder wiring next"),
            WorkspaceMode::Cw => ("N/A", "CW is staged to use external Rust decoder backend"),
            WorkspaceMode::Fldigi => ("N/A", "FLDIGI modem bridge is staged via external integration crate"),
        };

        ui.heading(mode.label());
        ui.separator();

        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Backend:").strong());
            ui.label(RichText::new("mfsk-core").monospace().color(Color32::LIGHT_GREEN));
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
        ui.group(|ui| {
            ui.label(RichText::new("Mode Status").strong());
            ui.separator();
            ui.label(status);
            ui.label("Architecture is now unified around external modem backends to reduce maintenance burden.");
            if mode != WorkspaceMode::Ft8 {
                ui.label(RichText::new("FT8 is currently live; this mode tab is scaffolded and ready for fast follow-on decode wiring.").color(Color32::GRAY));
            }
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
            WorkspaceMode::Ft4
            | WorkspaceMode::Fst4
            | WorkspaceMode::Wspr
            | WorkspaceMode::Jt9
            | WorkspaceMode::Jt65
            | WorkspaceMode::Q65
            | WorkspaceMode::Msk144
            | WorkspaceMode::Cw
            | WorkspaceMode::Fldigi => self.draw_mfsk_mode_workspace(ui, snapshot, self.workspace_mode),
        }
    }

    fn draw_radio_control_strip(&self, ui: &mut egui::Ui, snapshot: &GuiState) {
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
                self.send_command(GuiCommand::TogglePtt);
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
                .selectable_label(!tuning.auto_visual && tuning.waterfall_speed == WaterfallSpeed::Slow, "Slow")
                .clicked()
            {
                tuning.auto_visual = false;
                tuning.waterfall_speed = WaterfallSpeed::Slow;
            }
            if ui
                .selectable_label(!tuning.auto_visual && tuning.waterfall_speed == WaterfallSpeed::Mid, "Mid")
                .clicked()
            {
                tuning.auto_visual = false;
                tuning.waterfall_speed = WaterfallSpeed::Mid;
            }
            if ui
                .selectable_label(!tuning.auto_visual && tuning.waterfall_speed == WaterfallSpeed::Fast, "Fast")
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
        // Give background workers a handle so they can trigger repaints directly.
        let _ = self.repaint_ctx.get_or_init(|| ctx.clone());
        // Safety-net repaint in case no worker data arrives for a long time.
        ctx.request_repaint_after(Duration::from_secs(1));

        // Drain FT8 decodes from the shared pending queue into app-local log.
        {
            let mut s = self.state.lock().expect("ui state lock poisoned");
            s.workspace_mode = self.workspace_mode;
            s.ft8_deep_decode = self.ft8_deep_decode;
            s.radio_spectrum_desired = self.civ_spectrum_on;
            if !s.ft8_pending.is_empty() {
                self.ft8_log.extend(s.ft8_pending.drain(..));
            }
        }
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
        self.process_ft8_tx_pipeline(&snapshot);

        egui::TopBottomPanel::top("header").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("RigForge Operator Console");
                ui.separator();
                ui.label(format!(
                    "Callsign: {}",
                    self.station_callsign_or_default()
                ));
                ui.label(format!(
                    "Grid: {}",
                    self.station_grid_or_default()
                ));
                if !self.station_qth.trim().is_empty() {
                    ui.label(format!("QTH: {}", self.station_qth.trim()));
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
                            self.ft8_autoseq = p.autoseq;
                            self.ft8_cq_only_view = p.cq_only_view;
                            self.civ_spectrum_on = p.civ_spectrum_on;
                            self.ft8_halt_after_tx = p.halt_after_tx;
                            self.ft8_hold_tx_freq = p.hold_tx_freq;
                            self.rx_tone_hz = p.rx_tone_hz;
                            self.tx_tone_hz = p.tx_tone_hz;
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
                let status_color = if self.profile_dirty { Color32::YELLOW } else { Color32::GRAY };
                ui.label(RichText::new(&self.profile_io_status).small().color(status_color));
            });
        });

        egui::TopBottomPanel::bottom("radio_strip")
            .resizable(false)
            .default_height(38.0)
            .show(ctx, |ui| {
                self.draw_radio_control_strip(ui, &snapshot);
            });

        egui::SidePanel::left("signals")
            .resizable(true)
            .default_width(760.0)
            .min_width(480.0)
            .show(ctx, |ui| {
                ui.group(|ui| self.draw_status(ui, &snapshot));
                ui.add_space(4.0);
                ui.group(|ui| self.draw_radio_waterfall(ui, ctx, &snapshot));
                ui.add_space(4.0);
                ui.group(|ui| self.draw_audio_waterfall(ui, ctx, &snapshot));
                ui.add_space(4.0);
                ui.group(|ui| self.draw_station_profile(ui));
                ui.add_space(4.0);
                ui.group(|ui| self.draw_band_controls(ui, &snapshot));
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            self.draw_workspace(ui, ctx, &snapshot);
        });
    }
}

impl Drop for RigforgeGuiApp {
    fn drop(&mut self) {
        self.force_stop_tx();
        self.persist_profile("Saved on exit");
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
                    }
                    thread::sleep(Duration::from_millis(250));
                    continue;
                }

                if !spectrum_enabled {
                    match rt.block_on(stream_radio.enable_spectrum_stream(Duration::from_millis(2_500))) {
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
                match rt.block_on(stream_radio.try_scope_waveform_bins_stream(Duration::from_millis(300))) {
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
                    let mode = ticker_state.lock().expect("ui state lock poisoned").mode.clone();
                    let speed = if t.auto_visual {
                        let m = mode.to_ascii_uppercase();
                        if m.contains("DATA") || m.contains("FT8") || m.contains("JS8") || m.contains("RTTY") || m.contains("CW") {
                            WaterfallSpeed::Fast
                        } else {
                            WaterfallSpeed::Mid
                        }
                    } else {
                        t.waterfall_speed
                    };
                    match speed {
                        WaterfallSpeed::Fast => 42,
                        WaterfallSpeed::Mid  => 83,
                        WaterfallSpeed::Slow => 167,
                    }
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
                        let mut s = state.lock().expect("ui state lock poisoned");
                        match rt.block_on(radio.set_ptt(ptt_target)) {
                            Ok(_) => {
                                s.ptt_on = ptt_target;
                                s.last_error = None;
                            }
                            Err(err) => s.last_error = Some(err.to_string()),
                        }
                        poll_radio_core_state(&rt, &radio, &state);
                    }
                    GuiCommand::SetPtt(target) => {
                        let mut s = state.lock().expect("ui state lock poisoned");
                        match rt.block_on(radio.set_ptt(target)) {
                            Ok(_) => {
                                s.ptt_on = target;
                                s.last_error = None;
                            }
                            Err(err) => s.last_error = Some(err.to_string()),
                        }
                        drop(s);
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
                        let workspace_mode = state.lock().expect("ui state lock poisoned").workspace_mode;
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
                        let current = match rt.block_on(radio.get_control(ControlId::AfGain)).ok().flatten() {
                            Some(ControlValue::U8(v)) => v,
                            _ => 100,
                        };
                        let target = if delta.is_negative() {
                            current.saturating_sub(delta.unsigned_abs() as u8)
                        } else {
                            current.saturating_add(delta as u8).min(255)
                        };
                        if let Err(err) = rt.block_on(radio.set_control(ControlId::AfGain, ControlValue::U8(target))) {
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
    let spectrum_enabled = state.lock().expect("ui state lock poisoned").radio_spectrum_enabled;
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
        if let Some(freq) = status.frequency_hz { s.frequency_hz = Some(freq); }
        if let Some(mode) = status.mode { s.mode = mode; }
        if let Some(details) = status.mode_details {
            s.data_mode = Some(details.data_mode);
            s.filter = details.filter;
        }
        s.last_update = Some(Instant::now());
    }
    if let Some(v) = af { s.af_gain = Some(v); }
    if let Some(v) = rf { s.rf_gain = Some(v); }
    if let Some(v) = pwr { s.rf_power = Some(v); }
    if let Some(v) = filt { s.filter = Some(v); }
}

fn apply_waterfall_bins(next: &mut GuiState, bins: &[u8]) {
    let row = downsample_bins(bins, RADIO_WF_WIDTH);
    if next.radio_waterfall_rows.len() >= RADIO_WF_HEIGHT {
        next.radio_waterfall_rows.pop_front();
    }
    next.radio_waterfall_rows.push_back(row);
}

fn read_u8_control(rt: &tokio::runtime::Runtime, radio: &IcomCiVRadio, id: ControlId) -> Option<u8> {
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

fn spawn_audio_spectrum_worker(
    state: Arc<Mutex<GuiState>>,
    stop: Arc<AtomicBool>,
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

        if !command_exists("arecord") {
            let mut s = state.lock().expect("ui state lock poisoned");
            s.audio_spectrum_status = "UNAVAILABLE (arecord missing)".to_string();
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

        // 12 kHz decimation pipeline for FT8 decode
        let can_decode = sample_rate_hz == 48_000;
        let mut decimator = if can_decode { Some(Decimator::new(sample_rate_hz)) } else { None };
        if can_decode {
            warm_ft8_decoder();
        }
        // 15-second accumulation buffer at 12 kHz (180 000 samples)
        let mut ft8_buf: Vec<f32> = Vec::with_capacity(12_000 * 16);
        let mut last_ft8_period: u64 = 0;

        while !stop.load(Ordering::Relaxed) {
            let chunk_samples = {
                let t = display_tuning.lock().expect("tuning lock poisoned");
                let mode = {
                    let s = state.lock().expect("ui state lock poisoned");
                    s.mode.clone()
                };
                let speed = if t.auto_visual {
                    let m = mode.to_ascii_uppercase();
                    if m.contains("DATA") || m.contains("FT8") || m.contains("JS8") || m.contains("RTTY") || m.contains("CW") {
                        WaterfallSpeed::Fast
                    } else {
                        WaterfallSpeed::Mid
                    }
                } else {
                    t.waterfall_speed
                };
                match speed {
                    WaterfallSpeed::Fast => (sample_rate_hz / 24) as usize,
                    WaterfallSpeed::Mid  => (sample_rate_hz / 12) as usize,
                    WaterfallSpeed::Slow => (sample_rate_hz / 6)  as usize,
                }
            };
            let chunk_bytes = (chunk_samples * 2).max(512);
            match stream.read_chunk(chunk_bytes) {
                Ok(samples) => {
                    let samples_f32: Vec<f32> = samples.iter().map(|&s| s as f32 / i16::MAX as f32).collect();
                    // ── Display ring buffer + FFT ──────────────────────────
                    for &x in &samples_f32 { ring.push_back(x); }
                    while ring.len() > FFT_SIZE { ring.pop_front(); }
                    let nfill = ring.len();
                    for (i, b) in fft_buf.iter_mut().enumerate() {
                        *b = if i < nfill {
                            let w = 0.5 - 0.5 * (2.0 * PI * i as f32 / (nfill.max(2) - 1) as f32).cos();
                            Complex::new(ring[i] * w, 0.0)
                        } else {
                            Complex::new(0.0, 0.0)
                        };
                    }
                    audio_fft.process(&mut fft_buf);
                    let bins = fft_buffer_to_display_bins(&fft_buf, AUDIO_BINS, sample_rate_hz);
                    {
                        let mut s = state.lock().expect("ui state lock poisoned");
                        if s.audio_waterfall_rows.len() >= AUDIO_WF_HEIGHT {
                            s.audio_waterfall_rows.pop_front();
                        }
                        s.audio_waterfall_rows.push_back(bins);
                        s.audio_spectrum_status = "LIVE".to_string();
                    }

                    // ── FT8 decode accumulator ─────────────────────────────
                    if let Some(ref mut dec) = decimator {
                        let active_workspace_mode = state.lock().expect("ui state lock poisoned").workspace_mode;
                        if active_workspace_mode == WorkspaceMode::Ft8 {
                            let ds = dec.process(&samples_f32);
                            ft8_buf.extend_from_slice(&ds);
                            // Keep at most 15 seconds worth of 12 kHz samples
                            let max_buf = 12_000 * 15;
                            if ft8_buf.len() > max_buf {
                                ft8_buf.drain(..ft8_buf.len() - max_buf);
                            }
                            // Trigger at the FIRST chunk of each new 15-second period.
                            // No period_pos window: current_period change fires within one
                            // chunk duration (~42 ms) of the boundary, when the rolling buffer
                            // still contains the complete previous period's signal.
                            let now_s = SystemTime::now()
                                .duration_since(UNIX_EPOCH)
                                .map(|d| d.as_secs_f64())
                                .unwrap_or(0.0);
                            let current_period = (now_s / 15.0) as u64;
                            // Wait for a near-full capture window before first/ongoing decode.
                            // This avoids startup partial-period triggers that produce edge-lag
                            // sync artifacts and waste decode passes.
                            if current_period != last_ft8_period && ft8_buf.len() >= max_buf - 1_200 {
                                last_ft8_period = current_period;
                                let utc = utc_hhmmss_millis(now_s - 15.0);
                                let deep_decode = state.lock().expect("ui state lock poisoned").ft8_deep_decode;
                                let pending = PendingFt8Decode {
                                    samples: ft8_buf.clone(),
                                    utc,
                                    deep_decode,
                                };
                                let in_progress = decode_in_progress.clone();
                                if in_progress
                                    .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
                                    .is_ok()
                                {
                                    info!(
                                        buf_samples = pending.samples.len(),
                                        utc = %pending.utc,
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
                                    info!("FT8 decode deferred: previous decode pass still running");
                                }
                            }
                        }
                    }

                    if let Some(ctx) = repaint_ctx.get() {
                        ctx.request_repaint();
                    }
                }
                Err(err) => {
                    state.lock().expect("ui state lock poisoned").audio_spectrum_status =
                        format!("NO INPUT ({err})");
                    thread::sleep(Duration::from_millis(100));
                }
            }
        }
    })
}

fn warm_ft8_decoder() {
    let warmup_audio = vec![0i16; 12_000 * 15];
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
            pending.deep_decode,
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
fn run_ft8_decode(samples: Vec<f32>, state: Arc<Mutex<GuiState>>, utc: String, deep_decode: bool) -> u128 {
    let started = Instant::now();
    let audio_i16: Vec<i16> = samples
        .iter()
        .map(|&x| {
            let s = x.clamp(-1.0, 1.0);
            (s * i16::MAX as f32).round() as i16
        })
        .collect();

    // mfsk-core FT8 decode (12 kHz slot-aligned audio), mapped to the
    // library's WSJT-X depth presets for clearer latency/recall behavior.
    let outcome = if deep_decode {
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
    };

    let mut results: Vec<Ft8DecodeEntry> = Vec::new();
    for r in &outcome.results {
        if let Some(msg) = unpack77(r.message77()) {
            let is_cq = msg.starts_with("CQ");
            let snr = r.snr_db.round() as i8;
            debug!(
                freq = r.freq_hz,
                dt_s = r.dt_sec,
                snr,
                msg,
                "FT8 decode OK"
            );
            results.push(Ft8DecodeEntry {
                utc: utc.clone(),
                snr_db: snr,
                dt_s: r.dt_sec,
                freq_hz: r.freq_hz.max(0.0).round() as u32,
                message: msg,
                is_cq,
            });
        }
    }

    let elapsed_ms = started.elapsed().as_millis() as u128;
    info!(
        deep_decode,
        decoded = results.len(),
        elapsed_ms = elapsed_ms as u64,
        over_slot = elapsed_ms > FT8_SLOT_MS,
        "FT8 decode pass complete"
    );

    if !results.is_empty() {
        let mut s = state.lock().expect("ui state lock poisoned");
        s.ft8_pending.extend(results);
    }

    elapsed_ms
}

fn ft8_period_progress() -> (f32, bool) {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);
    let pos = secs % 15.0;
    ((pos / 15.0) as f32, pos < 12.64)
}

fn filter_bandwidth_hz(mode: &str, filter: Option<u8>) -> u32 {
    let f = filter.unwrap_or(1);
    let m = mode.to_ascii_uppercase();
    if m.contains("CW") {
        match f { 1 => 500, 2 => 250, 3 => 100, _ => 500 }
    } else if m.contains("FM") {
        match f { 1 => 15_000, 2 => 10_000, 3 => 7_000, _ => 15_000 }
    } else if m.contains("RTTY") {
        match f { 1 => 500, 2 => 350, 3 => 250, _ => 500 }
    } else {
        // USB / LSB / Data — IC-7300 defaults
        match f { 1 => 3_000, 2 => 2_400, 3 => 1_800, _ => 3_000 }
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
    if m.contains("DATA") || m.contains("FT8") || m.contains("JS8") || m.contains("RTTY") || m.contains("CW") {
        (45, 0)
    } else if m.contains("FM") {
        (120, 1)
    } else {
        (90, 1)
    }
}

fn is_transient_civ_read_error(message: &str) -> bool {
    let m = message.to_ascii_lowercase();
    m.contains("failed to read ci-v response")
        || m.contains("timed out")
        || m.contains("timeout")
}

fn command_exists(cmd: &str) -> bool {
    Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {} >/dev/null 2>&1", cmd))
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn build_waterfall_image(rows: &VecDeque<Vec<u8>>, width: usize, height: usize, gamma: f32) -> ColorImage {
    let mut pixels = vec![Color32::BLACK; width * height];
    let empty_row = vec![0u8; width];
    let missing = height.saturating_sub(rows.len());
    for y in 0..height {
        let src_row = if y < missing { &empty_row } else { rows.get(y - missing).unwrap_or(&empty_row) };
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
        bytes.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect()
    }

    #[test]
    fn decode_pcm_samples_handles_little_endian_i16() {
        let bytes = [0x00, 0x00, 0x01, 0x00, 0xFF, 0xFF, 0xFE, 0xFF];
        let samples = decode_pcm_samples(&bytes);
        assert_eq!(samples, vec![0i16, 1, -1, -2]);
    }

    #[test]
    fn apply_waterfall_bins_caps_rows_and_preserves_latest() {
        let mut state = GuiState::default();
        for i in 0..RADIO_WF_HEIGHT + 3 {
            apply_waterfall_bins(&mut state, &[i as u8; 8]);
        }

        assert_eq!(state.radio_waterfall_rows.len(), RADIO_WF_HEIGHT);
        assert_eq!(state.radio_waterfall_rows.back().unwrap()[0], (RADIO_WF_HEIGHT + 2) as u8);
    }

    #[test]
    fn compute_audio_spectrum_bins_returns_expected_length() {
        fn compute_audio_spectrum_bins(samples: &[i16], bins: usize, sample_rate_hz: u32) -> Vec<u8> {
            let n = samples.len().min(FFT_SIZE).max(2);
            let mut planner = FftPlanner::<f32>::new();
            let fft = planner.plan_fft_forward(n);
            let mut buf: Vec<Complex<f32>> = samples.iter().take(n).enumerate().map(|(i, &s)| {
                let w = 0.5 - 0.5 * (2.0 * PI * i as f32 / (n - 1) as f32).cos();
                Complex::new(s as f32 / i16::MAX as f32 * w, 0.0)
            }).collect();
            fft.process(&mut buf);
            fft_buffer_to_display_bins(&buf, bins, sample_rate_hz)
        }
        let bins = compute_audio_spectrum_bins(&[0i16; 256], AUDIO_BINS, 48_000);
        assert_eq!(bins.len(), AUDIO_BINS);
        assert!(bins.iter().all(|&v| v <= u8::MAX));
    }

}
