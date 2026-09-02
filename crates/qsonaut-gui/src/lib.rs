mod activity;
mod automation_hunter;
mod automation_integration;
mod band_plan;
mod contest;
mod decode_model;
mod graphics;
mod hostbridge_radio;
mod local_ai;
mod modes;
mod panels;
mod profile;
mod profile_manager;
mod radio_faq;
mod radio_runtime;
mod rendering;
mod reporting;
mod runtime;
mod server_integration;
mod tx_audio;
mod ui_format;
mod ui_widgets;
mod visuals;
mod window_geometry;
mod workers;

use anyhow::{anyhow, Context, Result};
use eframe::egui::{self, Color32, ColorImage, RichText, TextureHandle, TextureOptions};
use hostbridge_radio::{remote_media_queue, HostBridgeRadio, RadioHandle};
use qsonaut_accelerate::{
    AccelerationReport, ActiveBackend, ComputePreference, DecodeTelemetry, DecodeTrace,
};
use qsonaut_audio::{play_pcm_blocking, AudioService, NULL_INPUT_DEVICE, NULL_OUTPUT_DEVICE};
use qsonaut_automation::{
    Action, AutomationEvent, AutomationHost, Capability, CapabilitySet, EventKind,
    ExternalSourceConfig, RuleComponent, RuleComponentConfig,
};
use qsonaut_core::{
    AppConfig, AppEvent, AppEventBus, AudioConfig, ContestOperatingMode, ContestProfile,
    FoxHoundRole, RadioConfig, SplitPolicy,
};
use qsonaut_hostbridge_client::{HostBridgeClient, HostBridgeConfig, HostBridgeEvent};
use qsonaut_hostbridge_protocol::HostHello;
use qsonaut_log::{
    app_config_dir, clear_log, hamdb_cache_path, log_file_path, read_log_tail, AdifExportFilter,
    HamDbCache, HamDbCacheEntry, QsoLog, QsoRecord,
};
use qsonaut_pskreporter::{
    ReceptionReport, ReportSender, Reporter, ReporterConfig, ReporterTuning,
};
use qsonaut_radio::{
    drivers::{
        open_dxlab, open_model_with_radio_address, open_null, open_rigctld, ConfiguredRadio,
    },
    enumerate_serial_port_descriptors,
    models::{find_model, Manufacturer, Protocol, POPULAR_RADIOS},
    BaseMode, ControlId, ControlValue, IcomCiVRadio, MeterId, Mode, OperatingMode, Radio,
    SerialPortDescriptor, TunerStatus,
};
use qsonaut_server_client::{
    log_idempotency_key, new_instance_id, ConnectionConfig as ServerConnectionConfig,
    ConnectionState as ServerConnectionState, Presence as ServerPresence, ServerClient,
};
use qsonaut_third_party::sstv as qsonaut_sstv;
use rustfft::{num_complex::Complex, FftPlanner};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::f32::consts::PI;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::broadcast::error::TryRecvError;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

const QSONAUT_ICON_PNG: &[u8] = include_bytes!("../../../assets/branding/qsonaut-icon.png");
const QSONAUT_GITHUB_URL: &str = "https://github.com/nicksbar/QSONaut";
const QSONAUT_ISSUES_URL: &str = "https://github.com/nicksbar/QSONaut/issues";
const QSONAUT_WEBSITE_URL: &str = "https://qsonaut.com";
const AUDIO_FAQ: &str = include_str!("../../../docs/audio_faq.md");

#[derive(Clone, Debug, Default, Deserialize)]
pub(crate) struct QsonautPerson {
    pub(crate) name: Option<String>,
    pub(crate) callsign: Option<String>,
    pub(crate) grid: Option<String>,
    pub(crate) power_dbm: Option<i8>,
    pub(crate) role: Option<String>,
    #[serde(default)]
    pub(crate) modes: Vec<String>,
    #[serde(default = "default_enabled")]
    pub(crate) enabled: bool,
}

fn default_enabled() -> bool {
    true
}

fn qsonaut_people(raw: Option<&'static str>) -> Vec<QsonautPerson> {
    let Some(raw) = raw.filter(|value| !value.trim().is_empty()) else {
        return Vec::new();
    };
    serde_json::from_str(raw).unwrap_or_default()
}

pub(crate) fn qsonaut_demo_people() -> Vec<QsonautPerson> {
    qsonaut_people(option_env!("QSONAUT_CONTRIBUTORS"))
        .into_iter()
        .chain(qsonaut_people(option_env!("QSONAUT_TESTERS")))
        .filter(|person| person.enabled)
        .collect()
}

fn qsonaut_credit_text(raw: Option<&'static str>) -> String {
    let Some(raw) = raw.filter(|value| !value.trim().is_empty()) else {
        return "None listed".to_string();
    };
    let people = qsonaut_people(Some(raw));
    if people.is_empty() {
        return raw.to_string();
    }
    people
        .into_iter()
        .filter(|person| person.enabled)
        .map(|person| {
            let identity = match (person.name, person.callsign) {
                (Some(name), Some(callsign)) => format!("{name} ({callsign})"),
                (Some(name), None) | (None, Some(name)) => name,
                (None, None) => "Unnamed contributor".to_string(),
            };
            person
                .role
                .filter(|role| !role.trim().is_empty())
                .map_or(identity.clone(), |role| format!("{identity} · {role}"))
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn qsonaut_contributors() -> String {
    qsonaut_credit_text(option_env!("QSONAUT_CONTRIBUTORS"))
}

fn qsonaut_testers() -> String {
    qsonaut_credit_text(option_env!("QSONAUT_TESTERS"))
}

fn effective_audio_input_device(backend: &str, input: Option<String>) -> Option<String> {
    if backend.eq_ignore_ascii_case("hostbridge") {
        Some("hostbridge://capture".to_string())
    } else {
        input
    }
}

fn effective_audio_output_device(backend: &str, output: Option<String>) -> Option<String> {
    if backend.eq_ignore_ascii_case("hostbridge") {
        Some("hostbridge://playback".to_string())
    } else {
        output
    }
}

use activity::{draw_activity_icon, OperatingActivity};
use automation_hunter::{
    AchievementKind, CustomAchievementRule, ExternalSendRecord, HunterAlert, HunterMetric,
};
pub(crate) use automation_integration::{
    bootstrap_automation_host, external_source_transport, normalize_app_event_for_automation,
};
#[cfg(test)]
use automation_integration::{configured_external_transports, parse_automation_hook_detail};
pub(crate) use band_plan::workspace_frequency_for_current_band;
use band_plan::{
    band_for_frequency, band_picker_plan, workspace_radio_preset,
    workspace_radio_preset_for_frequency, WorkspaceMode, WORKSPACE_MODES,
};
use decode_model::{
    digital_activity_stats, ft8_activity_stats, operator_call_hit, DigitalDecodeEntry,
    DigitalSlotGate, Ft8DecodeEntry, Ft8SlotGate, OperatorCallHit, PendingFt8Decode, PotaSpot,
};
pub use graphics::{
    GraphicsAdapterInfo, GraphicsPowerPreference, GraphicsPreferences, GRAPHICS_ADAPTER_ENV,
    GRAPHICS_POWER_ENV,
};
use local_ai::{
    LocalImageEvent, LocalImageProvider, LocalImageSettings, LocalModelInfo, LocalModelRole,
};
use modes::exchange::{
    callsign_eq, is_probable_callsign, next_reply_period, next_tx_period, parse_message,
    select_candidate, should_finalize_after_tx, should_repeat_cq, should_retry_after_decode,
    AutoReplyPolicy, AutoTxStopPolicy, Exchange, ParsedMessage, QsoSession, QsoStage,
    ReplyCandidate, SLOT_SECONDS,
};
pub(crate) use modes::ft8_types::{Ft8SeqState, Ft8TxQueuePolicy, PendingManualFt8Reply};
use modes::voice::VoiceContestField;
use profile::{
    active_operator_profile_name, default_contest_fake_split_offset_hz, default_cw_tone_hz,
    default_cw_wpm, default_max_attempts as default_ft8_max_attempts,
    default_psk_batch_interval_secs, default_psk_max_pending, default_psk_repeat_cache_secs,
    default_ptt_lead_ms, default_ptt_tail_ms, default_rx_tone_hz, default_tx_tone_hz,
    default_waterfall_deck_height, list_operator_profiles, load_global_settings,
    load_operator_profile, load_operator_profile_named, load_radio_profile_library,
    save_operator_profile, save_operator_profile_named, save_radio_profile_library,
    select_operator_profile, OperatorProfile, RadioProfile, OPERATOR_PROFILE_FILE,
    OPERATOR_PROFILE_VERSION,
};
use radio_faq::{help_for_model, render_document};
#[cfg(test)]
use radio_runtime::spawn_radio_init;
use radio_runtime::{
    join_handle_for_shutdown, radio_config_from_operator_profile, request_radio_session_stop,
    spawn_radio_init_with_hostbridge, stop_radio_session,
};
use rendering::{
    band_edges_for_frequency, draw_primary_meter, draw_voltage_graph, effective_visual_profile,
    filter_bandwidth_hz, meter_color, meter_color_for_context, meter_label, meter_percent,
    meter_reading, meter_reading_for_model, meter_tooltip, meter_value, mode_meter_order,
    native_channel_width_hz, record_voltage_sample, scope_projection_for_mode,
    scope_span_for_filter, scope_span_hz, scope_span_label, sideband_scope_edges, status_color,
    theme_accent, theme_muted, theme_success, theme_warning, METER_LABEL_WIDTH,
    VOLTAGE_HISTORY_CAPACITY,
};
use reporting::{
    enrich_qso_from_hamdb, qso_adif_path, qso_log_path, qso_timestamp, spawn_hamdb_lookup,
    start_psk_reporter, submit_psk_report,
};
pub(crate) use runtime::constructor::{
    audio_config_from_operator_profile, configure_unix_gui_environment, spawn_acceleration_probe,
    spawn_device_scan,
};
#[cfg(test)]
use tx_audio::FT8_TX_AUDIO_START_S;
use tx_audio::{
    build_ft8_tx_pcm, build_native_digital_tx_pcm, run_digital_tx_job, run_ft8_tx_job,
    DigitalTxChatEntry, DigitalTxEvent, DigitalTxJob, Ft8ChatDirection, Ft8ChatLine,
    Ft8TxChatEntry, Ft8TxEvent, Ft8TxJob,
};
use ui_format::{format_signal_report, ft8_period_progress, qso_stage_label, utc_hhmmss_millis};
pub(crate) use ui_widgets::radio_control_max;
use ui_widgets::{
    draw_ai_icon, draw_radio_about_icon, draw_speaker_icon, format_swr_display,
    native_radio_profile, radio_baud_rates, radio_supports_band, styled_selection_button,
    swr_chart_value,
};
use visuals::{
    audio_cursor_level, build_scope_waterfall_image, downsample_bins, fft_buffer_to_display_bins,
    scale_scope_levels,
};
use window_geometry::WindowGeometry;
#[cfg(test)]
use workers::decode::{
    prepare_early_digital_slot, prepare_early_ft8_slot, run_native_digital_decode,
};
#[cfg(test)]
use workers::radio::apply_waterfall_bins;
use workers::spawn_audio_spectrum_worker;

const RADIO_WF_WIDTH: usize = 360;
const RADIO_WF_HEIGHT: usize = 180;
const MAX_RADIO_WF_BINS: usize = 1_024;
const AUDIO_BINS: usize = 512;
const AUDIO_WF_HEIGHT: usize = 120;
const AUDIO_MAX_FREQ_HZ: u32 = 4_000;
// 8192 samples @ 48 kHz = 170 ms window, ~5.9 Hz/bin, ~683 useful bins for 0-4 kHz.
const FFT_SIZE: usize = 8192;
const AUDIO_MONITOR_PROFILE_VERSION: u32 = 12;
const GUI_SCALE_BASE: f32 = 1.2;
const GUI_SCALE_MAX: f32 = 2.0;
const GUI_SCALE_MIN: f32 = 0.45;
const QSO_LOG_FILE: &str = "log.toml";
const QSO_ADIF_FILE: &str = "log.adi";
const HAMDB_CACHE_TTL_SECONDS: u64 = 30 * 24 * 60 * 60;
// The generated waveform starts at +0.5 s and ends at about +13.14 s.
const FT8_EARLY_DECODE_S: f64 = 13.2;
const FT8_SLOT_SAMPLES: usize = 12_000 * 15;
// The shared WSJT adapter's WSJT-X depth/recall ladder is calibrated at 1.3. In particular,
// D2 scales this to WSJT-X's 2.0 early-pass threshold; using 1.9 here had
// unintentionally raised the early gate to ~2.92 and discarded weak signals.
const FT8_FAST_SYNC_MIN: f32 = 1.3;
const FT8_FAST_MAX_CAND: usize = 96;
const FT8_ADAPTIVE_OFFSET_LIMIT_S: f32 = 2.5;
const FT4_SLOT_SECONDS: f64 = 7.5;
const FT4_SLOT_SAMPLES: usize = 12_000 * 15 / 2;
// FT4 occupies 103 x 48 ms after its nominal +0.5 s start.
const FT4_EARLY_DECODE_S: f64 = 6.6;
const FT4_ADAPTIVE_OFFSET_LIMIT_S: f32 = 1.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum SignalPanelTab {
    #[default]
    Achievements,
    Station,
    Contest,
    Reporting,
    Settings,
    Ai,
    Server,
    RadioTuning,
    AppLog,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum ProfileDrawerTab {
    #[default]
    Profile,
    Radio,
    Tuning,
    DigitalTiming,
    Monitoring,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RadioHelpDocument {
    Audio,
    Manufacturer,
    Model,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum AppLogLevelFilter {
    #[default]
    All,
    Info,
    Warning,
    Error,
}

impl AppLogLevelFilter {
    const ALL: [Self; 4] = [Self::All, Self::Info, Self::Warning, Self::Error];

    const fn label(self) -> &'static str {
        match self {
            Self::All => "All levels",
            Self::Info => "Info+",
            Self::Warning => "Warnings+",
            Self::Error => "Errors only",
        }
    }
}

fn default_true() -> bool {
    true
}

#[cfg(target_os = "windows")]
const PLATFORM_GUI_SCALE_BASE: f32 = GUI_SCALE_BASE * 0.75;
#[cfg(not(target_os = "windows"))]
const PLATFORM_GUI_SCALE_BASE: f32 = GUI_SCALE_BASE;

fn platform_gui_scale_from_percent(percent: u32) -> f32 {
    (PLATFORM_GUI_SCALE_BASE * percent as f32 / 100.0).clamp(GUI_SCALE_MIN, GUI_SCALE_MAX)
}

fn platform_gui_scale_percent(scale: f32) -> f32 {
    scale / PLATFORM_GUI_SCALE_BASE * 100.0
}

const OS_DPI_ADJUSTMENT_MIN: f32 = 0.75;
const OS_DPI_ADJUSTMENT_MAX: f32 = 1.50;

fn os_dpi_adjustment() -> (f32, &'static str, &'static str) {
    #[cfg(target_os = "windows")]
    const OS_NAME: &str = "Windows";
    #[cfg(target_os = "windows")]
    const ENV_NAME: &str = "QSONAUT_WINDOWS_DPI_ADJUSTMENT";
    #[cfg(target_os = "linux")]
    const OS_NAME: &str = "Linux";
    #[cfg(target_os = "linux")]
    const ENV_NAME: &str = "QSONAUT_LINUX_DPI_ADJUSTMENT";
    #[cfg(target_os = "macos")]
    const OS_NAME: &str = "macOS";
    #[cfg(target_os = "macos")]
    const ENV_NAME: &str = "QSONAUT_MACOS_DPI_ADJUSTMENT";
    #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
    const OS_NAME: &str = "Other";
    #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
    const ENV_NAME: &str = "QSONAUT_DPI_ADJUSTMENT";

    let adjustment = std::env::var(ENV_NAME)
        .ok()
        .and_then(|value| value.trim().parse::<f32>().ok())
        .unwrap_or(1.0)
        .clamp(OS_DPI_ADJUSTMENT_MIN, OS_DPI_ADJUSTMENT_MAX);
    (adjustment, OS_NAME, ENV_NAME)
}

fn parse_workspace_mode_token(mode: &str) -> Option<WorkspaceMode> {
    match mode.trim().to_ascii_uppercase().as_str() {
        "FT8" => Some(WorkspaceMode::Ft8),
        "FT4" => Some(WorkspaceMode::Ft4),
        "FST4" => Some(WorkspaceMode::Fst4),
        "WSPR" => Some(WorkspaceMode::Wspr),
        "JT9" => Some(WorkspaceMode::Jt9),
        "JT65" => Some(WorkspaceMode::Jt65),
        "Q65" => Some(WorkspaceMode::Q65),
        "MSK144" => Some(WorkspaceMode::Msk144),
        "CW" => Some(WorkspaceMode::Cw),
        "VOICE" | "SSB" | "PHONE" => Some(WorkspaceMode::Voice),
        "SSTV" => Some(WorkspaceMode::Sstv),
        _ => None,
    }
}

fn radio_mode_label(mode: &str, data_mode: Option<bool>) -> String {
    // The radio's mode response is authoritative when it already includes
    // the data suffix. A separate DataMode query can briefly lag or report a
    // stale value while the IC-7300 is settling; allowing that value to erase
    // an explicit suffix makes the mode button flicker between USB and USB-D.
    if mode.ends_with("-D") {
        return mode.to_string();
    }
    let base_mode = mode.strip_suffix("-D").unwrap_or(mode);
    match data_mode {
        Some(false) => base_mode.to_string(),
        Some(true) if !mode.ends_with("-D") => format!("{base_mode}-D"),
        _ => mode.to_string(),
    }
}

fn workspace_mode_supports_native_tx(mode: WorkspaceMode) -> bool {
    matches!(
        mode,
        WorkspaceMode::Ft4
            | WorkspaceMode::Fst4
            | WorkspaceMode::Jt9
            | WorkspaceMode::Jt65
            | WorkspaceMode::Q65
            | WorkspaceMode::Cw
            | WorkspaceMode::Sstv
    )
}

fn parse_tx_target_from_compose(compose: &str, operator_call: &str) -> Option<String> {
    let parsed = parse_message(compose)?;
    if parsed.is_cq {
        return None;
    }

    let operator_call = operator_call.trim();
    if callsign_eq(&parsed.from, operator_call) {
        return parsed.to;
    }
    if parsed
        .to
        .as_deref()
        .is_some_and(|to| callsign_eq(to, operator_call))
    {
        return Some(parsed.from);
    }
    parsed.to.or(Some(parsed.from))
}

fn parse_bool_env(var: &str) -> bool {
    std::env::var(var)
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on" | "enabled"
            )
        })
        .unwrap_or(false)
}

fn should_move_tx_to_decode(message: &ParsedMessage, continuing_exchange: bool) -> bool {
    !continuing_exchange && message.is_cq
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
enum WaterfallSpeed {
    Slow,
    Mid,
    Fast,
}

impl WaterfallSpeed {
    fn label(self) -> &'static str {
        match self {
            Self::Slow => "Slow · ~4.5 rows/s",
            Self::Mid => "Mid · ~8 rows/s",
            Self::Fast => "Fast · ~20 rows/s",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
enum RadioScopeView {
    #[default]
    Narrow,
    Overview,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScopeProjection {
    Full,
    LowerSideband,
    UpperSideband,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
enum WaterfallTheme {
    #[default]
    RadioBlue,
    Inferno,
    Phosphor,
    Monochrome,
}

impl WaterfallTheme {
    fn label(self) -> &'static str {
        match self {
            Self::RadioBlue => "Radio blue",
            Self::Inferno => "Inferno",
            Self::Phosphor => "Phosphor",
            Self::Monochrome => "Monochrome",
        }
    }
}

#[derive(Debug, Clone)]
struct DisplayTuning {
    audio_auto_visual: bool,
    audio_waterfall_speed: WaterfallSpeed,
    radio_auto_visual: bool,
    radio_waterfall_speed: WaterfallSpeed,
}

impl Default for DisplayTuning {
    fn default() -> Self {
        Self {
            audio_auto_visual: true,
            audio_waterfall_speed: WaterfallSpeed::Mid,
            radio_auto_visual: true,
            radio_waterfall_speed: WaterfallSpeed::Mid,
        }
    }
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

#[derive(Debug, Clone)]
struct ReceivedSstvImage {
    id: String,
    path: Option<String>,
    mode: Option<qsonaut_sstv::SstvMode>,
    frequency_hz: Option<u64>,
    width: usize,
    height: usize,
    rgb: Vec<u8>,
    received_unix_ms: u128,
    analysis: Option<String>,
    debug_audio_path: Option<String>,
    debug_metadata_path: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SstvOverlayCorner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl SstvOverlayCorner {
    const ALL: [Self; 4] = [
        Self::TopLeft,
        Self::TopRight,
        Self::BottomLeft,
        Self::BottomRight,
    ];

    const fn label(self) -> &'static str {
        match self {
            Self::TopLeft => "Top left",
            Self::TopRight => "Top right",
            Self::BottomLeft => "Bottom left",
            Self::BottomRight => "Bottom right",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SstvAiPipelineMode {
    StationQsl,
    AnalyzeReceived,
    ReinterpretReceived,
}

impl SstvAiPipelineMode {
    const ALL: [Self; 3] = [
        Self::StationQsl,
        Self::AnalyzeReceived,
        Self::ReinterpretReceived,
    ];

    const fn label(self) -> &'static str {
        match self {
            Self::StationQsl => "Station QSL",
            Self::AnalyzeReceived => "Analyze received",
            Self::ReinterpretReceived => "Reinterpret received",
        }
    }
}

#[derive(Debug, Clone)]
struct GuiState {
    frequency_hz: Option<u64>,
    /// Active VFO selector: 0 = A, 1 = B. Drivers without reliable readback
    /// remain on the safe startup assumption of VFO A.
    active_vfo: u8,
    mode: String,
    data_mode: Option<bool>,
    filter: Option<u8>,
    af_gain: Option<u8>,
    rf_gain: Option<u8>,
    rf_power: Option<u8>,
    rf_power_write_pending: Option<u8>,
    squelch: Option<u8>,
    preamp: Option<u8>,
    attenuator: Option<u8>,
    noise_blank: Option<bool>,
    noise_reduction: Option<bool>,
    noise_reduction_level: Option<u8>,
    ip_plus: Option<bool>,
    notch_auto: Option<bool>,
    notch_manual: Option<bool>,
    agc: Option<u8>,
    swr: Option<u8>,
    signal_meter: Option<u8>,
    power_meter: Option<u8>,
    alc_meter: Option<u8>,
    compression_meter: Option<u8>,
    current_meter: Option<u8>,
    voltage_meter: Option<u8>,
    voltage_history: VecDeque<u8>,
    temperature_meter: Option<u8>,
    supported_controls: HashSet<ControlId>,
    supported_meters: HashSet<MeterId>,
    tuner_status: Option<TunerStatus>,
    swr_sweep_active: bool,
    swr_sweep_status: String,
    swr_sweep_points: Vec<(u64, u8)>,
    swr_sweep_start_hz: u64,
    swr_sweep_stop_hz: u64,
    swr_sweep_step_hz: u64,
    swr_sweep_interval_ms: u64,
    swr_sweep_band: Option<String>,
    radio_power_on: Option<bool>,
    radio_power_supported: bool,
    radio_power_command_pending: bool,
    radio_power_settling: bool,
    radio_power_wake_deadline: Option<Instant>,
    ptt_on: bool,
    radio_spectrum_desired: bool,
    radio_spectrum_enabled: bool,
    radio_waterfall_status: String,
    radio_waterfall_rows: VecDeque<Vec<u8>>,
    radio_waterfall_revision: u64,
    radio_scope_contrast: f32,
    radio_scope_span_code: u8,
    radio_scope_vbw_wide: bool,
    radio_scope_hold: bool,
    radio_scope_reference_tenths_db: i16,
    radio_scope_view: RadioScopeView,
    radio_scope_settings_dirty: bool,
    audio_spectrum_status: String,
    audio_device_sample_rate_hz: Option<u32>,
    audio_device_channels: Option<u16>,
    audio_device_sample_format: Option<String>,
    audio_input_fallback_attempts: Vec<String>,
    audio_monitor_adjustment_ppm: i32,
    audio_monitor_buffered_ms: u32,
    audio_monitor_underruns: u64,
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
    cw_live_text: String,
    cw_auto_target: bool,
    cw_auto_retarget: bool,
    cw_retarget_remaining_s: Option<u8>,
    cw_auto_target_tone_hz: Option<u32>,
    recording_enabled: bool,
    recording_modes: HashSet<WorkspaceMode>,
    recording_full_width: bool,
    recording_stream: bool,
    cw_recording_status: String,
    cw_wpm: u8,
    sstv_status: String,
    sstv_progress: Option<f32>,
    sstv_rgb: Vec<u8>,
    sstv_width: usize,
    sstv_height: usize,
    sstv_revision: u64,
    sstv_tuning_offset_hz: i32,
    sstv_auto_target: bool,
    sstv_locked_offset_hz: Option<i32>,
    sstv_reset_generation: u64,
    sstv_rx_mode: Option<qsonaut_sstv::SstvMode>,
    sstv_detected_mode: Option<qsonaut_sstv::SstvMode>,
    sstv_saved_path: Option<String>,
    sstv_received_images: VecDeque<ReceivedSstvImage>,
    sstv_received_revision: u64,
    sstv_debug_capture_requested: bool,
    sstv_debug_status: String,
    ft4_last_decode_period: Option<u64>,
    digital_tx_period: Option<(WorkspaceMode, u64)>,
    selected_audio_hz: u32,
    fst4_submode: modes::fst4::Submode,
    compute_backend: ActiveBackend,
    ft8_compute_telemetry: Option<DecodeTelemetry>,
    digital_compute_telemetry: Option<DecodeTelemetry>,
    psk_report_sender: Option<ReportSender>,
    last_error: Option<String>,
    last_update: Option<Instant>,
}

impl Default for GuiState {
    fn default() -> Self {
        Self {
            frequency_hz: None,
            active_vfo: 0,
            mode: "(unknown)".to_string(),
            data_mode: None,
            filter: None,
            af_gain: None,
            rf_gain: None,
            rf_power: None,
            rf_power_write_pending: None,
            squelch: None,
            preamp: None,
            attenuator: None,
            noise_blank: None,
            noise_reduction: None,
            noise_reduction_level: None,
            ip_plus: None,
            notch_auto: None,
            notch_manual: None,
            agc: None,
            swr: None,
            signal_meter: None,
            power_meter: None,
            alc_meter: None,
            compression_meter: None,
            current_meter: None,
            voltage_meter: None,
            voltage_history: VecDeque::with_capacity(VOLTAGE_HISTORY_CAPACITY),
            temperature_meter: None,
            supported_controls: HashSet::new(),
            supported_meters: HashSet::new(),
            tuner_status: None,
            swr_sweep_active: false,
            swr_sweep_status: "Idle".to_string(),
            swr_sweep_points: Vec::new(),
            swr_sweep_start_hz: 14_060_000,
            swr_sweep_stop_hz: 14_080_000,
            swr_sweep_step_hz: 1_000,
            swr_sweep_interval_ms: 500,
            swr_sweep_band: None,
            radio_power_on: None,
            radio_power_supported: false,
            radio_power_command_pending: false,
            radio_power_settling: false,
            radio_power_wake_deadline: None,
            ptt_on: false,
            radio_spectrum_desired: false,
            radio_spectrum_enabled: false,
            radio_waterfall_status: "OFF".to_string(),
            radio_waterfall_rows: VecDeque::with_capacity(RADIO_WF_HEIGHT),
            radio_waterfall_revision: 0,
            radio_scope_contrast: 1.2,
            radio_scope_span_code: 0,
            radio_scope_vbw_wide: false,
            radio_scope_hold: false,
            radio_scope_reference_tenths_db: 0,
            radio_scope_view: RadioScopeView::Narrow,
            radio_scope_settings_dirty: false,
            audio_spectrum_status: "INIT".to_string(),
            audio_device_sample_rate_hz: None,
            audio_device_channels: None,
            audio_device_sample_format: None,
            audio_input_fallback_attempts: Vec::new(),
            audio_monitor_adjustment_ppm: 0,
            audio_monitor_buffered_ms: 0,
            audio_monitor_underruns: 0,
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
            cw_live_text: String::new(),
            cw_auto_target: false,
            cw_auto_retarget: false,
            cw_retarget_remaining_s: None,
            cw_auto_target_tone_hz: None,
            recording_enabled: false,
            recording_modes: HashSet::new(),
            recording_full_width: true,
            recording_stream: true,
            cw_recording_status: "Recording off".to_string(),
            cw_wpm: default_cw_wpm(),
            sstv_status: "READY: Auto (VIS) · waiting for a complete SSTV header".to_string(),
            sstv_progress: None,
            sstv_rgb: Vec::new(),
            sstv_width: qsonaut_sstv::WIDTH,
            sstv_height: qsonaut_sstv::HEIGHT,
            sstv_revision: 0,
            sstv_tuning_offset_hz: 0,
            sstv_auto_target: true,
            sstv_locked_offset_hz: None,
            sstv_reset_generation: 0,
            sstv_rx_mode: None,
            sstv_detected_mode: None,
            sstv_saved_path: None,
            sstv_received_images: VecDeque::with_capacity(24),
            sstv_received_revision: 0,
            sstv_debug_capture_requested: false,
            sstv_debug_status: "Debug capture idle".to_string(),
            ft4_last_decode_period: None,
            digital_tx_period: None,
            selected_audio_hz: default_rx_tone_hz(),
            fst4_submode: modes::fst4::Submode::default(),
            compute_backend: ActiveBackend::CpuSimd,
            ft8_compute_telemetry: None,
            digital_compute_telemetry: None,
            psk_report_sender: None,
            last_error: None,
            last_update: None,
        }
    }
}

#[derive(Debug, Clone)]
enum GuiCommand {
    TuneDelta(i64),
    TuneTo(u64),
    CycleMode,
    SetRadioMode(Mode),
    AfGainDelta(i16),
    ApplyWorkspace {
        mode: WorkspaceMode,
        frequency_hz: u64,
    },
    SetFilter(u8),
    SetControl(ControlId, ControlValue),
    SetPtt(bool),
    SetPttWithAck(bool, mpsc::Sender<std::result::Result<(), String>>),
    SetPower(bool),
    StartTuner,
    StartSwrSweep {
        start_hz: u64,
        stop_hz: u64,
        step_hz: u64,
        interval_ms: u64,
    },
    Quit,
}

pub fn run_gui(config: AppConfig) -> Result<Option<GraphicsPreferences>> {
    let build_profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    info!(build_profile, "QSONaut GUI startup");
    info!(
        enabled_backends = ?wgpu::Instance::enabled_backend_features(),
        "WGPU backend features compiled"
    );
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

    let app_icon = eframe::icon_data::from_png_bytes(QSONAUT_ICON_PNG)
        .context("embedded QSONaut icon is not a valid PNG")?;
    let renderer = preferred_renderer();
    let stored_geometry = WindowGeometry::load();
    info!(?stored_geometry, "Restoring window geometry");
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size(WINDOW_DEFAULT_SIZE)
        .with_min_inner_size(WINDOW_MIN_SIZE)
        .with_title("QSONaut — Amateur Radio Mission Control")
        .with_icon(app_icon.clone())
        .with_resizable(true)
        // eframe also enforces this for WGPU, but keeping it explicit makes
        // the startup visibility contract clear at the application boundary.
        .with_visible(false);
    if let Some(geometry) = stored_geometry {
        viewport = geometry.apply(viewport);
    }
    let graphics_preferences = GraphicsPreferences::from_environment();
    let graphics_restart_request = Arc::new(Mutex::new(None));
    info!(
        power = graphics_preferences.power.label(),
        requested_adapter = ?graphics_preferences.adapter,
        "Applying graphics policy"
    );
    let options = eframe::NativeOptions {
        viewport,
        renderer,
        wgpu_options: graphics_preferences.wgpu_configuration(),
        // QSONaut restores geometry through the builder above so winit applies
        // it once, before the window is ever shown.
        persist_window: false,
        ..Default::default()
    };

    let app_config = config.clone();
    let app_graphics_preferences = graphics_preferences.clone();
    let app_graphics_restart_request = Arc::clone(&graphics_restart_request);
    info!(title = "QSONaut", renderer = %renderer, "Calling eframe::run_native");
    let result = eframe::run_native(
        "QSONaut",
        options,
        Box::new(move |cc| {
            info!(renderer = %renderer, "eframe app creation callback entered");
            let (active_graphics_adapter, available_graphics_adapters) = cc
                .wgpu_render_state
                .as_ref()
                .map(|render_state| {
                    let active = GraphicsAdapterInfo::from_wgpu(&render_state.adapter.get_info());
                    let available = render_state
                        .available_adapters
                        .iter()
                        .map(|adapter| GraphicsAdapterInfo::from_wgpu(&adapter.get_info()))
                        .collect::<Vec<_>>();
                    (Some(active), available)
                })
                .unwrap_or_default();
            if let Some(active) = active_graphics_adapter.as_ref() {
                info!(
                    adapter = %active.name,
                    backend = %active.backend,
                    device_type = %active.device_type,
                    driver = %active.driver,
                    driver_info = %active.driver_info,
                    "WGPU graphics adapter active"
                );
            }
            Ok(Box::new(QsonautGuiApp::new(
                app_config.clone(),
                cc,
                &app_icon,
                renderer,
                stored_geometry,
                app_graphics_preferences.clone(),
                active_graphics_adapter,
                available_graphics_adapters,
                Arc::clone(&app_graphics_restart_request),
            )))
        }),
    );

    match result {
        Ok(_) => {
            info!("eframe run_native completed normally");
        }
        Err(err) => {
            error!(error = %err, "eframe run_native failed");
            return Err(anyhow!("eframe launch failed: {err}"));
        }
    }

    let restart = graphics_restart_request
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    Ok(restart)
}

const WINDOW_GEOMETRY_FILE: &str = "window.json";
const WINDOW_MIN_SIZE: [f32; 2] = [980.0, 680.0];
const WINDOW_DEFAULT_SIZE: [f32; 2] = [1280.0, 860.0];

#[derive(Default)]
struct DeviceInventory {
    audio_inputs: Vec<String>,
    audio_outputs: Vec<String>,
    serial_ports: Vec<String>,
    serial_port_labels: HashMap<String, String>,
    detected_models: Vec<String>,
}

fn preferred_renderer() -> eframe::Renderer {
    // QSONaut uses eframe's modern cross-platform GPU backend everywhere.
    eframe::Renderer::Wgpu
}

struct QsonautGuiApp {
    config: AppConfig,
    app_events: AppEventBus,
    automation_event_rx: tokio::sync::broadcast::Receiver<AppEvent>,
    automation_host: AutomationHost,
    automation_status: String,
    last_radio_state_signature: Option<String>,
    automation_external_transports: HashSet<String>,
    automation_external_outbox: VecDeque<ExternalSendRecord>,
    hunter_unlocked: HashSet<AchievementKind>,
    hunter_acknowledged: HashSet<AchievementKind>,
    hunter_show_acknowledged: bool,
    hunter_alerts_enabled: bool,
    hunter_feed: VecDeque<HunterAlert>,
    hunter_unique_heard: HashSet<String>,
    hunter_directed_hits: u32,
    hunter_dupe_blocks: u32,
    hunter_decode_bursts: u32,
    hunter_custom_rules: Vec<CustomAchievementRule>,
    radio_profiles: Vec<RadioProfile>,
    mode_radio_profile: std::collections::BTreeMap<String, String>,
    radio_profile_name_input: String,
    hunter_custom_title_input: String,
    hunter_custom_detail_input: String,
    hunter_custom_metric_input: HunterMetric,
    hunter_custom_threshold_input: u32,
    hunter_custom_enabled_input: bool,
    external_ingress_source: String,
    external_ingress_author: String,
    external_ingress_channel: String,
    external_ingress_message: String,
    state: Arc<Mutex<GuiState>>,
    command_tx: Option<mpsc::Sender<GuiCommand>>,
    radio_worker_stop: Arc<AtomicBool>,
    swr_sweep_abort: Arc<AtomicBool>,
    audio_worker_stop: Arc<AtomicBool>,
    radio_init_rx: Option<mpsc::Receiver<Option<RadioHandle>>>,
    cat_test_rx: Option<mpsc::Receiver<Result<String, String>>>,
    cat_test_status: Option<Result<String, String>>,
    /// Whether to restart the radio worker after a CAT connection test. The
    /// test pauses the worker to release the exclusively-owned serial port.
    cat_test_restart_radio: bool,
    hamdb_lookup_rx: Option<mpsc::Receiver<Option<HamDbCacheEntry>>>,
    hamdb_profile_lookup_rx: Option<mpsc::Receiver<Option<HamDbCacheEntry>>>,
    pota_spots: Vec<PotaSpot>,
    pota_lookup_rx: Option<mpsc::Receiver<Result<Vec<PotaSpot>, String>>>,
    pota_last_lookup: Instant,
    pota_history: VecDeque<(Instant, usize)>,
    pota_last_updated: Option<Instant>,
    pota_last_error: Option<String>,
    radio_init_attempted: bool,
    radio_worker_handle: Option<std::thread::JoinHandle<()>>,
    parked_radio_sessions: HashMap<String, RadioSession>,
    audio_worker_handle: Option<std::thread::JoinHandle<()>>,
    radio_waterfall_texture: Option<TextureHandle>,
    radio_waterfall_texture_revision: u64,
    radio_waterfall_texture_bins: usize,
    radio_waterfall_texture_view: RadioScopeView,
    radio_waterfall_texture_theme: WaterfallTheme,
    audio_waterfall_texture: Option<TextureHandle>,
    audio_waterfall_texture_revision: u64,
    audio_waterfall_texture_bins: usize,
    audio_waterfall_texture_theme: WaterfallTheme,
    sstv_texture: Option<TextureHandle>,
    sstv_texture_revision: u64,
    sstv_tx_armed: bool,
    sstv_tuning_offset_hz: i32,
    sstv_auto_target: bool,
    sstv_rx_width_percent: u8,
    sstv_tx_rgb: Vec<u8>,
    sstv_tx_base_rgb: Vec<u8>,
    sstv_tx_width: usize,
    sstv_tx_height: usize,
    sstv_tx_revision: u64,
    sstv_tx_texture: Option<TextureHandle>,
    sstv_tx_texture_revision: u64,
    sstv_tx_mode: qsonaut_sstv::SstvMode,
    sstv_overlay_callsign: bool,
    sstv_overlay_grid: bool,
    sstv_overlay_frequency: bool,
    sstv_overlay_mode: bool,
    sstv_overlay_corner: SstvOverlayCorner,
    sstv_overlay_background: Color32,
    sstv_overlay_background_opacity: f32,
    sstv_background_zoom: f32,
    sstv_background_pan_x: f32,
    sstv_background_pan_y: f32,
    sstv_overlay_revision: u64,
    sstv_file_dialog: egui_file_dialog::FileDialog,
    sstv_image_path: String,
    sstv_ai_prompt: String,
    local_image_settings: LocalImageSettings,
    local_image_models: Vec<LocalModelInfo>,
    local_image_status: String,
    local_image_refresh_started: bool,
    local_image_event_tx: mpsc::Sender<LocalImageEvent>,
    local_image_event_rx: mpsc::Receiver<LocalImageEvent>,
    sstv_ai_pipeline_mode: SstvAiPipelineMode,
    sstv_active_received_id: Option<String>,
    sstv_show_prior_received: bool,
    sstv_received_textures: HashMap<String, TextureHandle>,
    sstv_received_texture_revision: u64,
    sstv_reinterpret_prompt: String,
    workspace_mode: WorkspaceMode,
    activity: OperatingActivity,
    fst4_submode: modes::fst4::Submode,
    display_tuning: Arc<Mutex<DisplayTuning>>,
    repaint_ctx: Arc<OnceLock<egui::Context>>,
    // FT8 workspace UX state (app-local, not shared with workers)
    ft8_log: Vec<Ft8DecodeEntry>,
    ft8_tx_chat: VecDeque<Ft8TxChatEntry>,
    ft8_seen_decode_period: Option<u64>,
    qso_log: QsoLog,
    qso_selected: Option<u64>,
    qso_log_status: String,
    qso_log_dirty: bool,
    ft8_compose: String,
    ft8_selected: Option<usize>,
    ft8_autoseq: bool,
    ft8_auto_reply_policy: AutoReplyPolicy,
    ft8_auto_answer_cq: bool,
    automation_unlocked: bool,
    logo_clicks: VecDeque<Instant>,
    logo_spin_until: Option<Instant>,
    ft8_session: Option<QsoSession>,
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
    ptt_allowed: Arc<AtomicBool>,
    ft8_tx_event_tx: mpsc::Sender<Ft8TxEvent>,
    ft8_tx_event_rx: mpsc::Receiver<Ft8TxEvent>,
    ft8_last_tx_was_cq: bool,
    digital_compose: String,
    digital_selected: Option<DigitalDecodeEntry>,
    digital_last_click: Option<(u64, u32, String, Instant)>,
    digital_seq_target: Option<String>,
    ft4_session: Option<QsoSession>,
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
    monitor_volume: Arc<AtomicU32>,
    native_autoseq_mode: Option<WorkspaceMode>,
    native_auto_reply_policy: AutoReplyPolicy,
    native_stop_policy: AutoTxStopPolicy,
    native_sessions: HashMap<WorkspaceMode, QsoSession>,
    native_seen_decodes: HashMap<WorkspaceMode, HashSet<(u64, u32, String)>>,
    native_last_tx_periods: HashMap<WorkspaceMode, u64>,
    native_attempts: HashMap<WorkspaceMode, u8>,
    ft8_stop_policy: AutoTxStopPolicy,
    ft8_max_attempts: u8,
    ft4_stop_policy: AutoTxStopPolicy,
    ft4_max_attempts: u8,
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
    station_rig: String,
    station_antenna: String,
    station_notes: String,
    llm_prompt_context: String,
    sstv_image_requirements: String,
    llm_model_notes: String,
    voice_callsign: String,
    voice_grid: String,
    voice_state: String,
    voice_rst_sent: String,
    voice_rst_received: String,
    voice_contest_serial_sent: String,
    voice_contest_serial_received: String,
    voice_notes: String,
    voice_contest_fields: Vec<VoiceContestField>,
    voice_qso_started_at: Option<u64>,
    voice_lookup_requested: String,
    voice_lookup_status: String,
    voice_hamdb: Option<HamDbCacheEntry>,
    contest_enabled: bool,
    contest_operating_mode: ContestOperatingMode,
    contest_split_policy: SplitPolicy,
    contest_fox_hound_role: FoxHoundRole,
    contest_exchange_template: String,
    contest_serial_start: u32,
    contest_serial_step: u32,
    contest_dupe_check: bool,
    contest_serial_current: u32,
    contest_fake_split_offset_hz: u32,
    civ_spectrum_on: bool,
    rx_tone_hz: u32,
    tx_tone_hz: u32,
    ptt_lead_ms: u64,
    ptt_tail_ms: u64,
    cw_wpm: u8,
    cw_tone_hz: u16,
    recording_enabled: bool,
    recording_modes: std::collections::BTreeMap<String, bool>,
    recording_full_width: bool,
    recording_stream: bool,
    selected_profile_name: String,
    new_profile_name: String,
    new_profile_tab_editing: bool,
    pending_profile_delete: Option<String>,
    available_profiles: Vec<String>,
    profile_io_status: String,
    profile_dirty: bool,
    app_log_text: String,
    app_log_status: String,
    app_log_filter: String,
    app_log_level_filter: AppLogLevelFilter,
    app_log_follow: bool,
    app_log_last_refresh: Instant,
    audio_input_devices: Vec<String>,
    audio_output_devices: Vec<String>,
    radio_serial_ports: Vec<String>,
    radio_serial_port_labels: HashMap<String, String>,
    radio_detected_models: Vec<String>,
    device_scan: Option<mpsc::Receiver<DeviceInventory>>,
    hostbridge_catalog: Option<HostHello>,
    hostbridge_scan: Option<mpsc::Receiver<Result<HostHello, String>>>,
    hostbridge_scan_status: String,
    radio_scope_contrast: f32,
    radio_scope_span_code: u8,
    radio_scope_vbw_wide: bool,
    radio_scope_hold: bool,
    radio_scope_reference_tenths_db: i16,
    radio_scope_view: RadioScopeView,
    radio_scope_lock_if_to_filter: bool,
    waterfall_theme: WaterfallTheme,
    radio_waterfall_theme: WaterfallTheme,
    waterfall_deck_height: f32,
    show_signal_panel: bool,
    show_meter_panel: bool,
    meter_panel_was_tx: bool,
    meter_panel_close_deadline: Option<Instant>,
    show_profile_drawer: bool,
    profile_drawer_anchor: Option<egui::Pos2>,
    radio_faq_window_open: bool,
    radio_guide_window_open: bool,
    radio_faq_document: RadioHelpDocument,
    radio_guide_document: RadioHelpDocument,
    radio_help_window_model: String,
    profile_drawer_tab: ProfileDrawerTab,
    signal_panel_tab: SignalPanelTab,
    device_restart_required: bool,
    audio_restart_required: bool,
    gui_scale: f32,
    os_dpi_adjustment: f32,
    graphics_active: GraphicsPreferences,
    graphics_pending: GraphicsPreferences,
    active_graphics_adapter: Option<GraphicsAdapterInfo>,
    available_graphics_adapters: Vec<GraphicsAdapterInfo>,
    graphics_restart_request: Arc<Mutex<Option<GraphicsPreferences>>>,
    compute_preference: ComputePreference,
    acceleration_report: AccelerationReport,
    acceleration_probe: Option<mpsc::Receiver<AccelerationReport>>,
    psk_reporter_enabled: bool,
    pota_enabled: bool,
    psk_batch_interval_secs: u64,
    psk_repeat_cache_secs: u64,
    psk_max_pending: usize,
    psk_reporter: Option<Reporter>,
    server_client: Option<ServerClient>,
    server_active_club: Option<(String, String)>,
    server_active_event: Option<(String, String)>,
    server_instance_id: String,
    server_last_presence: Instant,
    brand_icon: TextureHandle,
    selected_renderer: eframe::Renderer,
    first_frame_logged: bool,
    last_viewport_log: Option<String>,
    window_geometry: Option<WindowGeometry>,
}

// Runtime resources owned by a radio tab. The selected tab controls the UI,
// while every tab keeps its workers and telemetry alive. Inactive tabs are
// prevented from asserting PTT unless the unattended automation unlock is in
// effect.
struct RadioSession {
    profile: OperatorProfile,
    view_state: TabViewState,
    config: RadioConfig,
    audio_config: AudioConfig,
    state: Arc<Mutex<GuiState>>,
    command_tx: Option<mpsc::Sender<GuiCommand>>,
    worker_stop: Arc<AtomicBool>,
    audio_worker_stop: Arc<AtomicBool>,
    swr_sweep_abort: Arc<AtomicBool>,
    display_tuning: Arc<Mutex<DisplayTuning>>,
    monitor_volume: Arc<AtomicU32>,
    ft8_tx_active: Arc<AtomicBool>,
    digital_tx_active: Arc<AtomicBool>,
    ptt_allowed: Arc<AtomicBool>,
    init_rx: Option<mpsc::Receiver<Option<RadioHandle>>>,
    init_attempted: bool,
    worker_handle: Option<std::thread::JoinHandle<()>>,
    audio_worker_handle: Option<std::thread::JoinHandle<()>>,
}

#[derive(Default)]
struct TabViewState {
    ft8_log: Vec<Ft8DecodeEntry>,
    ft8_tx_chat: VecDeque<Ft8TxChatEntry>,
    ft8_seen_decode_period: Option<u64>,
    ft8_compose: String,
    ft8_selected: Option<usize>,
    digital_compose: String,
    digital_selected: Option<DigitalDecodeEntry>,
    digital_tx_chat: VecDeque<DigitalTxChatEntry>,
    ft4_seen_decodes: HashSet<(u64, u32, String)>,
    ft4_seen_decode_period: Option<u64>,
    native_seen_decodes: HashMap<WorkspaceMode, HashSet<(u64, u32, String)>>,
}

impl QsonautGuiApp {
    fn refresh_acceleration_report(&mut self) {
        self.acceleration_report = AccelerationReport::pending(self.compute_preference);
        self.acceleration_probe = Some(spawn_acceleration_probe(self.compute_preference));
    }

    fn take_tab_view_state(&mut self) -> TabViewState {
        TabViewState {
            ft8_log: std::mem::take(&mut self.ft8_log),
            ft8_tx_chat: std::mem::take(&mut self.ft8_tx_chat),
            ft8_seen_decode_period: self.ft8_seen_decode_period.take(),
            ft8_compose: std::mem::take(&mut self.ft8_compose),
            ft8_selected: self.ft8_selected.take(),
            digital_compose: std::mem::take(&mut self.digital_compose),
            digital_selected: self.digital_selected.take(),
            digital_tx_chat: std::mem::take(&mut self.digital_tx_chat),
            ft4_seen_decodes: std::mem::take(&mut self.ft4_seen_decodes),
            ft4_seen_decode_period: self.ft4_seen_decode_period.take(),
            native_seen_decodes: std::mem::take(&mut self.native_seen_decodes),
        }
    }

    fn restore_tab_view_state(&mut self, view: TabViewState) {
        self.ft8_log = view.ft8_log;
        self.ft8_tx_chat = view.ft8_tx_chat;
        self.ft8_seen_decode_period = view.ft8_seen_decode_period;
        self.ft8_compose = view.ft8_compose;
        self.ft8_selected = view.ft8_selected;
        self.digital_compose = view.digital_compose;
        self.digital_selected = view.digital_selected;
        self.digital_tx_chat = view.digital_tx_chat;
        self.ft4_seen_decodes = view.ft4_seen_decodes;
        self.ft4_seen_decode_period = view.ft4_seen_decode_period;
        self.native_seen_decodes = view.native_seen_decodes;
    }

    fn radio_config_for_profile(&self, profile: &OperatorProfile) -> RadioConfig {
        radio_config_from_operator_profile(profile)
    }

    fn radio_tab_status(
        &self,
        name: &str,
        active_snapshot: &GuiState,
    ) -> (char, Color32, String, String) {
        let (
            radio_enabled,
            audio_enabled,
            radio_status,
            audio_status,
            frequency_hz,
            workspace_mode,
        ) = if name == self.selected_profile_name {
            (
                self.config.radio.enabled,
                self.config.audio.enabled,
                active_snapshot.radio_waterfall_status.clone(),
                active_snapshot.audio_spectrum_status.clone(),
                active_snapshot.frequency_hz,
                active_snapshot.workspace_mode,
            )
        } else if let Some(session) = self.parked_radio_sessions.get(name) {
            match session.state.lock() {
                Ok(state) => (
                    session.config.enabled,
                    session.audio_config.enabled,
                    state.radio_waterfall_status.clone(),
                    state.audio_spectrum_status.clone(),
                    state.frequency_hz,
                    state.workspace_mode,
                ),
                Err(_) => (
                    session.config.enabled,
                    session.audio_config.enabled,
                    "UNAVAILABLE (state lock failed)".to_string(),
                    "UNAVAILABLE (state lock failed)".to_string(),
                    None,
                    WorkspaceMode::Ft8,
                ),
            }
        } else {
            (
                false,
                false,
                "UNAVAILABLE (tab not initialized)".to_string(),
                "UNAVAILABLE (tab not initialized)".to_string(),
                None,
                WorkspaceMode::Ft8,
            )
        };
        let radio_failed = radio_status_is_failed(radio_enabled, &radio_status);
        let audio_failed = audio_enabled
            && (audio_status.starts_with("NO INPUT")
                || audio_status.starts_with("ERROR")
                || audio_status.starts_with("SESSION STOPPED"));
        let band = frequency_hz
            .map(band_for_frequency)
            .filter(|band| !band.is_empty())
            .unwrap_or("—");
        let identity = format!("{band} · {}", workspace_mode.label());
        let radio_detail = if radio_failed || radio_status.starts_with("CONNECTING") {
            format!("Radio: {radio_status}")
        } else if radio_enabled {
            "Radio: connected".to_string()
        } else {
            "Radio: off".to_string()
        };
        let audio_detail = if audio_failed || audio_status == "INIT" {
            format!("Audio: {audio_status}")
        } else if audio_enabled {
            "Audio: live RX".to_string()
        } else {
            "Audio: off".to_string()
        };
        let detail = format!("{radio_detail} · {audio_detail}");
        if radio_failed || audio_failed {
            ('!', Color32::from_rgb(255, 125, 105), identity, detail)
        } else if radio_status.starts_with("CONNECTING") || audio_status == "INIT" {
            ('◌', Color32::from_rgb(255, 205, 105), identity, detail)
        } else if !radio_enabled && !audio_enabled {
            ('○', Color32::GRAY, identity, detail)
        } else {
            ('●', Color32::from_rgb(125, 225, 150), identity, detail)
        }
    }

    fn any_tx_armed(&self, snapshot: &GuiState) -> bool {
        snapshot.ptt_on
            || self.ft8_autoseq
            || self.ft4_autoseq
            || self.ft8_tx_active.load(Ordering::Acquire)
            || self.ft8_tx_queued_period.is_some()
            || self.digital_tx_active.load(Ordering::Acquire)
            || self.sstv_tx_armed
    }

    fn active_radio_profile_name(&self) -> Option<&str> {
        let mode = self.workspace_mode.label();
        self.mode_radio_profile
            .get(mode)
            .or_else(|| self.mode_radio_profile.get("Other"))
            .map(String::as_str)
            .filter(|name| !name.trim().is_empty())
    }

    fn disarm_all_tx(&mut self, reason: &str) {
        self.disarm_all_tx_with_persistence(reason, true);
    }

    fn disarm_all_tx_with_persistence(&mut self, reason: &str, persist: bool) {
        self.force_stop_tx();
        self.stop_native_digital_tx();
        self.ft8_autoseq = false;
        self.ft4_autoseq = false;
        self.sstv_tx_armed = false;
        self.ft8_stop_policy = AutoTxStopPolicy::Continuous;
        self.ft4_stop_policy = AutoTxStopPolicy::Continuous;
        self.ft4_session = None;
        self.digital_seq_target = None;
        self.digital_tx_started = None;
        self.digital_last_tx_message = None;
        self.ft8_seq_status = reason.to_string();
        self.digital_tx_status = reason.to_string();
        if persist {
            self.profile_dirty = true;
            self.persist_profile("All TX disarmed");
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

    fn station_grid_for_ft8(&self) -> String {
        self.station_grid_or_default()
            .chars()
            .take(4)
            .collect::<String>()
            .to_ascii_uppercase()
    }

    fn emit_operator_profile_hook(&self, detail: impl Into<String>) {
        self.app_events.publish(AppEvent::AutomationHook {
            kind: "operator_profile".to_string(),
            source: "gui.operator_profile".to_string(),
            detail: detail.into(),
        });
    }

    fn handle_logo_click(&mut self) {
        let now = Instant::now();
        while self
            .logo_clicks
            .front()
            .is_some_and(|clicked| now.duration_since(*clicked) > Duration::from_secs(10))
        {
            self.logo_clicks.pop_front();
        }
        self.logo_clicks.push_back(now);
        if self.logo_clicks.len() >= 10 {
            self.logo_clicks.clear();
            self.automation_unlocked = true;
            for session in self.parked_radio_sessions.values() {
                session.ptt_allowed.store(true, Ordering::Release);
            }
            self.logo_spin_until = Some(now + Duration::from_millis(700));
            self.profile_dirty = true;
            self.persist_profile("Automation controls unlocked");
            self.profile_io_status = "Automation controls unlocked".to_string();
        }
    }

    fn emit_radio_state_hook_if_changed(&mut self, snapshot: &GuiState) {
        let frequency_hz = snapshot.frequency_hz.unwrap_or_default();
        let data_mode = snapshot
            .data_mode
            .map_or("unknown", |value| if value { "true" } else { "false" });
        let filter = snapshot
            .filter
            .map(|value| value.to_string())
            .unwrap_or_else(|| "unknown".to_string());
        let signature = format!(
            "frequency_hz={frequency_hz} mode={} data_mode={data_mode} filter={filter} ptt_on={}",
            snapshot.mode, snapshot.ptt_on
        );

        if self.last_radio_state_signature.as_deref() == Some(signature.as_str()) {
            return;
        }
        self.last_radio_state_signature = Some(signature.clone());
        self.app_events.publish(AppEvent::AutomationHook {
            kind: "radio_state".to_string(),
            source: "gui.radio".to_string(),
            detail: signature,
        });
    }

    fn publish_external_ingress_message(&mut self) {
        let source = self.external_ingress_source.trim();
        let author = self.external_ingress_author.trim();
        let channel = self.external_ingress_channel.trim();
        let message = self.external_ingress_message.trim();
        if source.is_empty() || author.is_empty() || message.is_empty() {
            warn!(
                source_present = !source.is_empty(),
                author_present = !author.is_empty(),
                message_present = !message.is_empty(),
                "External ingress rejected: required metadata missing"
            );
            self.automation_status =
                "External ingress blocked: source, author, and message are required".to_string();
            return;
        }

        self.app_events.publish(AppEvent::ExternalMessageReceived {
            source: source.to_string(),
            author: author.to_string(),
            message: message.to_string(),
            channel: if channel.is_empty() {
                "(unspecified)".to_string()
            } else {
                channel.to_string()
            },
        });
        info!(source = %source, author = %author, channel = %if channel.is_empty() { "(unspecified)" } else { channel }, message_length = message.chars().count(), "External ingress accepted");
        self.automation_status =
            format!("External message injected from {source} as {author}: {message}");
        self.external_ingress_message.clear();
    }

    fn send_command(&self, cmd: GuiCommand) {
        if let Some(tx) = &self.command_tx {
            let _ = tx.send(cmd);
        }
    }

    fn refresh_device_lists(&mut self) {
        info!("Device inventory refresh requested");
        self.device_scan = Some(spawn_device_scan());
    }

    fn apply_device_inventory(&mut self, inventory: DeviceInventory) {
        info!(
            audio_inputs = inventory.audio_inputs.len(),
            audio_outputs = inventory.audio_outputs.len(),
            serial_ports = inventory.serial_ports.len(),
            detected_models = inventory.detected_models.len(),
            "Device inventory applied"
        );
        self.audio_input_devices = inventory.audio_inputs;
        self.audio_output_devices = inventory.audio_outputs;
        self.radio_serial_ports = inventory.serial_ports;
        self.radio_serial_port_labels = inventory.serial_port_labels;
        self.radio_detected_models = inventory.detected_models;
    }
}

impl eframe::App for QsonautGuiApp {
    fn save(&mut self, _storage: &mut dyn eframe::Storage) {
        if let Some(geometry) = self.window_geometry {
            geometry.save();
        }
    }

    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        runtime::update::update(self, ctx, frame);
    }
}

impl Drop for QsonautGuiApp {
    fn drop(&mut self) {
        let shutdown_started = Instant::now();
        let parked_profile_count = self.parked_radio_sessions.len();
        self.force_stop_tx();
        self.stop_native_digital_tx();
        // A clean shutdown must not rewrite a valid profile with runtime
        // defaults if startup did not finish loading its settings.
        if self.profile_dirty {
            self.persist_profile("Saved on exit");
        }
        if self.qso_log_dirty {
            self.persist_qso_log("Saved on exit");
        }
        self.radio_worker_stop.store(true, Ordering::Relaxed);
        self.audio_worker_stop.store(true, Ordering::Relaxed);
        if let Some(tx) = &self.command_tx {
            let _ = tx.send(GuiCommand::Quit);
        }
        for session in self.parked_radio_sessions.values() {
            request_radio_session_stop(session);
        }
        if let Some(handle) = self.radio_worker_handle.take() {
            join_handle_for_shutdown(handle, "active radio");
        }
        for (_, mut session) in std::mem::take(&mut self.parked_radio_sessions) {
            if let Some(handle) = session.worker_handle.take() {
                join_handle_for_shutdown(handle, "parked radio");
            }
            if let Some(handle) = session.audio_worker_handle.take() {
                join_handle_for_shutdown(handle, "parked audio");
            }
        }
        if let Some(handle) = self.audio_worker_handle.take() {
            join_handle_for_shutdown(handle, "active audio");
        }
        info!(
            elapsed_ms = shutdown_started.elapsed().as_millis(),
            parked_profiles = parked_profile_count,
            "GUI worker shutdown completed"
        );
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

fn is_transient_civ_read_error(message: &str) -> bool {
    let m = message.to_ascii_lowercase();
    m.contains("failed to read ci-v response") || m.contains("timed out") || m.contains("timeout")
}

fn radio_port_inventory(
    descriptors: Vec<SerialPortDescriptor>,
) -> (Vec<String>, HashMap<String, String>, Vec<String>) {
    let mut ports = Vec::with_capacity(descriptors.len());
    let mut labels = HashMap::with_capacity(descriptors.len());
    let mut models = Vec::new();

    for descriptor in descriptors {
        ports.push(descriptor.port_name.clone());
        labels.insert(descriptor.port_name, descriptor.display_name);
        if let Some(model) = descriptor.likely_radio {
            models.push(model);
        }
    }

    ports.sort();
    ports.dedup();
    models.sort();
    models.dedup();
    (ports, labels, models)
}

#[cfg(test)]
fn format_meter_value(value: Option<u8>) -> String {
    value
        .map(|value| format!("{:.0}%", f32::from(value) * 100.0 / 255.0))
        .unwrap_or_else(|| "—".to_string())
}

fn radio_status_is_failed(enabled: bool, status: &str) -> bool {
    enabled
        && ((status.starts_with("UNAVAILABLE") && !status.contains("no scope stream"))
            || status.starts_with("SESSION STOPPED"))
}

/// Append completed FT8 results to the UI-owned log and report how many old
/// rows must be removed to enforce the configured limit.
fn append_ft8_log_entries(
    log: &mut Vec<Ft8DecodeEntry>,
    decodes: &[Ft8DecodeEntry],
    max_entries: usize,
) -> usize {
    log.extend_from_slice(decodes);
    log.len().saturating_sub(max_entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_retry_status_does_not_mark_healthy_radio_offline() {
        assert!(!radio_status_is_failed(true, "ENABLE RETRY"));
        assert!(!radio_status_is_failed(true, "CONFIG ERROR"));
        assert!(radio_status_is_failed(
            true,
            "UNAVAILABLE (connection failed)"
        ));
        assert!(radio_status_is_failed(
            true,
            "SESSION STOPPED (radio worker failed)"
        ));
    }

    #[test]
    fn radio_session_stop_contract_signals_every_owned_worker() {
        let worker_stop = Arc::new(AtomicBool::new(false));
        let audio_worker_stop = Arc::new(AtomicBool::new(false));
        let swr_sweep_abort = Arc::new(AtomicBool::new(false));
        let (command_tx, command_rx) = mpsc::channel();

        radio_runtime::request_radio_session_stop_handles(
            Some(&command_tx),
            &worker_stop,
            &audio_worker_stop,
            &swr_sweep_abort,
        );

        assert!(worker_stop.load(Ordering::Relaxed));
        assert!(audio_worker_stop.load(Ordering::Relaxed));
        assert!(swr_sweep_abort.load(Ordering::Relaxed));
        assert!(matches!(command_rx.try_recv(), Ok(GuiCommand::Quit)));

        // Parked sessions may already have relinquished their command sender;
        // stopping them must still signal all cancellation flags.
        worker_stop.store(false, Ordering::Relaxed);
        audio_worker_stop.store(false, Ordering::Relaxed);
        swr_sweep_abort.store(false, Ordering::Relaxed);
        radio_runtime::request_radio_session_stop_handles(
            None,
            &worker_stop,
            &audio_worker_stop,
            &swr_sweep_abort,
        );
        assert!(worker_stop.load(Ordering::Relaxed));
        assert!(audio_worker_stop.load(Ordering::Relaxed));
        assert!(swr_sweep_abort.load(Ordering::Relaxed));
    }

    #[test]
    fn worker_disabled_constructor_supports_safe_tx_pipeline_transitions() {
        let icon = eframe::icon_data::from_png_bytes(QSONAUT_ICON_PNG).expect("test icon");
        let mut config = AppConfig::default();
        config.radio.enabled = false;
        let context = egui::Context::default();
        let mut app = QsonautGuiApp::new_with_context(
            config,
            false,
            false,
            &context,
            &icon,
            eframe::Renderer::Wgpu,
            None,
            GraphicsPreferences::from_environment(),
            None,
            Vec::new(),
            Arc::new(Mutex::new(None)),
        );

        app.force_stop_tx();
        assert_eq!(app.ft8_seq_state, Ft8SeqState::Idle);
        assert_eq!(app.ft8_seq_status, "TX force-stopped");
        app.ft8_tx_event_tx.send(Ft8TxEvent::PttConfirmed).unwrap();
        app.process_ft8_tx_pipeline();
        assert!(app.ft8_seq_status.contains("PTT confirmed"));
        app.ft8_tx_queued_period = Some(12);
        app.ft8_queued_tx_message = Some("CQ N0CALL AA00".to_string());
        app.ft8_last_tx_was_cq = true;
        app.ft8_autoseq = true;
        app.ft8_tx_event_tx.send(Ft8TxEvent::AudioStarted).unwrap();
        app.process_ft8_tx_pipeline();
        assert!(app.ft8_seq_status.contains("waveform on the air"));
        app.ft8_tx_event_tx.send(Ft8TxEvent::Complete).unwrap();
        app.process_ft8_tx_pipeline();
        assert_eq!(app.ft8_seq_state, Ft8SeqState::CqArmed);
        app.ft8_tx_event_tx
            .send(Ft8TxEvent::Failed("test failure".to_string()))
            .unwrap();
        app.process_ft8_tx_pipeline();
        assert_eq!(app.ft8_seq_state, Ft8SeqState::Idle);
        assert!(app.ft8_seq_status.contains("test failure"));

        app.digital_tx_event_tx
            .send(DigitalTxEvent::AudioStarted(WorkspaceMode::Ft4, 8))
            .unwrap();
        app.digital_queued_tx_message = Some("CQ N0CALL AA00".to_string());
        app.process_native_digital_tx_pipeline();
        assert!(app.digital_tx_status.contains("waveform on the air"));
        app.digital_tx_event_tx
            .send(DigitalTxEvent::Complete)
            .unwrap();
        app.process_native_digital_tx_pipeline();
        assert_eq!(app.ft4_last_tx_period, Some(8));
        app.digital_tx_event_tx
            .send(DigitalTxEvent::Failed("digital failure".to_string()))
            .unwrap();
        app.process_native_digital_tx_pipeline();
        assert!(app.digital_tx_status.contains("digital failure"));
    }

    #[test]
    fn tx_safety_detects_and_clears_every_transmit_path() {
        let icon = eframe::icon_data::from_png_bytes(QSONAUT_ICON_PNG).expect("test icon");
        let mut config = AppConfig::default();
        config.radio.enabled = false;
        let context = egui::Context::default();
        let mut app = QsonautGuiApp::new_with_context(
            config,
            false,
            false,
            &context,
            &icon,
            eframe::Renderer::Wgpu,
            None,
            GraphicsPreferences::from_environment(),
            None,
            Vec::new(),
            Arc::new(Mutex::new(None)),
        );

        let mut snapshot = GuiState::default();
        assert!(!app.any_tx_armed(&snapshot));
        for arm in [
            |app: &mut QsonautGuiApp| app.ft8_autoseq = true,
            |app: &mut QsonautGuiApp| app.ft4_autoseq = true,
            |app: &mut QsonautGuiApp| app.ft8_tx_queued_period = Some(1),
            |app: &mut QsonautGuiApp| app.sstv_tx_armed = true,
        ] {
            arm(&mut app);
            assert!(app.any_tx_armed(&snapshot));
            app.ft8_autoseq = false;
            app.ft4_autoseq = false;
            app.ft8_tx_queued_period = None;
            app.sstv_tx_armed = false;
        }
        snapshot.ptt_on = true;
        assert!(app.any_tx_armed(&snapshot));
        snapshot.ptt_on = false;
        app.ft8_tx_active.store(true, Ordering::Release);
        assert!(app.any_tx_armed(&snapshot));

        app.ft8_autoseq = true;
        app.ft4_autoseq = true;
        app.sstv_tx_armed = true;
        app.ft8_tx_queued_period = Some(9);
        app.digital_seq_target = Some("W1AW".to_string());
        app.digital_tx_started = Some((WorkspaceMode::Ft4, 9));
        app.digital_last_tx_message = Some("CQ W1AW".to_string());
        app.disarm_all_tx_with_persistence("safety test", false);

        assert!(!app.ft8_autoseq);
        assert!(!app.ft4_autoseq);
        assert!(!app.sstv_tx_armed);
        assert!(app.ft8_tx_queued_period.is_none());
        assert!(app.digital_seq_target.is_none());
        assert!(app.digital_tx_started.is_none());
        assert!(app.digital_last_tx_message.is_none());
        assert_eq!(app.ft8_seq_status, "safety test");
        assert_eq!(app.digital_tx_status, "safety test");
        app.ft8_tx_active.store(false, Ordering::Release);
    }

    #[test]
    fn radio_profile_application_maps_persisted_controls_to_hal_commands() {
        let icon = eframe::icon_data::from_png_bytes(QSONAUT_ICON_PNG).expect("test icon");
        let mut config = AppConfig::default();
        config.radio.enabled = false;
        let context = egui::Context::default();
        let mut app = QsonautGuiApp::new_with_context(
            config,
            false,
            false,
            &context,
            &icon,
            eframe::Renderer::Wgpu,
            None,
            GraphicsPreferences::from_environment(),
            None,
            Vec::new(),
            Arc::new(Mutex::new(None)),
        );
        let snapshot = GuiState {
            mode: "USB".to_string(),
            data_mode: Some(true),
            filter: Some(3),
            af_gain: Some(12),
            rf_gain: Some(34),
            rf_power: Some(56),
            ..GuiState::default()
        };
        let read_back = app.read_radio_profile("saved", &snapshot);
        assert_eq!(read_back.name, "saved");
        assert_eq!(read_back.mode.as_deref(), Some("USB"));
        assert_eq!(read_back.data_mode, Some(true));
        assert_eq!(read_back.filter, Some(3));
        assert_eq!(read_back.rf_power, Some(56));
        app.apply_radio_profile(RadioProfile {
            name: "disconnected".to_string(),
            mode: None,
            data_mode: None,
            filter: None,
            af_gain: None,
            rf_gain: None,
            rf_power: None,
            preamp: None,
            attenuator: None,
            noise_blank: None,
            noise_reduction: None,
            agc: None,
        });
        assert_eq!(
            app.profile_io_status,
            "Radio tuning unavailable: radio is not connected"
        );
        app.state.lock().expect("state lock").frequency_hz = Some(14_074_000);
        let (tx, rx) = mpsc::channel();
        app.command_tx = Some(tx);

        app.apply_radio_profile(RadioProfile {
            name: "FT4 contest".to_string(),
            mode: Some("FT4".to_string()),
            data_mode: Some(true),
            filter: Some(2),
            af_gain: Some(11),
            rf_gain: Some(22),
            rf_power: Some(33),
            preamp: Some(true),
            attenuator: Some(false),
            noise_blank: Some(true),
            noise_reduction: Some(false),
            agc: Some(3),
        });

        let commands: Vec<_> = rx.try_iter().collect();
        assert!(commands.iter().any(|command| matches!(
            command,
            GuiCommand::ApplyWorkspace {
                mode: WorkspaceMode::Ft4,
                frequency_hz: 14_074_000
            }
        )));
        assert!(commands
            .iter()
            .any(|command| matches!(command, GuiCommand::SetFilter(2))));
        for expected in [
            GuiCommand::SetControl(ControlId::AfGain, ControlValue::U8(11)),
            GuiCommand::SetControl(ControlId::RfGain, ControlValue::U8(22)),
            GuiCommand::SetControl(ControlId::RfPower, ControlValue::U8(33)),
            GuiCommand::SetControl(ControlId::Preamp, ControlValue::Bool(true)),
            GuiCommand::SetControl(ControlId::Attenuator, ControlValue::Bool(false)),
            GuiCommand::SetControl(ControlId::NoiseBlanker, ControlValue::Bool(true)),
            GuiCommand::SetControl(ControlId::NoiseReduction, ControlValue::Bool(false)),
            GuiCommand::SetControl(ControlId::Agc, ControlValue::U8(3)),
        ] {
            assert!(
                commands.iter().any(|command| match (&expected, command) {
                    (
                        GuiCommand::SetControl(expected_id, expected_value),
                        GuiCommand::SetControl(actual_id, actual_value),
                    ) => expected_id == actual_id && expected_value == actual_value,
                    _ => false,
                }),
                "missing HAL command: {expected:?}"
            );
        }
        assert_eq!(app.profile_io_status, "Applied radio profile FT4 contest");
    }

    #[test]
    fn operator_profile_translation_preserves_versioned_audio_and_radio_settings() {
        let icon = eframe::icon_data::from_png_bytes(QSONAUT_ICON_PNG).expect("test icon");
        let config = AppConfig::default();
        let context = egui::Context::default();
        let app = QsonautGuiApp::new_with_context(
            config.clone(),
            false,
            false,
            &context,
            &icon,
            eframe::Renderer::Wgpu,
            None,
            GraphicsPreferences::from_environment(),
            None,
            Vec::new(),
            Arc::new(Mutex::new(None)),
        );
        let mut profile = app.current_operator_profile();
        profile.radio.enabled = true;
        profile.radio.backend = "native".to_string();
        profile.radio.model = "IC-7300".to_string();
        profile.radio.endpoint = "127.0.0.1:4532".to_string();
        profile.radio.serial_port = Some("/dev/ttyUSB0".to_string());
        profile.radio.baud_rate = 230_400;
        profile.radio.civ_address = 0x94;
        profile.radio.controller_civ_address = 0xE0;
        let radio = radio_config_from_operator_profile(&profile);
        assert!(radio.enabled);
        assert_eq!(radio.backend, "native");
        assert_eq!(radio.model, "IC-7300");
        assert_eq!(radio.baud_rate, 230_400);
        assert_eq!(radio.civ_address, 0x94);

        profile.profile_version = 2;
        profile.audio.enabled = false;
        profile.audio.input_device = Some("input".to_string());
        profile.audio.output_device = Some("output".to_string());
        profile.audio.monitor_enabled = true;
        profile.audio.monitor_output_device = Some("monitor".to_string());
        profile.audio.monitor_volume = 4.0;
        profile.audio.sample_rate_hz = 44_100;
        profile.audio.channels = 2;
        let legacy_audio = audio_config_from_operator_profile(&profile, &config.audio);
        assert_eq!(legacy_audio.enabled, config.audio.enabled);
        assert_eq!(legacy_audio.input_device, config.audio.input_device);
        assert_eq!(legacy_audio.output_device, config.audio.output_device);
        assert_eq!(legacy_audio.monitor_enabled, config.audio.monitor_enabled);
        assert_eq!(
            legacy_audio.monitor_output_device,
            config.audio.monitor_output_device
        );
        assert_eq!(legacy_audio.monitor_volume, config.audio.monitor_volume);

        profile.profile_version = AUDIO_MONITOR_PROFILE_VERSION;
        let current_audio = audio_config_from_operator_profile(&profile, &config.audio);
        assert!(current_audio.monitor_enabled);
        assert_eq!(
            current_audio.monitor_output_device.as_deref(),
            Some("monitor")
        );
        assert_eq!(current_audio.monitor_volume, 2.0);
        assert_eq!(current_audio.sample_rate_hz, 44_100);
        assert_eq!(current_audio.channels, 2);

        let serialized = toml::to_string(&profile).expect("serialize profile");
        assert!(serialized.contains("audio_input_device = \"input\""));
        assert!(serialized.contains("audio_output_device = \"output\""));
        assert!(serialized.contains("audio_monitor_output_device = \"monitor\""));
    }

    #[test]
    fn radio_initialization_routes_supported_and_unsupported_backends() {
        let none = spawn_radio_init(
            "none".to_string(),
            "IC-7300".to_string(),
            String::new(),
            String::new(),
            115_200,
            0xE0,
            0x94,
        )
        .recv_timeout(std::time::Duration::from_secs(1))
        .expect("none backend result");
        assert!(none.is_none());

        let null = spawn_radio_init(
            "mock".to_string(),
            "IC-7300".to_string(),
            String::new(),
            String::new(),
            115_200,
            0xE0,
            0x94,
        )
        .recv_timeout(std::time::Duration::from_secs(1))
        .expect("mock backend result");
        assert!(null.is_some());

        for backend in ["rigctld", "dxlab", "rigctl", "commander"] {
            let configured = spawn_radio_init(
                backend.to_string(),
                "IC-7300".to_string(),
                String::new(),
                "127.0.0.1:4532".to_string(),
                115_200,
                0xE0,
                0x94,
            )
            .recv_timeout(std::time::Duration::from_secs(1))
            .expect("external backend result");
            assert!(configured.is_some(), "backend {backend} should configure");
        }

        let unknown_model = spawn_radio_init(
            "native".to_string(),
            "not-a-real-model".to_string(),
            String::new(),
            String::new(),
            115_200,
            0xE0,
            0x94,
        )
        .recv_timeout(std::time::Duration::from_secs(1))
        .expect("unknown model result");
        assert!(unknown_model.is_none());

        let unsupported = spawn_radio_init(
            "vendor-specific-backend".to_string(),
            "IC-7300".to_string(),
            String::new(),
            String::new(),
            115_200,
            0xE0,
            0x94,
        )
        .recv_timeout(std::time::Duration::from_secs(1))
        .expect("unsupported backend result");
        assert!(unsupported.is_none());
    }

    #[test]
    fn meter_display_orders_rx_and_tx_values_for_operator_context() {
        assert_eq!(mode_meter_order(false)[0], MeterId::Voltage);
        assert_eq!(mode_meter_order(false)[1], MeterId::Current);
        assert_eq!(mode_meter_order(true)[0], MeterId::Voltage);
        assert_eq!(mode_meter_order(true)[1], MeterId::Current);
        assert_eq!(mode_meter_order(true)[2], MeterId::Power);
        assert_eq!(meter_label(MeterId::Temperature), "TEMP");
    }

    #[test]
    fn voltage_history_keeps_a_long_rolling_window() {
        let mut history = VecDeque::new();
        for value in 0..=u8::MAX {
            record_voltage_sample(&mut history, value);
        }
        assert_eq!(history.len(), VOLTAGE_HISTORY_CAPACITY);
        assert_eq!(history.front(), Some(&76));
        assert_eq!(history.back(), Some(&u8::MAX));
    }

    #[test]
    fn current_meter_is_brighter_during_transmit() {
        assert_ne!(
            meter_color_for_context(MeterId::Current, Some(100), false),
            meter_color_for_context(MeterId::Current, Some(100), true)
        );
    }

    #[test]
    fn meter_display_normalizes_hal_levels() {
        assert_eq!(meter_percent(0), 0.0);
        assert_eq!(meter_percent(255), 1.0);
        assert_eq!(format_meter_value(Some(128)), "50%");
        assert_eq!(format_meter_value(None), "—");
        assert_eq!(meter_reading(MeterId::Signal, Some(72)), "S6 · 28%");
        assert_eq!(meter_reading(MeterId::Signal, Some(120)), "S9 +6 dB · 47%");
        assert_eq!(meter_reading(MeterId::Power, Some(128)), "50%");
        assert_eq!(meter_reading(MeterId::Voltage, Some(128)), "REL 128/255");
        assert_eq!(
            meter_reading_for_model(MeterId::Voltage, Some(145), "IC-7300"),
            "13.5 V"
        );
        assert_eq!(
            meter_reading_for_model(MeterId::Voltage, Some(145), "FTDX10"),
            "REL 145/255"
        );
    }

    #[test]
    fn radio_profiles_apply_only_to_the_native_backend() {
        assert_eq!(
            native_radio_profile("native", "IC-7300").map(|profile| profile.model),
            Some("IC-7300")
        );
        assert!(native_radio_profile("rigctld", "IC-7300").is_none());
        assert!(native_radio_profile("null", "IC-7300").is_none());
    }

    #[test]
    fn unprofiled_radio_controls_use_safe_generic_limits() {
        assert_eq!(
            super::radio_control_max("mcHF", ControlId::NoiseReductionLevel, 15),
            15
        );
        assert_eq!(super::radio_control_max("mcHF", ControlId::Agc, 4), 4);
        assert_eq!(
            super::radio_control_max("FTDX10", ControlId::NoiseReductionLevel, 15),
            15
        );
        assert_eq!(super::radio_control_max("FTDX10", ControlId::Agc, 4), 4);
    }

    #[test]
    fn null_profiles_preserve_configured_audio_devices() {
        assert_eq!(
            effective_audio_input_device("null", Some("Physical microphone".to_string())),
            Some("Physical microphone".to_string())
        );
        assert_eq!(
            effective_audio_output_device("mock", Some("Physical speakers".to_string())),
            Some("Physical speakers".to_string())
        );
        assert_eq!(
            effective_audio_input_device("native", Some("Physical microphone".to_string())),
            Some("Physical microphone".to_string())
        );
        assert_eq!(effective_audio_output_device("native", None), None);
    }

    #[test]
    fn band_visibility_follows_known_radio_capabilities() {
        let hf = native_radio_profile("native", "IC-7300");
        assert!(radio_supports_band(hf, "20m"));
        assert!(!radio_supports_band(hf, "2m"));

        let vhf_uhf = native_radio_profile("native", "IC-9700");
        assert!(!radio_supports_band(vhf_uhf, "20m"));
        assert!(radio_supports_band(vhf_uhf, "2m"));

        let all_mode = native_radio_profile("native", "IC-705");
        assert!(radio_supports_band(all_mode, "20m"));
        assert!(radio_supports_band(all_mode, "2m"));
    }

    #[test]
    fn unknown_radio_band_capabilities_remain_unfiltered() {
        assert!(radio_supports_band(None, "20m"));
        assert!(radio_supports_band(None, "2m"));
    }

    #[test]
    fn swr_display_uses_documented_ic7300_ratio_anchors() {
        assert_eq!(format_swr_display("IC-7300", Some(0)), "1.00:1 (0% meter)");
        assert_eq!(
            format_swr_display("IC-7300", Some(48)),
            "1.50:1 (19% meter)"
        );
        assert_eq!(
            format_swr_display("IC-7300", Some(80)),
            "2.00:1 (31% meter)"
        );
        assert_eq!(
            format_swr_display("IC-7300", Some(120)),
            "3.00:1 (47% meter)"
        );
        assert_eq!(
            format_swr_display("IC-7300", Some(121)),
            ">3.00:1 (47% meter)"
        );
        assert_eq!(format_swr_display("IC-7300", None), "unavailable");
    }

    #[test]
    fn swr_display_does_not_claim_unverified_vendor_ratios() {
        assert_eq!(format_swr_display("FTDX10", Some(128)), "SWR meter 50%");
        assert!((swr_chart_value("FTDX10", 128) - 50.196).abs() < 0.01);
        assert!((swr_chart_value("IC-7300", 80) - 2.0).abs() < f32::EPSILON);
    }

    fn decode_pcm_samples(bytes: &[u8]) -> Vec<i16> {
        bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|chunk| i16::from_le_bytes(*chunk))
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
            apply_waterfall_bins(&mut state, &[(i % 161) as u8; 8]);
        }

        assert_eq!(state.radio_waterfall_rows.len(), RADIO_WF_HEIGHT);
        let latest = state.radio_waterfall_rows.back().unwrap()[0];
        let expected = ((RADIO_WF_HEIGHT + 2) % 161) as u8;
        assert_eq!(latest, scale_scope_levels(&[expected], 1.2)[0]);
    }

    #[test]
    fn audio_cursor_level_tracks_selected_frequency() {
        let mut rows = VecDeque::new();
        let mut row = vec![0u8; AUDIO_BINS];
        let selected_hz = 1_500;
        let selected_bin = ((selected_hz as f32 / AUDIO_MAX_FREQ_HZ as f32)
            * (AUDIO_BINS - 1) as f32)
            .round() as usize;
        row[selected_bin] = 210;
        rows.push_back(row);

        assert_eq!(audio_cursor_level(&rows, selected_hz), 210);
        assert_eq!(audio_cursor_level(&rows, 3_000), 0);
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
    fn completed_ft8_results_are_appended_to_the_visible_log() {
        let entry = |period, message: &str| Ft8DecodeEntry {
            period,
            utc: "12:00:00".to_string(),
            snr_db: -12,
            dt_s: 0.1,
            freq_hz: 1_500,
            message: message.to_string(),
            is_cq: message.starts_with("CQ "),
        };
        let mut log = Vec::new();

        let removed = append_ft8_log_entries(
            &mut log,
            &[entry(42, "CQ K1ABC FN42"), entry(42, "W1AW K1ABC -12")],
            80,
        );

        assert_eq!(removed, 0);
        assert_eq!(log.len(), 2);
        assert_eq!(log[0].message, "CQ K1ABC FN42");
        assert_eq!(log[1].message, "W1AW K1ABC -12");
    }

    #[test]
    fn completed_ft8_results_keep_the_newest_rows_when_log_is_full() {
        let entry = |period| Ft8DecodeEntry {
            period,
            utc: format!("12:00:{period:02}"),
            snr_db: -12,
            dt_s: 0.1,
            freq_hz: 1_500,
            message: format!("CQ K{period}ABC FN42"),
            is_cq: true,
        };
        let mut log = vec![entry(1), entry(2)];

        let removed = append_ft8_log_entries(&mut log, &[entry(3)], 2);
        log.drain(..removed);

        assert_eq!(log.iter().map(|row| row.period).collect::<Vec<_>>(), [2, 3]);
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
            let n = samples.len().clamp(2, FFT_SIZE);
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
    fn automatic_digital_visuals_use_the_fast_radio_scope_cadence() {
        let tuning = DisplayTuning::default();
        assert_eq!(effective_visual_profile(&tuning, "USB-D", true), (50, 0));
        assert_eq!(effective_visual_profile(&tuning, "FT8", false), (50, 0));
    }

    #[test]
    fn labels_all_core_display_choices() {
        assert_eq!(AppLogLevelFilter::All.label(), "All levels");
        assert_eq!(AppLogLevelFilter::Info.label(), "Info+");
        assert_eq!(AppLogLevelFilter::Warning.label(), "Warnings+");
        assert_eq!(AppLogLevelFilter::Error.label(), "Errors only");
        assert_eq!(WaterfallSpeed::Slow.label(), "Slow · ~4.5 rows/s");
        assert_eq!(WaterfallSpeed::Mid.label(), "Mid · ~8 rows/s");
        assert_eq!(WaterfallSpeed::Fast.label(), "Fast · ~20 rows/s");
        assert_eq!(WaterfallTheme::RadioBlue.label(), "Radio blue");
        assert_eq!(WaterfallTheme::Inferno.label(), "Inferno");
        assert_eq!(WaterfallTheme::Phosphor.label(), "Phosphor");
        assert_eq!(WaterfallTheme::Monochrome.label(), "Monochrome");
    }

    #[test]
    fn clamps_platform_gui_scale_and_preserves_round_trip_values() {
        assert_eq!(platform_gui_scale_from_percent(25), GUI_SCALE_MIN);
        assert_eq!(platform_gui_scale_from_percent(250), GUI_SCALE_MAX);
        let scale = platform_gui_scale_from_percent(125);
        assert!((platform_gui_scale_percent(scale) - 125.0).abs() < 0.01);
    }

    #[test]
    fn labels_sstv_choices_and_operator_call_badges() {
        assert_eq!(SstvOverlayCorner::ALL.len(), 4);
        assert_eq!(SstvOverlayCorner::BottomRight.label(), "Bottom right");
        assert_eq!(SstvAiPipelineMode::ALL.len(), 3);
        assert_eq!(
            SstvAiPipelineMode::AnalyzeReceived.label(),
            "Analyze received"
        );
        assert_eq!(call_hit_badge(OperatorCallHit::DirectedToMe).0, "📡 YOU!");
        assert_eq!(call_hit_badge(OperatorCallHit::Mentioned).0, "✨ YOUR CALL");
    }

    #[test]
    fn manual_waterfall_speeds_match_ic7300_scope_values() {
        let mut tuning = DisplayTuning {
            radio_auto_visual: false,
            ..DisplayTuning::default()
        };
        tuning.radio_waterfall_speed = WaterfallSpeed::Slow;
        assert_eq!(effective_visual_profile(&tuning, "FT8", true), (220, 2));
        tuning.radio_waterfall_speed = WaterfallSpeed::Mid;
        assert_eq!(effective_visual_profile(&tuning, "FT8", true), (120, 1));
        tuning.radio_waterfall_speed = WaterfallSpeed::Fast;
        assert_eq!(effective_visual_profile(&tuning, "FT8", true), (50, 0));
    }

    #[test]
    fn audio_and_radio_visual_tuning_are_independent() {
        let mut tuning = DisplayTuning {
            audio_auto_visual: false,
            radio_auto_visual: false,
            ..DisplayTuning::default()
        };
        tuning.audio_waterfall_speed = WaterfallSpeed::Slow;
        tuning.radio_waterfall_speed = WaterfallSpeed::Fast;

        assert_eq!(effective_visual_profile(&tuning, "USB", false), (220, 2));
        assert_eq!(effective_visual_profile(&tuning, "USB", true), (50, 0));
    }

    #[test]
    fn radio_port_inventory_collects_labels_and_detected_models() {
        let descriptors = vec![
            SerialPortDescriptor {
                port_name: "/dev/ttyUSB0".to_string(),
                display_name: "/dev/ttyUSB0 — Icom IC-7300 (CI-V)".to_string(),
                likely_radio: Some("Icom IC-7300 (CI-V)".to_string()),
            },
            SerialPortDescriptor {
                port_name: "/dev/ttyUSB1".to_string(),
                display_name: "/dev/ttyUSB1 — USB serial".to_string(),
                likely_radio: None,
            },
        ];

        let (ports, labels, models) = radio_port_inventory(descriptors);
        assert_eq!(ports, vec!["/dev/ttyUSB0", "/dev/ttyUSB1"]);
        assert_eq!(
            labels.get("/dev/ttyUSB0").map(String::as_str),
            Some("/dev/ttyUSB0 — Icom IC-7300 (CI-V)")
        );
        assert_eq!(models, vec!["Icom IC-7300 (CI-V)"]);
    }

    #[test]
    fn filter_locked_scope_covers_the_useful_sideband() {
        assert_eq!(scope_span_for_filter("USB-D", Some(1)), 1);
        assert_eq!(
            scope_span_hz(scope_span_for_filter("USB-D", Some(1))),
            5_000
        );
        assert_eq!(scope_span_for_filter("FM", Some(1)), 2);
        assert_eq!(scope_span_label(0), "±2.5 kHz");
    }

    #[test]
    fn display_policy_covers_filter_modes_and_scope_span_limits() {
        assert_eq!(filter_bandwidth_hz("CW", Some(2)), 250);
        assert_eq!(filter_bandwidth_hz("FM", Some(3)), 7_000);
        assert_eq!(filter_bandwidth_hz("RTTY", Some(2)), 350);
        assert_eq!(filter_bandwidth_hz("USB", Some(3)), 1_800);
        assert_eq!(filter_bandwidth_hz("USB", None), 3_000);
        assert_eq!(filter_bandwidth_hz("USB", Some(9)), 3_000);

        assert_eq!(scope_span_for_filter("CW", Some(1)), 0);
        assert_eq!(scope_span_for_filter("FM", Some(1)), 2);
        assert_eq!(scope_span_for_filter("FM", Some(9)), 2);
        assert_eq!(scope_span_label(7), "±500 kHz");
        assert_eq!(scope_span_label(99), "±500 kHz");
        assert_eq!(scope_span_hz(6), 250_000);
        assert_eq!(scope_span_hz(99), 500_000);
    }

    #[test]
    fn display_policy_classifies_modes_errors_and_band_edges() {
        assert_eq!(
            scope_projection_for_mode("LSB-D"),
            ScopeProjection::LowerSideband
        );
        assert_eq!(
            scope_projection_for_mode("USB"),
            ScopeProjection::UpperSideband
        );
        assert_eq!(
            scope_projection_for_mode("DATA"),
            ScopeProjection::UpperSideband
        );
        assert_eq!(
            scope_projection_for_mode("DIGI"),
            ScopeProjection::UpperSideband
        );
        assert_eq!(scope_projection_for_mode("FM"), ScopeProjection::Full);

        assert!(is_transient_civ_read_error("CI-V response timed out"));
        assert!(is_transient_civ_read_error("failed to read CI-V response"));
        assert!(is_transient_civ_read_error("serial timeout"));
        assert!(!is_transient_civ_read_error("invalid mode response"));

        assert_eq!(band_edges_for_frequency(None), None);
        assert_eq!(
            band_edges_for_frequency(Some(14_074_000)),
            Some((14_000_000, 14_350_000, "20m"))
        );
        assert_eq!(
            band_edges_for_frequency(Some(145_000_000)),
            Some((144_000_000, 148_000_000, "2m"))
        );
        assert_eq!(band_edges_for_frequency(Some(1_000_000)), None);
    }

    #[test]
    fn sideband_edges_round_to_kilohertz_and_saturate() {
        assert_eq!(
            sideband_scope_edges(14_074_123, 5_001, ScopeProjection::UpperSideband),
            Some((14_074_000, 14_080_000))
        );
        assert_eq!(
            sideband_scope_edges(14_074_123, 5_001, ScopeProjection::LowerSideband),
            Some((14_069_000, 14_075_000))
        );
        assert_eq!(
            sideband_scope_edges(100, u64::MAX, ScopeProjection::LowerSideband),
            Some((0, 1_000))
        );
    }

    #[test]
    fn meter_policy_maps_hal_values_and_warning_colors() {
        let snapshot = GuiState {
            signal_meter: Some(1),
            power_meter: Some(2),
            swr: Some(3),
            alc_meter: Some(4),
            compression_meter: Some(5),
            current_meter: Some(6),
            voltage_meter: Some(7),
            temperature_meter: Some(8),
            ..GuiState::default()
        };
        for (id, value) in [
            (MeterId::Signal, 1),
            (MeterId::Power, 2),
            (MeterId::Swr, 3),
            (MeterId::Alc, 4),
            (MeterId::Compression, 5),
            (MeterId::Current, 6),
            (MeterId::Voltage, 7),
            (MeterId::Temperature, 8),
        ] {
            assert_eq!(meter_value(&snapshot, id), Some(value));
        }
        assert_eq!(meter_value(&GuiState::default(), MeterId::Signal), None);
        assert_eq!(meter_color(MeterId::Signal, None), Color32::GRAY);
        assert_eq!(
            meter_color(MeterId::Swr, Some(129)),
            Color32::from_rgb(100, 210, 150)
        );
        assert_eq!(meter_color(MeterId::Swr, Some(130)), Color32::YELLOW);
        assert_eq!(meter_color(MeterId::Swr, Some(190)), Color32::RED);
        assert_eq!(
            meter_color(MeterId::Power, Some(220)),
            Color32::from_rgb(255, 145, 100)
        );
        assert_eq!(
            meter_color(MeterId::Temperature, Some(255)),
            Color32::from_rgb(100, 210, 150)
        );
    }

    #[test]
    fn meter_renderers_cover_empty_and_saturated_operator_states() {
        let context = egui::Context::default();
        let mut history = VecDeque::from([0, 64, 128, 255]);
        let _ = context.run(Default::default(), |context| {
            egui::CentralPanel::default().show(context, |ui| {
                draw_voltage_graph(ui, &VecDeque::new(), "—");
                draw_voltage_graph(ui, &history, "13.8 V");
                let rect = egui::Rect::from_min_size(ui.min_rect().min, egui::vec2(420.0, 32.0));
                draw_primary_meter(ui, rect, "", "—", -1.0, Color32::GRAY);
                draw_primary_meter(
                    ui,
                    rect.translate(egui::vec2(0.0, 40.0)),
                    "S9",
                    "100%",
                    1.0,
                    Color32::GREEN,
                );
                draw_primary_meter(
                    ui,
                    rect.translate(egui::vec2(0.0, 80.0)),
                    "POWER",
                    "50%",
                    0.5,
                    Color32::YELLOW,
                );
            });
        });
        history.clear();
        assert!(history.is_empty());
    }

    #[test]
    fn display_policy_labels_and_auto_profiles_cover_all_operator_choices() {
        assert_eq!(
            [
                WaterfallSpeed::Slow,
                WaterfallSpeed::Mid,
                WaterfallSpeed::Fast
            ]
            .map(WaterfallSpeed::label),
            ["Slow · ~4.5 rows/s", "Mid · ~8 rows/s", "Fast · ~20 rows/s"]
        );
        assert_eq!(
            [
                WaterfallTheme::RadioBlue,
                WaterfallTheme::Inferno,
                WaterfallTheme::Phosphor,
                WaterfallTheme::Monochrome,
            ]
            .map(WaterfallTheme::label),
            ["Radio blue", "Inferno", "Phosphor", "Monochrome"]
        );
        assert_eq!(call_hit_badge(OperatorCallHit::DirectedToMe).0, "📡 YOU!");
        assert_eq!(call_hit_badge(OperatorCallHit::Mentioned).0, "✨ YOUR CALL");
        assert_eq!(
            [
                Ft8SeqState::Idle,
                Ft8SeqState::CqArmed,
                Ft8SeqState::ReplyArmed,
                Ft8SeqState::TxQueued,
            ]
            .map(Ft8SeqState::label),
            ["IDLE", "CQ ARMED", "REPLY ARMED", "TX QUEUED"]
        );

        let mut tuning = DisplayTuning {
            radio_auto_visual: false,
            audio_auto_visual: false,
            ..DisplayTuning::default()
        };
        for speed in [
            (WaterfallSpeed::Slow, (220, 2)),
            (WaterfallSpeed::Mid, (120, 1)),
            (WaterfallSpeed::Fast, (50, 0)),
        ] {
            tuning.radio_waterfall_speed = speed.0;
            assert_eq!(effective_visual_profile(&tuning, "USB", true), speed.1);
        }
        tuning.radio_auto_visual = true;
        assert_eq!(effective_visual_profile(&tuning, "FT8-DATA", true), (50, 0));
        assert_eq!(effective_visual_profile(&tuning, "FM", true), (120, 1));
        assert_eq!(effective_visual_profile(&tuning, "USB", true), (90, 1));
        tuning.audio_auto_visual = true;
        assert_eq!(effective_visual_profile(&tuning, "USB", false), (90, 1));
    }

    #[test]
    fn narrow_scope_uses_asymmetric_full_resolution_edges_for_each_sideband() {
        assert_eq!(
            scope_projection_for_mode("USB-D"),
            ScopeProjection::UpperSideband
        );
        assert_eq!(
            sideband_scope_edges(14_074_000, 5_000, ScopeProjection::UpperSideband),
            Some((14_074_000, 14_079_000))
        );
        assert_eq!(
            scope_projection_for_mode("LSB"),
            ScopeProjection::LowerSideband
        );
        assert_eq!(
            sideband_scope_edges(7_074_000, 5_000, ScopeProjection::LowerSideband),
            Some((7_069_000, 7_074_000))
        );
        assert_eq!(scope_projection_for_mode("CW"), ScopeProjection::Full);
        assert_eq!(
            sideband_scope_edges(7_074_000, 5_000, ScopeProjection::Full),
            None
        );
    }

    #[test]
    fn scope_level_scaling_uses_the_documented_zero_to_160_range() {
        assert_eq!(scale_scope_levels(&[0, 80, 160], 1.0), vec![0, 128, 255]);
        assert!(scale_scope_levels(&[40], 2.0)[0] > scale_scope_levels(&[40], 1.0)[0]);
    }

    #[test]
    fn radio_waterfall_preserves_native_bin_edges() {
        let mut state = GuiState {
            radio_scope_contrast: 1.0,
            ..GuiState::default()
        };
        apply_waterfall_bins(&mut state, &[0, 0, 160, 0, 0]);
        assert_eq!(
            state.radio_waterfall_rows.back().unwrap(),
            &[0, 0, 255, 0, 0]
        );
    }

    #[test]
    fn radio_waterfall_draws_narrow_and_overview_scopes_headlessly() {
        let icon = eframe::icon_data::from_png_bytes(QSONAUT_ICON_PNG).expect("test icon");
        let mut app = QsonautGuiApp::new_with_context(
            AppConfig::default(),
            false,
            false,
            &egui::Context::default(),
            &icon,
            eframe::Renderer::Wgpu,
            None,
            GraphicsPreferences::from_environment(),
            None,
            Vec::new(),
            Arc::new(Mutex::new(None)),
        );
        app.civ_spectrum_on = true;
        let context = egui::Context::default();
        let mut snapshot = GuiState {
            frequency_hz: Some(14_074_000),
            mode: "USB".to_string(),
            radio_waterfall_revision: 1,
            radio_waterfall_rows: std::collections::VecDeque::from([
                vec![0, 32, 96, 160],
                vec![160, 96, 32, 0],
            ]),
            ..GuiState::default()
        };

        for scope in [RadioScopeView::Narrow, RadioScopeView::Overview] {
            app.radio_scope_view = scope;
            let _ = context.run(Default::default(), |context| {
                egui::CentralPanel::default().show(context, |ui| {
                    app.draw_radio_waterfall(ui, context, &snapshot);
                });
            });
        }

        snapshot.radio_waterfall_revision = 2;
        let _ = context.run(Default::default(), |context| {
            egui::CentralPanel::default().show(context, |ui| {
                app.draw_radio_waterfall(ui, context, &snapshot);
            });
        });
        assert!(app.radio_waterfall_texture.is_some());
    }

    #[test]
    fn hunter_panel_renders_empty_and_populated_activity_headlessly() {
        let icon = eframe::icon_data::from_png_bytes(QSONAUT_ICON_PNG).expect("test icon");
        let mut app = QsonautGuiApp::new_with_context(
            AppConfig::default(),
            false,
            false,
            &egui::Context::default(),
            &icon,
            eframe::Renderer::Wgpu,
            None,
            GraphicsPreferences::from_environment(),
            None,
            Vec::new(),
            Arc::new(Mutex::new(None)),
        );
        app.qso_log.contacts.clear();
        app.qso_selected = None;
        let context = egui::Context::default();
        let snapshot = GuiState {
            frequency_hz: Some(14_074_000),
            ..GuiState::default()
        };

        let _ = context.run(Default::default(), |context| {
            egui::CentralPanel::default().show(context, |ui| {
                app.draw_hunter_panel(ui, &snapshot);
            });
        });

        app.hunter_unique_heard.insert("K1ABC".to_string());
        app.hunter_directed_hits = 1;
        app.hunter_dupe_blocks = 2;
        app.hunter_decode_bursts = 3;
        app.hunter_unlocked.insert(AchievementKind::FirstDecode);
        app.hunter_feed.push_back(HunterAlert {
            utc: "12:34:56".to_string(),
            title: "Signal Hunter".to_string(),
            detail: "Captured a decode".to_string(),
            accent: Color32::LIGHT_GREEN,
        });
        let _ = context.run(Default::default(), |context| {
            egui::CentralPanel::default().show(context, |ui| {
                app.draw_hunter_panel(ui, &snapshot);
            });
        });
        app.hunter_show_acknowledged = true;
        app.hunter_acknowledged.insert(AchievementKind::FirstDecode);
        let _ = context.run(Default::default(), |context| {
            egui::CentralPanel::default().show(context, |ui| {
                app.draw_hunter_panel(ui, &snapshot);
            });
        });
    }

    #[test]
    fn contact_log_renders_empty_and_selected_contact_editor_headlessly() {
        let icon = eframe::icon_data::from_png_bytes(QSONAUT_ICON_PNG).expect("test icon");
        let mut app = QsonautGuiApp::new_with_context(
            AppConfig::default(),
            false,
            false,
            &egui::Context::default(),
            &icon,
            eframe::Renderer::Wgpu,
            None,
            GraphicsPreferences::from_environment(),
            None,
            Vec::new(),
            Arc::new(Mutex::new(None)),
        );
        app.qso_log.contacts.clear();
        app.qso_selected = None;
        let context = egui::Context::default();
        let snapshot = GuiState {
            frequency_hz: Some(14_074_000),
            mode: "USB".to_string(),
            ..GuiState::default()
        };

        let _ = context.run(Default::default(), |context| {
            egui::CentralPanel::default().show(context, |ui| {
                app.draw_contact_log(ui, &snapshot);
            });
        });

        let contact = QsoRecord::new("K1ABC", "FT8", "20m", 14_074_000, 1, 1);
        app.qso_selected = Some(contact.id);
        app.qso_log.contacts.push(contact);
        let _ = context.run(Default::default(), |context| {
            egui::CentralPanel::default().show(context, |ui| {
                app.draw_contact_log(ui, &snapshot);
            });
        });
        assert_eq!(app.qso_selected, Some(app.qso_log.contacts[0].id));
    }

    #[test]
    fn app_log_panel_filters_and_renders_levelled_lines_headlessly() {
        let icon = eframe::icon_data::from_png_bytes(QSONAUT_ICON_PNG).expect("test icon");
        let mut app = QsonautGuiApp::new_with_context(
            AppConfig::default(),
            false,
            false,
            &egui::Context::default(),
            &icon,
            eframe::Renderer::Wgpu,
            None,
            GraphicsPreferences::from_environment(),
            None,
            Vec::new(),
            Arc::new(Mutex::new(None)),
        );
        app.app_log_status = "test log".to_string();
        app.app_log_last_refresh = Instant::now();
        app.app_log_text = [
            "2026-09-01 INFO RX decode received",
            "2026-09-01 WARN Audio fallback",
            "2026-09-01 ERROR Radio failed",
            "2026-09-01 DEBUG worker detail",
            "2026-09-01 TRACE parser detail",
        ]
        .join("\n");
        let context = egui::Context::default();
        let _ = context.run(Default::default(), |context| {
            egui::CentralPanel::default().show(context, |ui| {
                app.draw_app_log_panel(ui);
            });
        });
        app.app_log_filter = "missing".to_string();
        app.app_log_level_filter = AppLogLevelFilter::Error;
        let _ = context.run(Default::default(), |context| {
            egui::CentralPanel::default().show(context, |ui| {
                app.draw_app_log_panel(ui);
            });
        });
    }

    #[test]
    fn audio_waterfall_renders_mode_specific_cursor_scopes_headlessly() {
        let icon = eframe::icon_data::from_png_bytes(QSONAUT_ICON_PNG).expect("test icon");
        let mut app = QsonautGuiApp::new_with_context(
            AppConfig::default(),
            false,
            false,
            &egui::Context::default(),
            &icon,
            eframe::Renderer::Wgpu,
            None,
            GraphicsPreferences::from_environment(),
            None,
            Vec::new(),
            Arc::new(Mutex::new(None)),
        );
        let context = egui::Context::default();
        let snapshot = GuiState {
            audio_waterfall_revision: 1,
            audio_waterfall_rows: std::collections::VecDeque::from([vec![0; 512], vec![160; 512]]),
            filter: Some(2),
            ..GuiState::default()
        };
        for mode in [
            WorkspaceMode::Ft8,
            WorkspaceMode::Ft4,
            WorkspaceMode::Fst4,
            WorkspaceMode::Wspr,
            WorkspaceMode::Jt9,
            WorkspaceMode::Jt65,
            WorkspaceMode::Q65,
            WorkspaceMode::Cw,
            WorkspaceMode::Sstv,
        ] {
            app.workspace_mode = mode;
            app.sstv_auto_target = mode == WorkspaceMode::Sstv;
            let _ = context.run(Default::default(), |context| {
                egui::CentralPanel::default().show(context, |ui| {
                    app.draw_audio_waterfall(ui, context, &snapshot);
                });
            });
        }
        assert!(app.audio_waterfall_texture.is_some());
    }

    #[test]
    fn gui_state_defaults_to_sharp_radio_scope_vbw() {
        assert!(!GuiState::default().radio_scope_vbw_wide);
    }

    #[test]
    fn decode_workspace_height_split_fits_available_viewport() {
        for available_height in [180.0, 320.0, 600.0, 1_000.0] {
            let (decode_height, tx_height) =
                QsonautGuiApp::split_decode_workspace_height(available_height);
            assert!(decode_height >= 0.0);
            assert!((decode_height + tx_height + 4.0 - available_height).abs() < 0.01);
            assert!(tx_height <= 180.0);
        }
    }

    #[test]
    fn decode_workspace_height_split_does_not_force_large_minimums() {
        let (decode_height, tx_height) = QsonautGuiApp::split_decode_workspace_height(180.0);
        assert!(decode_height < 120.0);
        assert_eq!(tx_height, 72.0);
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
    fn ft8_slot_gate_honors_adaptive_decode_threshold() {
        let mut gate = Ft8SlotGate::default();

        assert!(!gate.observe_at(700, 13.0, 13.8, true));
        assert!(!gate.observe_at(701, 0.1, 13.8, false));
        assert!(!gate.observe_at(701, 13.79, 13.8, true));
        assert!(gate.observe_at(701, 13.8, 13.8, true));
        assert!(!gate.observe_at(701, 14.2, 13.8, true));
    }

    #[test]
    fn ft8_slot_gate_waits_for_buffer_readiness_at_decode_time() {
        let mut gate = Ft8SlotGate::default();

        assert!(!gate.observe_at(800, 13.0, 13.2, true));
        assert!(!gate.observe_at(801, 0.0, 13.2, false));
        assert!(!gate.observe_at(801, 13.2, 13.2, false));
        assert!(gate.observe_at(801, 13.25, 13.2, true));
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
    fn digital_slot_gate_reset_requires_new_boundary_again() {
        let mut gate = DigitalSlotGate::default();
        assert!(!gate.boundary(10, true));
        assert!(gate.boundary(11, true));
        gate.reset();
        assert!(!gate.boundary(20, true));
        assert!(gate.boundary(21, true));
    }

    #[test]
    fn phase1_target_modes_have_slot_and_decoder_support() {
        let targets = [WorkspaceMode::Ft8, WorkspaceMode::Ft4, WorkspaceMode::Jt9];
        for mode in targets {
            assert!(
                mode.core_slot_seconds().is_some(),
                "{} should define slot timing",
                mode.label()
            );
            assert!(
                mode.has_native_decoder(),
                "{} should have a native decoder path",
                mode.label()
            );
        }
    }

    #[test]
    fn native_digital_tx_builders_generate_audio() {
        for mode in [
            WorkspaceMode::Ft4,
            WorkspaceMode::Wspr,
            WorkspaceMode::Fst4,
            WorkspaceMode::Jt9,
            WorkspaceMode::Jt65,
            WorkspaceMode::Q65,
        ] {
            let message = if mode == WorkspaceMode::Wspr {
                "K1ABC FN42 37"
            } else {
                "CQ W1AW AA00"
            };
            let (pcm, offset) = build_native_digital_tx_pcm(
                mode,
                message,
                1_500,
                modes::fst4::Submode::default(),
                20,
                600,
            )
            .unwrap_or_else(|error| panic!("{} synthesis failed: {error}", mode.label()));
            assert!(!pcm.is_empty(), "{} synthesis was empty", mode.label());
            assert!(pcm.iter().any(|sample| *sample != 0));
            assert!(offset >= 0.0);
        }
    }

    #[test]
    fn cw_builder_generates_audio() {
        let (pcm, offset) = build_native_digital_tx_pcm(
            WorkspaceMode::Cw,
            "SOS",
            600,
            modes::fst4::Submode::default(),
            20,
            600,
        )
        .expect("CW synthesis");
        assert_eq!(offset, 0.0);
        assert!(pcm.iter().any(|sample| *sample != 0));
        assert!(pcm[pcm.len() - 12_000..].iter().all(|sample| *sample == 0));
    }

    #[test]
    fn cw_builder_stream_decode_flushes_the_final_character() {
        let (pcm, _) = build_native_digital_tx_pcm(
            WorkspaceMode::Cw,
            "N7UF",
            700,
            modes::fst4::Submode::default(),
            20,
            700,
        )
        .expect("CW synthesis");
        let mut samples = vec![0.0_f32; 12_000];
        samples.extend(
            pcm.into_iter()
                .map(|sample| sample as f32 / i16::MAX as f32)
                .collect::<Vec<_>>(),
        );
        let mut decoder = qsonaut_third_party::cw::CwChannel::new(12_000, 700, 20);
        let mut text = String::new();
        for event in decoder.push_samples_with_audio(&samples).0 {
            if let qsonaut_third_party::cw::CwDecode::Character(character) = event {
                text.push(character);
            }
        }
        for event in decoder.finish() {
            if let qsonaut_third_party::cw::CwDecode::Character(character) = event {
                text.push(character);
            }
        }
        assert!(text.contains("N7UF"), "decoded CW text was {text:?}");
    }

    #[test]
    fn cw_builder_survives_wpm_and_chunk_boundary_variation() {
        for wpm in [5, 10, 20, 40] {
            let (pcm, _) = build_native_digital_tx_pcm(
                WorkspaceMode::Cw,
                "CQ N7UF",
                700,
                modes::fst4::Submode::default(),
                wpm,
                700,
            )
            .expect("CW synthesis");
            let mut samples = vec![0.0_f32; 12_000];
            samples.extend(
                pcm.into_iter()
                    .map(|sample| sample as f32 / i16::MAX as f32),
            );
            let mut decoder = qsonaut_third_party::cw::CwChannel::new(12_000, 700, wpm);
            let mut text = String::new();
            let mut cursor = 0;
            for width in [1, 7, 31, 127, 503, 1_001] {
                let end = (cursor + width).min(samples.len());
                for event in decoder.push_samples_with_audio(&samples[cursor..end]).0 {
                    if let qsonaut_third_party::cw::CwDecode::Character(character) = event {
                        text.push(character);
                    }
                }
                cursor = end;
                if cursor == samples.len() {
                    break;
                }
            }
            while cursor < samples.len() {
                let end = (cursor + 257).min(samples.len());
                for event in decoder.push_samples_with_audio(&samples[cursor..end]).0 {
                    if let qsonaut_third_party::cw::CwDecode::Character(character) = event {
                        text.push(character);
                    }
                }
                cursor = end;
            }
            for event in decoder.finish() {
                if let qsonaut_third_party::cw::CwDecode::Character(character) = event {
                    text.push(character);
                }
            }
            assert!(
                text.contains("N7UF"),
                "{wpm} WPM decoded CW text was {text:?}"
            );
        }
    }

    #[test]
    fn cw_builder_rejects_unsupported_punctuation() {
        let error = build_native_digital_tx_pcm(
            WorkspaceMode::Cw,
            "CQ?",
            600,
            modes::fst4::Submode::default(),
            20,
            600,
        )
        .expect_err("punctuation must be rejected");
        assert!(error.to_string().contains("does not support '?'"));
    }

    #[test]
    fn ft4_workspace_adapter_decodes_generated_audio() {
        let (pcm, offset_s) = build_native_digital_tx_pcm(
            WorkspaceMode::Ft4,
            "CQ W1AW AA00",
            1_500,
            modes::fst4::Submode::default(),
            20,
            600,
        )
        .expect("FT4 synthesis");
        let mut slot = vec![0.0f32; (7.5 * 12_000.0) as usize];
        let start = (offset_s * 12_000.0) as usize;
        for (dst, sample) in slot[start..].iter_mut().zip(pcm) {
            *dst = sample as f32 / i16::MAX as f32;
        }
        let state = Arc::new(Mutex::new(GuiState::default()));
        run_native_digital_decode(
            WorkspaceMode::Ft4,
            modes::fst4::Submode::default(),
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
    fn ft4_null_fixture_tolerates_level_and_timing_variation() {
        let (pcm, offset_s) = build_native_digital_tx_pcm(
            WorkspaceMode::Ft4,
            "CQ W1AW AA00",
            1_500,
            modes::fst4::Submode::default(),
            20,
            600,
        )
        .expect("FT4 synthesis");
        for (amplitude, start_s) in [(0.03_f32, 0.5_f64), (0.06, 0.9), (0.12, 1.2)] {
            let mut slot = vec![0.0_f32; (7.5 * 12_000.0) as usize];
            let start = ((offset_s + start_s - 0.5) * 12_000.0).round() as usize;
            for (dst, sample) in slot[start..].iter_mut().zip(pcm.iter().copied()) {
                *dst = sample as f32 / i16::MAX as f32 * amplitude;
            }
            let state = Arc::new(Mutex::new(GuiState::default()));
            run_native_digital_decode(
                WorkspaceMode::Ft4,
                modes::fst4::Submode::default(),
                slot,
                10,
                "00:01:15.000".to_string(),
                1_500,
                false,
                state.clone(),
            );
            assert!(
                state
                    .lock()
                    .expect("state")
                    .digital_decodes
                    .iter()
                    .any(|entry| entry.message == "CQ W1AW AA00"),
                "FT4 failed at amplitude {amplitude} and start {start_s}s"
            );
        }
    }

    #[test]
    fn jt9_workspace_adapter_decodes_generated_audio() {
        let (pcm, offset_s) = build_native_digital_tx_pcm(
            WorkspaceMode::Jt9,
            "CQ W1AW AA00",
            1_500,
            modes::fst4::Submode::default(),
            20,
            600,
        )
        .expect("JT9 synthesis");
        let slot_samples = (60.0 * 12_000.0) as usize;
        let mut slot = vec![0.0f32; slot_samples];
        let start = (offset_s * 12_000.0) as usize;
        for (dst, sample) in slot[start..].iter_mut().zip(pcm) {
            *dst = sample as f32 / i16::MAX as f32;
        }
        let state = Arc::new(Mutex::new(GuiState::default()));
        run_native_digital_decode(
            WorkspaceMode::Jt9,
            modes::fst4::Submode::default(),
            slot,
            10,
            "00:10:00.000".to_string(),
            1_500,
            false,
            state.clone(),
        );
        let shared = state.lock().expect("state");
        assert!(
            shared.digital_decodes.iter().any(|entry| {
                entry.mode == WorkspaceMode::Jt9
                    && entry.message.contains("W1AW")
                    && entry.message.contains("AA00")
            }),
            "JT9 decode did not surface expected callsign/grid payload"
        );
    }

    #[test]
    fn early_ft4_capture_contains_a_deliberately_late_decodable_waveform() {
        let (pcm, _) = build_native_digital_tx_pcm(
            WorkspaceMode::Ft4,
            "CQ W1AW AA00",
            1_500,
            modes::fst4::Submode::default(),
            20,
            600,
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
        let slot = prepare_early_digital_slot(&rolling, captured, FT4_SLOT_SAMPLES, 0.0);
        let state = Arc::new(Mutex::new(GuiState::default()));
        run_native_digital_decode(
            WorkspaceMode::Ft4,
            modes::fst4::Submode::default(),
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
        let audio = qsonaut_modems::AudioBlock::new(12_000, slot).expect("normalized audio");
        let outcome = qsonaut_third_party::wsjt::decode_ft8(
            &audio,
            &qsonaut_third_party::wsjt::WsjtDecodeConfig {
                frequency_min_hz: 100.0,
                frequency_max_hz: 3_000.0,
                sync_min: FT8_FAST_SYNC_MIN,
                max_candidates: FT8_FAST_MAX_CAND,
                ..qsonaut_third_party::wsjt::WsjtDecodeConfig::default()
            },
        )
        .expect("FT8 audio and mode are valid");

        let messages: Vec<String> = outcome
            .events
            .into_iter()
            .map(|event| event.message)
            .collect();
        assert!(
            messages.iter().any(|message| message == "CQ W1AW AA00"),
            "early decode messages: {messages:?}"
        );
    }

    fn add_deterministic_noise(samples: &mut [f32], amplitude: f32) {
        let mut state = 0x6d2b_79f5_u32;
        for sample in samples {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let unit = ((state >> 8) as f32 / 16_777_215.0) * 2.0 - 1.0;
            *sample += unit * amplitude;
        }
    }

    #[test]
    fn ft8_fixture_tolerates_level_timing_and_low_noise_variation() {
        let pcm = build_ft8_tx_pcm("CQ W1AW AA00", 1_500).expect("FT8 PCM");
        for (amplitude, timing_offset_s, noise) in [
            (0.03_f32, -0.2_f64, 0.0002_f32),
            (0.06, 0.0, 0.0005),
            (0.12, 0.2, 0.0008),
        ] {
            let mut slot = vec![0.0_f32; FT8_SLOT_SAMPLES];
            let start = ((FT8_TX_AUDIO_START_S + timing_offset_s) * 12_000.0)
                .round()
                .max(0.0) as usize;
            for (dst, sample) in slot[start..].iter_mut().zip(pcm.iter().copied()) {
                *dst = sample as f32 / i16::MAX as f32 * amplitude;
            }
            add_deterministic_noise(&mut slot, noise);
            let audio =
                qsonaut_modems::AudioBlock::new(12_000, slot).expect("normalized FT8 audio");
            let outcome = qsonaut_third_party::wsjt::decode_ft8(
                &audio,
                &qsonaut_third_party::wsjt::WsjtDecodeConfig {
                    frequency_min_hz: 100.0,
                    frequency_max_hz: 3_000.0,
                    sync_min: FT8_FAST_SYNC_MIN,
                    max_candidates: FT8_FAST_MAX_CAND,
                    ..qsonaut_third_party::wsjt::WsjtDecodeConfig::default()
                },
            )
            .expect("FT8 audio and mode are valid");
            assert!(
                outcome
                    .events
                    .iter()
                    .any(|event| event.message == "CQ W1AW AA00"),
                "FT8 failed at amplitude {amplitude}, timing {timing_offset_s}s, noise {noise}: {:?}",
                outcome.events
            );
        }
    }

    #[test]
    fn parse_automation_hook_detail_extracts_key_value_pairs() {
        let fields = parse_automation_hook_detail(
            "enabled=true mode=run split=fake role=hound serial_start=1",
        );
        assert_eq!(fields.get("enabled").map(String::as_str), Some("true"));
        assert_eq!(fields.get("mode").map(String::as_str), Some("run"));
        assert_eq!(fields.get("split").map(String::as_str), Some("fake"));
        assert_eq!(fields.get("role").map(String::as_str), Some("hound"));
        assert_eq!(fields.get("serial_start").map(String::as_str), Some("1"));
    }

    #[test]
    fn normalize_contest_profile_event_for_automation() {
        let event = normalize_app_event_for_automation(AppEvent::ContestProfileChanged {
            enabled: true,
            operating_mode: "run".to_string(),
            split_policy: "off".to_string(),
            fox_hound_role: "disabled".to_string(),
        })
        .expect("automation event");

        assert_eq!(event.kind, EventKind::ContestState);
        assert_eq!(
            event.fields.get("enabled").map(String::as_str),
            Some("true")
        );
        assert_eq!(
            event.fields.get("operating_mode").map(String::as_str),
            Some("run")
        );
        assert_eq!(
            event.fields.get("split_policy").map(String::as_str),
            Some("off")
        );
    }

    #[test]
    fn normalize_external_message_event_for_automation() {
        let event = normalize_app_event_for_automation(AppEvent::ExternalMessageReceived {
            source: "discord:shack".to_string(),
            author: "W1AW".to_string(),
            message: "!rig".to_string(),
            channel: "#qsonaut".to_string(),
        })
        .expect("automation event");

        assert_eq!(event.kind, EventKind::ExternalMessage);
        assert_eq!(event.source, "discord:shack");
        assert_eq!(event.fields.get("author").map(String::as_str), Some("W1AW"));
        assert_eq!(
            event.fields.get("message").map(String::as_str),
            Some("!rig")
        );
        assert_eq!(
            event.fields.get("channel").map(String::as_str),
            Some("#qsonaut")
        );
    }

    #[test]
    fn normalize_app_events_preserves_tags_and_structured_fields() {
        let callsign = normalize_app_event_for_automation(AppEvent::CallsignHit {
            mode: "FT8".to_string(),
            call: "W1AW".to_string(),
            snr_db: -12.5,
            freq_hz: 1_500,
            message: "CQ W1AW FN42".to_string(),
            directed_to_me: true,
        })
        .expect("callsign event");
        assert_eq!(callsign.kind, EventKind::CallsignHit);
        assert!(callsign.tags.iter().any(|tag| tag == "directed_to_me"));
        assert_eq!(
            callsign.fields.get("snr").map(String::as_str),
            Some("-12.5")
        );

        let qso = normalize_app_event_for_automation(AppEvent::QsoLogged {
            mode: "CW".to_string(),
            call: "K1ABC".to_string(),
            band: "20m".to_string(),
            frequency_hz: 14_060_000,
        })
        .expect("qso event");
        assert_eq!(qso.kind, EventKind::QsoLogged);
        assert_eq!(
            qso.fields.get("frequency_hz").map(String::as_str),
            Some("14060000")
        );

        let mut fields = BTreeMap::new();
        fields.insert("frequency_hz".to_string(), "14074000".to_string());
        let server = normalize_app_event_for_automation(AppEvent::ServerMessageReceived {
            kind: "radio_state".to_string(),
            fields,
        })
        .expect("server event");
        assert_eq!(server.kind, EventKind::ServerMessage);
        assert!(server.tags.iter().any(|tag| tag == "radio_state"));
        assert_eq!(
            server.fields.get("kind").map(String::as_str),
            Some("radio_state")
        );
    }

    #[test]
    fn normalize_automation_hooks_maps_supported_kinds_and_rejects_unknown_events() {
        for (kind, expected) in [
            ("contest_state", EventKind::ContestState),
            ("operator_profile", EventKind::OperatorProfile),
            ("callsign_hit", EventKind::CallsignHit),
            ("qso_logged", EventKind::QsoLogged),
            ("radio_state", EventKind::RadioState),
        ] {
            let event = normalize_app_event_for_automation(AppEvent::AutomationHook {
                kind: kind.to_string(),
                source: "test".to_string(),
                detail: "enabled=true".to_string(),
            })
            .expect("supported hook");
            assert_eq!(event.kind, expected);
            assert_eq!(
                event.fields.get("enabled").map(String::as_str),
                Some("true")
            );
        }
        assert!(
            normalize_app_event_for_automation(AppEvent::AutomationHook {
                kind: "not_supported".to_string(),
                source: "test".to_string(),
                detail: String::new(),
            })
            .is_none()
        );
        assert!(normalize_app_event_for_automation(AppEvent::ShutdownRequested).is_none());
        assert!(
            normalize_app_event_for_automation(AppEvent::DeviceDiscovered {
                subsystem: "radio".to_string(),
                name: "IC-7300".to_string(),
                detail: "test".to_string(),
            })
            .is_none()
        );
    }

    #[test]
    fn parse_tx_target_from_compose_uses_operator_role() {
        assert_eq!(
            parse_tx_target_from_compose("K1ABC N0CALL -10", "N0CALL").as_deref(),
            Some("K1ABC")
        );
        assert_eq!(
            parse_tx_target_from_compose("N0CALL K1ABC -12", "N0CALL").as_deref(),
            Some("K1ABC")
        );
        assert_eq!(
            parse_tx_target_from_compose("CQ N0CALL FN20", "N0CALL"),
            None
        );
    }

    #[test]
    fn configured_external_transports_extracts_declared_source_kinds() {
        let config = RuleComponentConfig {
            component: qsonaut_automation::ComponentManifest {
                id: "test.component".to_string(),
                name: "Test".to_string(),
                version: "0.1.0".to_string(),
                description: String::new(),
                subscriptions: Default::default(),
                requests: Default::default(),
            },
            sources: vec![
                ExternalSourceConfig::Discord {
                    token_env: "QSONAUT_DISCORD_TOKEN".to_string(),
                    guild_id: None,
                    channel_ids: vec!["1".to_string()],
                },
                ExternalSourceConfig::Irc {
                    server: "irc.libera.chat".to_string(),
                    port: 6697,
                    tls: true,
                    nickname: "QSONautBot".to_string(),
                    channels: vec!["#qsonaut".to_string()],
                    password_env: None,
                },
            ],
            rules: vec![],
        };

        let transports = configured_external_transports(&config);
        assert!(transports.contains("discord"));
        assert!(transports.contains("irc"));
        assert_eq!(transports.len(), 2);
    }

    #[test]
    fn external_source_transport_requires_transport_prefix() {
        assert_eq!(
            external_source_transport("discord:shack").as_deref(),
            Some("discord")
        );
        assert_eq!(
            external_source_transport("irc:#qsonaut").as_deref(),
            Some("irc")
        );
        assert_eq!(external_source_transport("bare-source"), None);
    }

    #[test]
    fn parse_workspace_mode_token_recognizes_supported_labels() {
        assert_eq!(parse_workspace_mode_token("FT8"), Some(WorkspaceMode::Ft8));
        assert_eq!(parse_workspace_mode_token("ft4"), Some(WorkspaceMode::Ft4));
        assert_eq!(
            parse_workspace_mode_token("ssb"),
            Some(WorkspaceMode::Voice)
        );
        assert_eq!(
            parse_workspace_mode_token("sstv"),
            Some(WorkspaceMode::Sstv)
        );
        assert_eq!(
            parse_workspace_mode_token("JT65"),
            Some(WorkspaceMode::Jt65)
        );
        assert_eq!(parse_workspace_mode_token("unknown"), None);
    }

    #[test]
    fn radio_mode_label_honors_reported_data_mode() {
        assert_eq!(radio_mode_label("LSB-D", Some(false)), "LSB-D");
        assert_eq!(radio_mode_label("USB", Some(true)), "USB-D");
        assert_eq!(radio_mode_label("USB-D", Some(true)), "USB-D");
    }

    #[test]
    fn workspace_mode_supports_native_tx_matches_current_backends() {
        assert!(workspace_mode_supports_native_tx(WorkspaceMode::Ft4));
        assert!(workspace_mode_supports_native_tx(WorkspaceMode::Jt9));
        assert!(workspace_mode_supports_native_tx(WorkspaceMode::Cw));
        assert!(workspace_mode_supports_native_tx(WorkspaceMode::Sstv));
        assert!(!workspace_mode_supports_native_tx(WorkspaceMode::Voice));
        assert!(!workspace_mode_supports_native_tx(WorkspaceMode::Ft8));
        assert!(!workspace_mode_supports_native_tx(WorkspaceMode::Wspr));
    }

    #[test]
    fn workspace_mode_selection_uses_mode_center_for_current_band() {
        assert_eq!(
            workspace_frequency_for_current_band(WorkspaceMode::Ft4, Some(7_074_000)),
            Some(7_076_000)
        );
        assert_eq!(
            workspace_frequency_for_current_band(WorkspaceMode::Ft8, Some(7_099_000)),
            Some(7_074_000)
        );
        assert_eq!(
            workspace_frequency_for_current_band(WorkspaceMode::Ft8, None),
            None
        );
        assert_eq!(
            workspace_frequency_for_current_band(WorkspaceMode::Sstv, Some(14_074_000)),
            Some(14_230_000)
        );
    }

    #[test]
    fn contributor_metadata_handles_empty_invalid_and_enabled_people() {
        assert!(qsonaut_people(None).is_empty());
        assert!(qsonaut_people(Some("   ")).is_empty());
        assert!(qsonaut_people(Some("not json")).is_empty());

        let people = qsonaut_people(Some(
            r#"[{"name":"Ada","callsign":"K1ADA","role":"Tester","enabled":true},{"name":"Hidden","enabled":false}]"#,
        ));
        assert_eq!(people.len(), 2);
        assert!(people[0].enabled);
        assert!(!people[1].enabled);
    }

    #[test]
    fn contributor_credit_text_formats_identity_role_and_fallbacks() {
        assert_eq!(qsonaut_credit_text(None), "None listed");
        assert_eq!(qsonaut_credit_text(Some("not json")), "not json");
        assert_eq!(
            qsonaut_credit_text(Some(
                r#"[{"name":"Ada","callsign":"K1ADA","role":"Tester"},{"name":"Grace"},{"callsign":"W1GRACE"},{"role":"ignored","enabled":false},{"enabled":true}]"#,
            )),
            "Ada (K1ADA) · Tester, Grace, W1GRACE, Unnamed contributor"
        );
    }

    #[test]
    fn workspace_mode_tokens_cover_aliases_and_whitespace() {
        for (token, expected) in [
            (" FST4 ", WorkspaceMode::Fst4),
            ("WSPR", WorkspaceMode::Wspr),
            ("JT9", WorkspaceMode::Jt9),
            ("JT65", WorkspaceMode::Jt65),
            ("Q65", WorkspaceMode::Q65),
            ("MSK144", WorkspaceMode::Msk144),
            ("CW", WorkspaceMode::Cw),
            ("PHONE", WorkspaceMode::Voice),
        ] {
            assert_eq!(parse_workspace_mode_token(token), Some(expected));
        }
    }

    #[test]
    fn simulated_backends_preserve_audio_device_choices() {
        assert_eq!(
            effective_audio_input_device("null", Some("microphone".to_string())),
            Some("microphone".to_string())
        );
        assert_eq!(effective_audio_input_device("MOCK", None), None);
        assert_eq!(
            effective_audio_input_device("native", Some("microphone".to_string())),
            Some("microphone".to_string())
        );
        assert_eq!(
            effective_audio_output_device("null", Some("speakers".to_string())),
            Some("speakers".to_string())
        );
        assert_eq!(effective_audio_output_device("MOCK", None), None);
        assert_eq!(
            effective_audio_output_device("native", Some("speakers".to_string())),
            Some("speakers".to_string())
        );
    }

    #[test]
    fn native_tx_policy_and_radio_mode_label_cover_conservative_defaults() {
        for mode in [
            WorkspaceMode::Ft4,
            WorkspaceMode::Fst4,
            WorkspaceMode::Jt9,
            WorkspaceMode::Jt65,
            WorkspaceMode::Q65,
            WorkspaceMode::Cw,
            WorkspaceMode::Sstv,
        ] {
            assert!(workspace_mode_supports_native_tx(mode));
        }
        for mode in [
            WorkspaceMode::Ft8,
            WorkspaceMode::Wspr,
            WorkspaceMode::Msk144,
            WorkspaceMode::Voice,
        ] {
            assert!(!workspace_mode_supports_native_tx(mode));
        }
        assert_eq!(radio_mode_label("USB-D", None), "USB-D");
        assert_eq!(radio_mode_label("LSB-D", Some(true)), "LSB-D");
        assert_eq!(radio_mode_label("USB", Some(false)), "USB");
    }

    #[test]
    fn compose_target_parser_handles_cq_local_and_remote_roles() {
        assert_eq!(
            parse_tx_target_from_compose("CQ K1ABC FN42", "N0CALL"),
            None
        );
        assert_eq!(
            parse_tx_target_from_compose("N0CALL K1ABC -10", "N0CALL"),
            Some("K1ABC".to_string())
        );
        assert_eq!(
            parse_tx_target_from_compose("K1ABC N0CALL -10", "N0CALL"),
            Some("K1ABC".to_string())
        );
        assert_eq!(
            parse_tx_target_from_compose("K1ABC W9XYZ -10", "N0CALL"),
            Some("K1ABC".to_string())
        );
        assert_eq!(
            parse_tx_target_from_compose("not a message", "N0CALL"),
            None
        );
    }

    #[test]
    fn qso_timestamp_validates_adif_date_and_time_shapes() {
        let mut record = QsoRecord::new("K1ABC", "FT8", "20m", 14_074_000, 0, 1);
        record.qso_date = "20260901".to_string();
        record.time_on = "1234".to_string();
        assert_eq!(
            qso_timestamp(&record).as_deref(),
            Some("2026-09-01T12:34:00Z")
        );
        record.time_on = "123456".to_string();
        assert_eq!(
            qso_timestamp(&record).as_deref(),
            Some("2026-09-01T12:34:56Z")
        );
        for (date, time) in [
            ("", "1234"),
            ("2026091", "1234"),
            ("2026A901", "1234"),
            ("20260901", "12"),
            ("20260901", "12A4"),
        ] {
            record.qso_date = date.to_string();
            record.time_on = time.to_string();
            assert_eq!(qso_timestamp(&record), None, "invalid {date} {time}");
        }
    }

    #[test]
    fn hamdb_enrichment_fills_missing_fields_but_preserves_operator_values() {
        let temp = tempfile::tempdir().expect("temp cache directory");
        let cache = HamDbCache::open(&temp.path().join("hamdb.sqlite")).expect("cache");
        cache
            .upsert(&HamDbCacheEntry {
                callsign: "K1ABC".to_string(),
                grid: "FN42".to_string(),
                state: "MA".to_string(),
                fetched_at_unix: 100,
                ..HamDbCacheEntry::default()
            })
            .expect("cache entry");

        let mut record = QsoRecord::new(" k1abc ", "FT8", "20m", 14_074_000, 0, 1);
        enrich_qso_from_hamdb(&mut record, &cache, 101);
        assert_eq!(record.grid, "FN42");
        assert_eq!(record.state, "MA");
        assert!(record.hamdb.is_some());

        record.grid = "EM00".to_string();
        record.state = "TX".to_string();
        enrich_qso_from_hamdb(&mut record, &cache, 101);
        assert_eq!(record.grid, "EM00");
        assert_eq!(record.state, "TX");

        let mut missing = QsoRecord::new("", "FT8", "20m", 14_074_000, 0, 1);
        enrich_qso_from_hamdb(&mut missing, &cache, 101);
        assert!(missing.hamdb.is_none());
        let mut stale = QsoRecord::new("K1ABC", "FT8", "20m", 14_074_000, 0, 1);
        enrich_qso_from_hamdb(&mut stale, &cache, 100 + HAMDB_CACHE_TTL_SECONDS + 1);
        assert!(stale.hamdb.is_none());
    }
}
