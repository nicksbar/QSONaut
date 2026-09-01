mod activity;
mod automation_hunter;
mod band_plan;
mod contest;
mod decode_model;
mod graphics;
mod local_ai;
mod modes;
mod panels;
mod profile;
mod radio_faq;
mod server_integration;
mod tx_audio;
mod ui_format;
mod ui_widgets;
mod visuals;
mod window_geometry;
mod workers;

use anyhow::{anyhow, Context, Result};
use eframe::egui::{self, Color32, ColorImage, RichText, TextureHandle, TextureOptions};
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
    if matches!(backend.to_ascii_lowercase().as_str(), "null" | "mock") {
        Some(NULL_INPUT_DEVICE.to_string())
    } else {
        input
    }
}

fn effective_audio_output_device(backend: &str, output: Option<String>) -> Option<String> {
    if matches!(backend.to_ascii_lowercase().as_str(), "null" | "mock") {
        Some(NULL_OUTPUT_DEVICE.to_string())
    } else {
        output
    }
}

use activity::{draw_activity_icon, OperatingActivity};
use automation_hunter::{
    AchievementKind, CustomAchievementRule, ExternalSendRecord, HunterAlert, HunterMetric,
};
use band_plan::{
    band_for_frequency, band_picker_plan, workspace_band_plan, workspace_radio_preset,
    workspace_radio_preset_for_frequency, WorkspaceMode, HF_WORKSPACE_MODES, OTHER_WORKSPACE_MODES,
    WORKSPACE_MODES,
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
use modes::voice::VoiceContestField;
use profile::{
    active_operator_profile_name, default_contest_fake_split_offset_hz, default_cw_tone_hz,
    default_cw_wpm, default_gui_scale, default_max_attempts as default_ft8_max_attempts,
    default_psk_batch_interval_secs, default_psk_max_pending, default_psk_repeat_cache_secs,
    default_ptt_lead_ms, default_ptt_tail_ms, default_rx_tone_hz, default_tx_tone_hz,
    default_waterfall_deck_height, list_operator_profiles, load_operator_profile,
    load_operator_profile_named, load_radio_profile_library, remove_operator_profile_named,
    save_operator_profile, save_operator_profile_named, save_radio_profile_library,
    select_operator_profile, OperatorProfile, RadioProfile, OPERATOR_PROFILE_FILE,
    OPERATOR_PROFILE_VERSION,
};
use radio_faq::{help_for_model, render_document};
#[cfg(test)]
use tx_audio::FT8_TX_AUDIO_START_S;
use tx_audio::{
    build_ft8_tx_pcm, build_native_digital_tx_pcm, run_digital_tx_job, run_ft8_tx_job,
    DigitalTxChatEntry, DigitalTxEvent, DigitalTxJob, Ft8ChatDirection, Ft8ChatLine,
    Ft8TxChatEntry, Ft8TxEvent, Ft8TxJob,
};
use ui_format::{format_signal_report, ft8_period_progress, qso_stage_label, utc_hhmmss_millis};
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
const GUI_SCALE_PROFILE_VERSION: u32 = 8;
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

fn parse_automation_hook_detail(detail: &str) -> BTreeMap<String, String> {
    detail
        .split_whitespace()
        .filter_map(|token| {
            let (key, value) = token.split_once('=')?;
            let key = key.trim();
            if key.is_empty() {
                return None;
            }
            Some((key.to_string(), value.trim().to_string()))
        })
        .collect()
}

fn normalize_app_event_for_automation(event: AppEvent) -> Option<AutomationEvent> {
    match event {
        AppEvent::ContestProfileChanged {
            enabled,
            operating_mode,
            split_policy,
            fox_hound_role,
        } => Some(
            AutomationEvent::new(EventKind::ContestState, "app.contest_profile")
                .field("enabled", enabled.to_string())
                .field("operating_mode", operating_mode)
                .field("split_policy", split_policy)
                .field("fox_hound_role", fox_hound_role),
        ),
        AppEvent::CallsignHit {
            mode,
            call,
            snr_db,
            freq_hz,
            message,
            directed_to_me,
        } => {
            let event = AutomationEvent::new(EventKind::CallsignHit, "app.callsign_hit")
                .field("mode", mode)
                .field("call", call)
                .field("snr", format!("{snr_db:+.1}"))
                .field("freq_hz", freq_hz.to_string())
                .field("message", message)
                .field("directed_to_me", directed_to_me.to_string());
            Some(if directed_to_me {
                event.tag("directed_to_me")
            } else {
                event
            })
        }
        AppEvent::QsoLogged {
            mode,
            call,
            band,
            frequency_hz,
        } => Some(
            AutomationEvent::new(EventKind::QsoLogged, "app.qso_log")
                .field("mode", mode)
                .field("call", call)
                .field("band", band)
                .field("frequency_hz", frequency_hz.to_string()),
        ),
        AppEvent::ExternalMessageReceived {
            source,
            author,
            message,
            channel,
        } => Some(
            AutomationEvent::new(EventKind::ExternalMessage, source.clone())
                .field("source", source)
                .field("author", author)
                .field("message", message)
                .field("channel", channel),
        ),
        AppEvent::ServerMessageReceived { kind, fields } => {
            let mut event = AutomationEvent::new(EventKind::ServerMessage, "qsonaut-server")
                .field("kind", kind.clone())
                .tag(kind);
            for (key, value) in fields {
                event = event.field(key, value);
            }
            Some(event)
        }
        AppEvent::AutomationHook {
            kind,
            source,
            detail,
        } => {
            let event_kind = match kind.as_str() {
                "contest_state" => EventKind::ContestState,
                "operator_profile" => EventKind::OperatorProfile,
                "callsign_hit" => EventKind::CallsignHit,
                "qso_logged" => EventKind::QsoLogged,
                "radio_state" => EventKind::RadioState,
                _ => return None,
            };
            let mut event = AutomationEvent::new(event_kind, source)
                .field("kind", kind)
                .field("detail", detail.clone());
            for (key, value) in parse_automation_hook_detail(&detail) {
                event = event.field(key, value);
            }
            Some(event)
        }
        _ => None,
    }
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
        "FLDIGI" => Some(WorkspaceMode::Fldigi),
        _ => None,
    }
}

fn radio_mode_label(mode: &str, data_mode: Option<bool>) -> String {
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

fn workspace_frequency_for_current_band(
    mode: WorkspaceMode,
    current_frequency_hz: Option<u64>,
) -> Option<u64> {
    let band = current_frequency_hz
        .map(band_for_frequency)
        .filter(|band| !band.is_empty())?;
    workspace_band_plan(mode)
        .iter()
        .find(|(label, _)| *label == band)
        .map(|(_, frequency_hz)| *frequency_hz)
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

fn external_source_transport(source: &str) -> Option<String> {
    let (transport, _) = source.trim().split_once(':')?;
    let transport = transport.trim();
    if transport.is_empty() {
        None
    } else {
        Some(transport.to_ascii_lowercase())
    }
}

fn configured_external_transports(config: &RuleComponentConfig) -> HashSet<String> {
    config
        .sources
        .iter()
        .map(|source| match source {
            ExternalSourceConfig::Discord { .. } => "discord",
            ExternalSourceConfig::Irc { .. } => "irc",
        })
        .map(str::to_string)
        .collect()
}

fn bootstrap_automation_host() -> (AutomationHost, String, HashSet<String>) {
    let mut host = AutomationHost::default();
    let source = include_str!("../../../automation.example.toml");

    match RuleComponentConfig::from_toml(source) {
        Ok(config) => {
            let configured_transports = configured_external_transports(&config);
            let component_id = config.component.id.clone();
            if let Err(error) = host.register(RuleComponent::new(config)) {
                return (
                    host,
                    format!("Automation host active, component registration failed: {error}"),
                    HashSet::new(),
                );
            }

            let external_send_enabled = parse_bool_env("QSONAUT_AUTOMATION_ENABLE_EXTERNAL_SEND");
            let server_publish_enabled = parse_bool_env("QSONAUT_AUTOMATION_ENABLE_SERVER_PUBLISH");
            let mut grants = vec![Capability::UiNotification, Capability::ServerRead];
            if external_send_enabled {
                grants.push(Capability::ExternalSend);
            }
            if server_publish_enabled {
                grants.push(Capability::ServerPublish);
            }
            let grants = CapabilitySet::new(grants);
            host.set_grants(component_id.clone(), grants);

            let mut grant_status = vec!["ui_notification", "server_read"];
            if external_send_enabled {
                grant_status.push("external_send (env-enabled)");
            }
            if server_publish_enabled {
                grant_status.push("server_publish (env-enabled)");
            }
            (
                host,
                format!(
                    "Automation component loaded: {component_id} (granted: {})",
                    grant_status.join(", ")
                ),
                configured_transports,
            )
        }
        Err(error) => (
            host,
            format!("Automation config parse failed; runtime hooks paused: {error}"),
            HashSet::new(),
        ),
    }
}

fn start_psk_reporter(
    enabled: bool,
    callsign: &str,
    grid: &str,
    tuning: ReporterTuning,
    state: &Arc<Mutex<GuiState>>,
) -> Option<Reporter> {
    state
        .lock()
        .expect("ui state lock poisoned")
        .psk_report_sender = None;
    if !enabled || callsign.trim().is_empty() || callsign == "N0CALL" || grid == "AA00" {
        info!(enabled, callsign = %callsign, grid = %grid, "PSK Reporter not started: station identity is incomplete or reporting is disabled");
        return None;
    }
    let mut config = ReporterConfig::production(callsign, grid);
    config.tuning = tuning;
    let reporter = Reporter::start(config);
    info!(callsign = %callsign, grid = %grid, "PSK Reporter started");
    state
        .lock()
        .expect("ui state lock poisoned")
        .psk_report_sender = Some(reporter.sender());
    Some(reporter)
}

fn submit_psk_report(
    sender: &Option<ReportSender>,
    dial_frequency_hz: Option<u64>,
    audio_frequency_hz: u32,
    snr_db: f32,
    message: &str,
    mode: &str,
    received_at: u32,
) {
    let Some(sender) = sender else { return };
    let parsed = parse_message(message);
    let sender_locator = parsed
        .as_ref()
        .and_then(|parsed| match &parsed.exchange {
            Exchange::Grid(grid) => Some(grid.clone()),
            _ => None,
        })
        .unwrap_or_default();
    let callsign = parsed.map(|parsed| parsed.from).or_else(|| {
        message
            .split_whitespace()
            .find(|token| is_probable_callsign(token))
            .map(|token| token.trim_matches(['<', '>']).to_ascii_uppercase())
    });
    let (Some(callsign), Some(dial)) = (callsign, dial_frequency_hz) else {
        return;
    };
    let frequency_hz = dial.saturating_add(u64::from(audio_frequency_hz));
    let callsign_for_log = callsign.clone();
    let queued = sender.submit(ReceptionReport {
        sender_callsign: callsign,
        frequency_hz,
        snr_db: snr_db.round().clamp(-127.0, 127.0) as i8,
        mode: mode.to_string(),
        sender_locator,
        received_at,
    });
    if queued {
        debug!(callsign = %callsign_for_log, frequency_hz, mode = %mode, "PSK Reporter reception report queued");
    } else {
        warn!(callsign = %callsign_for_log, mode = %mode, "PSK Reporter reception report rejected: worker unavailable");
    }
}

fn should_move_tx_to_decode(message: &ParsedMessage, continuing_exchange: bool) -> bool {
    !continuing_exchange && message.is_cq
}

fn qso_log_path() -> PathBuf {
    app_config_dir().join(QSO_LOG_FILE)
}

fn qso_adif_path() -> PathBuf {
    app_config_dir().join(QSO_ADIF_FILE)
}

fn enrich_qso_from_hamdb(record: &mut QsoRecord, cache: &HamDbCache, now: u64) {
    let callsign = record.callsign.trim().to_ascii_uppercase();
    if callsign.is_empty() {
        return;
    }
    let cached = cache
        .get_fresh(&callsign, now, HAMDB_CACHE_TTL_SECONDS)
        .ok()
        .flatten();
    let Some(entry) = cached else {
        return;
    };
    // QSO-entered values always win over license-record values.
    if record.grid.trim().is_empty() {
        record.grid = entry.grid.clone();
    }
    if record.state.trim().is_empty() {
        record.state = entry.state.clone();
    }
    record.hamdb = Some(entry);
}

#[derive(Debug, Deserialize)]
struct HamDbResponse {
    hamdb: HamDbPayload,
}

#[derive(Debug, Deserialize)]
struct HamDbPayload {
    callsign: HamDbCallsign,
}

#[derive(Debug, Deserialize)]
struct HamDbCallsign {
    call: String,
    #[serde(default)]
    class: String,
    #[serde(default)]
    expires: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    grid: String,
    #[serde(default, alias = "lat")]
    latitude: String,
    #[serde(default, alias = "lon")]
    longitude: String,
    #[serde(default, alias = "fname")]
    first_name: String,
    #[serde(default, alias = "mi")]
    middle_name: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    suffix: String,
    #[serde(default, alias = "addr1")]
    address_line_1: String,
    #[serde(default, alias = "addr2")]
    address_line_2: String,
    #[serde(default)]
    state: String,
    #[serde(default)]
    zip: String,
    #[serde(default)]
    country: String,
}

#[derive(Debug, Deserialize)]
struct PotaApiSpot {
    activator: Option<String>,
    reference: Option<String>,
    name: Option<String>,
    frequency: Option<String>,
    mode: Option<String>,
}

fn spawn_hamdb_lookup(
    callsign: String,
    completed_at_unix: u64,
) -> mpsc::Receiver<Option<HamDbCacheEntry>> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let result = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .ok()
            .and_then(|client| {
                client
                    .get(format!(
                        "https://api.hamdb.org/{}/json/QSONaut",
                        callsign.trim()
                    ))
                    .send()
                    .ok()
            })
            .filter(|response| response.status().is_success())
            .and_then(|response| response.json::<HamDbResponse>().ok())
            .map(|response| HamDbCacheEntry {
                callsign: response.hamdb.callsign.call.trim().to_ascii_uppercase(),
                class: response.hamdb.callsign.class.trim().to_string(),
                expires: response.hamdb.callsign.expires.trim().to_string(),
                status: response.hamdb.callsign.status.trim().to_string(),
                grid: response.hamdb.callsign.grid.trim().to_ascii_uppercase(),
                latitude: response.hamdb.callsign.latitude.trim().to_string(),
                longitude: response.hamdb.callsign.longitude.trim().to_string(),
                first_name: response.hamdb.callsign.first_name.trim().to_string(),
                middle_name: response.hamdb.callsign.middle_name.trim().to_string(),
                name: response.hamdb.callsign.name.trim().to_string(),
                suffix: response.hamdb.callsign.suffix.trim().to_string(),
                address_line_1: response.hamdb.callsign.address_line_1.trim().to_string(),
                address_line_2: response.hamdb.callsign.address_line_2.trim().to_string(),
                state: response.hamdb.callsign.state.trim().to_ascii_uppercase(),
                zip: response.hamdb.callsign.zip.trim().to_string(),
                country: response.hamdb.callsign.country.trim().to_string(),
                fetched_at_unix: completed_at_unix,
            });
        let _ = tx.send(result);
    });
    rx
}

fn qso_timestamp(record: &QsoRecord) -> Option<String> {
    let date = record.qso_date.trim();
    let time = record.time_on.trim();
    if date.len() != 8
        || time.len() < 4
        || !date.bytes().all(|byte| byte.is_ascii_digit())
        || !time.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let seconds = if time.len() >= 6 { &time[4..6] } else { "00" };
    Some(format!(
        "{}-{}-{}T{}:{}:{}Z",
        &date[0..4],
        &date[4..6],
        &date[6..8],
        &time[0..2],
        &time[2..4],
        seconds
    ))
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
struct PendingManualFt8Reply {
    compose: String,
    target: String,
    session: QsoSession,
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

/// Keep warning text readable in both egui visual modes.
pub(crate) fn theme_warning(ui: &egui::Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::YELLOW
    } else {
        Color32::from_rgb(146, 92, 0)
    }
}

pub(crate) fn theme_muted(ui: &egui::Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::GRAY
    } else {
        Color32::from_rgb(75, 85, 99)
    }
}

pub(crate) fn theme_accent(ui: &egui::Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::LIGHT_BLUE
    } else {
        Color32::from_rgb(29, 78, 121)
    }
}

pub(crate) fn theme_success(ui: &egui::Ui) -> Color32 {
    if ui.visuals().dark_mode {
        Color32::LIGHT_GREEN
    } else {
        Color32::from_rgb(21, 128, 61)
    }
}

pub(crate) fn status_color(ui: &egui::Ui, status: &str) -> Color32 {
    if status.contains('🔥') {
        Color32::from_rgb(255, 92, 48)
    } else if status.contains('⚠') {
        theme_warning(ui)
    } else if status.contains('🏁') || status.contains('✅') {
        theme_success(ui)
    } else if status.contains('🔒') {
        theme_accent(ui)
    } else {
        theme_muted(ui)
    }
}

#[derive(Default)]
struct DeviceInventory {
    audio_inputs: Vec<String>,
    audio_outputs: Vec<String>,
    serial_ports: Vec<String>,
    serial_port_labels: HashMap<String, String>,
    detected_models: Vec<String>,
}

/// WASAPI and serial enumeration each take hundreds of milliseconds, which is
/// long enough to delay the first paint and leave a ghost window on Windows.
fn spawn_device_scan() -> mpsc::Receiver<DeviceInventory> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        info!("Device inventory scan started");
        let (serial_ports, serial_port_labels, detected_models) =
            radio_port_inventory(enumerate_serial_port_descriptors().unwrap_or_default());
        let inventory = DeviceInventory {
            audio_inputs: std::iter::once(NULL_INPUT_DEVICE.to_string())
                .chain(AudioService::input_devices().unwrap_or_default())
                .collect(),
            audio_outputs: std::iter::once(NULL_OUTPUT_DEVICE.to_string())
                .chain(AudioService::output_devices().unwrap_or_default())
                .collect(),
            serial_ports,
            serial_port_labels,
            detected_models,
        };
        info!(
            audio_inputs = inventory.audio_inputs.len(),
            audio_outputs = inventory.audio_outputs.len(),
            serial_ports = inventory.serial_ports.len(),
            detected_models = inventory.detected_models.len(),
            "Device inventory scan completed"
        );
        let _ = tx.send(inventory);
    });
    rx
}

/// Spawns radio initialization on a background thread with timeout. Returns a receiver for
/// the result (or None if a timeout of ~5 seconds occurs). This prevents serial port
/// operations from blocking the UI window appearance.
fn spawn_radio_init(
    backend: String,
    model: String,
    port: String,
    endpoint: String,
    baud_rate: u32,
    controller_civ_address: u8,
    radio_civ_address: u8,
) -> mpsc::Receiver<Option<ConfiguredRadio>> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let start = std::time::Instant::now();
        let _timeout = std::time::Duration::from_secs(5);

        let radio = match backend.trim().to_ascii_lowercase().as_str() {
            "none" => None,
            "null" | "mock" => Some(open_null()),
            "rigctld" | "rigctl" => Some(open_rigctld(endpoint)),
            "dxlab" | "dxlab-commander" | "commander" => Some(open_dxlab(endpoint)),
            "native" => match open_model_with_radio_address(
                &model,
                &port,
                baud_rate,
                controller_civ_address,
                Some(radio_civ_address),
            ) {
                Ok(radio) => Some(radio),
                Err(err) => {
                    error!(
                        backend = %backend,
                        model = %model,
                        endpoint = %endpoint,
                        port = %port,
                        baud = baud_rate,
                        error = %err,
                        elapsed = ?start.elapsed(),
                        "Radio initialization failed"
                    );
                    None
                }
            },
            unsupported => {
                error!(backend = unsupported, "Unsupported radio backend");
                None
            }
        };

        match radio {
            Some(radio) => {
                let _ = tx.send(Some(radio));
            }
            None => {
                let _ = tx.send(None);
            }
        }
    });
    rx
}

fn request_radio_session_stop(session: &RadioSession) {
    session.worker_stop.store(true, Ordering::Relaxed);
    session.audio_worker_stop.store(true, Ordering::Relaxed);
    session.swr_sweep_abort.store(true, Ordering::Relaxed);
    if let Some(tx) = &session.command_tx {
        let _ = tx.send(GuiCommand::Quit);
    }
}

fn join_radio_session(mut session: RadioSession) {
    if let Some(handle) = session.worker_handle.take() {
        let _ = handle.join();
    }
    if let Some(handle) = session.audio_worker_handle.take() {
        let _ = handle.join();
    }
}

fn stop_radio_session(session: RadioSession) {
    request_radio_session_stop(&session);
    join_radio_session(session);
}

fn radio_config_from_operator_profile(profile: &OperatorProfile) -> RadioConfig {
    RadioConfig {
        enabled: profile.radio_enabled,
        backend: profile.radio_backend.clone(),
        endpoint: profile.radio_endpoint.clone(),
        model: profile.radio_model.clone(),
        serial_port: profile.radio_serial_port.clone(),
        baud_rate: profile.radio_baud_rate,
        civ_address: profile.radio_civ_address,
        controller_civ_address: profile.radio_controller_civ_address,
    }
}

fn audio_config_from_operator_profile(
    profile: &OperatorProfile,
    fallback: &AudioConfig,
) -> AudioConfig {
    if profile.profile_version < 3 {
        return fallback.clone();
    }
    AudioConfig {
        enabled: profile.audio_enabled,
        input_device: profile.audio_input_device.clone(),
        output_device: profile.audio_output_device.clone(),
        monitor_enabled: if profile.profile_version >= AUDIO_MONITOR_PROFILE_VERSION {
            profile.audio_monitor_enabled
        } else {
            fallback.monitor_enabled
        },
        monitor_output_device: if profile.profile_version >= AUDIO_MONITOR_PROFILE_VERSION {
            profile.audio_monitor_output_device.clone()
        } else {
            fallback.monitor_output_device.clone()
        },
        monitor_volume: if profile.profile_version >= AUDIO_MONITOR_PROFILE_VERSION {
            profile.audio_monitor_volume.clamp(0.0, 2.0)
        } else {
            fallback.monitor_volume
        },
        sample_rate_hz: profile.audio_sample_rate_hz,
        channels: profile.audio_channels,
    }
}

fn spawn_acceleration_probe(preference: ComputePreference) -> mpsc::Receiver<AccelerationReport> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(AccelerationReport::probe(preference));
    });
    rx
}

fn preferred_renderer() -> eframe::Renderer {
    // QSONaut uses eframe's modern cross-platform GPU backend everywhere.
    eframe::Renderer::Wgpu
}

fn configure_unix_gui_environment() {
    #[cfg(target_os = "linux")]
    {
        if std::env::var_os("NO_AT_BRIDGE").is_none() {
            std::env::set_var("NO_AT_BRIDGE", "1");
        }
    }
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
    radio_init_rx: Option<mpsc::Receiver<Option<ConfiguredRadio>>>,
    cat_test_rx: Option<mpsc::Receiver<Result<String, String>>>,
    cat_test_status: Option<Result<String, String>>,
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
    init_rx: Option<mpsc::Receiver<Option<ConfiguredRadio>>>,
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
    #[allow(clippy::too_many_arguments)]
    fn new(
        mut config: AppConfig,
        cc: &eframe::CreationContext<'_>,
        app_icon: &egui::IconData,
        selected_renderer: eframe::Renderer,
        stored_geometry: Option<WindowGeometry>,
        graphics_preferences: GraphicsPreferences,
        active_graphics_adapter: Option<GraphicsAdapterInfo>,
        available_graphics_adapters: Vec<GraphicsAdapterInfo>,
        graphics_restart_request: Arc<Mutex<Option<GraphicsPreferences>>>,
    ) -> Self {
        let ctx = &cc.egui_ctx;
        // Keep egui's bundled font fallback chain active. It includes the
        // monochrome Noto Emoji and emoji icon fonts, making emoji rendering
        // independent of the host OS font installation.
        ctx.set_fonts(egui::FontDefinitions::default());
        let brand_image = ColorImage::from_rgba_unmultiplied(
            [app_icon.width as usize, app_icon.height as usize],
            &app_icon.rgba,
        );
        let brand_icon =
            ctx.load_texture("qsonaut-brand-icon", brand_image, TextureOptions::LINEAR);
        if let Some(profile) = load_operator_profile() {
            if profile.profile_version >= 3 {
                config.audio.input_device = profile.audio_input_device;
                config.audio.enabled = profile.audio_enabled;
                config.audio.output_device = profile.audio_output_device;
                config.audio.sample_rate_hz = profile.audio_sample_rate_hz;
                config.audio.channels = profile.audio_channels;
                if profile.profile_version >= AUDIO_MONITOR_PROFILE_VERSION {
                    config.audio.monitor_enabled = profile.audio_monitor_enabled;
                    config.audio.monitor_output_device = profile.audio_monitor_output_device;
                    config.audio.monitor_volume = profile.audio_monitor_volume.clamp(0.0, 2.0);
                }
                config.radio.enabled = profile.radio_enabled;
                config.radio.serial_port = profile.radio_serial_port;
                config.radio.backend = profile.radio_backend;
                config.radio.endpoint = profile.radio_endpoint;
                if config.radio.backend.trim().eq_ignore_ascii_case("none") {
                    config.radio.backend = "native".to_string();
                }
                if profile.profile_version >= 8 {
                    config.radio.model = profile.radio_model;
                    config.radio.baud_rate = profile.radio_baud_rate;
                }
                config.radio.civ_address = profile.radio_civ_address;
                config.radio.controller_civ_address = profile.radio_controller_civ_address;
            }
        }

        let available_profiles = list_operator_profiles();
        let active_profile_name = active_operator_profile_name();
        let selected_profile_name = available_profiles
            .iter()
            .find(|name| name.eq_ignore_ascii_case(&active_profile_name))
            .cloned()
            .unwrap_or_else(|| "Default".to_string());

        let state = Arc::new(Mutex::new(GuiState::default()));
        if let Some(profile) = load_operator_profile_named(&selected_profile_name) {
            if let Ok(mut state) = state.lock() {
                state.recording_enabled = profile.recording_enabled;
                state.recording_modes = profile
                    .recording_modes
                    .iter()
                    .filter(|(_, enabled)| **enabled)
                    .filter_map(|(mode, _)| parse_workspace_mode_token(mode))
                    .collect();
                state.recording_full_width = profile.recording_full_width;
                state.recording_stream = profile.recording_stream;
            }
        }
        let app_events = AppEventBus::new(256);
        let automation_event_rx = app_events.subscribe();
        let (automation_host, automation_status, automation_external_transports) =
            bootstrap_automation_host();
        let radio_worker_stop = Arc::new(AtomicBool::new(false));
        let swr_sweep_abort = Arc::new(AtomicBool::new(false));
        let audio_worker_stop = Arc::new(AtomicBool::new(false));
        let display_tuning = Arc::new(Mutex::new(DisplayTuning::default()));

        let repaint_ctx: Arc<OnceLock<egui::Context>> = Arc::new(OnceLock::new());

        // Spawn radio initialization on a background thread to avoid blocking UI appearance
        let (radio_init_rx, radio_waterfall_status_init) = if config.radio.enabled {
            let port = config.radio.serial_port.clone().unwrap_or_default();
            let rx = spawn_radio_init(
                config.radio.backend.clone(),
                config.radio.model.clone(),
                port,
                config.radio.endpoint.clone(),
                config.radio.baud_rate,
                config.radio.controller_civ_address,
                config.radio.civ_address,
            );
            (Some(rx), "CONNECTING…".to_string())
        } else {
            (None, "UNAVAILABLE (radio disabled)".to_string())
        };

        // Set initial radio status
        {
            let mut s = state.lock().expect("ui state lock poisoned");
            s.radio_waterfall_status = radio_waterfall_status_init;
            if radio_init_rx.is_none() {
                s.last_error = Some(
                    "Radio is disabled in config; UI running in monitor-only mode".to_string(),
                );
            }
        }

        let (command_tx, radio_worker_handle) = (None, None);

        // Every saved profile owns a live runtime. Inactive tabs continue
        // receiving and decoding, but their PTT path is disabled.
        let mut parked_radio_sessions = HashMap::new();
        for profile_name in &available_profiles {
            if profile_name == &selected_profile_name {
                continue;
            }
            let Some(profile) = load_operator_profile_named(profile_name) else {
                continue;
            };
            let session_config = radio_config_from_operator_profile(&profile);
            let session_audio_config = audio_config_from_operator_profile(&profile, &config.audio);
            let session_state = Arc::new(Mutex::new(GuiState::default()));
            let (init_rx, status) = if session_config.enabled {
                let port = session_config.serial_port.clone().unwrap_or_default();
                (
                    Some(spawn_radio_init(
                        session_config.backend.clone(),
                        session_config.model.clone(),
                        port,
                        session_config.endpoint.clone(),
                        session_config.baud_rate,
                        session_config.controller_civ_address,
                        session_config.civ_address,
                    )),
                    "CONNECTING…",
                )
            } else {
                (None, "UNAVAILABLE (radio disabled)")
            };
            if let Ok(mut state) = session_state.lock() {
                state.radio_waterfall_status = status.to_string();
                state.workspace_mode = parse_workspace_mode_token(&profile.workspace_mode)
                    .unwrap_or(WorkspaceMode::Ft8);
                state.ft8_deep_decode = profile.deep_decode;
                state.ft4_deep_decode = profile.ft4_deep_decode;
                state.selected_audio_hz = profile.rx_tone_hz;
                state.cw_wpm = profile.cw_wpm.clamp(5, 40);
                state.recording_enabled = profile.recording_enabled;
                state.recording_modes = profile
                    .recording_modes
                    .iter()
                    .filter(|(_, enabled)| **enabled)
                    .filter_map(|(mode, _)| parse_workspace_mode_token(mode))
                    .collect();
                state.recording_full_width = profile.recording_full_width;
                state.recording_stream = profile.recording_stream;
                state.radio_spectrum_desired = profile.civ_spectrum_on;
                state.radio_scope_vbw_wide = profile.radio_scope_vbw_wide;
                state.radio_scope_view = profile.radio_scope_view;
            }
            let session_audio_worker_stop = Arc::new(AtomicBool::new(false));
            let session_swr_sweep_abort = Arc::new(AtomicBool::new(false));
            let session_display_tuning = Arc::new(Mutex::new(DisplayTuning::default()));
            let session_monitor_volume = Arc::new(AtomicU32::new(
                session_audio_config.monitor_volume.to_bits(),
            ));
            let session_ft8_tx_active = Arc::new(AtomicBool::new(false));
            let session_digital_tx_active = Arc::new(AtomicBool::new(false));
            let session_ptt_allowed = Arc::new(AtomicBool::new(false));
            let session_audio_worker_handle = Some(spawn_audio_spectrum_worker(
                session_state.clone(),
                session_audio_worker_stop.clone(),
                session_ft8_tx_active.clone(),
                session_digital_tx_active.clone(),
                session_audio_config.enabled,
                session_audio_config.sample_rate_hz,
                session_audio_config.channels,
                effective_audio_input_device(
                    &session_config.backend,
                    session_audio_config.input_device.clone(),
                ),
                session_audio_config.monitor_enabled,
                effective_audio_output_device(
                    &session_config.backend,
                    session_audio_config
                        .monitor_output_device
                        .clone()
                        .or_else(|| session_audio_config.output_device.clone()),
                ),
                session_monitor_volume.clone(),
                repaint_ctx.clone(),
                session_display_tuning.clone(),
            ));
            info!(
                profile = profile_name,
                radio_enabled = profile.radio_enabled,
                audio_enabled = profile.audio_enabled,
                "Profile runtime initialization queued"
            );
            parked_radio_sessions.insert(
                profile_name.clone(),
                RadioSession {
                    profile,
                    view_state: TabViewState::default(),
                    config: session_config,
                    audio_config: session_audio_config,
                    state: session_state,
                    command_tx: None,
                    worker_stop: Arc::new(AtomicBool::new(false)),
                    audio_worker_stop: session_audio_worker_stop,
                    swr_sweep_abort: session_swr_sweep_abort,
                    display_tuning: session_display_tuning,
                    monitor_volume: session_monitor_volume,
                    ft8_tx_active: session_ft8_tx_active,
                    digital_tx_active: session_digital_tx_active,
                    ptt_allowed: session_ptt_allowed,
                    init_rx,
                    init_attempted: false,
                    worker_handle: None,
                    audio_worker_handle: session_audio_worker_handle,
                },
            );
        }

        let ft8_tx_active = Arc::new(AtomicBool::new(false));
        let ptt_allowed = Arc::new(AtomicBool::new(true));
        let digital_tx_active = Arc::new(AtomicBool::new(false));
        let monitor_volume = Arc::new(AtomicU32::new(config.audio.monitor_volume.to_bits()));
        let audio_worker_handle = Some(spawn_audio_spectrum_worker(
            state.clone(),
            audio_worker_stop.clone(),
            ft8_tx_active.clone(),
            digital_tx_active.clone(),
            config.audio.enabled,
            config.audio.sample_rate_hz,
            config.audio.channels,
            effective_audio_input_device(&config.radio.backend, config.audio.input_device.clone()),
            config.audio.monitor_enabled,
            effective_audio_output_device(
                &config.radio.backend,
                config
                    .audio
                    .monitor_output_device
                    .clone()
                    .or_else(|| config.audio.output_device.clone()),
            ),
            monitor_volume.clone(),
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
        let mut station_rig = String::new();
        let mut station_antenna = String::new();
        let mut station_notes = String::new();
        let mut llm_prompt_context = String::new();
        let mut sstv_image_requirements = String::new();
        let mut llm_model_notes = String::new();
        let mut ft8_follow_log = true;
        let mut ft8_max_log_entries = 300usize;
        let mut ft8_deep_decode = false;
        let mut ft4_deep_decode = false;
        let mut ft4_autoseq = false;
        let mut ft4_auto_reply_policy = AutoReplyPolicy::default();
        let mut ft4_cq_only_view = false;
        let mut ft4_follow_log = true;
        let mut ft4_max_log_entries = 300usize;
        let mut ft4_max_attempts = default_ft8_max_attempts();
        let mut ft8_autoseq = false;
        let mut ft8_auto_reply_policy = AutoReplyPolicy::default();
        let mut ft8_auto_answer_cq = false;
        let mut automation_unlocked = false;
        let mut ft8_cq_only_view = false;
        let mut civ_spectrum_on = false;
        let mut radio_scope_vbw_wide = false;
        let mut radio_scope_view = RadioScopeView::Narrow;
        let mut waterfall_theme = WaterfallTheme::default();
        let mut radio_waterfall_theme = WaterfallTheme::default();
        let mut waterfall_deck_height = default_waterfall_deck_height();
        let ft8_stop_policy = AutoTxStopPolicy::Continuous;
        let mut ft8_max_attempts = default_ft8_max_attempts();
        let mut ft8_hold_tx_freq = false;
        let mut rx_tone_hz = default_rx_tone_hz();
        let mut tx_tone_hz = default_tx_tone_hz();
        let mut ptt_lead_ms = default_ptt_lead_ms();
        let mut ptt_tail_ms = default_ptt_tail_ms();
        let mut cw_wpm = default_cw_wpm();
        let mut cw_tone_hz = default_cw_tone_hz();
        let mut recording_enabled = false;
        let mut recording_modes = std::collections::BTreeMap::new();
        let mut recording_full_width = true;
        let mut recording_stream = true;
        let mut gui_scale = default_gui_scale();
        let mut compute_preference = ComputePreference::Auto;
        let mut psk_reporter_enabled = false;
        let mut pota_enabled = true;
        let mut psk_batch_interval_secs = default_psk_batch_interval_secs();
        let mut psk_repeat_cache_secs = default_psk_repeat_cache_secs();
        let mut psk_max_pending = default_psk_max_pending();
        let mut server_instance_id = new_instance_id();
        let mut contest_enabled = config.contest.enabled;
        let mut contest_operating_mode = config.contest.operating_mode;
        let mut contest_split_policy = config.contest.split_policy;
        let mut contest_fox_hound_role = config.contest.fox_hound_role;
        let mut contest_exchange_template =
            config.contest.exchange_template.clone().unwrap_or_default();
        let mut contest_serial_start = config.contest.serial_start.max(1);
        let mut contest_serial_step = config.contest.serial_step.max(1);
        let mut contest_dupe_check = config.contest.dupe_check;
        let mut contest_serial_current = contest_serial_start;
        let mut contest_fake_split_offset_hz = default_contest_fake_split_offset_hz();
        let mut hunter_unlocked = HashSet::new();
        let mut hunter_acknowledged = HashSet::new();
        let mut hunter_alerts_enabled = true;
        let mut hunter_custom_rules = Vec::new();
        let mut radio_profiles = load_radio_profile_library();
        let mut mode_radio_profile = std::collections::BTreeMap::new();
        let mut workspace_mode = WorkspaceMode::Ft8;
        let profile_io_status: String;

        if let Some(p) = load_operator_profile() {
            station_callsign = p.callsign;
            station_grid = p.grid;
            station_qth = p.qth;
            station_rig = p.station_rig;
            station_antenna = p.station_antenna;
            station_notes = p.station_notes;
            llm_prompt_context = p.llm_prompt_context;
            sstv_image_requirements = p.sstv_image_requirements;
            llm_model_notes = p.llm_model_notes;
            ft8_follow_log = p.follow_log;
            ft8_max_log_entries = p.max_log_entries.clamp(80, 1000);
            ft8_deep_decode = p.deep_decode;
            ft4_deep_decode = p.ft4_deep_decode;
            // Transmit automation is never restored as armed at startup.
            ft4_autoseq = false;
            ft4_auto_reply_policy = p.ft4_auto_reply_policy;
            ft4_cq_only_view = p.ft4_cq_only_view;
            ft4_follow_log = p.ft4_follow_log;
            ft4_max_log_entries = p.ft4_max_log_entries.clamp(80, 300);
            ft4_max_attempts = p.ft4_max_attempts.clamp(1, 20);
            // Transmit automation is never restored as armed at startup.
            ft8_autoseq = false;
            ft8_auto_reply_policy = p.auto_reply_policy;
            // Unattended CQ answering must be explicitly re-enabled each run.
            ft8_auto_answer_cq = false;
            automation_unlocked = p.automation_unlocked;
            ft8_cq_only_view = p.cq_only_view;
            civ_spectrum_on = p.civ_spectrum_on;
            radio_scope_vbw_wide = p.radio_scope_vbw_wide;
            radio_scope_view = p.radio_scope_view;
            waterfall_theme = p.waterfall_theme;
            radio_waterfall_theme = p.radio_waterfall_theme;
            waterfall_deck_height = p.waterfall_deck_height.clamp(170.0, 560.0);
            ft8_max_attempts = p.ft8_max_attempts.clamp(1, 20);
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
            ptt_lead_ms = p.ptt_lead_ms.clamp(0, 500);
            ptt_tail_ms = p.ptt_tail_ms.clamp(0, 500);
            cw_wpm = p.cw_wpm.clamp(5, 40);
            cw_tone_hz = p.cw_tone_hz.clamp(200, 3_000);
            recording_enabled = p.recording_enabled;
            recording_modes = p.recording_modes;
            recording_full_width = p.recording_full_width;
            recording_stream = p.recording_stream;
            gui_scale = if p.profile_version >= GUI_SCALE_PROFILE_VERSION {
                p.gui_scale.clamp(GUI_SCALE_MIN, GUI_SCALE_MAX)
            } else {
                // v3 called this physical size 160%; it is the v4 100% baseline.
                default_gui_scale()
            };
            compute_preference = p.compute_preference;
            psk_reporter_enabled = p.psk_reporter_enabled;
            pota_enabled = p.pota_enabled;
            psk_batch_interval_secs = p.psk_batch_interval_secs.clamp(60, 3_600);
            psk_repeat_cache_secs = p.psk_repeat_cache_secs.clamp(60, 3_600);
            psk_max_pending = p.psk_max_pending.clamp(1, 2_048);
            server_instance_id = p.server_instance_id;
            if let Some(server) = p.server {
                config.server = server;
            }
            contest_enabled = p.contest_enabled;
            contest_operating_mode = p.contest_operating_mode;
            contest_split_policy = p.contest_split_policy;
            contest_fox_hound_role = p.contest_fox_hound_role;
            contest_exchange_template = p.contest_exchange_template;
            contest_serial_start = p.contest_serial_start.max(1);
            contest_serial_step = p.contest_serial_step.max(1);
            contest_dupe_check = p.contest_dupe_check;
            contest_serial_current = p.contest_serial_current.max(contest_serial_start).max(1);
            contest_fake_split_offset_hz = p.contest_fake_split_offset_hz.clamp(0, 2_000);
            hunter_unlocked = p.hunter_unlocked.into_iter().collect();
            hunter_acknowledged = p.hunter_acknowledged.into_iter().collect();
            hunter_alerts_enabled = p.hunter_alerts_enabled;
            hunter_custom_rules = p.hunter_custom_rules;
            if radio_profiles.is_empty() {
                radio_profiles = p.radio_profiles.clone();
                for name in list_operator_profiles() {
                    if let Some(legacy_profile) = load_operator_profile_named(&name) {
                        for candidate in legacy_profile.radio_profiles {
                            if !radio_profiles
                                .iter()
                                .any(|profile| profile.name == candidate.name)
                            {
                                radio_profiles.push(candidate);
                            }
                        }
                    }
                }
                if !radio_profiles.is_empty() {
                    if let Err(error) = save_radio_profile_library(&radio_profiles) {
                        warn!(%error, "Failed to migrate radio profiles to global library");
                    } else {
                        info!(
                            count = radio_profiles.len(),
                            "Migrated radio profiles to global library"
                        );
                    }
                }
            }
            mode_radio_profile = p.mode_radio_profile;
            workspace_mode =
                parse_workspace_mode_token(&p.workspace_mode).unwrap_or(WorkspaceMode::Ft8);
            config.station.callsign = Some(station_callsign.clone());
            config.station.grid = Some(station_grid.clone());
            config.contest = ContestProfile {
                enabled: contest_enabled,
                operating_mode: contest_operating_mode,
                split_policy: contest_split_policy,
                fox_hound_role: contest_fox_hound_role,
                exchange_template: if contest_exchange_template.trim().is_empty() {
                    None
                } else {
                    Some(contest_exchange_template.trim().to_string())
                },
                serial_start: contest_serial_start,
                serial_step: contest_serial_step,
                dupe_check: contest_dupe_check,
            };
            profile_io_status = format!("Loaded {}", OPERATOR_PROFILE_FILE);
        } else {
            let bootstrap = OperatorProfile {
                profile_version: OPERATOR_PROFILE_VERSION,
                callsign: station_callsign.clone(),
                grid: station_grid.clone(),
                qth: station_qth.clone(),
                station_rig: station_rig.clone(),
                station_antenna: station_antenna.clone(),
                station_notes: station_notes.clone(),
                llm_prompt_context: llm_prompt_context.clone(),
                sstv_image_requirements: sstv_image_requirements.clone(),
                llm_model_notes: llm_model_notes.clone(),
                follow_log: ft8_follow_log,
                max_log_entries: ft8_max_log_entries,
                deep_decode: ft8_deep_decode,
                ft4_deep_decode,
                ft4_autoseq,
                ft4_auto_reply_policy,
                ft4_cq_only_view,
                ft4_follow_log,
                ft4_max_log_entries,
                ft4_max_attempts,
                autoseq: ft8_autoseq,
                auto_reply_policy: ft8_auto_reply_policy,
                auto_answer_cq: ft8_auto_answer_cq,
                automation_unlocked,
                cq_only_view: ft8_cq_only_view,
                civ_spectrum_on,
                radio_scope_vbw_wide,
                radio_scope_view,
                waterfall_theme,
                radio_waterfall_theme,
                waterfall_auto_visual: true,
                waterfall_speed: WaterfallSpeed::Mid,
                waterfall_deck_height,
                halt_after_tx: false,
                ft8_max_attempts,
                hold_tx_freq: ft8_hold_tx_freq,
                rx_tone_hz,
                tx_tone_hz,
                ptt_lead_ms,
                ptt_tail_ms,
                cw_wpm,
                cw_tone_hz,
                recording_enabled,
                recording_modes: recording_modes.clone(),
                recording_full_width,
                recording_stream,
                audio_input_device: config.audio.input_device.clone(),
                audio_enabled: config.audio.enabled,
                audio_output_device: config.audio.output_device.clone(),
                audio_monitor_enabled: config.audio.monitor_enabled,
                audio_monitor_output_device: config.audio.monitor_output_device.clone(),
                audio_monitor_volume: config.audio.monitor_volume.clamp(0.0, 2.0),
                audio_sample_rate_hz: config.audio.sample_rate_hz,
                audio_channels: config.audio.channels,
                radio_enabled: config.radio.enabled,
                radio_serial_port: config.radio.serial_port.clone(),
                radio_backend: config.radio.backend.clone(),
                radio_endpoint: config.radio.endpoint.clone(),
                radio_model: config.radio.model.clone(),
                radio_baud_rate: config.radio.baud_rate,
                radio_civ_address: config.radio.civ_address,
                radio_controller_civ_address: config.radio.controller_civ_address,
                gui_scale,
                compute_preference,
                psk_reporter_enabled,
                pota_enabled,
                psk_batch_interval_secs,
                psk_repeat_cache_secs,
                psk_max_pending,
                server_instance_id: server_instance_id.clone(),
                server: Some(config.server.clone()),
                contest_enabled,
                contest_operating_mode,
                contest_split_policy,
                contest_fox_hound_role,
                contest_exchange_template: contest_exchange_template.trim().to_string(),
                contest_serial_start,
                contest_serial_step,
                contest_dupe_check,
                contest_serial_current,
                contest_fake_split_offset_hz,
                hunter_unlocked: Vec::new(),
                hunter_acknowledged: Vec::new(),
                hunter_alerts_enabled: true,
                hunter_custom_rules: Vec::new(),
                radio_profiles: Vec::new(),
                mode_radio_profile: std::collections::BTreeMap::new(),
                workspace_mode: workspace_mode.label().to_string(),
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

        if server_instance_id.is_empty() {
            server_instance_id = new_instance_id();
        }

        let server_client = (config.server.enabled
            && !config.server.url.trim().is_empty()
            && !config.server.device_token.trim().is_empty())
        .then(|| {
            ServerClient::spawn(ServerConnectionConfig {
                server_url: config.server.url.trim().to_string(),
                device_token: config.server.device_token.trim().to_string(),
                client_version: env!("CARGO_PKG_VERSION").to_string(),
                queue_path: app_config_dir().join("server-log-queue.json"),
                share_logs: config.server.share_logs,
            })
        });

        if !native_radio_profile(&config.radio.backend, &config.radio.model)
            .is_some_and(|profile| profile.capabilities.spectrum)
        {
            civ_spectrum_on = false;
        }

        let (ft8_tx_event_tx, ft8_tx_event_rx) = mpsc::channel();
        let (digital_tx_event_tx, digital_tx_event_rx) = mpsc::channel();
        let (local_image_event_tx, local_image_event_rx) = mpsc::channel();
        let (qso_log, qso_log_status) = match QsoLog::load(&qso_log_path()) {
            Ok(log) => {
                let count = log.contacts.len();
                (log, format!("Loaded {count} contacts"))
            }
            Err(error) => (QsoLog::default(), format!("Log load failed: {error}")),
        };

        let acceleration_report = AccelerationReport::pending(compute_preference);
        let acceleration_probe = Some(spawn_acceleration_probe(compute_preference));
        let (os_dpi_adjustment, os_name, os_dpi_env) = os_dpi_adjustment();
        info!(
            os = os_name,
            adjustment = os_dpi_adjustment,
            env = os_dpi_env,
            "Using OS DPI adjustment"
        );
        // Applied before the first paint so the window is never laid out at one
        // scale and immediately re-laid out at another.
        ctx.set_zoom_factor(gui_scale * os_dpi_adjustment);
        let psk_reporter = start_psk_reporter(
            psk_reporter_enabled,
            &station_callsign,
            &station_grid,
            ReporterTuning {
                batch_interval_secs: psk_batch_interval_secs,
                repeat_cache_secs: psk_repeat_cache_secs,
                max_pending: psk_max_pending,
            },
            &state,
        );
        let psk_sender = psk_reporter.as_ref().map(Reporter::sender);
        for session in parked_radio_sessions.values() {
            if let Ok(mut session_state) = session.state.lock() {
                session_state.psk_report_sender = psk_sender.clone();
            }
        }

        Self {
            config,
            app_events,
            automation_event_rx,
            automation_host,
            automation_status,
            last_radio_state_signature: None,
            automation_external_transports,
            automation_external_outbox: VecDeque::new(),
            hunter_unlocked,
            hunter_acknowledged,
            hunter_show_acknowledged: false,
            hunter_alerts_enabled,
            hunter_feed: VecDeque::new(),
            hunter_unique_heard: HashSet::new(),
            hunter_directed_hits: 0,
            hunter_dupe_blocks: 0,
            hunter_decode_bursts: 0,
            hunter_custom_rules,
            radio_profiles,
            mode_radio_profile,
            radio_profile_name_input: String::new(),
            hunter_custom_title_input: String::new(),
            hunter_custom_detail_input: String::new(),
            hunter_custom_metric_input: HunterMetric::UniqueHeard,
            hunter_custom_threshold_input: 1,
            hunter_custom_enabled_input: true,
            external_ingress_source: "discord:shack".to_string(),
            external_ingress_author: "operator".to_string(),
            external_ingress_channel: "#qsonaut".to_string(),
            external_ingress_message: "!rig".to_string(),
            state,
            command_tx,
            radio_worker_stop,
            swr_sweep_abort,
            audio_worker_stop,
            radio_init_rx,
            cat_test_rx: None,
            cat_test_status: None,
            hamdb_lookup_rx: None,
            hamdb_profile_lookup_rx: None,
            pota_spots: Vec::new(),
            pota_lookup_rx: None,
            pota_last_lookup: Instant::now() - Duration::from_secs(60),
            pota_history: VecDeque::new(),
            pota_last_updated: None,
            pota_last_error: None,
            logo_clicks: VecDeque::new(),
            logo_spin_until: None,
            radio_init_attempted: false,
            radio_worker_handle,
            parked_radio_sessions,
            audio_worker_handle,
            radio_waterfall_texture: None,
            radio_waterfall_texture_revision: 0,
            radio_waterfall_texture_bins: 0,
            radio_waterfall_texture_view: RadioScopeView::Narrow,
            radio_waterfall_texture_theme: WaterfallTheme::RadioBlue,
            audio_waterfall_texture: None,
            audio_waterfall_texture_revision: 0,
            audio_waterfall_texture_bins: 0,
            audio_waterfall_texture_theme: WaterfallTheme::RadioBlue,
            sstv_texture: None,
            sstv_texture_revision: 0,
            sstv_tx_armed: false,
            sstv_tuning_offset_hz: 0,
            sstv_auto_target: true,
            sstv_rx_width_percent: 43,
            sstv_tx_rgb: Vec::new(),
            sstv_tx_base_rgb: Vec::new(),
            sstv_tx_width: qsonaut_sstv::WIDTH,
            sstv_tx_height: qsonaut_sstv::HEIGHT,
            sstv_tx_revision: 0,
            sstv_tx_texture: None,
            sstv_tx_texture_revision: 0,
            sstv_tx_mode: qsonaut_sstv::SstvMode::MartinM1,
            sstv_overlay_callsign: true,
            sstv_overlay_grid: true,
            sstv_overlay_frequency: false,
            sstv_overlay_mode: true,
            sstv_overlay_corner: SstvOverlayCorner::BottomLeft,
            sstv_overlay_background: Color32::BLACK,
            sstv_overlay_background_opacity: 0.65,
            sstv_background_zoom: 1.0,
            sstv_background_pan_x: 0.0,
            sstv_background_pan_y: 0.0,
            sstv_overlay_revision: 0,
            sstv_file_dialog: egui_file_dialog::FileDialog::new(),
            sstv_image_path: String::new(),
            sstv_ai_prompt: String::new(),
            local_image_settings: LocalImageSettings::load(),
            local_image_models: Vec::new(),
            local_image_status: "Local image server not checked".to_string(),
            local_image_refresh_started: false,
            local_image_event_tx,
            local_image_event_rx,
            sstv_ai_pipeline_mode: SstvAiPipelineMode::StationQsl,
            sstv_active_received_id: None,
            sstv_show_prior_received: false,
            sstv_received_textures: HashMap::new(),
            sstv_received_texture_revision: 0,
            sstv_reinterpret_prompt: String::new(),
            workspace_mode,
            activity: OperatingActivity::General,
            fst4_submode: modes::fst4::Submode::default(),
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
            automation_unlocked,
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
            ptt_allowed,
            ft8_tx_event_tx,
            ft8_tx_event_rx,
            ft8_last_tx_was_cq: false,
            digital_compose: String::new(),
            digital_selected: None,
            digital_last_click: None,
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
            monitor_volume,
            native_autoseq_mode: None,
            native_auto_reply_policy: AutoReplyPolicy::default(),
            native_stop_policy: AutoTxStopPolicy::Continuous,
            native_sessions: HashMap::new(),
            native_seen_decodes: HashMap::new(),
            native_last_tx_periods: HashMap::new(),
            native_attempts: HashMap::new(),
            ft8_stop_policy,
            ft8_max_attempts,
            ft4_stop_policy: AutoTxStopPolicy::Continuous,
            ft4_max_attempts,
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
            station_rig,
            station_antenna,
            station_notes,
            llm_prompt_context,
            sstv_image_requirements,
            llm_model_notes,
            voice_callsign: String::new(),
            voice_grid: String::new(),
            voice_state: String::new(),
            voice_rst_sent: "59".to_string(),
            voice_rst_received: "59".to_string(),
            voice_contest_serial_sent: String::new(),
            voice_contest_serial_received: String::new(),
            voice_notes: String::new(),
            voice_contest_fields: Vec::new(),
            voice_qso_started_at: None,
            voice_lookup_requested: String::new(),
            voice_lookup_status: String::new(),
            voice_hamdb: None,
            contest_enabled,
            contest_operating_mode,
            contest_split_policy,
            contest_fox_hound_role,
            contest_exchange_template,
            contest_serial_start,
            contest_serial_step,
            contest_dupe_check,
            contest_serial_current,
            contest_fake_split_offset_hz,
            civ_spectrum_on,
            rx_tone_hz,
            tx_tone_hz,
            ptt_lead_ms,
            ptt_tail_ms,
            cw_wpm,
            cw_tone_hz,
            recording_enabled,
            recording_modes,
            recording_full_width,
            recording_stream,
            selected_profile_name,
            new_profile_name: String::new(),
            new_profile_tab_editing: false,
            pending_profile_delete: None,
            available_profiles,
            profile_io_status,
            profile_dirty: false,
            app_log_text: String::new(),
            app_log_status: String::new(),
            app_log_filter: String::new(),
            app_log_level_filter: AppLogLevelFilter::All,
            app_log_follow: true,
            app_log_last_refresh: Instant::now() - Duration::from_secs(1),
            audio_input_devices: Vec::new(),
            audio_output_devices: Vec::new(),
            radio_serial_ports: Vec::new(),
            radio_serial_port_labels: HashMap::new(),
            radio_detected_models: Vec::new(),
            device_scan: Some(spawn_device_scan()),
            radio_scope_contrast: 1.2,
            radio_scope_span_code: 0,
            radio_scope_vbw_wide,
            radio_scope_hold: false,
            radio_scope_reference_tenths_db: 0,
            radio_scope_view,
            radio_scope_lock_if_to_filter: true,
            waterfall_theme,
            radio_waterfall_theme,
            waterfall_deck_height,
            show_signal_panel: true,
            show_meter_panel: false,
            meter_panel_was_tx: false,
            meter_panel_close_deadline: None,
            show_profile_drawer: false,
            profile_drawer_anchor: None,
            radio_faq_window_open: false,
            radio_guide_window_open: false,
            radio_faq_document: RadioHelpDocument::Model,
            radio_guide_document: RadioHelpDocument::Model,
            radio_help_window_model: String::new(),
            profile_drawer_tab: ProfileDrawerTab::Profile,
            signal_panel_tab: SignalPanelTab::Achievements,
            device_restart_required: false,
            audio_restart_required: false,
            gui_scale,
            os_dpi_adjustment,
            graphics_active: graphics_preferences.clone(),
            graphics_pending: graphics_preferences,
            active_graphics_adapter,
            available_graphics_adapters,
            graphics_restart_request,
            compute_preference,
            acceleration_report,
            acceleration_probe,
            psk_reporter_enabled,
            pota_enabled,
            psk_batch_interval_secs,
            psk_repeat_cache_secs,
            psk_max_pending,
            psk_reporter,
            server_client,
            server_active_club: None,
            server_active_event: None,
            server_instance_id,
            server_last_presence: Instant::now() - Duration::from_secs(60),
            brand_icon,
            selected_renderer,
            first_frame_logged: false,
            last_viewport_log: None,
            window_geometry: stored_geometry,
        }
    }

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

    fn apply_tab_preferences(&mut self, profile: &OperatorProfile) {
        self.workspace_mode =
            parse_workspace_mode_token(&profile.workspace_mode).unwrap_or(WorkspaceMode::Ft8);
        self.ft8_follow_log = profile.follow_log;
        self.ft8_max_log_entries = profile.max_log_entries.clamp(80, 1000);
        self.ft8_deep_decode = profile.deep_decode;
        self.ft8_auto_reply_policy = profile.auto_reply_policy;
        self.ft8_cq_only_view = profile.cq_only_view;
        self.ft8_max_attempts = profile.ft8_max_attempts.clamp(1, 20);
        self.ft8_hold_tx_freq = profile.profile_version >= 3 && profile.hold_tx_freq;
        self.ft4_deep_decode = profile.ft4_deep_decode;
        self.ft4_auto_reply_policy = profile.ft4_auto_reply_policy;
        self.ft4_cq_only_view = profile.ft4_cq_only_view;
        self.ft4_follow_log = profile.ft4_follow_log;
        self.ft4_max_log_entries = profile.ft4_max_log_entries.clamp(80, 300);
        self.ft4_max_attempts = profile.ft4_max_attempts.clamp(1, 20);
        self.automation_unlocked = profile.automation_unlocked;
        self.civ_spectrum_on = profile.civ_spectrum_on;
        self.radio_scope_vbw_wide = profile.radio_scope_vbw_wide;
        self.radio_scope_view = profile.radio_scope_view;
        self.waterfall_theme = profile.waterfall_theme;
        self.radio_waterfall_theme = profile.radio_waterfall_theme;
        // Waterfall geometry is application-wide. Do not reload the profile's
        // historical value when switching radio tabs; doing so makes a tab
        // switch snap the shared deck back to that profile's default.
        if let Ok(mut tuning) = self.display_tuning.lock() {
            tuning.audio_auto_visual = profile.waterfall_auto_visual;
            tuning.audio_waterfall_speed = profile.waterfall_speed;
        }
        self.rx_tone_hz = profile.rx_tone_hz;
        self.tx_tone_hz = if self.ft8_hold_tx_freq {
            profile.tx_tone_hz
        } else {
            profile.rx_tone_hz
        };
        self.ptt_lead_ms = profile.ptt_lead_ms.clamp(0, 500);
        self.ptt_tail_ms = profile.ptt_tail_ms.clamp(0, 500);
        self.cw_wpm = profile.cw_wpm.clamp(5, 40);
        self.cw_tone_hz = profile.cw_tone_hz.clamp(200, 3_000);
        self.recording_enabled = profile.recording_enabled;
        self.recording_modes = profile.recording_modes.clone();
        self.recording_full_width = profile.recording_full_width;
        self.recording_stream = profile.recording_stream;
        if let Ok(mut state) = self.state.lock() {
            state.recording_enabled = self.recording_enabled;
            state.recording_modes = self
                .recording_modes
                .iter()
                .filter(|(_, enabled)| **enabled)
                .filter_map(|(mode, _)| parse_workspace_mode_token(mode))
                .collect();
            state.recording_full_width = self.recording_full_width;
            state.recording_stream = self.recording_stream;
        }
        self.contest_enabled = profile.contest_enabled;
        self.contest_operating_mode = profile.contest_operating_mode;
        self.contest_split_policy = profile.contest_split_policy;
        self.contest_fox_hound_role = profile.contest_fox_hound_role;
        self.contest_exchange_template = profile.contest_exchange_template.clone();
        self.contest_serial_start = profile.contest_serial_start.max(1);
        self.contest_serial_step = profile.contest_serial_step.max(1);
        self.contest_dupe_check = profile.contest_dupe_check;
        self.contest_serial_current = profile
            .contest_serial_current
            .max(self.contest_serial_start)
            .max(1);
        self.contest_fake_split_offset_hz = profile.contest_fake_split_offset_hz.clamp(0, 2_000);
        self.mode_radio_profile = profile.mode_radio_profile.clone();

        // Switching tabs never restores armed or in-flight transmit state.
        self.ft8_autoseq = false;
        self.ft4_autoseq = false;
        self.sstv_tx_armed = false;
        self.ft8_stop_policy = AutoTxStopPolicy::Continuous;
        self.ft4_stop_policy = AutoTxStopPolicy::Continuous;
    }

    fn radio_config_for_profile(&self, profile: &OperatorProfile) -> RadioConfig {
        radio_config_from_operator_profile(profile)
    }

    fn park_active_radio_session(&mut self) {
        let name = self.selected_profile_name.clone();
        let profile = self.current_operator_profile();
        let view_state = self.take_tab_view_state();
        let session = RadioSession {
            profile,
            view_state,
            config: self.config.radio.clone(),
            audio_config: self.config.audio.clone(),
            state: std::mem::replace(&mut self.state, Arc::new(Mutex::new(GuiState::default()))),
            command_tx: self.command_tx.take(),
            worker_stop: self.radio_worker_stop.clone(),
            audio_worker_stop: self.audio_worker_stop.clone(),
            swr_sweep_abort: self.swr_sweep_abort.clone(),
            display_tuning: self.display_tuning.clone(),
            monitor_volume: self.monitor_volume.clone(),
            ft8_tx_active: self.ft8_tx_active.clone(),
            digital_tx_active: self.digital_tx_active.clone(),
            ptt_allowed: self.ptt_allowed.clone(),
            init_rx: self.radio_init_rx.take(),
            init_attempted: self.radio_init_attempted,
            worker_handle: self.radio_worker_handle.take(),
            audio_worker_handle: self.audio_worker_handle.take(),
        };
        session.ptt_allowed.store(false, Ordering::Release);
        if let Some(previous) = self.parked_radio_sessions.insert(name, session) {
            stop_radio_session(previous);
        }
        info!(profile = %self.selected_profile_name, "Profile runtime moved to background with PTT disabled");
    }

    fn start_active_radio_session(&mut self) {
        self.radio_worker_stop = Arc::new(AtomicBool::new(false));
        if !self.config.radio.enabled {
            self.radio_init_rx = None;
            self.radio_init_attempted = true;
            if let Ok(mut state) = self.state.lock() {
                state.radio_waterfall_status = "UNAVAILABLE (radio disabled)".to_string();
            }
            return;
        }
        let port = self.config.radio.serial_port.clone().unwrap_or_default();
        self.radio_init_rx = Some(spawn_radio_init(
            self.config.radio.backend.clone(),
            self.config.radio.model.clone(),
            port,
            self.config.radio.endpoint.clone(),
            self.config.radio.baud_rate,
            self.config.radio.controller_civ_address,
            self.config.radio.civ_address,
        ));
        self.radio_init_attempted = false;
        if let Ok(mut state) = self.state.lock() {
            state.radio_waterfall_status = "CONNECTING…".to_string();
            state.last_error = None;
        }
    }

    fn pump_parked_radio_sessions(&mut self) {
        let repaint_ctx = self.repaint_ctx.clone();
        for (name, session) in &mut self.parked_radio_sessions {
            if session
                .audio_worker_handle
                .as_ref()
                .is_some_and(std::thread::JoinHandle::is_finished)
            {
                if let Some(handle) = session.audio_worker_handle.take() {
                    let _ = handle.join();
                }
                if let Ok(mut state) = session.state.lock() {
                    state.audio_spectrum_status = "STOPPED (audio worker failed)".to_string();
                }
                warn!(profile = name, "Profile audio worker stopped");
            }
            if session
                .worker_handle
                .as_ref()
                .is_some_and(std::thread::JoinHandle::is_finished)
            {
                if let Some(handle) = session.worker_handle.take() {
                    let _ = handle.join();
                }
                session.command_tx = None;
                if let Ok(mut state) = session.state.lock() {
                    state.radio_waterfall_status = "STOPPED (radio worker failed)".to_string();
                }
                warn!(profile = name, "Profile radio worker stopped");
            }
            if session.init_attempted || !session.config.enabled {
                continue;
            }
            let Some(rx) = &session.init_rx else { continue };
            match rx.try_recv() {
                Ok(Some(radio)) => {
                    session.init_attempted = true;
                    let (tx, command_rx) = mpsc::channel::<GuiCommand>();
                    session.worker_handle = Some(workers::radio::spawn_radio_worker(
                        radio,
                        session.state.clone(),
                        session.worker_stop.clone(),
                        session.swr_sweep_abort.clone(),
                        session.display_tuning.clone(),
                        command_rx,
                        repaint_ctx.clone(),
                        session.ptt_allowed.clone(),
                    ));
                    session.command_tx = Some(tx);
                    info!(
                        profile = name,
                        "Started inactive radio worker with PTT disabled"
                    );
                }
                Ok(None) | Err(mpsc::TryRecvError::Disconnected) => {
                    session.init_attempted = true;
                    session.init_rx = None;
                    warn!(
                        profile = name,
                        "Inactive profile radio initialization failed"
                    );
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
    }

    fn draw_activity_selector(&mut self, ui: &mut egui::Ui) {
        let selected_activity = self.activity;
        let server_context = self.server_client.as_ref().map(ServerClient::status);
        let activity_button_label = self
            .server_active_event
            .as_ref()
            .map(|(_, name)| format!("🏁 Contest · {name}"))
            .or_else(|| {
                self.server_active_club
                    .as_ref()
                    .map(|(_, name)| format!("🌐 {} · {name}", selected_activity.label()))
            })
            .unwrap_or_else(|| format!("📻 {}", selected_activity.label()));
        let previous_interact_height = ui.spacing().interact_size.y;
        ui.spacing_mut().interact_size.y = 28.0;
        let activity_menu = ui.menu_button(
            RichText::new(activity_button_label)
                .strong()
                .color(Color32::from_rgb(255, 190, 105)),
            |ui| {
                ui.label(RichText::new("OPERATING ACTIVITY").strong());
                ui.separator();
                ui.horizontal_wrapped(|ui| {
                    for activity in OperatingActivity::ALL {
                        let (rect, response) =
                            ui.allocate_exact_size(egui::vec2(78.0, 52.0), egui::Sense::click());
                        let fill = if selected_activity == activity {
                            ui.visuals().selection.bg_fill
                        } else if response.hovered() {
                            ui.visuals().widgets.hovered.bg_fill
                        } else {
                            ui.visuals().widgets.inactive.bg_fill
                        };
                        ui.painter().rect_filled(rect, 4.0, fill);
                        draw_activity_icon(
                            ui.painter(),
                            activity,
                            rect.center_top() + egui::vec2(0.0, 14.0),
                            ui.visuals().text_color(),
                        );
                        ui.painter().text(
                            rect.center_bottom() - egui::vec2(0.0, 8.0),
                            egui::Align2::CENTER_CENTER,
                            activity.label(),
                            egui::FontId::proportional(12.0),
                            ui.visuals().text_color(),
                        );
                        if response.clicked() {
                            info!(activity = %activity.label(), "Operating activity changed");
                            self.activity = activity;
                            ui.close();
                        }
                    }
                });
                if let Some(server_context) = &server_context {
                    if !server_context.clubs.is_empty() || !server_context.active_events.is_empty()
                    {
                        ui.separator();
                        ui.label(
                            RichText::new("🌐 SERVER ACTIVITIES")
                                .strong()
                                .color(Color32::from_rgb(110, 220, 255)),
                        );
                        if !server_context.clubs.is_empty() {
                            ui.label(RichText::new("🌐 ACTIVE CLUB").small().strong());
                            for club in &server_context.clubs {
                                let selected = self
                                    .server_active_club
                                    .as_ref()
                                    .is_some_and(|(id, _)| id == &club.id);
                                let label = club
                                    .callsign
                                    .as_deref()
                                    .map(|callsign| format!("{} · {}", club.name, callsign))
                                    .unwrap_or_else(|| club.name.clone());
                                if ui.selectable_label(selected, label).clicked() {
                                    self.server_active_club =
                                        Some((club.id.clone(), club.name.clone()));
                                    self.server_active_event = None;
                                    ui.close();
                                }
                            }
                        }
                        if !server_context.active_events.is_empty() {
                            ui.label(RichText::new("🏁 ACTIVE CONTESTS").small().strong());
                            for contest in &server_context.active_events {
                                let selected = self
                                    .server_active_event
                                    .as_ref()
                                    .is_some_and(|(id, _)| id == &contest.id);
                                let label = if contest.contest_name.is_empty() {
                                    format!("{} · {}", contest.name, contest.club_name)
                                } else {
                                    format!(
                                        "{} · {} · {}",
                                        contest.name, contest.contest_name, contest.club_name
                                    )
                                };
                                ui.horizontal(|ui| {
                                    if ui.selectable_label(selected, label).clicked() {
                                        self.activity = OperatingActivity::Contest;
                                        self.server_active_club = Some((
                                            contest.club_id.clone(),
                                            contest.club_name.clone(),
                                        ));
                                        self.server_active_event =
                                            Some((contest.id.clone(), contest.name.clone()));
                                        ui.close();
                                    }
                                    let starts = contest
                                        .starts_at
                                        .get(0..16)
                                        .unwrap_or(&contest.starts_at)
                                        .replace('T', " ");
                                    let ends = contest
                                        .ends_at
                                        .get(0..16)
                                        .unwrap_or(&contest.ends_at)
                                        .replace('T', " ");
                                    ui.label(
                                        RichText::new(format!(
                                            "{starts} → {ends} · {} op",
                                            contest.participant_count
                                        ))
                                        .small()
                                        .color(Color32::GRAY),
                                    );
                                });
                            }
                        }
                        if (self.server_active_club.is_some() || self.server_active_event.is_some())
                            && ui.small_button("✕ CLEAR SERVER ACTIVITY").clicked()
                        {
                            self.server_active_club = None;
                            self.server_active_event = None;
                            ui.close();
                        }
                    }
                }
                let profile = self.activity.profile();
                let band_summary = if profile.bands.is_unrestricted() {
                    "all core".to_string()
                } else {
                    profile.bands.labels().join(", ")
                };
                let mode_summary = if profile.modes.is_unrestricted() {
                    "all core".to_string()
                } else {
                    profile
                        .modes
                        .modes()
                        .iter()
                        .map(|mode| mode.label())
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                ui.separator();
                ui.label(
                    RichText::new(format!(
                        "{}  ·  Bands {}  ·  Modes {}",
                        profile.tx_cq, band_summary, mode_summary,
                    ))
                    .small()
                    .color(Color32::GRAY),
                );
            },
        );
        activity_menu
            .response
            .on_hover_text("Choose the operating activity and any active server event");
        ui.spacing_mut().interact_size.y = previous_interact_height;
    }

    fn draw_header_branding(&mut self, ui: &mut egui::Ui) {
        let spin_angle = self.logo_spin_until.map_or(0.0, |until| {
            let remaining = until.saturating_duration_since(Instant::now());
            (1.0 - remaining.as_secs_f32() / 0.7).clamp(0.0, 1.0) * std::f32::consts::TAU
        });
        let logo = egui::Image::new((self.brand_icon.id(), egui::vec2(56.0, 56.0)))
            .corner_radius(8.0)
            .rotate(spin_angle, egui::Vec2::splat(0.5))
            .sense(egui::Sense::click());
        let logo_response = ui
            .add(logo)
            .on_hover_text("QSONaut mission control — click for the application animation");
        if logo_response.clicked() {
            self.handle_logo_click();
        }
        if self
            .logo_spin_until
            .is_some_and(|until| Instant::now() < until)
        {
            ui.ctx().request_repaint();
        } else {
            self.logo_spin_until = None;
        }
        ui.vertical(|ui| {
            ui.label(
                RichText::new("QSONaut")
                    .strong()
                    .size(32.0)
                    .color(Color32::from_rgb(109, 224, 255)),
            );
            ui.label(
                RichText::new("AMATEUR RADIO MISSION CONTROL")
                    .strong()
                    .size(10.0)
                    .color(Color32::from_rgb(255, 137, 108)),
            );
        });
        self.draw_activity_selector(ui);
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
        let radio_failed = radio_enabled
            && ((radio_status.starts_with("UNAVAILABLE")
                && !radio_status.contains("no scope stream"))
                || radio_status.starts_with("SESSION STOPPED"));
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

    fn set_tab_workers_running(&mut self, name: &str, running: bool) {
        if name == self.selected_profile_name {
            self.config.radio.enabled = running;
            self.config.audio.enabled = running;
            self.ptt_allowed.store(running, Ordering::Release);
            self.profile_dirty = true;
            self.persist_profile(if running {
                "Radio and audio workers started for"
            } else {
                "Radio and audio workers stopped for"
            });
            if running {
                self.reconnect_radio();
                self.restart_audio();
            } else {
                self.radio_worker_stop.store(true, Ordering::Relaxed);
                self.audio_worker_stop.store(true, Ordering::Relaxed);
                if let Some(tx) = &self.command_tx {
                    let _ = tx.send(GuiCommand::Quit);
                }
                if let Some(handle) = self.radio_worker_handle.take() {
                    let _ = handle.join();
                }
                if let Some(handle) = self.audio_worker_handle.take() {
                    let _ = handle.join();
                }
                self.command_tx = None;
                if let Ok(mut state) = self.state.lock() {
                    state.radio_waterfall_status = "STOPPED (by operator)".to_string();
                    state.audio_spectrum_status = "STOPPED (by operator)".to_string();
                }
            }
            return;
        }
        let Some(session) = self.parked_radio_sessions.get_mut(name) else {
            return;
        };
        session.config.enabled = running;
        session.audio_config.enabled = running;
        session.profile.radio_enabled = running;
        session.profile.audio_enabled = running;
        if running {
            session.worker_stop = Arc::new(AtomicBool::new(false));
            session.audio_worker_stop = Arc::new(AtomicBool::new(false));
            let port = session.config.serial_port.clone().unwrap_or_default();
            session.init_rx = Some(spawn_radio_init(
                session.config.backend.clone(),
                session.config.model.clone(),
                port,
                session.config.endpoint.clone(),
                session.config.baud_rate,
                session.config.controller_civ_address,
                session.config.civ_address,
            ));
            session.init_attempted = false;
            if session.audio_worker_handle.is_none() {
                session.audio_worker_handle = Some(spawn_audio_spectrum_worker(
                    session.state.clone(),
                    session.audio_worker_stop.clone(),
                    session.ft8_tx_active.clone(),
                    session.digital_tx_active.clone(),
                    true,
                    session.audio_config.sample_rate_hz,
                    session.audio_config.channels,
                    effective_audio_input_device(
                        &session.config.backend,
                        session.audio_config.input_device.clone(),
                    ),
                    session.audio_config.monitor_enabled,
                    effective_audio_output_device(
                        &session.config.backend,
                        session
                            .audio_config
                            .monitor_output_device
                            .clone()
                            .or_else(|| session.audio_config.output_device.clone()),
                    ),
                    session.monitor_volume.clone(),
                    self.repaint_ctx.clone(),
                    session.display_tuning.clone(),
                ));
            }
        } else {
            session.worker_stop.store(true, Ordering::Relaxed);
            session.audio_worker_stop.store(true, Ordering::Relaxed);
            if let Some(tx) = &session.command_tx {
                let _ = tx.send(GuiCommand::Quit);
            }
            if let Some(handle) = session.worker_handle.take() {
                let _ = handle.join();
            }
            if let Some(handle) = session.audio_worker_handle.take() {
                let _ = handle.join();
            }
            session.command_tx = None;
            session.init_rx = None;
        }
        let _ = save_operator_profile_named(name, &session.profile);
    }

    fn switch_radio_tab(&mut self, name: &str) {
        self.switch_radio_tab_with_save(name, true);
    }

    fn switch_radio_tab_with_save(&mut self, name: &str, save_previous: bool) {
        if name == self.selected_profile_name {
            return;
        }
        let Some(profile) = load_operator_profile_named(name) else {
            self.profile_io_status = format!("Profile ‘{name}’ was not found");
            return;
        };

        self.disarm_all_tx_with_persistence("Radio tab switch: all TX disarmed", save_previous);
        if save_previous {
            self.persist_profile("Saved");
        }
        self.park_active_radio_session();
        self.selected_profile_name = name.to_string();
        self.new_profile_name = name.to_string();
        self.config.radio = self.radio_config_for_profile(&profile);

        if let Some(session) = self.parked_radio_sessions.remove(name) {
            let session_profile = session.profile;
            let session_view = session.view_state;
            self.config.radio = session.config;
            self.config.audio = session.audio_config;
            self.state = session.state;
            self.command_tx = session.command_tx;
            self.radio_worker_stop = session.worker_stop;
            self.audio_worker_stop = session.audio_worker_stop;
            self.swr_sweep_abort = session.swr_sweep_abort;
            self.display_tuning = session.display_tuning;
            self.monitor_volume = session.monitor_volume;
            self.ft8_tx_active = session.ft8_tx_active;
            self.ptt_allowed = session.ptt_allowed;
            self.digital_tx_active = session.digital_tx_active;
            self.radio_init_rx = session.init_rx;
            self.radio_init_attempted = session.init_attempted;
            self.radio_worker_handle = session.worker_handle;
            self.audio_worker_handle = session.audio_worker_handle;
            self.ptt_allowed.store(true, Ordering::Release);
            self.apply_tab_preferences(&session_profile);
            self.restore_tab_view_state(session_view);
            info!(
                profile = name,
                radio_running = self.command_tx.is_some(),
                audio_running = self.audio_worker_handle.is_some(),
                "Profile tab activated without reconnecting workers"
            );
        } else {
            self.config.audio = audio_config_from_operator_profile(&profile, &self.config.audio);
            self.state = Arc::new(Mutex::new(GuiState::default()));
            self.command_tx = None;
            self.radio_worker_handle = None;
            self.radio_worker_stop = Arc::new(AtomicBool::new(false));
            self.audio_worker_stop = Arc::new(AtomicBool::new(false));
            self.swr_sweep_abort = Arc::new(AtomicBool::new(false));
            self.display_tuning = Arc::new(Mutex::new(DisplayTuning::default()));
            self.monitor_volume =
                Arc::new(AtomicU32::new(self.config.audio.monitor_volume.to_bits()));
            self.ft8_tx_active = Arc::new(AtomicBool::new(false));
            self.ptt_allowed = Arc::new(AtomicBool::new(true));
            self.digital_tx_active = Arc::new(AtomicBool::new(false));
            self.start_active_radio_session();
            self.audio_worker_handle = Some(spawn_audio_spectrum_worker(
                self.state.clone(),
                self.audio_worker_stop.clone(),
                self.ft8_tx_active.clone(),
                self.digital_tx_active.clone(),
                self.config.audio.enabled,
                self.config.audio.sample_rate_hz,
                self.config.audio.channels,
                effective_audio_input_device(
                    &self.config.radio.backend,
                    self.config.audio.input_device.clone(),
                ),
                self.config.audio.monitor_enabled,
                effective_audio_output_device(
                    &self.config.radio.backend,
                    self.config
                        .audio
                        .monitor_output_device
                        .clone()
                        .or_else(|| self.config.audio.output_device.clone()),
                ),
                self.monitor_volume.clone(),
                self.repaint_ctx.clone(),
                self.display_tuning.clone(),
            ));
            self.apply_tab_preferences(&profile);
            self.restore_tab_view_state(TabViewState::default());
            info!(
                profile = name,
                "Radio tab created and initialization queued"
            );
        }
        if let Err(error) = select_operator_profile(name) {
            warn!(profile = name, %error, "Failed to persist active tab selection");
        }
        self.radio_waterfall_texture = None;
        self.audio_waterfall_texture = None;
        self.sstv_texture = None;
        self.profile_io_status = format!("Radio tab ‘{name}’ active");
    }

    fn persist_profile(&mut self, status_prefix: &str) {
        if let Err(error) = save_radio_profile_library(&self.radio_profiles) {
            warn!(%error, "Radio profile library save failed");
        }
        match save_operator_profile_named(
            &self.selected_profile_name,
            &self.current_operator_profile(),
        ) {
            Ok(_) => {
                info!(profile = %self.selected_profile_name, status = %status_prefix, "Operator profile saved");
                self.profile_io_status =
                    format!("{status_prefix} profile ‘{}’", self.selected_profile_name);
                self.available_profiles = list_operator_profiles();
                self.profile_dirty = false;
            }
            Err(err) => {
                warn!(profile = %self.selected_profile_name, error = %err, "Operator profile save failed");
                self.profile_io_status = format!("Save failed: {err}");
            }
        }
    }

    fn create_profile_from_tab_name(&mut self) {
        let name = self.new_profile_name.trim().to_string();
        if name.is_empty() {
            self.profile_io_status = "Profile name cannot be empty".to_string();
            return;
        }
        if self
            .available_profiles
            .iter()
            .any(|profile| profile.eq_ignore_ascii_case(&name))
        {
            self.profile_io_status = format!("Profile ‘{name}’ already exists");
            return;
        }
        match save_operator_profile_named(&name, &self.current_operator_profile()) {
            Ok(()) => {
                self.available_profiles = list_operator_profiles();
                self.new_profile_name.clear();
                self.new_profile_tab_editing = false;
                self.switch_radio_tab(&name);
                self.profile_io_status = format!("Created profile ‘{name}’");
                self.profile_dirty = false;
            }
            Err(error) => {
                self.profile_io_status = format!("Profile creation failed: {error}");
            }
        }
    }

    fn rename_selected_profile(&mut self) {
        let old_name = self.selected_profile_name.clone();
        let new_name = self.new_profile_name.trim().to_string();
        if new_name.is_empty() {
            self.profile_io_status = "Profile name cannot be empty".to_string();
            return;
        }
        if new_name.eq_ignore_ascii_case(&old_name) {
            self.new_profile_name = old_name;
            return;
        }
        if self
            .available_profiles
            .iter()
            .any(|profile| profile.eq_ignore_ascii_case(&new_name))
        {
            self.profile_io_status = format!("Profile ‘{new_name}’ already exists");
            return;
        }
        let profile = self.current_operator_profile();
        match save_operator_profile_named(&new_name, &profile) {
            Ok(()) => match remove_operator_profile_named(&old_name) {
                Ok(()) => {
                    if let Err(error) = select_operator_profile(&new_name) {
                        self.profile_io_status =
                            format!("Renamed profile but active selection failed: {error}");
                        return;
                    }
                    self.selected_profile_name = new_name.clone();
                    self.available_profiles = list_operator_profiles();
                    self.new_profile_name = new_name.clone();
                    self.profile_dirty = false;
                    self.profile_io_status = format!("Renamed profile to ‘{new_name}’");
                }
                Err(error) => {
                    self.profile_io_status = format!("Old profile cleanup failed: {error}");
                }
            },
            Err(error) => {
                self.profile_io_status = format!("Profile rename failed: {error}");
            }
        }
    }

    fn delete_operator_profile(&mut self, name: &str) {
        if self.available_profiles.len() <= 1 {
            self.profile_io_status = "The last profile cannot be deleted".to_string();
            return;
        }
        let Some(replacement) = self
            .available_profiles
            .iter()
            .find(|candidate| !candidate.eq_ignore_ascii_case(name))
            .cloned()
        else {
            self.profile_io_status = "No replacement profile is available".to_string();
            return;
        };

        if let Err(error) = remove_operator_profile_named(name) {
            warn!(profile = name, %error, "Operator profile deletion failed");
            self.profile_io_status = format!("Profile deletion failed: {error}");
            return;
        }

        let was_active = name == self.selected_profile_name;
        if was_active {
            self.switch_radio_tab_with_save(&replacement, false);
            if let Some(session) = self.parked_radio_sessions.remove(name) {
                stop_radio_session(session);
            }
        } else if let Some(session) = self.parked_radio_sessions.remove(name) {
            stop_radio_session(session);
        }
        self.available_profiles = list_operator_profiles();
        if self.selected_profile_name.eq_ignore_ascii_case(name) {
            self.selected_profile_name = replacement.clone();
            let _ = select_operator_profile(&replacement);
        }
        info!(
            profile = name,
            "Operator profile deleted and radio tab stopped"
        );
        self.profile_io_status = format!("Deleted profile ‘{name}’ and stopped its tab");
        self.profile_dirty = false;
    }

    fn persist_qso_log(&mut self, status_prefix: &str) {
        match self.qso_log.save(&qso_log_path()) {
            Ok(()) => {
                info!(contacts = self.qso_log.contacts.len(), status = %status_prefix, "QSO log saved");
                self.qso_log_status = format!("{status_prefix} {}", QSO_LOG_FILE);
                self.qso_log_dirty = false;
            }
            Err(error) => {
                warn!(error = %error, path = %qso_log_path().display(), "QSO log save failed");
                self.qso_log_status = format!("Log save failed: {error}");
            }
        }
    }

    fn pump_hamdb_lookup(&mut self) {
        let Some(rx) = self.hamdb_lookup_rx.as_ref() else {
            return;
        };
        let entry = match rx.try_recv() {
            Ok(Some(entry)) => entry,
            Ok(None) | Err(mpsc::TryRecvError::Disconnected) => {
                warn!(callsign = %self.voice_lookup_requested, "HamDB callsign lookup returned no record");
                self.voice_lookup_status = "HamDB: callsign not found".to_string();
                self.hamdb_lookup_rx = None;
                return;
            }
            Err(mpsc::TryRecvError::Empty) => return,
        };
        let cache = HamDbCache::open(&hamdb_cache_path()).ok();
        let voice_match = self
            .voice_lookup_requested
            .eq_ignore_ascii_case(&entry.callsign);
        if voice_match {
            info!(callsign = %entry.callsign, "HamDB Voice contact lookup completed");
            if self.voice_grid.trim().is_empty() {
                self.voice_grid = entry.grid.clone();
            }
            if self.voice_state.trim().is_empty() {
                self.voice_state = entry.state.clone();
            }
            self.voice_hamdb = Some(entry.clone());
            self.voice_lookup_status = "HamDB: operator found".to_string();
        }
        let mut log_updated = false;
        for record in self
            .qso_log
            .contacts
            .iter_mut()
            .filter(|record| record.callsign.eq_ignore_ascii_case(&entry.callsign))
        {
            log_updated = true;
            if record.grid.trim().is_empty() {
                record.grid = entry.grid.clone();
            }
            if record.state.trim().is_empty() {
                record.state = entry.state.clone();
            }
            record.hamdb = Some(entry.clone());
        }
        if let Some(cache) = cache {
            let _ = cache.upsert(&entry);
        }
        if log_updated {
            self.qso_log_dirty = true;
            self.persist_qso_log("HamDB details saved to");
        }
        self.hamdb_lookup_rx = None;
    }

    fn pump_hamdb_profile_lookup(&mut self) {
        let Some(rx) = self.hamdb_profile_lookup_rx.as_ref() else {
            return;
        };
        let entry = match rx.try_recv() {
            Ok(Some(entry)) => entry,
            Ok(None) | Err(mpsc::TryRecvError::Disconnected) => {
                warn!(callsign = %self.station_callsign, "HamDB operator profile lookup returned no record");
                self.profile_io_status = "HamDB did not return a license record".to_string();
                self.hamdb_profile_lookup_rx = None;
                return;
            }
            Err(mpsc::TryRecvError::Empty) => return,
        };
        info!(callsign = %entry.callsign, "HamDB operator profile lookup completed");
        self.station_callsign = entry.callsign.clone();
        if !entry.grid.trim().is_empty() {
            self.station_grid = entry.grid.clone();
        }
        let qth = [
            entry.address_line_1.trim(),
            entry.address_line_2.trim(),
            entry.state.trim(),
            entry.country.trim(),
        ]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(", ");
        if !qth.is_empty() {
            self.station_qth = qth;
        }
        self.config.station.callsign = Some(self.station_callsign.clone());
        self.config.station.grid =
            (!self.station_grid.trim().is_empty()).then(|| self.station_grid.clone());
        let cache = HamDbCache::open(&hamdb_cache_path()).ok();
        if let Some(cache) = cache {
            let _ = cache.upsert(&entry);
        }
        self.profile_dirty = true;
        self.persist_profile("Loaded license profile from HamDB");
        self.emit_operator_profile_hook("profile_loaded_from_hamdb");
        self.hamdb_profile_lookup_rx = None;
    }

    fn pump_pota_spots(&mut self) {
        if !self.pota_enabled {
            return;
        }
        if let Some(rx) = &self.pota_lookup_rx {
            match rx.try_recv() {
                Ok(spots) => {
                    match spots {
                        Ok(spots) => {
                            let activators = spots
                                .iter()
                                .map(|spot| spot.activator.as_str())
                                .collect::<HashSet<_>>()
                                .len();
                            info!(
                                spots = spots.len(),
                                activators,
                                elapsed_ms = self.pota_last_lookup.elapsed().as_millis() as u64,
                                "POTA activator spots refreshed"
                            );
                            self.pota_spots = spots;
                            self.pota_last_updated = Some(Instant::now());
                            self.pota_last_error = None;
                            self.pota_history.push_back((Instant::now(), activators));
                            while self.pota_history.len() > 60 {
                                self.pota_history.pop_front();
                            }
                        }
                        Err(error) => {
                            warn!(
                                error = %error,
                                elapsed_ms = self.pota_last_lookup.elapsed().as_millis() as u64,
                                "POTA activator spot lookup failed"
                            );
                            self.pota_last_error = Some(error);
                        }
                    }
                    self.pota_lookup_rx = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    let error = "POTA lookup worker disconnected before returning a result";
                    warn!(
                        elapsed_ms = self.pota_last_lookup.elapsed().as_millis() as u64,
                        "POTA activator spot lookup worker disconnected"
                    );
                    self.pota_last_error = Some(error.to_string());
                    self.pota_lookup_rx = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if self.pota_lookup_rx.is_some()
            || self.pota_last_lookup.elapsed() < Duration::from_secs(30)
        {
            return;
        }
        self.pota_last_lookup = Instant::now();
        info!("POTA activator spot lookup started");
        let (tx, rx) = mpsc::channel();
        self.pota_lookup_rx = Some(rx);
        thread::spawn(move || {
            let result = reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .map_err(|error| error.to_string())
                .and_then(|client| {
                    client
                        .get("https://api.pota.app/spot/activator")
                        .send()
                        .map_err(|error| error.to_string())
                })
                .and_then(|response| {
                    response
                        .error_for_status()
                        .map_err(|error| error.to_string())
                })
                .and_then(|response| {
                    response
                        .json::<Vec<PotaApiSpot>>()
                        .map_err(|error| error.to_string())
                })
                .map(|spots| {
                    spots
                        .into_iter()
                        .filter_map(|spot| {
                            Some(PotaSpot {
                                activator: spot.activator?.trim().to_ascii_uppercase(),
                                reference: spot.reference?.trim().to_string(),
                                name: spot.name?.trim().to_string(),
                                frequency_hz: spot.frequency?.parse::<f64>().ok()?.round() as u64
                                    * 1_000,
                                mode: spot.mode?.trim().to_ascii_uppercase(),
                            })
                        })
                        .collect()
                });
            let _ = tx.send(result);
        });
    }

    fn load_profile_from_hamdb(&mut self) {
        let callsign = self.station_callsign.trim().to_ascii_uppercase();
        if callsign.is_empty() || !is_probable_callsign(&callsign) {
            self.profile_io_status =
                "Enter a valid callsign before loading HamDB profile".to_string();
            return;
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or_default();
        info!(callsign = %callsign, "HamDB operator profile lookup started");
        self.hamdb_profile_lookup_rx = Some(spawn_hamdb_lookup(callsign, now));
        self.profile_io_status = "Loading license record from HamDB…".to_string();
    }

    fn refresh_hamdb_for_contact(&mut self, index: usize) {
        let Some(record) = self.qso_log.contacts.get(index) else {
            return;
        };
        let callsign = record.callsign.trim().to_ascii_uppercase();
        if callsign.is_empty() {
            self.qso_log_status = "HamDB lookup requires a callsign".to_string();
            return;
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or_default();
        self.hamdb_lookup_rx = Some(spawn_hamdb_lookup(callsign, now));
        self.qso_log_status = "Refreshing HamDB details…".to_string();
    }

    fn append_qso(&mut self, mut record: QsoRecord, status: &str) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or_default();
        let cache = HamDbCache::open(&hamdb_cache_path()).ok();
        if let Some(cache) = cache.as_ref() {
            enrich_qso_from_hamdb(&mut record, cache, now);
        }
        if cache
            .as_ref()
            .and_then(|cache| {
                cache
                    .get_fresh(&record.callsign, now, HAMDB_CACHE_TTL_SECONDS)
                    .ok()
            })
            .flatten()
            .is_none()
        {
            self.hamdb_lookup_rx = Some(spawn_hamdb_lookup(record.callsign.clone(), now));
        }
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
        let published = self.qso_log.contacts.last().cloned();
        if let Some(last) = &published {
            self.app_events.publish(AppEvent::QsoLogged {
                mode: last.mode.clone(),
                call: last.callsign.clone(),
                band: last.band.clone(),
                frequency_hz: last.frequency_hz,
            });
        }
        self.qso_selected = self.qso_log.contacts.last().map(|contact| contact.id);
        self.qso_log_dirty = true;
        self.persist_qso_log(status);
        if let Some(record) = &published {
            self.publish_qso_to_server(record);
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

    fn read_radio_profile(&self, name: &str, snapshot: &GuiState) -> RadioProfile {
        RadioProfile {
            name: name.to_string(),
            mode: Some(snapshot.mode.clone()),
            data_mode: snapshot.data_mode,
            filter: snapshot.filter,
            af_gain: snapshot.af_gain,
            rf_gain: snapshot.rf_gain,
            rf_power: snapshot.rf_power,
            preamp: None,
            attenuator: None,
            noise_blank: None,
            noise_reduction: None,
            agc: None,
        }
    }

    fn apply_radio_profile(&mut self, profile: RadioProfile) {
        let Some(tx) = &self.command_tx else {
            self.profile_io_status = "Radio tuning unavailable: radio is not connected".to_string();
            return;
        };
        if let Some(mode) = profile.mode.as_deref() {
            if let Some(workspace_mode) = WORKSPACE_MODES
                .iter()
                .copied()
                .find(|candidate| candidate.label().eq_ignore_ascii_case(mode))
            {
                let frequency_hz = self.state.lock().ok().and_then(|state| state.frequency_hz);
                if let Some(frequency_hz) = frequency_hz {
                    let _ = tx.send(GuiCommand::ApplyWorkspace {
                        mode: workspace_mode,
                        frequency_hz,
                    });
                }
            }
        }
        if let Some(filter) = profile.filter {
            let _ = tx.send(GuiCommand::SetFilter(filter));
        }
        for (control, value) in [
            (ControlId::AfGain, profile.af_gain.map(ControlValue::U8)),
            (ControlId::RfGain, profile.rf_gain.map(ControlValue::U8)),
            (ControlId::RfPower, profile.rf_power.map(ControlValue::U8)),
            (ControlId::Preamp, profile.preamp.map(ControlValue::Bool)),
            (
                ControlId::Attenuator,
                profile.attenuator.map(ControlValue::Bool),
            ),
            (
                ControlId::NoiseBlanker,
                profile.noise_blank.map(ControlValue::Bool),
            ),
            (
                ControlId::NoiseReduction,
                profile.noise_reduction.map(ControlValue::Bool),
            ),
            (ControlId::Agc, profile.agc.map(ControlValue::U8)),
        ] {
            if let Some(value) = value {
                let _ = tx.send(GuiCommand::SetControl(control, value));
            }
        }
        self.profile_io_status = format!("Applied radio profile {}", profile.name);
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

    fn current_operator_profile(&self) -> OperatorProfile {
        let display_tuning = self
            .display_tuning
            .lock()
            .expect("display tuning lock poisoned");
        OperatorProfile {
            profile_version: OPERATOR_PROFILE_VERSION,
            callsign: self.station_callsign_or_default().to_string(),
            grid: self.station_grid_or_default().to_string(),
            qth: self.station_qth.trim().to_string(),
            station_rig: self.station_rig.trim().to_string(),
            station_antenna: self.station_antenna.trim().to_string(),
            station_notes: self.station_notes.trim().to_string(),
            llm_prompt_context: self.llm_prompt_context.trim().to_string(),
            sstv_image_requirements: self.sstv_image_requirements.trim().to_string(),
            llm_model_notes: self.llm_model_notes.trim().to_string(),
            follow_log: self.ft8_follow_log,
            max_log_entries: self.ft8_max_log_entries.clamp(80, 1000),
            deep_decode: self.ft8_deep_decode,
            ft4_deep_decode: self.ft4_deep_decode,
            ft4_autoseq: self.ft4_autoseq,
            ft4_auto_reply_policy: self.ft4_auto_reply_policy,
            ft4_cq_only_view: self.ft4_cq_only_view,
            ft4_follow_log: self.ft4_follow_log,
            ft4_max_log_entries: self.ft4_max_log_entries.clamp(80, 300),
            ft4_max_attempts: self.ft4_max_attempts.clamp(1, 20),
            autoseq: self.ft8_autoseq,
            auto_reply_policy: self.ft8_auto_reply_policy,
            auto_answer_cq: self.ft8_auto_answer_cq,
            automation_unlocked: self.automation_unlocked,
            cq_only_view: self.ft8_cq_only_view,
            civ_spectrum_on: self.civ_spectrum_on,
            radio_scope_vbw_wide: self.radio_scope_vbw_wide,
            radio_scope_view: self.radio_scope_view,
            waterfall_theme: self.waterfall_theme,
            radio_waterfall_theme: self.radio_waterfall_theme,
            waterfall_auto_visual: display_tuning.audio_auto_visual,
            waterfall_speed: display_tuning.audio_waterfall_speed,
            waterfall_deck_height: self.waterfall_deck_height,
            // This control is deliberately one-shot and is not restored on launch.
            halt_after_tx: false,
            ft8_max_attempts: self.ft8_max_attempts.clamp(1, 20),
            hold_tx_freq: self.ft8_hold_tx_freq,
            rx_tone_hz: self.rx_tone_hz,
            tx_tone_hz: self.tx_tone_hz,
            ptt_lead_ms: self.ptt_lead_ms.clamp(0, 500),
            ptt_tail_ms: self.ptt_tail_ms.clamp(0, 500),
            cw_wpm: self.cw_wpm.clamp(5, 40),
            cw_tone_hz: self.cw_tone_hz.clamp(200, 3_000),
            recording_enabled: self.recording_enabled,
            recording_modes: self.recording_modes.clone(),
            recording_full_width: self.recording_full_width,
            recording_stream: self.recording_stream,
            audio_input_device: self.config.audio.input_device.clone(),
            audio_enabled: self.config.audio.enabled,
            audio_output_device: self.config.audio.output_device.clone(),
            audio_monitor_enabled: self.config.audio.monitor_enabled,
            audio_monitor_output_device: self.config.audio.monitor_output_device.clone(),
            audio_monitor_volume: self.config.audio.monitor_volume.clamp(0.0, 2.0),
            audio_sample_rate_hz: self.config.audio.sample_rate_hz,
            audio_channels: self.config.audio.channels,
            radio_enabled: self.config.radio.enabled,
            radio_serial_port: self.config.radio.serial_port.clone(),
            radio_backend: self.config.radio.backend.clone(),
            radio_endpoint: self.config.radio.endpoint.clone(),
            radio_model: self.config.radio.model.clone(),
            radio_baud_rate: self.config.radio.baud_rate,
            radio_civ_address: self.config.radio.civ_address,
            radio_controller_civ_address: self.config.radio.controller_civ_address,
            gui_scale: self.gui_scale.clamp(GUI_SCALE_MIN, GUI_SCALE_MAX),
            compute_preference: self.compute_preference,
            psk_reporter_enabled: self.psk_reporter_enabled,
            pota_enabled: self.pota_enabled,
            psk_batch_interval_secs: self.psk_batch_interval_secs.clamp(60, 3_600),
            psk_repeat_cache_secs: self.psk_repeat_cache_secs.clamp(60, 3_600),
            psk_max_pending: self.psk_max_pending.clamp(1, 2_048),
            server_instance_id: self.server_instance_id.clone(),
            server: Some(self.config.server.clone()),
            contest_enabled: self.contest_enabled,
            contest_operating_mode: self.contest_operating_mode,
            contest_split_policy: self.contest_split_policy,
            contest_fox_hound_role: self.contest_fox_hound_role,
            contest_exchange_template: self.contest_exchange_template.trim().to_string(),
            contest_serial_start: self.contest_serial_start.max(1),
            contest_serial_step: self.contest_serial_step.max(1),
            contest_dupe_check: self.contest_dupe_check,
            contest_serial_current: self
                .contest_serial_current
                .max(self.contest_serial_start.max(1)),
            contest_fake_split_offset_hz: self.contest_fake_split_offset_hz,
            hunter_unlocked: self.hunter_unlocked.iter().copied().collect(),
            hunter_acknowledged: self.hunter_acknowledged.iter().copied().collect(),
            hunter_alerts_enabled: self.hunter_alerts_enabled,
            hunter_custom_rules: self.hunter_custom_rules.clone(),
            // Reusable radio definitions are global; the profile only stores
            // the per-mode assignment above.
            radio_profiles: Vec::new(),
            mode_radio_profile: self.mode_radio_profile.clone(),
            workspace_mode: self.workspace_mode.label().to_string(),
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

    fn restart_psk_reporter(&mut self) {
        self.psk_reporter = None;
        self.psk_reporter = start_psk_reporter(
            self.psk_reporter_enabled,
            self.station_callsign.trim(),
            self.station_grid.trim(),
            ReporterTuning {
                batch_interval_secs: self.psk_batch_interval_secs,
                repeat_cache_secs: self.psk_repeat_cache_secs,
                max_pending: self.psk_max_pending,
            },
            &self.state,
        );
        let sender = self.psk_reporter.as_ref().map(Reporter::sender);
        for session in self.parked_radio_sessions.values() {
            if let Ok(mut state) = session.state.lock() {
                state.psk_report_sender = sender.clone();
            }
        }
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

    fn draw_tx_safety_card(&mut self, ui: &mut egui::Ui, snapshot: &GuiState) {
        let armed = self.any_tx_armed(snapshot);
        let (fill, border, status, detail) = if armed {
            (
                Color32::from_rgb(73, 35, 24),
                Color32::from_rgb(255, 137, 61),
                "🔥 TRANSMIT ARMED",
                "Digital automation, SSTV, queued audio, or PTT can transmit",
            )
        } else {
            (
                Color32::from_rgb(22, 48, 59),
                Color32::from_rgb(77, 184, 211),
                "🔒 ALL TX DISARMED",
                "Safe state · arm explicitly from a transmit workspace",
            )
        };

        egui::Frame::group(ui.style())
            .fill(fill)
            .stroke(egui::Stroke::new(2.0_f32, border))
            .show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new(status).strong().size(17.0).color(border));
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
                                1.5_f32,
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
        match self.workspace_mode {
            WorkspaceMode::Ft8 => self.draw_ft8_workspace(ui, ctx, snapshot),
            WorkspaceMode::Ft4 => self.draw_ft4_workspace(ui, snapshot),
            WorkspaceMode::Fst4 => self.draw_fst4_workspace(ui, snapshot),
            WorkspaceMode::Wspr => self.draw_wspr_workspace(ui, snapshot),
            WorkspaceMode::Jt9 => self.draw_jt9_workspace(ui, snapshot),
            WorkspaceMode::Jt65 => self.draw_jt65_workspace(ui, snapshot),
            WorkspaceMode::Q65 => self.draw_q65_workspace(ui, snapshot),
            WorkspaceMode::Cw => self.draw_cw_workspace(ui, snapshot),
            WorkspaceMode::Voice => self.draw_voice_workspace(ui, snapshot),
            WorkspaceMode::Sstv => self.draw_sstv_workspace(ui, ctx, snapshot),
            WorkspaceMode::Msk144 | WorkspaceMode::Fldigi => {
                self.draw_mfsk_mode_workspace(ui, snapshot, self.workspace_mode)
            }
        }
    }

    fn draw_bounded_workspace(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        snapshot: &GuiState,
    ) {
        if matches!(
            self.workspace_mode,
            WorkspaceMode::Ft8 | WorkspaceMode::Ft4 | WorkspaceMode::Voice | WorkspaceMode::Sstv
        ) {
            self.draw_workspace(ui, ctx, snapshot);
        } else {
            egui::ScrollArea::both()
                .id_salt("workspace_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| self.draw_workspace(ui, ctx, snapshot));
        }
    }

    fn split_decode_workspace_height(available_height: f32) -> (f32, f32) {
        const GAP: f32 = 4.0;
        const TX_MIN: f32 = 72.0;
        const TX_MAX: f32 = 180.0;
        let tx_height = (available_height * 0.22).clamp(TX_MIN, TX_MAX);
        let decode_height = (available_height - GAP - tx_height).max(0.0);
        (decode_height, tx_height)
    }

    fn draw_about_button(&self, ui: &mut egui::Ui) {
        let (about_rect, about_button) =
            ui.allocate_exact_size(egui::vec2(28.0, 28.0), egui::Sense::click());
        let about_color = Color32::from_rgb(120, 210, 235);
        draw_radio_about_icon(&ui.painter_at(about_rect), about_rect, about_color);
        let about_button = about_button.on_hover_text("About QSONaut");
        egui::Popup::menu(&about_button).show(|ui| {
            ui.set_min_width(300.0);
            ui.heading("QSONaut");
            ui.label("Amateur Radio Mission Control");
            ui.label(format!("Version {}", env!("CARGO_PKG_VERSION")));
            ui.separator();
            ui.label("Original author");
            ui.label(RichText::new("N7UF").strong());
            ui.label("Copyright © 2026 N7UF and contributors");
            ui.label("Released under the MIT License.");
            ui.separator();
            ui.label(RichText::new("Contributors").strong());
            ui.label(qsonaut_contributors());
            ui.label(RichText::new("Testers").strong());
            ui.label(qsonaut_testers());
            ui.separator();
            ui.horizontal_wrapped(|ui| {
                ui.hyperlink_to("GitHub", QSONAUT_GITHUB_URL);
                ui.hyperlink_to("File an issue", QSONAUT_ISSUES_URL);
                ui.hyperlink_to("qsonaut.com", QSONAUT_WEBSITE_URL);
            });
        });
    }

    fn draw_meter_display(&mut self, ui: &mut egui::Ui, snapshot: &GuiState) {
        let primary_id = if snapshot.ptt_on {
            MeterId::Power
        } else {
            MeterId::Signal
        };
        let primary_value = meter_value(snapshot, primary_id);
        let primary_label = if snapshot.ptt_on { "POWER" } else { "" };
        let primary_reading = meter_reading(primary_id, primary_value);
        let radio_model = self.config.radio.model.as_str();
        let (primary_rect, primary_response) =
            ui.allocate_exact_size(egui::vec2(280.0, 24.0), egui::Sense::click());
        draw_primary_meter(
            ui,
            primary_rect,
            primary_label,
            &primary_reading,
            primary_value.map(meter_percent).unwrap_or_default(),
            meter_color(primary_id, primary_value),
        );
        let s_click = primary_response.on_hover_text("Click to open the live radio meter drawer");
        if s_click.clicked() {
            self.show_meter_panel = !self.show_meter_panel;
            self.meter_panel_close_deadline = None;
        }

        if self.show_meter_panel {
            let drawer_position = egui::pos2(primary_rect.left(), primary_rect.bottom() + 5.0);
            egui::Area::new(ui.id().with("meter_drawer_overlay"))
                .order(egui::Order::Foreground)
                .fixed_pos(drawer_position)
                .show(ui.ctx(), |ui| {
                    egui::Frame::group(ui.style())
                        .fill(Color32::from_rgba_unmultiplied(18, 30, 42, 245))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(format!(
                                        "TX / PA METERS · {}",
                                        radio_mode_label(&snapshot.mode, snapshot.data_mode)
                                    ))
                                    .strong()
                                    .color(
                                        if snapshot.ptt_on {
                                            Color32::from_rgb(255, 145, 120)
                                        } else {
                                            Color32::from_rgb(120, 225, 255)
                                        },
                                    ),
                                );
                                if snapshot.ptt_on {
                                    ui.label(
                                        RichText::new("● TRANSMIT").strong().color(Color32::RED),
                                    );
                                }
                            });
                            ui.separator();
                            if snapshot.ptt_on
                                && snapshot.supported_controls.contains(&ControlId::RfPower)
                            {
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new("TX SET").monospace().strong())
                                        .on_hover_text(
                                            "Configured RF transmit power, not measured output",
                                        );
                                    let value = snapshot.rf_power;
                                    ui.label(meter_reading(MeterId::Power, value));
                                });
                            }
                            for id in mode_meter_order(snapshot.ptt_on) {
                                if id == MeterId::Signal {
                                    continue;
                                }
                                if !snapshot.supported_meters.contains(&id) {
                                    continue;
                                }
                                let value = meter_value(snapshot, id);
                                ui.horizontal(|ui| {
                                    let reading = meter_reading_for_model(id, value, radio_model);
                                    let label_height =
                                        if id == MeterId::Voltage { 28.0 } else { 18.0 };
                                    let label_color = if id == MeterId::Current && snapshot.ptt_on {
                                        Color32::from_rgb(150, 255, 225)
                                    } else {
                                        Color32::WHITE
                                    };
                                    ui.add_sized(
                                        egui::vec2(METER_LABEL_WIDTH, label_height),
                                        egui::Label::new(
                                            RichText::new(meter_label(id))
                                                .monospace()
                                                .strong()
                                                .color(label_color),
                                        ),
                                    )
                                    .on_hover_text(meter_tooltip(id));
                                    if id == MeterId::Voltage {
                                        draw_voltage_graph(ui, &snapshot.voltage_history, &reading);
                                        return;
                                    }
                                    let meter_response = ui.add(
                                        egui::ProgressBar::new(
                                            value.map(meter_percent).unwrap_or_default(),
                                        )
                                        .desired_width(ui.available_width().max(100.0))
                                        .desired_height(14.0)
                                        .fill(meter_color_for_context(id, value, snapshot.ptt_on)),
                                    );
                                    let reading_width = 136.0;
                                    let reading_rect = egui::Rect::from_min_max(
                                        egui::pos2(
                                            meter_response.rect.right() - reading_width,
                                            meter_response.rect.top() + 1.0,
                                        ),
                                        egui::pos2(
                                            meter_response.rect.right() - 3.0,
                                            meter_response.rect.bottom() - 1.0,
                                        ),
                                    );
                                    ui.painter().rect_filled(
                                        reading_rect,
                                        egui::CornerRadius::same(3),
                                        Color32::from_rgba_unmultiplied(10, 20, 29, 225),
                                    );
                                    ui.painter().text(
                                        reading_rect.right_center() - egui::vec2(5.0, 0.0),
                                        egui::Align2::RIGHT_CENTER,
                                        reading,
                                        egui::FontId::monospace(11.0),
                                        Color32::WHITE,
                                    );
                                });
                            }
                        });
                });
        }
    }

    fn draw_header_identity_and_activity(&self, ui: &mut egui::Ui) {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.horizontal_wrapped(|ui| {
            let activity_profile = self.activity.profile();
            ui.label(
                RichText::new(format!("CALL · {}", activity_profile.tx_cq))
                    .size(18.0)
                    .monospace()
                    .color(Color32::from_rgb(255, 190, 105)),
            )
            .on_hover_text(format!(
                "Call behavior\nActivity: {}\nTransmit calling text: {}",
                self.activity.label(),
                activity_profile.tx_cq
            ));
            ui.label(RichText::new("·").color(Color32::DARK_GRAY));
            ui.label(
                RichText::new(format!(
                    "📍 {} · {}",
                    self.station_callsign_or_default(),
                    self.station_grid_or_default()
                ))
                .strong()
                .size(18.0)
                .color(Color32::from_rgb(255, 210, 110)),
            );
            if let Some(client) = &self.server_client {
                let server_status = client.status();
                if server_status.state == ServerConnectionState::Connected
                    && server_status.active_event_count > 0
                {
                    let label = if self.activity == OperatingActivity::Contest
                        && self.contest_enabled
                    {
                        "✅ SERVER CONTEST · PARTICIPATING".to_string()
                    } else {
                        format!("✅ SERVER CONTEST · {} ACTIVE", server_status.active_event_count)
                    };
                    ui.label(
                        RichText::new(label)
                            .size(15.0)
                            .strong()
                            .color(Color32::from_rgb(125, 225, 150)),
                    )
                    .on_hover_text(format!(
                        "Server contest status\nConnected server events: {}\nThis indicator reflects shared contest activity.",
                        server_status.active_event_count
                    ));
                }
            }
        });
    }

    fn draw_power_button(&mut self, ui: &mut egui::Ui, snapshot: &GuiState) {
        let power_known = snapshot.radio_power_on.is_some();
        let power_on = snapshot.radio_power_on.unwrap_or(false);
        let (power_rect, power_button) = ui.allocate_exact_size(
            egui::vec2(28.0, 28.0),
            if snapshot.radio_power_supported && !snapshot.radio_power_command_pending {
                egui::Sense::click()
            } else {
                egui::Sense::hover()
            },
        );
        let power_color = if !snapshot.radio_power_supported {
            Color32::DARK_GRAY
        } else if !power_known {
            Color32::GRAY
        } else if power_on {
            Color32::LIGHT_GREEN
        } else {
            Color32::GRAY
        };
        let painter = ui.painter_at(power_rect);
        let center = power_rect.center();
        painter.circle_stroke(center, 8.0, egui::Stroke::new(2.0_f32, power_color));
        painter.line_segment(
            [
                egui::pos2(center.x, center.y - 11.0),
                egui::pos2(center.x, center.y + 1.0),
            ],
            egui::Stroke::new(2.0_f32, power_color),
        );
        if power_button.clicked() {
            self.state
                .lock()
                .expect("ui state lock poisoned")
                .radio_power_command_pending = true;
            self.send_command(GuiCommand::SetPower(!power_on));
        }
        power_button.on_hover_text(if !power_known {
            "Radio power: unknown · click to turn on"
        } else if power_on {
            "Radio power: ON · click to turn off"
        } else {
            "Radio power: OFF · click to turn on"
        });
    }

    fn draw_rf_power_control(&mut self, ui: &mut egui::Ui, snapshot: &GuiState) {
        let supported = snapshot.supported_controls.contains(&ControlId::RfPower);
        let ready = snapshot.radio_power_on == Some(true)
            && !snapshot.radio_power_command_pending
            && !snapshot.radio_power_settling;
        let color = if supported && ready {
            Color32::from_rgb(255, 190, 105)
        } else {
            Color32::GRAY
        };
        let power_button = ui
            .add_enabled(
                supported && ready,
                egui::Button::new(RichText::new("⚡").size(17.0).color(color)),
            )
            .on_hover_text(if !supported {
                "RF transmit power control is not supported by this radio profile"
            } else if !ready {
                "RF transmit power is unavailable while the radio is offline or waking"
            } else {
                "Open RF transmit power control"
            });
        egui::Popup::menu(&power_button)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
                ui.vertical_centered(|ui| {
                    let mut percent = snapshot
                        .rf_power
                        .map(|value| f32::from(value) * 100.0 / 255.0)
                        .unwrap_or_default();
                    let response = ui.add(
                        egui::Slider::new(&mut percent, 0.0..=100.0)
                            .vertical()
                            .show_value(false),
                    );
                    ui.label(format!("{percent:.0}%"));
                    if response.changed() && response.drag_stopped() {
                        let normalized = (percent.clamp(0.0, 100.0) * 255.0 / 100.0).round() as u8;
                        {
                            let mut state = self.state.lock().expect("ui state lock poisoned");
                            state.rf_power = Some(normalized);
                            state.rf_power_write_pending = Some(normalized);
                        }
                        self.send_command(GuiCommand::SetControl(
                            ControlId::RfPower,
                            ControlValue::U8(normalized),
                        ));
                    }
                });
            });
    }

    fn draw_connection_status(&mut self, ui: &mut egui::Ui, snapshot: &GuiState) {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Connections").strong());
            ui.separator();
            ui.label(
                RichText::new(if snapshot.frequency_hz.is_some() {
                    "Radio CONNECTED"
                } else {
                    "Radio OFFLINE"
                })
                .color(if snapshot.frequency_hz.is_some() {
                    Color32::LIGHT_GREEN
                } else {
                    Color32::GRAY
                }),
            );
            let (server_label, server_color) = self
                .server_client
                .as_ref()
                .map(|client| match client.status().state {
                    ServerConnectionState::Connected => {
                        ("QSONaut Server CONNECTED", Color32::LIGHT_GREEN)
                    }
                    ServerConnectionState::Connecting | ServerConnectionState::Reconnecting => {
                        ("QSONaut Server CONNECTING", theme_warning(ui))
                    }
                    ServerConnectionState::Disabled | ServerConnectionState::Stopped => {
                        ("QSONaut Server OFFLINE", Color32::GRAY)
                    }
                })
                .unwrap_or(("QSONaut Server DISABLED", Color32::GRAY));
            ui.separator();
            ui.label(RichText::new(server_label).color(server_color));
            ui.separator();
            let pota_activators = self
                .pota_spots
                .iter()
                .map(|spot| spot.activator.as_str())
                .collect::<HashSet<_>>()
                .len();
            let pota_label = if !self.pota_enabled {
                "🌲 POTA OFF".to_string()
            } else if self.pota_lookup_rx.is_some() {
                "🌲 POTA …".to_string()
            } else {
                format!("🌲 POTA {pota_activators}")
            };
            let pota_button = ui
                .selectable_label(self.pota_enabled && !self.pota_spots.is_empty(), pota_label)
                .on_hover_text("Show live POTA activator statistics and spots");
            egui::Popup::menu(&pota_button).show(|ui| self.draw_pota_panel(ui));
            ui.separator();
            if !self.psk_reporter_enabled {
                ui.label(RichText::new("PSK OFF").color(Color32::GRAY))
                    .on_hover_text(
                        "Enable in the Reporting panel to batch decoded stations to PSK Reporter",
                    );
            } else if let Some(reporter) = &self.psk_reporter {
                let status = reporter.status();
                let (label, color) = if status.last_error.is_some() {
                    ("PSK ERROR".to_string(), Color32::from_rgb(255, 110, 100))
                } else if !status.active {
                    ("PSK STOPPED".to_string(), theme_warning(ui))
                } else {
                    (
                        format!("PSK {}q · {} sent", status.queued, status.sent),
                        Color32::LIGHT_GREEN,
                    )
                };
                ui.label(RichText::new(label).color(color)).on_hover_text(
                    status
                        .last_error
                        .as_deref()
                        .map(|error| format!("network error: {error}"))
                        .unwrap_or_else(|| {
                            format!(
                                "PSK Reporter batching every ~{} s · same callsign re-reported after {} s · {} max pending",
                                self.psk_batch_interval_secs,
                                self.psk_repeat_cache_secs,
                                self.psk_max_pending
                            )
                        }),
                );
            } else {
                ui.label(RichText::new("PSK WAITING").color(theme_warning(ui)))
                    .on_hover_text("Set a real callsign and grid before reporting");
            }
            ui.separator();
            ui.label(
                RichText::new(format!("Compute {}", self.acceleration_report.summary()))
                    .color(Color32::from_rgb(180, 150, 255)),
            )
            .on_hover_text(self.acceleration_report.hardware_detail());
            ui.separator();
            for label in ["IRC", "Discord"] {
                ui.label(RichText::new(format!("{label} N/A")).color(Color32::GRAY));
                ui.separator();
            }
            if let Some(error) = &snapshot.last_error {
                ui.label(RichText::new("⚠ NEEDS ATTENTION").color(theme_warning(ui)))
                    .on_hover_text(error);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                self.draw_about_button(ui);
            });
        });
    }

    fn draw_waterfall_buttons(&mut self, ui: &mut egui::Ui, snapshot: &GuiState) {
        let audio_button = ui
            .button(RichText::new("〰").size(16.0).color(Color32::LIGHT_BLUE))
            .on_hover_text("Audio waterfall controls");
        egui::Popup::menu(&audio_button)
            // Keep the drawer alive while nested combo boxes and sliders are
            // interacting with it. The banner redraws it every frame.
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
                ui.set_min_width(250.0);
                ui.label(RichText::new("AUDIO WATERFALL").strong());
                ui.label(RichText::new("Live audio spectrum display").small());
                ui.separator();
                ui.label(RichText::new("Theme").strong());
                ui.horizontal_wrapped(|ui| {
                    for theme in [
                        WaterfallTheme::RadioBlue,
                        WaterfallTheme::Inferno,
                        WaterfallTheme::Phosphor,
                        WaterfallTheme::Monochrome,
                    ] {
                        if ui
                            .selectable_label(self.waterfall_theme == theme, theme.label())
                            .clicked()
                        {
                            self.waterfall_theme = theme;
                            self.profile_dirty = true;
                            self.persist_profile("Waterfall theme saved to");
                        }
                    }
                });
                ui.label(RichText::new("Audio display speed").strong());
                {
                    let mut tuning = self.display_tuning.lock().expect("tuning lock poisoned");
                    let selected = if tuning.audio_auto_visual {
                        "Auto"
                    } else {
                        tuning.audio_waterfall_speed.label()
                    };
                    ui.horizontal_wrapped(|ui| {
                        if ui
                            .selectable_label(tuning.audio_auto_visual, selected)
                            .clicked()
                        {
                            tuning.audio_auto_visual = true;
                        }
                        for speed in [
                            WaterfallSpeed::Fast,
                            WaterfallSpeed::Mid,
                            WaterfallSpeed::Slow,
                        ] {
                            let selected =
                                !tuning.audio_auto_visual && tuning.audio_waterfall_speed == speed;
                            if ui.selectable_label(selected, speed.label()).clicked() {
                                tuning.audio_auto_visual = false;
                                tuning.audio_waterfall_speed = speed;
                            }
                        }
                    });
                }
            });

        let radio_scope_available = self.config.radio.enabled
            && native_radio_profile(&self.config.radio.backend, &self.config.radio.model)
                .is_some_and(|profile| profile.capabilities.spectrum);
        if radio_scope_available {
            let radio_button = ui
                .button(
                    RichText::new("🌈")
                        .size(15.0)
                        .color(Color32::from_rgb(180, 220, 255)),
                )
                .on_hover_text("Native CI-V waterfall controls");
            egui::Popup::menu(&radio_button)
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                .show(|ui| {
                    ui.set_min_width(280.0);
                    ui.label(RichText::new("RADIO WATERFALL").strong());
                    ui.label(RichText::new("Native scope stream controls").small());
                    ui.separator();
                    if ui
                        .checkbox(&mut self.civ_spectrum_on, "Enable radio waterfall")
                        .changed()
                    {
                        self.profile_dirty = true;
                        self.persist_profile("Radio waterfall setting saved to");
                    }
                    ui.label(RichText::new("Radio waterfall theme").strong());
                    ui.horizontal_wrapped(|ui| {
                        for theme in [
                            WaterfallTheme::RadioBlue,
                            WaterfallTheme::Inferno,
                            WaterfallTheme::Phosphor,
                            WaterfallTheme::Monochrome,
                        ] {
                            if ui
                                .selectable_label(
                                    self.radio_waterfall_theme == theme,
                                    theme.label(),
                                )
                                .clicked()
                            {
                                self.radio_waterfall_theme = theme;
                                self.profile_dirty = true;
                                self.persist_profile("Radio waterfall theme saved to");
                            }
                        }
                    });
                    ui.label(RichText::new("Native sweep speed").strong());
                    let mut visual_changed = false;
                    {
                        let mut tuning = self.display_tuning.lock().expect("tuning lock poisoned");
                        ui.horizontal_wrapped(|ui| {
                            ui.label(if tuning.radio_auto_visual {
                                "Auto (mode-driven)"
                            } else {
                                "Native speed"
                            });
                            for speed in [
                                WaterfallSpeed::Fast,
                                WaterfallSpeed::Mid,
                                WaterfallSpeed::Slow,
                            ] {
                                let selected = !tuning.radio_auto_visual
                                    && tuning.radio_waterfall_speed == speed;
                                if ui.selectable_label(selected, speed.label()).clicked() {
                                    tuning.radio_auto_visual = false;
                                    tuning.radio_waterfall_speed = speed;
                                    visual_changed = true;
                                }
                            }
                        });
                    }
                    if visual_changed {
                        self.state
                            .lock()
                            .expect("ui state lock poisoned")
                            .radio_scope_settings_dirty = true;
                    }
                    let mut scope_changed = false;
                    ui.horizontal(|ui| {
                        scope_changed |= ui
                            .selectable_value(
                                &mut self.radio_scope_view,
                                RadioScopeView::Narrow,
                                "Narrow",
                            )
                            .changed();
                        scope_changed |= ui
                            .selectable_value(
                                &mut self.radio_scope_view,
                                RadioScopeView::Overview,
                                "Overview",
                            )
                            .changed();
                    });
                    scope_changed |= ui
                        .checkbox(&mut self.radio_scope_vbw_wide, "Wide VBW")
                        .changed();
                    ui.checkbox(&mut self.radio_scope_lock_if_to_filter, "Match span to FIL");
                    if self.radio_scope_lock_if_to_filter {
                        self.radio_scope_span_code =
                            scope_span_for_filter(&snapshot.mode, snapshot.filter);
                        ui.label(format!(
                            "Automatic span: {}",
                            scope_span_label(self.radio_scope_span_code)
                        ));
                    } else {
                        ui.horizontal_wrapped(|ui| {
                            for (code, label) in [
                                (0_u8, "±2.5 kHz"),
                                (1, "±5 kHz"),
                                (2, "±10 kHz"),
                                (3, "±25 kHz"),
                                (4, "±50 kHz"),
                                (5, "±100 kHz"),
                                (6, "±250 kHz"),
                                (7, "±500 kHz"),
                            ] {
                                if ui
                                    .selectable_label(self.radio_scope_span_code == code, label)
                                    .clicked()
                                {
                                    self.radio_scope_span_code = code;
                                    scope_changed = true;
                                }
                            }
                        });
                    }
                    scope_changed |= ui.checkbox(&mut self.radio_scope_hold, "Hold").changed();
                    scope_changed |= ui
                        .add(
                            egui::Slider::new(
                                &mut self.radio_scope_reference_tenths_db,
                                -200..=200,
                            )
                            .step_by(5.0)
                            .custom_formatter(|value, _| format!("{:.1} dB", value / 10.0))
                            .text("Reference"),
                        )
                        .changed();
                    if scope_changed {
                        self.state
                            .lock()
                            .expect("ui state lock poisoned")
                            .radio_scope_settings_dirty = true;
                    }
                });
        }

        ui.add_enabled(
            false,
            egui::Button::new(
                RichText::new("📡")
                    .size(13.0)
                    .color(Color32::from_gray(145)),
            ),
        )
        .on_disabled_hover_text(
            "IQ/SDR waterfall support is not enabled yet — development needs a radio that offers a supported IQ/SDR stream 😢",
        );
    }

    fn draw_banner_radio_controls(&mut self, ui: &mut egui::Ui, snapshot: &GuiState) {
        let supports_levels = snapshot.supported_controls.contains(&ControlId::AfGain);
        ui.horizontal(|ui| {
            ui.scope(|ui| {
                ui.spacing_mut().item_spacing.x = 7.0;
                ui.spacing_mut().button_padding.x = 4.0;
                ui.label(RichText::new("Radio").strong());
                if ui
                    .small_button("-1 kHz")
                    .on_hover_text("Tune the radio down by 1 kHz")
                    .clicked()
                {
                    self.send_command(GuiCommand::TuneDelta(-1_000));
                }
                if ui
                    .small_button("+1 kHz")
                    .on_hover_text("Tune the radio up by 1 kHz")
                    .clicked()
                {
                    self.send_command(GuiCommand::TuneDelta(1_000));
                }
                if ui
                    .add_enabled(supports_levels, egui::Button::new("AF-").small())
                    .on_disabled_hover_text("Audio receive gain is not supported by this radio")
                    .on_hover_text("Decrease audio receive gain")
                    .clicked()
                {
                    self.send_command(GuiCommand::AfGainDelta(-5));
                }
                if ui
                    .add_enabled(supports_levels, egui::Button::new("AF+").small())
                    .on_disabled_hover_text("Audio receive gain is not supported by this radio")
                    .on_hover_text("Increase audio receive gain")
                    .clicked()
                {
                    self.send_command(GuiCommand::AfGainDelta(5));
                }
            });
            ui.separator();
            ui.label(RichText::new("Op mode").strong());
            for mode in HF_WORKSPACE_MODES {
                let response = ui
                    .add(
                        egui::Button::selectable(
                            self.workspace_mode == mode,
                            RichText::new(mode.label()).size(12.0),
                        )
                        .small(),
                    )
                    .on_hover_text(format!("Switch workspace to {}", mode.label()));
                if response.clicked() {
                    self.workspace_mode = mode;
                    self.profile_dirty = true;
                    self.persist_profile("Mode saved to");
                    if let Some(frequency_hz) =
                        workspace_frequency_for_current_band(mode, snapshot.frequency_hz)
                    {
                        self.send_command(GuiCommand::ApplyWorkspace { mode, frequency_hz });
                    }
                }
            }
            for mode in OTHER_WORKSPACE_MODES {
                let enabled = !mode.is_uhf();
                let response = ui
                    .add_enabled(
                        enabled,
                        egui::Button::selectable(
                            self.workspace_mode == mode,
                            RichText::new(mode.label()).size(12.0),
                        )
                        .small(),
                    )
                    .on_hover_text(if enabled {
                        format!("Switch workspace to {}", mode.label())
                    } else {
                        format!(
                            "{} is disabled without a configured UHF radio",
                            mode.label()
                        )
                    });
                if response.clicked() && enabled {
                    self.workspace_mode = mode;
                    self.profile_dirty = true;
                    self.persist_profile("Mode saved to");
                    if let Some(frequency_hz) =
                        workspace_frequency_for_current_band(mode, snapshot.frequency_hz)
                    {
                        self.send_command(GuiCommand::ApplyWorkspace { mode, frequency_hz });
                    }
                }
            }
        });
    }
}

impl eframe::App for QsonautGuiApp {
    fn save(&mut self, _storage: &mut dyn eframe::Storage) {
        if let Some(geometry) = self.window_geometry {
            geometry.save();
        }
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.pump_hamdb_lookup();
        if let Some(rx) = &self.cat_test_rx {
            match rx.try_recv() {
                Ok(result) => {
                    self.cat_test_status = Some(result);
                    self.cat_test_rx = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.cat_test_status = Some(Err("CAT test worker stopped unexpectedly".into()));
                    self.cat_test_rx = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if let Some(geometry) = WindowGeometry::read(ctx, self.window_geometry) {
            self.window_geometry = Some(geometry);
        }
        let viewport_state = ctx.input(|input| {
            let viewport = input.viewport();
            format!(
                "outer={:?}; inner={:?}; maximized={:?}; minimized={:?}; focused={:?}; close_requested={:?}",
                viewport.outer_rect,
                viewport.inner_rect,
                viewport.maximized,
                viewport.minimized,
                viewport.focused,
                viewport.close_requested(),
            )
        });
        if self.last_viewport_log.as_deref() != Some(viewport_state.as_str()) {
            info!(state = %viewport_state, "Native viewport state changed");
            self.last_viewport_log = Some(viewport_state);
        }
        if !self.first_frame_logged {
            self.first_frame_logged = true;
            info!(
                renderer = %self.selected_renderer,
                os = std::env::consts::OS,
                os_dpi_adjustment = self.os_dpi_adjustment,
                zoom_factor = ctx.zoom_factor(),
                pixels_per_point = ctx.pixels_per_point(),
                effective_pixels_per_point = ctx.pixels_per_point(),
                "QSONaut first GUI frame reached"
            );
        }
        if !self.local_image_refresh_started {
            self.local_image_refresh_started = true;
            self.refresh_local_image_models();
        }
        // Zoom is layered on top of the OS DPI scale, so text, controls,
        // spacing, hit targets, and custom drawings stay in proportion.
        let target_zoom = self.gui_scale * self.os_dpi_adjustment;
        if (ctx.zoom_factor() - target_zoom).abs() > 0.001 {
            ctx.set_zoom_factor(target_zoom);
        }
        // Give background workers a handle so they can trigger repaints directly.
        let _ = self.repaint_ctx.get_or_init(|| ctx.clone());
        // Safety-net repaint in case no worker data arrives for a long time.
        ctx.request_repaint_after(Duration::from_secs(1));

        if let Some(rx) = &self.acceleration_probe {
            match rx.try_recv() {
                Ok(report) => {
                    self.acceleration_report = report;
                    self.acceleration_probe = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => self.acceleration_probe = None,
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }

        if let Some(rx) = &self.device_scan {
            match rx.try_recv() {
                Ok(inventory) => {
                    self.device_scan = None;
                    self.apply_device_inventory(inventory);
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    warn!("Device inventory scan worker disconnected before delivering results");
                    self.device_scan = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }

        // Keep inactive tabs receiving/decoding; only their PTT path is gated.
        self.pump_parked_radio_sessions();

        let active_audio_finished = self
            .audio_worker_handle
            .as_ref()
            .is_some_and(std::thread::JoinHandle::is_finished);
        if active_audio_finished {
            if let Some(handle) = self.audio_worker_handle.take() {
                let _ = handle.join();
            }
            warn!(profile = %self.selected_profile_name, "Active profile audio worker stopped");
            if let Ok(mut state) = self.state.lock() {
                state.audio_spectrum_status = "STOPPED (audio worker failed)".to_string();
            }
        }
        let active_radio_finished = self
            .radio_worker_handle
            .as_ref()
            .is_some_and(std::thread::JoinHandle::is_finished);
        if active_radio_finished {
            if let Some(handle) = self.radio_worker_handle.take() {
                let _ = handle.join();
            }
            self.command_tx = None;
            warn!(profile = %self.selected_profile_name, "Active profile radio worker stopped");
            if let Ok(mut state) = self.state.lock() {
                state.radio_waterfall_status = "STOPPED (radio worker failed)".to_string();
            }
        }

        // Poll for the selected radio initialization result from background thread
        if !self.radio_init_attempted {
            if let Some(rx) = &self.radio_init_rx {
                match rx.try_recv() {
                    Ok(Some(radio)) => {
                        // Radio initialization succeeded; start the worker
                        self.radio_init_attempted = true;
                        let (tx, rx) = mpsc::channel::<GuiCommand>();
                        let display_port = self
                            .config
                            .radio
                            .serial_port
                            .clone()
                            .unwrap_or_else(|| "auto".to_string());
                        info!(
                            backend = %self.config.radio.backend,
                            model = %self.config.radio.model,
                            endpoint = %self.config.radio.endpoint,
                            port = %display_port,
                            baud = self.config.radio.baud_rate,
                            "Starting GUI radio worker (deferred initialization)"
                        );
                        let handle = workers::radio::spawn_radio_worker(
                            radio,
                            self.state.clone(),
                            self.radio_worker_stop.clone(),
                            self.swr_sweep_abort.clone(),
                            self.display_tuning.clone(),
                            rx,
                            self.repaint_ctx.clone(),
                            self.ptt_allowed.clone(),
                        );
                        self.command_tx = Some(tx);
                        self.radio_worker_handle = Some(handle);
                    }
                    Ok(None) => {
                        // Radio initialization failed
                        self.radio_init_attempted = true;
                        self.radio_init_rx = None;
                        // Radio initialization failure is isolated to this
                        // profile. Its audio worker remains independent.
                        {
                            let mut s = self.state.lock().expect("ui state lock poisoned");
                            s.radio_waterfall_status =
                                "UNAVAILABLE (connection failed)".to_string();
                            s.last_error = Some(format!(
                                "Failed to initialize radio backend '{}' (model '{}', endpoint '{}', serial port '{}')",
                                self.config.radio.backend,
                                self.config.radio.model,
                                self.config.radio.endpoint,
                                self.config.radio.serial_port.as_deref().unwrap_or("auto"),
                            ));
                        }
                        warn!(profile = %self.selected_profile_name, "Radio initialization failed; profile runtime stopped");
                    }
                    Err(mpsc::TryRecvError::Disconnected) => {
                        // Thread panicked or dropped
                        self.radio_init_attempted = true;
                        self.radio_init_rx = None;
                        // Radio initialization failure is isolated to this
                        // profile. Its audio worker remains independent.
                        {
                            let mut s = self.state.lock().expect("ui state lock poisoned");
                            s.radio_waterfall_status =
                                "UNAVAILABLE (init thread crashed)".to_string();
                        }
                        warn!(profile = %self.selected_profile_name, "Radio initialization thread failed; profile runtime stopped");
                    }
                    Err(mpsc::TryRecvError::Empty) => {
                        // Still initializing...
                    }
                }
            }
        }

        // Drain FT8 decodes from the shared pending queue into app-local log.
        let (new_decodes, latest_decode_period) = {
            let mut s = self.state.lock().expect("ui state lock poisoned");
            s.workspace_mode = self.workspace_mode;
            s.fst4_submode = self.fst4_submode;
            s.ft8_deep_decode = self.ft8_deep_decode;
            s.ft4_deep_decode = self.ft4_deep_decode;
            s.selected_audio_hz = if self.workspace_mode == WorkspaceMode::Cw {
                u32::from(self.cw_tone_hz)
            } else {
                self.rx_tone_hz
            };
            s.sstv_tuning_offset_hz = self.sstv_tuning_offset_hz;
            s.sstv_auto_target = self.sstv_auto_target;
            s.cw_wpm = self.cw_wpm;
            s.compute_backend = self.acceleration_report.active;
            s.radio_spectrum_desired = self.civ_spectrum_on;
            s.radio_scope_contrast = self.radio_scope_contrast;
            s.radio_scope_span_code = self.radio_scope_span_code;
            s.radio_scope_vbw_wide = self.radio_scope_vbw_wide;
            s.radio_scope_hold = self.radio_scope_hold;
            s.radio_scope_reference_tenths_db = self.radio_scope_reference_tenths_db;
            s.radio_scope_view = self.radio_scope_view;
            (std::mem::take(&mut s.ft8_pending), s.ft8_last_decode_period)
        };
        let completed_decode_period =
            latest_decode_period.filter(|period| self.ft8_seen_decode_period != Some(*period));
        if completed_decode_period.is_some() {
            self.ft8_seen_decode_period = completed_decode_period;
        }
        self.process_ft8_tx_pipeline();
        self.process_native_digital_tx_pipeline();
        self.pump_server_automation_events();
        self.pump_automation_events();
        self.pump_hamdb_profile_lookup();
        self.pump_pota_spots();
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
        let completed_ft4_period =
            latest_ft4_period.filter(|period| self.ft4_seen_decode_period != Some(*period));
        if completed_ft4_period.is_some() {
            self.ft4_seen_decode_period = completed_ft4_period;
        }
        self.handle_ft4_decodes(&ft4_decodes, completed_ft4_period);
        for mode in [
            WorkspaceMode::Fst4,
            WorkspaceMode::Jt9,
            WorkspaceMode::Jt65,
            WorkspaceMode::Q65,
        ] {
            let native_decodes = {
                let shared = self.state.lock().expect("ui state lock poisoned");
                shared
                    .digital_decodes
                    .iter()
                    .filter(|entry| entry.mode == mode)
                    .cloned()
                    .collect::<Vec<_>>()
            };
            self.handle_native_sequence(mode, &native_decodes, None);
        }
        let max_entries = self.ft8_max_log_entries.max(80);
        self.handle_ft8_decodes(&new_decodes, completed_decode_period);
        let removed = append_ft8_log_entries(&mut self.ft8_log, &new_decodes, max_entries);
        if removed > 0 {
            self.ft8_log.drain(..removed);
            if let Some(sel) = self.ft8_selected {
                self.ft8_selected = sel.checked_sub(removed);
            }
        }
        if !new_decodes.is_empty() {
            info!(
                received = new_decodes.len(),
                visible_log_entries = self.ft8_log.len(),
                "FT8 decodes transferred to GUI log"
            );
        }
        if removed > 0 {
            debug!(removed, max_entries, "FT8 GUI log bounded");
        }

        let snapshot = self.state.lock().expect("ui state lock poisoned").clone();
        self.emit_radio_state_hook_if_changed(&snapshot);
        self.publish_server_presence(&snapshot);

        // Keep the detailed meter panel visible throughout TX and briefly
        // afterward so the operator can see the final transmit readings.
        let now = Instant::now();
        if snapshot.ptt_on {
            self.show_meter_panel = true;
            self.meter_panel_close_deadline = None;
        } else if self.meter_panel_was_tx {
            self.meter_panel_close_deadline = Some(now + Duration::from_secs(2));
        }
        self.meter_panel_was_tx = snapshot.ptt_on;
        if self
            .meter_panel_close_deadline
            .is_some_and(|deadline| now >= deadline)
        {
            self.show_meter_panel = false;
            self.meter_panel_close_deadline = None;
        }

        // Use a compact, stable rail so the responsive control rows do not
        // inherit stale panel height or consume the waterfall's workspace.
        egui::TopBottomPanel::top("header_control_deck")
            .resizable(false)
            .show(ctx, |ui| {
                ui.scope(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(7.0, 2.0);
                    ui.spacing_mut().button_padding = egui::vec2(7.0, 3.0);
                    ui.style_mut().override_font_id = Some(egui::FontId::proportional(13.0));
                    let visuals = ui.visuals_mut();
                    visuals.widgets.inactive.corner_radius = egui::CornerRadius::same(8);
                    visuals.widgets.hovered.corner_radius = egui::CornerRadius::same(8);
                    visuals.widgets.active.corner_radius = egui::CornerRadius::same(8);
                    let radio_tabs = self.available_profiles.clone();
                    let mut activate_tab = None;
                    let mut worker_action = None;
                    let mut open_config = None;
                    let mut commit_new_profile = false;
                    let mut section_divider_x = None;
                    let header_row = ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 3.0;
                        ui.allocate_ui_with_layout(
                            egui::vec2(360.0, 116.0),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                ui.horizontal(|ui| self.draw_header_branding(ui));
                                self.draw_header_identity_and_activity(ui);
                            },
                        );
                        let (divider_marker, _) =
                            ui.allocate_exact_size(egui::vec2(1.0, 1.0), egui::Sense::hover());
                        section_divider_x = Some(divider_marker.center().x);
                        ui.vertical(|ui| {
                            if !radio_tabs.is_empty() {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 3.0;
                            for name in radio_tabs {
                                let active = name == self.selected_profile_name;
                                let (indicator, indicator_color, identity, status) =
                                    self.radio_tab_status(&name, &snapshot);
                                let tab_fill = if active {
                                    Color32::from_rgb(24, 92, 116)
                                } else {
                                    Color32::from_rgb(48, 48, 52)
                                };
                                let tab_stroke = if active {
                                    Color32::from_rgb(88, 205, 235)
                                } else {
                                    indicator_color.linear_multiply(0.65)
                                };
                                egui::Frame::new()
                                    .fill(tab_fill)
                                    .stroke(egui::Stroke::new(1.0_f32, tab_stroke))
                                    .corner_radius(egui::CornerRadius::same(8))
                                    .inner_margin(egui::Margin::symmetric(5, 3))
                                    .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                    let text = RichText::new(format!("{indicator} {name} · {identity}"))
                                        .small()
                                        .strong()
                                        .color(if active {
                                            Color32::from_rgb(110, 220, 255)
                                        } else {
                                            indicator_color
                                        });
                                    if ui
                                        .add(egui::Button::selectable(active, text).small())
                                        .on_hover_text(format!("{name}: {status}"))
                                        .clicked()
                                    {
                                        activate_tab = Some(name.clone());
                                    }
                                    let radio_running = if active {
                                        self.command_tx.is_some()
                                            || (!self.radio_init_attempted
                                                && self.radio_init_rx.is_some())
                                    } else {
                                        self.parked_radio_sessions
                                            .get(&name)
                                            .is_some_and(|session| {
                                                session.command_tx.is_some()
                                                    || (!session.init_attempted
                                                        && session.init_rx.is_some())
                                            })
                                    };
                                    let audio_running = if active {
                                        self.audio_worker_handle.is_some()
                                    } else {
                                        self.parked_radio_sessions
                                            .get(&name)
                                            .is_some_and(|session| {
                                                session.audio_worker_handle.is_some()
                                            })
                                    };
                                    let workers_running = radio_running && audio_running;
                                    let worker_label = if workers_running { "■" } else { "▶" };
                                    let worker_hint = if workers_running {
                                        "Stop this profile's radio and audio workers"
                                    } else {
                                        "Start this profile's radio and audio workers"
                                    };
                                    let worker_button = egui::Button::new(worker_label)
                                    .small()
                                    .fill(if workers_running {
                                        Color32::from_rgb(126, 25, 39)
                                    } else {
                                        Color32::from_rgb(30, 105, 74)
                                    });
                                    if ui
                                        .add(worker_button)
                                        .on_hover_text(worker_hint)
                                        .clicked()
                                    {
                                        worker_action = Some((name.clone(), !workers_running));
                                    }
                                    let settings_response = ui
                                        .small_button("⚙")
                                        .on_hover_text("Open this radio tab's configuration");
                                    if settings_response.clicked() {
                                        open_config = Some((
                                            name.clone(),
                                            settings_response.rect.left_bottom()
                                                + egui::vec2(0.0, 4.0),
                                        ));
                                    }
                                    });
                                });
                            }
                            if self.new_profile_tab_editing {
                                let response = ui.add(
                                    egui::TextEdit::singleline(&mut self.new_profile_name)
                                        .desired_width(150.0)
                                        .hint_text("New profile name"),
                                );
                                commit_new_profile = response.lost_focus()
                                    || ui.input(|input| input.key_pressed(egui::Key::Enter));
                            } else if ui
                                .small_button("+")
                                .on_hover_text("Create a new radio profile")
                                .clicked()
                            {
                                self.new_profile_name.clear();
                                self.new_profile_tab_editing = true;
                            }
                        });
                        if let Some((name, running)) = worker_action {
                            self.set_tab_workers_running(&name, running);
                        } else if let Some(name) = activate_tab {
                            self.switch_radio_tab(&name);
                        } else if let Some((name, anchor)) = open_config {
                            if name != self.selected_profile_name {
                                self.switch_radio_tab(&name);
                            }
                            self.profile_drawer_anchor = Some(anchor);
                            self.show_profile_drawer = true;
                        }
                    } else {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("RADIOS")
                                    .small()
                                    .strong()
                                    .color(Color32::GRAY),
                            );
                        });
                    }
                    let row_divider_y = ui.cursor().top();
                    let row_divider_left = ui.max_rect().left();
                    ui.painter().line_segment(
                        [
                            egui::pos2(row_divider_left, row_divider_y),
                            egui::pos2(ui.max_rect().right(), row_divider_y),
                        ],
                        ui.visuals().widgets.noninteractive.bg_stroke,
                    );
                    ui.add_space(1.0);
                    ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 3.0;
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                    let frequency = snapshot
                        .frequency_hz
                        .map(|hz| format!("{:.6} MHz", hz as f64 / 1_000_000.0))
                        .unwrap_or_else(|| "RADIO OFFLINE".to_string());
                    ui.label(
                        RichText::new(frequency)
                            .monospace()
                            .strong()
                            .size(25.0)
                            .color(if snapshot.frequency_hz.is_some() {
                                Color32::from_rgb(120, 225, 255)
                            } else {
                                theme_warning(ui)
                            }),
                    );
                    let supports_vfo = snapshot
                        .supported_controls
                        .contains(&ControlId::Vfo);
                    let vfo = snapshot.active_vfo.min(1);
                    let vfo_label = if vfo == 0 { "VFO A" } else { "VFO B" };
                    if ui
                        .add_enabled(
                            supports_vfo && snapshot.radio_power_on == Some(true),
                            egui::Button::new(
                                RichText::new(vfo_label)
                                    .monospace()
                                    .strong()
                                    .color(if vfo == 0 {
                                        Color32::from_rgb(130, 220, 255)
                                    } else {
                                        Color32::from_rgb(255, 190, 105)
                                    }),
                            ),
                        )
                        .on_hover_text("Toggle the active radio VFO")
                        .clicked()
                    {
                        self.send_command(GuiCommand::SetControl(
                            ControlId::Vfo,
                            // Rigwright's CI-V VFO selector is a write-only
                            // command and its current HAL contract is U8.
                            ControlValue::U8(1 - vfo),
                        ));
                    }
                    let radio_profile = self
                        .config
                        .radio
                        .enabled
                        .then(|| {
                            native_radio_profile(
                                &self.config.radio.backend,
                                &self.config.radio.model,
                            )
                        })
                        .flatten();
                    let current_band = snapshot
                        .frequency_hz
                        .map(band_for_frequency)
                        .filter(|band| !band.is_empty())
                        .unwrap_or("—");
                    let band_menu = ui.menu_button(
                        RichText::new(current_band)
                            .strong()
                            .color(Color32::from_rgb(220, 190, 100)),
                        |ui| {
                            ui.label(RichText::new("AVAILABLE BANDS").strong());
                            ui.separator();
                            let current_hz = snapshot.frequency_hz.unwrap_or(0);
                            let activity_bands = self.activity.profile().bands.labels();
                            let mut visible_bands = 0;
                            ui.horizontal_wrapped(|ui| {
                                for (label, frequency_hz) in band_picker_plan(self.workspace_mode) {
                                    if !radio_supports_band(radio_profile, label) {
                                        continue;
                                    }
                                    visible_bands += 1;
                                    let selected = current_hz.abs_diff(frequency_hz) < 200_000;
                                    // Radio capability controls visibility; the
                                    // operating activity controls availability.
                                    // A mode's focused calling-frequency plan is
                                    // not a band restriction. Never hide or
                                    // disable a radio-supported band merely
                                    // because the current mode has no preset.
                                    let available_for_activity = activity_bands.contains(&label);
                                    if styled_selection_button(
                                        ui,
                                        label,
                                        selected,
                                        Color32::from_rgb(220, 190, 100),
                                        available_for_activity,
                                    )
                                    .on_hover_text(if available_for_activity {
                                        format!("{:.6} MHz", frequency_hz as f64 / 1_000_000.0)
                                    } else {
                                        format!(
                                            "{:.6} MHz · unavailable for {}",
                                            frequency_hz as f64 / 1_000_000.0,
                                            self.activity.label()
                                        )
                                    })
                                    .clicked()
                                    {
                                        self.send_command(GuiCommand::ApplyWorkspace {
                                            mode: self.workspace_mode,
                                            frequency_hz,
                                        });
                                        ui.close();
                                    }
                                }
                            });
                            if visible_bands == 0 {
                                ui.label(RichText::new("No bands available for this mode").weak());
                            }
                        },
                    );
                    band_menu.response.on_hover_text("Select the operating band");
                    self.draw_rf_power_control(ui, &snapshot);
                    ui.separator();
                    let current_radio_mode = radio_mode_label(&snapshot.mode, snapshot.data_mode);
                    let mode_menu = ui.menu_button(
                        RichText::new(current_radio_mode.clone())
                            .monospace()
                            .strong()
                            .color(Color32::WHITE),
                        |ui| {
                            ui.label(RichText::new("RADIO MODE").strong());
                            ui.separator();
                            for (mode, label) in [
                                (Mode::Usb, "USB"),
                                (Mode::Lsb, "LSB"),
                                (Mode::Cw, "CW"),
                                (Mode::Data, "USB-D"),
                                (Mode::Am, "AM"),
                                (Mode::Fm, "FM"),
                                (Mode::Rtty, "RTTY"),
                                (Mode::CwReverse, "CW-R"),
                                (Mode::RttyReverse, "RTTY-R"),
                            ] {
                                if styled_selection_button(
                                    ui,
                                    label,
                                    current_radio_mode == label,
                                    Color32::from_rgb(190, 215, 235),
                                    true,
                                )
                                .clicked()
                                {
                                    self.send_command(GuiCommand::SetRadioMode(mode));
                                    ui.close();
                                }
                            }
                        },
                    );
                    mode_menu.response.on_hover_text("Select the radio operating mode");
                    let supports_filter = snapshot.supported_controls.contains(&ControlId::Filter);
                    let filter_label = snapshot
                        .filter
                        .map(|filter| format!("FIL{filter}"))
                        .unwrap_or_else(|| "FIL?".to_string());
                    let filter_menu = ui.menu_button(
                        RichText::new(filter_label).monospace().color(Color32::GRAY),
                        |ui| {
                            ui.label(RichText::new("FILTER").strong());
                            ui.separator();
                            for filter in 1_u8..=3 {
                                if styled_selection_button(
                                    ui,
                                    &format!("FIL{filter}"),
                                    snapshot.filter == Some(filter),
                                    Color32::from_rgb(160, 205, 230),
                                    supports_filter,
                                )
                                .clicked()
                                {
                                    self.send_command(GuiCommand::SetFilter(filter));
                                    ui.close();
                                }
                            }
                        },
                    );
                    filter_menu.response.on_hover_text("Select the radio IF filter");
                    self.draw_banner_radio_controls(ui, &snapshot);
                    });
                    ui.horizontal(|ui| {
                        self.draw_meter_display(ui, &snapshot);
                    let radio_ready = snapshot.radio_power_on == Some(true)
                        && !snapshot.radio_power_command_pending;
                    let supports_control = |id| snapshot.supported_controls.contains(&id);
                        ui.scope(|ui| {
                        ui.spacing_mut().button_padding.y = 4.0;
                        ui.horizontal(|ui| {
                            ui.separator();
                            ui.horizontal(|ui| {
                        let speaker_color = if radio_ready {
                            Color32::LIGHT_BLUE
                        } else {
                            Color32::GRAY
                        };
                        let (speaker_rect, speaker_response) = ui.allocate_exact_size(
                            egui::vec2(22.0, 22.0),
                            egui::Sense::hover(),
                        );
                        draw_speaker_icon(&ui.painter_at(speaker_rect), speaker_rect, speaker_color);
                        speaker_response.on_hover_text("RX/TX volume controls");
                            });
                            for (label, id, value, tooltip) in [
                        ("AF", ControlId::AfGain, snapshot.af_gain, "Audio receive gain"),
                        ("RF", ControlId::RfGain, snapshot.rf_gain, "RF receive gain"),
                        ("SQ", ControlId::Squelch, snapshot.squelch, "Squelch threshold"),
                        ("TX", ControlId::RfPower, snapshot.rf_power, "RF transmit power"),
                            ] {
                        let color = if supports_control(id) && radio_ready {
                            Color32::LIGHT_BLUE
                        } else {
                            Color32::GRAY
                        };
                        ui.menu_button(
                            RichText::new(label).size(12.0).monospace().color(color),
                            |ui| {
                                let mut percent = value
                                    .map(|raw| f32::from(raw) * 100.0 / 255.0)
                                    .unwrap_or_default();
                                let response = ui.add_enabled(
                                    supports_control(id) && radio_ready,
                                    egui::Slider::new(&mut percent, 0.0..=100.0)
                                        .vertical()
                                        .show_value(false),
                                );
                                ui.label(format!("{percent:.0}%"));
                                if response.changed() && response.drag_stopped() {
                                    let normalized = (percent.clamp(0.0, 100.0) * 255.0 / 100.0)
                                        .round()
                                        as u8;
                                    self.send_command(GuiCommand::SetControl(
                                        id,
                                        ControlValue::U8(normalized),
                                    ));
                                }
                                response.on_hover_text(if !supports_control(id) {
                                    "This control is not supported by the loaded radio profile"
                                } else if !radio_ready {
                                    "Unavailable while the radio is offline or waking"
                                } else {
                                    tooltip
                                });
                            },
                        );
                            }
                        });
                        });
                    if supports_control(ControlId::Tuner) {
                        let tuner_color = if snapshot.tuner_status.is_some_and(|status| status.tuning) {
                            Color32::YELLOW
                        } else if snapshot.tuner_status.is_some_and(|status| status.enabled) {
                            Color32::LIGHT_GREEN
                        } else {
                            Color32::GRAY
                        };
                        ui.menu_button(RichText::new("TUNE").color(tuner_color), |ui| {
                            ui.label(if snapshot.tuner_status.is_some_and(|status| status.tuning) {
                                "Tuning in progress"
                            } else if snapshot.tuner_status.is_some_and(|status| status.enabled) {
                                "Tuner enabled"
                            } else {
                                "Tuner disabled"
                            });
                            ui.horizontal(|ui| {
                                let enabled = snapshot.tuner_status.is_some_and(|status| status.enabled);
                                if ui
                                    .add_enabled(
                                        radio_ready && !snapshot.swr_sweep_active,
                                        egui::Button::new(if enabled { "Disable" } else { "Enable" }),
                                    )
                                    .on_hover_text(if enabled {
                                        "Disable the radio's antenna tuner"
                                    } else {
                                        "Enable the radio's antenna tuner"
                                    })
                                    .clicked()
                                {
                                    self.send_command(GuiCommand::SetControl(
                                        ControlId::Tuner,
                                        ControlValue::Bool(!enabled),
                                    ));
                                    ui.close();
                                }
                                if ui
                                    .add_enabled(radio_ready && !snapshot.swr_sweep_active, egui::Button::new("Tune"))
                                    .on_hover_text("Start the radio's antenna-tuner cycle; this may transmit")
                                    .clicked()
                                {
                                    self.send_command(GuiCommand::StartTuner);
                                    ui.close();
                                }
                            });
                        }).response.on_hover_text("Enable the antenna tuner or start tuning");
                    }
                    if snapshot.supported_meters.contains(&MeterId::Swr) {
                        if let Some((band_start, band_stop, band_name)) =
                            band_edges_for_frequency(snapshot.frequency_hz)
                        {
                            let mut state = self.state.lock().expect("ui state lock poisoned");
                            if state.swr_sweep_band.as_deref() != Some(band_name) {
                                let width = band_stop.saturating_sub(band_start);
                                state.swr_sweep_start_hz = band_start;
                                state.swr_sweep_stop_hz = band_stop;
                                state.swr_sweep_step_hz = (width / 100).max(1_000);
                                state.swr_sweep_band = Some(band_name.to_string());
                            }
                        }
                        let swr_button = ui
                            .button(RichText::new("SWR").color(Color32::LIGHT_BLUE))
                            .on_hover_text("Read the SWR meter or scan the active band");
                        egui::Popup::menu(&swr_button)
                            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                            .show(|ui| {
                            ui.label("SWR meter and sweep");
                            if let Some((band_start, band_stop, band_name)) =
                                band_edges_for_frequency(snapshot.frequency_hz)
                            {
                                ui.label(format!(
                                    "Active band: {band_name} ({band_start}–{band_stop} Hz)"
                                ));
                            } else {
                                ui.label("Active band unavailable; enter a manual range");
                            }
                            ui.separator();
                            ui.label(format!(
                                "SWR: {}",
                                format_swr_display(&self.config.radio.model, snapshot.swr)
                            ));
                            ui.colored_label(
                                Color32::YELLOW,
                                "SWR sweep: RTTY carrier at approximately 30 W; TX is restored afterward.",
                            );
                            ui.horizontal(|ui| {
                                ui.label("Start");
                                ui.add(egui::DragValue::new(&mut self.state.lock().expect("ui state lock poisoned").swr_sweep_start_hz).speed(100.0));
                            });
                            ui.horizontal(|ui| {
                                ui.label("Stop");
                                ui.add(egui::DragValue::new(&mut self.state.lock().expect("ui state lock poisoned").swr_sweep_stop_hz).speed(100.0));
                            });
                            ui.horizontal(|ui| {
                                ui.label("Step");
                                ui.add(egui::DragValue::new(&mut self.state.lock().expect("ui state lock poisoned").swr_sweep_step_hz).speed(100.0));
                            });
                            ui.horizontal(|ui| {
                                ui.label("Interval ms");
                                ui.add(egui::DragValue::new(&mut self.state.lock().expect("ui state lock poisoned").swr_sweep_interval_ms).range(100..=10_000));
                            });
                            let sweep_enabled = radio_ready && !snapshot.swr_sweep_active;
                            if ui.add_enabled(sweep_enabled, egui::Button::new("Start TX SWR sweep")).clicked() {
                                self.swr_sweep_abort.store(false, Ordering::Relaxed);
                                let s = self.state.lock().expect("ui state lock poisoned");
                                self.send_command(GuiCommand::StartSwrSweep {
                                    start_hz: s.swr_sweep_start_hz,
                                    stop_hz: s.swr_sweep_stop_hz,
                                    step_hz: s.swr_sweep_step_hz,
                                    interval_ms: s.swr_sweep_interval_ms,
                                });
                            }
                            if snapshot.swr_sweep_active {
                                ui.spinner();
                                if ui.button("Stop sweep").clicked() {
                                    self.swr_sweep_abort.store(true, Ordering::Relaxed);
                                }
                            }
                            ui.label(&snapshot.swr_sweep_status);
                            let (rect, _) = ui.allocate_exact_size(egui::vec2(420.0, 180.0), egui::Sense::hover());
                            let painter = ui.painter_at(rect);
                            painter.rect_filled(rect, 2.0, Color32::from_gray(24));
                            let points = &snapshot.swr_sweep_points;
                            let chart_left = rect.left() + 42.0;
                            let chart_right = rect.right() - 8.0;
                            let chart_top = rect.top() + 10.0;
                            let chart_bottom = rect.bottom() - 22.0;
                            let chart_rect = egui::Rect::from_min_max(
                                egui::pos2(chart_left, chart_top),
                                egui::pos2(chart_right, chart_bottom),
                            );
                            let icom_swr_chart = native_radio_profile(
                                "native",
                                &self.config.radio.model,
                            )
                            .and_then(|profile| {
                                profile.calibrated_meter_value(MeterId::Swr, 0)
                            })
                            .is_some();
                            let chart_axes = if icom_swr_chart {
                                [(1.0_f32, "1.0:1"), (1.5, "1.5:1"), (2.0, "2.0:1"), (2.5, "2.5:1"), (3.0, "3.0:1")]
                            } else {
                                [(0.0_f32, "0%"), (25.0, "25%"), (50.0, "50%"), (75.0, "75%"), (100.0, "100%")]
                            };
                            let chart_min = chart_axes[0].0;
                            let chart_max = chart_axes[4].0;
                            for (value, label) in chart_axes {
                                let y = chart_bottom
                                    - ((value - chart_min) / (chart_max - chart_min)) * chart_rect.height();
                                painter.line_segment(
                                    [egui::pos2(chart_left, y), egui::pos2(chart_right, y)],
                                    egui::Stroke::new(1.0_f32, Color32::from_gray(65)),
                                );
                                painter.text(
                                    egui::pos2(rect.left() + 4.0, y - 7.0),
                                    egui::Align2::LEFT_TOP,
                                    label,
                                    egui::FontId::monospace(10.0),
                                    Color32::GRAY,
                                );
                            }
                            if points.len() > 1 {
                                let min_hz = points.first().map(|point| point.0).unwrap_or(0) as f32;
                                let max_hz = points.last().map(|point| point.0).unwrap_or(1).max(points.first().map(|point| point.0).unwrap_or(0) + 1) as f32;
                                let polyline: Vec<_> = points.iter().map(|(hz, raw)| {
                                    let x = chart_left + ((*hz as f32 - min_hz) / (max_hz - min_hz)) * chart_rect.width();
                                    let value = swr_chart_value(&self.config.radio.model, *raw)
                                        .clamp(chart_min, chart_max);
                                    let y = chart_bottom
                                        - ((value - chart_min) / (chart_max - chart_min)) * chart_rect.height();
                                    egui::pos2(x, y)
                                }).collect();
                                painter.add(egui::Shape::line(polyline.clone(), egui::Stroke::new(2.0_f32, Color32::LIGHT_GREEN)));
                                for point in polyline {
                                    painter.circle_filled(point, 2.5, Color32::WHITE);
                                }
                                painter.text(
                                    egui::pos2(chart_left, chart_bottom + 4.0),
                                    egui::Align2::LEFT_TOP,
                                    format!("{}", points.first().map(|point| point.0).unwrap_or(0)),
                                    egui::FontId::monospace(10.0),
                                    Color32::GRAY,
                                );
                                painter.text(
                                    egui::pos2(chart_right, chart_bottom + 4.0),
                                    egui::Align2::RIGHT_TOP,
                                    format!("{}", points.last().map(|point| point.0).unwrap_or(0)),
                                    egui::FontId::monospace(10.0),
                                    Color32::GRAY,
                                );
                            } else if points.len() == 1 {
                                painter.circle_filled(chart_rect.center(), 3.0, Color32::WHITE);
                            }
                            });
                    }
                    if supports_control(ControlId::NoiseBlanker) {
                        let color = match snapshot.noise_blank {
                            Some(true) => Color32::LIGHT_GREEN,
                            Some(false) => Color32::GRAY,
                            None => Color32::DARK_GRAY,
                        };
                        if ui
                            .add_enabled(
                                radio_ready,
                                egui::Button::new(RichText::new("NB").color(Color32::WHITE))
                                    .small()
                                    .fill(color),
                            )
                            .on_hover_text("Toggle noise blanker")
                            .clicked()
                        {
                            self.send_command(GuiCommand::SetControl(
                                ControlId::NoiseBlanker,
                                ControlValue::Bool(snapshot.noise_blank != Some(true)),
                            ));
                        }
                    }
                    if supports_control(ControlId::NoiseReduction) {
                        let color = match snapshot.noise_reduction {
                            Some(true) => Color32::LIGHT_GREEN,
                            Some(false) => Color32::GRAY,
                            None => Color32::DARK_GRAY,
                        };
                        if ui
                            .add_enabled(
                                radio_ready,
                                egui::Button::new(RichText::new("NR").color(Color32::WHITE))
                                    .small()
                                    .fill(color),
                            )
                            .on_hover_text("Toggle noise reduction")
                            .clicked()
                        {
                            self.send_command(GuiCommand::SetControl(
                                ControlId::NoiseReduction,
                                ControlValue::Bool(snapshot.noise_reduction != Some(true)),
                            ));
                        }
                    }
                    if supports_control(ControlId::NoiseReductionLevel) {
                        let max_level = native_radio_profile("native", &self.config.radio.model)
                            .and_then(|profile| profile.control_max(ControlId::NoiseReductionLevel))
                            .expect("supported NR level must have a profile bound");
                        ui.menu_button(
                            RichText::new("NRL").color(if snapshot.noise_reduction_level.is_some() {
                                Color32::LIGHT_BLUE
                            } else {
                                Color32::DARK_GRAY
                            }),
                            |ui| {
                                ui.label("Noise reduction level");
                                let mut level = snapshot.noise_reduction_level.unwrap_or(8) as f32;
                                let response = ui.add(
                                    egui::Slider::new(&mut level, 1.0..=f32::from(max_level))
                                        .step_by(1.0)
                                        .show_value(true),
                                );
                                if response.changed() && response.drag_stopped() {
                                    self.send_command(GuiCommand::SetControl(
                                        ControlId::NoiseReductionLevel,
                                        ControlValue::U8(level.round() as u8),
                                    ));
                                }
                            },
                        )
                        .response
                        .on_hover_text("Set the Yaesu noise-reduction level");
                    }
                    if supports_control(ControlId::IpPlus) {
                        let color = match snapshot.ip_plus {
                            Some(true) => Color32::LIGHT_GREEN,
                            Some(false) => Color32::GRAY,
                            None => Color32::DARK_GRAY,
                        };
                        if ui
                            .add_enabled(
                                radio_ready,
                                egui::Button::new(RichText::new("IP+").color(Color32::WHITE))
                                    .small()
                                    .fill(color),
                            )
                            .on_hover_text("Toggle Icom IP Plus receiver optimization")
                            .clicked()
                        {
                            self.send_command(GuiCommand::SetControl(
                                ControlId::IpPlus,
                                ControlValue::Bool(snapshot.ip_plus != Some(true)),
                            ));
                        }
                    }
                    if supports_control(ControlId::Notch) {
                        let notch_label = if snapshot.notch_manual == Some(true) {
                            "MN"
                        } else if snapshot.notch_auto == Some(true) {
                            "AN"
                        } else {
                            "NT"
                        };
                        let notch_color = if snapshot.notch_auto == Some(true)
                            || snapshot.notch_manual == Some(true)
                        {
                            Color32::LIGHT_GREEN
                        } else {
                            Color32::GRAY
                        };
                        ui.menu_button(
                            RichText::new(notch_label).color(notch_color),
                            |ui| {
                                for (label, auto, manual) in [
                                    ("Off", false, false),
                                    ("Auto notch", true, false),
                                    ("Manual notch", false, true),
                                ] {
                                    if ui
                                        .selectable_label(
                                            snapshot.notch_auto == Some(auto)
                                                && snapshot.notch_manual == Some(manual),
                                            label,
                                        )
                                        .clicked()
                                    {
                                        self.send_command(GuiCommand::SetControl(
                                            ControlId::Notch,
                                            ControlValue::Bool(auto),
                                        ));
                                        self.send_command(GuiCommand::SetControl(
                                            ControlId::ManualNotch,
                                            ControlValue::Bool(manual),
                                        ));
                                        ui.close();
                                    }
                                }
                            },
                        )
                        .response
                        .on_hover_text("Select off, auto notch, or manual notch");
                    }
                    if supports_control(ControlId::Agc) {
                        let max_agc = native_radio_profile("native", &self.config.radio.model)
                            .and_then(|profile| profile.control_max(ControlId::Agc))
                            .expect("supported AGC must have a profile bound");
                        let color = if snapshot.agc.is_some() {
                            Color32::LIGHT_BLUE
                        } else {
                            Color32::DARK_GRAY
                        };
                        ui.menu_button(RichText::new("AGC").color(color), |ui| {
                            for value in 0_u8..=max_agc {
                                if ui
                                    .selectable_label(snapshot.agc == Some(value), format!("AGC {value}"))
                                    .clicked()
                                {
                                    self.send_command(GuiCommand::SetControl(
                                        ControlId::Agc,
                                        ControlValue::U8(value),
                                    ));
                                    ui.close();
                                }
                            }
                        })
                        .response
                        .on_hover_text("Select the automatic gain-control level");
                    }
                    if !snapshot.supported_meters.is_empty() {
                        ui.menu_button(RichText::new("MTR").color(Color32::LIGHT_BLUE), |ui| {
                            ui.label("Normalized meter levels");
                            for (label, id, value) in [
                                ("SIG", MeterId::Signal, snapshot.signal_meter),
                                ("PWR", MeterId::Power, snapshot.power_meter),
                                ("SWR", MeterId::Swr, snapshot.swr),
                                ("ALC", MeterId::Alc, snapshot.alc_meter),
                                ("COMP", MeterId::Compression, snapshot.compression_meter),
                                ("I", MeterId::Current, snapshot.current_meter),
                                ("V", MeterId::Voltage, snapshot.voltage_meter),
                                ("TEMP", MeterId::Temperature, snapshot.temperature_meter),
                            ] {
                                if snapshot.supported_meters.contains(&id) {
                                    ui.horizontal(|ui| {
                                        ui.label(label);
                                        let fraction = value.map_or(0.0, |raw| f32::from(raw) / 255.0);
                                        ui.add(
                                            egui::ProgressBar::new(fraction)
                                                .desired_width(120.0),
                                        );
                                    });
                                }
                            }
                        })
                        .response
                        .on_hover_text("Normalized vendor meter levels; physical units and SWR ratios remain vendor-specific");
                    }
                    if supports_control(ControlId::Preamp) {
                        let color = match snapshot.preamp {
                            Some(value) if value > 0 => Color32::LIGHT_GREEN,
                            Some(_) => Color32::GRAY,
                            None => Color32::DARK_GRAY,
                        };
                        let max_preamp = native_radio_profile("native", &self.config.radio.model)
                            .and_then(|profile| profile.control_max(ControlId::Preamp))
                            .unwrap_or(0);
                        ui.menu_button(RichText::new("PRE").color(color), |ui| {
                            for value in 0_u8..=max_preamp {
                                if ui
                                    .selectable_label(
                                        snapshot.preamp == Some(value),
                                        format!("PRE {value}"),
                                    )
                                    .clicked()
                                {
                                    self.send_command(GuiCommand::SetControl(
                                        ControlId::Preamp,
                                        ControlValue::U8(value),
                                    ));
                                    ui.close();
                                }
                            }
                        })
                        .response
                        .on_hover_text("Select the radio preamplifier level");
                    }
                    if supports_control(ControlId::Attenuator) {
                        let attenuator_values = native_radio_profile(
                            "native",
                            &self.config.radio.model,
                        )
                        .and_then(|profile| {
                            profile.supported_control_values(ControlId::Attenuator)
                        })
                        .unwrap_or(&[]);
                        let color = match snapshot.attenuator {
                            Some(value) if value > 0 => Color32::from_rgb(255, 190, 105),
                            Some(_) => Color32::GRAY,
                            None => Color32::DARK_GRAY,
                        };
                        ui.menu_button(RichText::new("ATT").color(color), |ui| {
                            for &value in attenuator_values {
                                if ui
                                    .selectable_label(
                                        snapshot.attenuator == Some(value),
                                        format!("ATT {value}"),
                                    )
                                    .clicked()
                                {
                                    self.send_command(GuiCommand::SetControl(
                                        ControlId::Attenuator,
                                        ControlValue::U8(value),
                                    ));
                                    ui.close();
                                }
                            }
                        })
                        .response
                        .on_hover_text("Select the radio input attenuator level");
                    }
                    ui.separator();
                    self.draw_waterfall_buttons(ui, &snapshot);
                    let monitor_label = "MON";
                    let monitor_color = if self.config.audio.monitor_enabled {
                        Color32::LIGHT_GREEN
                    } else {
                        Color32::GRAY
                    };
                    if ui
                        .selectable_label(
                            self.config.audio.monitor_enabled,
                            RichText::new(monitor_label).color(monitor_color),
                        )
                        .on_hover_text("Toggle captured RX audio to the selected monitor output")
                        .clicked()
                    {
                        self.config.audio.monitor_enabled = !self.config.audio.monitor_enabled;
                        self.profile_dirty = true;
                        self.persist_profile("Audio monitor saved to");
                        self.restart_audio();
                    }
                    let (speaker_rect, speaker_button) = ui.allocate_exact_size(
                        egui::vec2(28.0, 28.0),
                        egui::Sense::click(),
                    );
                    let speaker_color = if self.config.audio.monitor_enabled {
                        Color32::LIGHT_GREEN
                    } else {
                        Color32::GRAY
                    };
                    let speaker_painter = ui.painter_at(speaker_rect);
                    let speaker_center = speaker_rect.center();
                    speaker_painter.rect_filled(
                        egui::Rect::from_center_size(
                            egui::pos2(speaker_center.x - 5.0, speaker_center.y),
                            egui::vec2(4.0, 9.0),
                        ),
                        1.0,
                        speaker_color,
                    );
                    speaker_painter.add(egui::Shape::convex_polygon(
                        vec![
                            egui::pos2(speaker_center.x - 3.0, speaker_center.y - 5.0),
                            egui::pos2(speaker_center.x + 3.0, speaker_center.y - 9.0),
                            egui::pos2(speaker_center.x + 3.0, speaker_center.y + 9.0),
                            egui::pos2(speaker_center.x - 3.0, speaker_center.y + 5.0),
                        ],
                        speaker_color,
                        egui::Stroke::NONE,
                    ));
                    speaker_painter.line_segment(
                        [
                            egui::pos2(speaker_center.x + 6.0, speaker_center.y - 5.0),
                            egui::pos2(speaker_center.x + 9.0, speaker_center.y - 2.0),
                        ],
                        egui::Stroke::new(1.5_f32, speaker_color),
                    );
                    speaker_painter.line_segment(
                        [
                            egui::pos2(speaker_center.x + 6.0, speaker_center.y + 5.0),
                            egui::pos2(speaker_center.x + 9.0, speaker_center.y + 2.0),
                        ],
                        egui::Stroke::new(1.5_f32, speaker_color),
                    );
                    let speaker_button = speaker_button.on_hover_text("RX monitor volume");
                    egui::Popup::menu(&speaker_button).show(|ui| {
                        ui.horizontal(|ui| {
                            let mut volume = self.config.audio.monitor_volume.clamp(0.0, 2.0);
                            let response = ui.add(
                                egui::Slider::new(&mut volume, 0.0..=2.0)
                                    .vertical()
                                    .show_value(false),
                            );
                            ui.vertical(|ui| {
                                ui.label("RX");
                                ui.label(format!("{:.0}%", volume * 100.0));
                            });
                            if response.changed() {
                                self.config.audio.monitor_volume = volume;
                                self.monitor_volume
                                    .store(volume.to_bits(), Ordering::Relaxed);
                                self.profile_dirty = true;
                                self.persist_profile("RX monitor volume saved to");
                            }
                        });
                    });
                    });
                    });
                    });
                    });
                    });
                    if let Some(divider_x) = section_divider_x {
                        ui.painter().line_segment(
                            [
                                egui::pos2(divider_x, header_row.response.rect.top()),
                                egui::pos2(divider_x, header_row.response.rect.bottom()),
                            ],
                            ui.visuals().widgets.noninteractive.bg_stroke,
                        );
                    }
                    if commit_new_profile {
                        self.create_profile_from_tab_name();
                    }
                });
            });

        egui::Area::new(egui::Id::new("radio_profile_power_top_right"))
            .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-8.0, 6.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let active_profile = self.active_radio_profile_name().unwrap_or("None");
                    ui.label(
                        RichText::new(format!("RADIO · {active_profile}"))
                            .small()
                            .color(if active_profile == "None" {
                                Color32::GRAY
                            } else {
                                Color32::from_rgb(255, 201, 92)
                            }),
                    )
                    .on_hover_text(format!(
                        "Active radio profile\n{active_profile}\nThis profile owns the radio connection and its per-radio settings."
                    ));
                    self.draw_power_button(ui, &snapshot);
                });
            });

        let supports_radio_scope =
            native_radio_profile(&self.config.radio.backend, &self.config.radio.model)
                .is_some_and(|profile| profile.capabilities.spectrum);
        let radio_scope_visible = self.civ_spectrum_on
            && supports_radio_scope
            && !snapshot.radio_waterfall_status.starts_with("UNAVAILABLE");
        // Bottom panels are stacked in declaration order: the first one owns
        // the outermost bottom strip. The monitor is rendered in the remaining
        // central region below, so its height follows the window naturally.
        egui::TopBottomPanel::bottom("connection_status")
            .resizable(false)
            .exact_height(30.0)
            .show(ctx, |ui| self.draw_connection_status(ui, &snapshot));

        egui::TopBottomPanel::bottom("global_contact_log")
            .resizable(true)
            .show_separator_line(true)
            .default_height(260.0)
            .height_range(150.0..=420.0)
            .show(ctx, |ui| {
                // Keep the log contents inside the panel's exact rectangle. This
                // mirrors the waterfall deck and prevents the editor controls
                // from expanding the panel and leaving a black overflow area.
                let log_rect = ui.available_rect_before_wrap();
                ui.allocate_rect(log_rect, egui::Sense::hover());
                let mut log_ui = ui.new_child(
                    egui::UiBuilder::new()
                        .id_salt("global_contact_log_contents")
                        .max_rect(log_rect)
                        .layout(egui::Layout::top_down(egui::Align::Min)),
                );
                log_ui.set_clip_rect(log_rect);
                self.draw_contact_log(&mut log_ui, &snapshot);
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
                    ui.horizontal_wrapped(|ui| {
                        for (tab, icon, label, icon_color) in [
                            (
                                SignalPanelTab::Achievements,
                                "🏆",
                                "ACHIEVEMENTS",
                                Color32::from_rgb(255, 201, 92),
                            ),
                            (
                                SignalPanelTab::Station,
                                "📡",
                                "STATION",
                                Color32::from_rgb(120, 225, 255),
                            ),
                            (
                                SignalPanelTab::Contest,
                                "🏁",
                                "CONTEST",
                                Color32::from_rgb(255, 151, 72),
                            ),
                            (
                                SignalPanelTab::Reporting,
                                "📡",
                                "REPORTING",
                                Color32::from_rgb(132, 228, 255),
                            ),
                            (
                                SignalPanelTab::Settings,
                                "⚙",
                                "SETTINGS",
                                Color32::from_rgb(190, 190, 205),
                            ),
                            (
                                SignalPanelTab::Ai,
                                "",
                                "AI",
                                Color32::from_rgb(205, 150, 255),
                            ),
                            (
                                SignalPanelTab::Server,
                                "🌐",
                                "SERVER",
                                Color32::from_rgb(110, 220, 255),
                            ),
                            (
                                SignalPanelTab::RadioTuning,
                                "📻",
                                "RADIO TUNING",
                                Color32::from_rgb(255, 190, 105),
                            ),
                            (
                                SignalPanelTab::AppLog,
                                "📋",
                                "APP LOG",
                                Color32::from_rgb(180, 190, 205),
                            ),
                        ] {
                            let selected = self.signal_panel_tab == tab;
                            let compact_ai_spacing = tab == SignalPanelTab::Ai;
                            let previous_item_spacing = ui.spacing().item_spacing.x;
                            if compact_ai_spacing {
                                ui.spacing_mut().item_spacing.x = 2.0;
                            }
                            let icon_response = if tab == SignalPanelTab::Ai {
                                let (icon_rect, response) = ui.allocate_exact_size(
                                    egui::vec2(13.0, 18.0),
                                    egui::Sense::click(),
                                );
                                draw_ai_icon(ui.painter(), icon_rect, icon_color);
                                Some(response)
                            } else {
                                None
                            };
                            let tab_text = if icon.is_empty() {
                                label.to_string()
                            } else {
                                format!("{icon} {label}")
                            };
                            let text = if selected {
                                RichText::new(tab_text)
                                    .strong()
                                    .color(Color32::from_rgb(120, 225, 255))
                            } else {
                                RichText::new(tab_text).color(icon_color)
                            };
                            let text_clicked = ui.selectable_label(selected, text).clicked();
                            if compact_ai_spacing {
                                ui.spacing_mut().item_spacing.x = previous_item_spacing;
                            }
                            if text_clicked
                                || icon_response.is_some_and(|response| response.clicked())
                            {
                                self.signal_panel_tab = tab;
                            }
                        }
                    });
                    ui.separator();
                    if self.signal_panel_tab == SignalPanelTab::AppLog {
                        self.draw_app_log_panel(ui);
                    } else {
                        egui::ScrollArea::vertical()
                            .id_salt("signals_scroll")
                            .auto_shrink([false, false])
                            .show(ui, |ui| match self.signal_panel_tab {
                                SignalPanelTab::Achievements => {
                                    self.draw_hunter_panel(ui, &snapshot)
                                }
                                SignalPanelTab::Station => self.draw_station_panel(ui),
                                SignalPanelTab::Contest => self.draw_contest_panel(ui),
                                SignalPanelTab::Reporting => self.draw_reporting_panel(ui),
                                SignalPanelTab::Settings => {
                                    self.draw_application_settings_panel(ui)
                                }
                                SignalPanelTab::Ai => self.draw_ai_panel(ui),
                                SignalPanelTab::Server => self.draw_server_panel(ui),
                                SignalPanelTab::RadioTuning => {
                                    self.draw_radio_tuning_panel(ui, &snapshot)
                                }
                                SignalPanelTab::AppLog => {
                                    unreachable!("app log has its own scroll area")
                                }
                            });
                    }
                });
        }

        if self.show_profile_drawer {
            let mut drawer_open = true;
            let drawer_anchor = self.profile_drawer_anchor.take();
            let mut drawer = egui::Window::new("⚙ Profile management")
                .open(&mut drawer_open)
                .default_width(460.0)
                .default_height(560.0)
                .min_width(360.0)
                .min_height(260.0)
                .max_width(560.0)
                .max_height(760.0)
                .resizable(true)
                .movable(true);
            if let Some(anchor) = drawer_anchor {
                drawer = drawer.current_pos(anchor);
            }
            drawer.show(ctx, |ui| {
                ui.label(
                    RichText::new(format!(
                        "{} · {}",
                        self.selected_profile_name,
                        if self.profile_dirty {
                            "unsaved changes"
                        } else {
                            "saved"
                        }
                    ))
                    .small()
                    .color(if self.profile_dirty {
                        theme_warning(ui)
                    } else {
                        Color32::GRAY
                    }),
                );
                ui.separator();
                ui.horizontal(|ui| {
                    for (tab, label) in [
                        (ProfileDrawerTab::Profile, "PROFILE"),
                        (ProfileDrawerTab::Radio, "RADIO"),
                        (ProfileDrawerTab::Tuning, "TUNING"),
                        (ProfileDrawerTab::DigitalTiming, "DIGITAL TIMING"),
                        (ProfileDrawerTab::Monitoring, "MONITORING"),
                    ] {
                        if ui
                            .selectable_label(self.profile_drawer_tab == tab, label)
                            .clicked()
                        {
                            self.profile_drawer_tab = tab;
                        }
                    }
                });
                ui.separator();
                egui::ScrollArea::vertical()
                    .id_salt("profile_management_drawer")
                    .auto_shrink([false, false])
                    .show(ui, |ui| match self.profile_drawer_tab {
                        ProfileDrawerTab::Profile => self.draw_profile_panel(ui),
                        ProfileDrawerTab::Radio => self.draw_radio_profile_settings(ui),
                        ProfileDrawerTab::Tuning => self.draw_radio_profile_assignments(ui),
                        ProfileDrawerTab::DigitalTiming => self.draw_digital_timing_settings(ui),
                        ProfileDrawerTab::Monitoring => self.draw_monitoring_settings(ui),
                    });
            });
            if !drawer_open {
                self.show_profile_drawer = false;
            }
        }

        if self.radio_faq_window_open || self.radio_guide_window_open {
            let help = help_for_model(&self.radio_help_window_model);
            let mut faq_open = self.radio_faq_window_open;
            let (faq_title, faq_document) = match self.radio_faq_document {
                RadioHelpDocument::Audio => ("Audio FAQ", AUDIO_FAQ),
                RadioHelpDocument::Manufacturer => ("Manufacturer FAQ", help.manufacturer_faq),
                RadioHelpDocument::Model => ("Model FAQ", help.model_faq),
            };
            egui::Window::new(format!("{} — {}", help.title, faq_title))
                .open(&mut faq_open)
                .default_width(620.0)
                .default_height(560.0)
                .min_width(360.0)
                .min_height(240.0)
                .resizable(true)
                .movable(true)
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            render_document(ui, faq_document);
                        });
                });
            self.radio_faq_window_open = faq_open;

            let mut guide_open = self.radio_guide_window_open;
            let (guide_title, guide_document) = match self.radio_guide_document {
                RadioHelpDocument::Audio => ("Audio FAQ", AUDIO_FAQ),
                RadioHelpDocument::Manufacturer => ("Manufacturer Guide", help.manufacturer_guide),
                RadioHelpDocument::Model => ("Model Guide", help.model_guide),
            };
            egui::Window::new(format!("{} — {}", help.title, guide_title))
                .open(&mut guide_open)
                .default_width(620.0)
                .default_height(560.0)
                .min_width(360.0)
                .min_height(240.0)
                .resizable(true)
                .movable(true)
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            render_document(ui, guide_document);
                        });
                });
            self.radio_guide_window_open = guide_open;
        }

        let monitor_has_data = !snapshot.audio_waterfall_rows.is_empty()
            || (radio_scope_visible && !snapshot.radio_waterfall_rows.is_empty());
        if monitor_has_data {
            // The waterfall deck is a real user-resizable panel. Its height
            // must not track incoming rows or silently change the workspace
            // layout while a radio is running.
            let resize_id = egui::Id::new("waterfall_deck").with("__resize");
            let resize_in_progress = ctx
                .read_response(resize_id)
                .is_some_and(|response| response.dragged());
            let mut waterfall_panel = egui::TopBottomPanel::top("waterfall_deck")
                .resizable(true)
                .default_height(self.waterfall_deck_height)
                .height_range(170.0..=ctx.content_rect().height().max(240.0) * 0.75)
                .show_separator_line(true);
            // egui keeps its own panel size between frames. Reassert the
            // application's last chosen height whenever the resize handle is
            // idle, otherwise a stale/clamped panel state can feed a smaller
            // rectangle back to the waterfall after an empty-input transition.
            if !resize_in_progress {
                waterfall_panel = waterfall_panel.min_height(self.waterfall_deck_height);
            }
            let waterfall_panel = waterfall_panel.show(ctx, |ui| {
                if radio_scope_visible {
                    let total_width = ui.available_width();
                    let radio_default_width = total_width * 0.5;
                    let radio_max_width = (total_width - 260.0).max(280.0);
                    egui::SidePanel::left("radio_waterfall_split")
                        .resizable(true)
                        .default_width(radio_default_width)
                        .width_range(280.0..=radio_max_width)
                        .show_inside(ui, |ui| {
                            self.draw_radio_waterfall(ui, ctx, &snapshot);
                        });
                    self.draw_audio_waterfall(ui, ctx, &snapshot);
                } else {
                    self.draw_audio_waterfall(ui, ctx, &snapshot);
                }
            });
            // Only accept a new height while the panel resize handle is being
            // dragged. The panel response also reflects layout constraints, so
            // copying its height every frame lets changing/empty waterfall
            // content overwrite the user's chosen height and creates a
            // shrink-to-minimum feedback loop.
            if resize_in_progress {
                let actual_height = waterfall_panel.response.rect.height();
                if actual_height.is_finite()
                    && (actual_height - self.waterfall_deck_height).abs() > 0.5
                {
                    self.waterfall_deck_height = actual_height.clamp(170.0, 560.0);
                    self.profile_dirty = true;
                }
            }
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            self.draw_bounded_workspace(ui, ctx, &snapshot);
        });
    }
}

impl Drop for QsonautGuiApp {
    fn drop(&mut self) {
        let shutdown_started = Instant::now();
        let parked_profile_count = self.parked_radio_sessions.len();
        self.force_stop_tx();
        self.stop_native_digital_tx();
        self.persist_profile("Saved on exit");
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
            let _ = handle.join();
        }
        for (_, session) in std::mem::take(&mut self.parked_radio_sessions) {
            join_radio_session(session);
        }
        if let Some(handle) = self.audio_worker_handle.take() {
            let _ = handle.join();
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

fn effective_visual_profile(tuning: &DisplayTuning, mode: &str, radio: bool) -> (u64, u8) {
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
        // A dense, fast waterfall makes short digital signals easier to tune
        // and preserves the radio's native scope cadence.
        (50, 0)
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

fn scope_projection_for_mode(mode: &str) -> ScopeProjection {
    let mode = mode.to_ascii_uppercase();
    if mode.contains("LSB") {
        ScopeProjection::LowerSideband
    } else if mode.contains("USB") || mode == "DATA" || mode.contains("DIG") {
        ScopeProjection::UpperSideband
    } else {
        ScopeProjection::Full
    }
}

fn sideband_scope_edges(
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

fn scope_span_label(span_code: u8) -> &'static str {
    match span_code.min(7) {
        0 => "±2.5 kHz",
        1 => "±5 kHz",
        2 => "±10 kHz",
        3 => "±25 kHz",
        4 => "±50 kHz",
        5 => "±100 kHz",
        6 => "±250 kHz",
        _ => "±500 kHz",
    }
}

fn scope_span_hz(span_code: u8) -> u64 {
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

fn scope_span_for_filter(mode: &str, filter: Option<u8>) -> u8 {
    let filter_width_hz = filter_bandwidth_hz(mode, filter);
    let required_half_span_hz = match scope_projection_for_mode(mode) {
        ScopeProjection::Full => filter_width_hz.div_ceil(2),
        ScopeProjection::LowerSideband | ScopeProjection::UpperSideband => filter_width_hz,
    };
    if required_half_span_hz <= 2_500 {
        0
    } else if required_half_span_hz <= 5_000 {
        1
    } else if required_half_span_hz <= 10_000 {
        2
    } else if required_half_span_hz <= 25_000 {
        3
    } else if required_half_span_hz <= 50_000 {
        4
    } else if required_half_span_hz <= 100_000 {
        5
    } else if required_half_span_hz <= 250_000 {
        6
    } else {
        7
    }
}

fn band_edges_for_frequency(frequency_hz: Option<u64>) -> Option<(u64, u64, &'static str)> {
    let freq = frequency_hz?;
    match freq {
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

fn general_meter_order() -> [MeterId; 8] {
    [
        MeterId::Voltage,
        MeterId::Current,
        MeterId::Signal,
        MeterId::Power,
        MeterId::Swr,
        MeterId::Alc,
        MeterId::Compression,
        MeterId::Temperature,
    ]
}

fn mode_meter_order(transmitting: bool) -> [MeterId; 8] {
    if transmitting {
        [
            MeterId::Voltage,
            MeterId::Current,
            MeterId::Power,
            MeterId::Swr,
            MeterId::Alc,
            MeterId::Compression,
            MeterId::Temperature,
            MeterId::Signal,
        ]
    } else {
        general_meter_order()
    }
}

fn meter_value(snapshot: &GuiState, id: MeterId) -> Option<u8> {
    match id {
        MeterId::Signal => snapshot.signal_meter,
        MeterId::Power => snapshot.power_meter,
        MeterId::Swr => snapshot.swr,
        MeterId::Alc => snapshot.alc_meter,
        MeterId::Compression => snapshot.compression_meter,
        MeterId::Current => snapshot.current_meter,
        MeterId::Voltage => snapshot.voltage_meter,
        MeterId::Temperature => snapshot.temperature_meter,
    }
}

fn meter_percent(value: u8) -> f32 {
    f32::from(value) / 255.0
}

const VOLTAGE_HISTORY_CAPACITY: usize = 180;
const METER_LABEL_WIDTH: f32 = 88.0;

fn record_voltage_sample(history: &mut VecDeque<u8>, value: u8) {
    history.push_back(value);
    while history.len() > VOLTAGE_HISTORY_CAPACITY {
        history.pop_front();
    }
}

fn meter_color_for_context(id: MeterId, value: Option<u8>, transmitting: bool) -> Color32 {
    if id == MeterId::Current && transmitting && value.is_some() {
        return Color32::from_rgb(110, 245, 215);
    }
    meter_color(id, value)
}

fn draw_voltage_graph(ui: &mut egui::Ui, history: &VecDeque<u8>, reading: &str) {
    let desired_size = egui::vec2(ui.available_width().max(100.0), 28.0);
    let (rect, _) = ui.allocate_exact_size(desired_size, egui::Sense::hover());
    let painter = ui.painter();
    let outer = rect.expand(1.0);
    painter.rect_filled(
        outer,
        egui::CornerRadius::same(7),
        Color32::from_rgb(10, 20, 29),
    );
    painter.rect_stroke(
        outer,
        egui::CornerRadius::same(7),
        egui::Stroke::new(1.0_f32, Color32::from_rgb(45, 75, 88)),
        egui::StrokeKind::Inside,
    );
    painter.rect_filled(
        rect,
        egui::CornerRadius::same(5),
        Color32::from_rgb(7, 18, 25),
    );
    painter.rect_stroke(
        rect,
        egui::CornerRadius::same(5),
        egui::Stroke::new(1.0_f32, Color32::from_rgb(28, 45, 57)),
        egui::StrokeKind::Inside,
    );

    if !history.is_empty() {
        let graph_rect = rect.shrink2(egui::vec2(3.0, 3.0));
        let capacity = VOLTAGE_HISTORY_CAPACITY.max(history.len());
        let points: Vec<egui::Pos2> = history
            .iter()
            .enumerate()
            .map(|(index, value)| {
                let x = egui::lerp(
                    graph_rect.left()..=graph_rect.right(),
                    (index + 1) as f32 / capacity as f32,
                );
                let y = egui::lerp(
                    graph_rect.bottom()..=graph_rect.top(),
                    meter_percent(*value),
                );
                egui::pos2(x, y)
            })
            .collect();
        painter.add(egui::Shape::line(
            points.clone(),
            egui::Stroke::new(2.0_f32, Color32::from_rgb(100, 225, 165)),
        ));
        if let Some(last) = points.last() {
            painter.circle_filled(*last, 3.0, Color32::from_rgb(150, 255, 205));
        }
    }

    let reading_width = 90.0;
    let reading_rect = egui::Rect::from_min_max(
        egui::pos2(rect.right() - reading_width, rect.top() + 2.0),
        egui::pos2(rect.right() - 3.0, rect.bottom() - 2.0),
    );
    painter.rect_filled(
        reading_rect,
        egui::CornerRadius::same(3),
        Color32::from_rgba_unmultiplied(10, 20, 29, 225),
    );
    painter.text(
        reading_rect.right_center() - egui::vec2(5.0, 0.0),
        egui::Align2::RIGHT_CENTER,
        reading,
        egui::FontId::monospace(11.0),
        Color32::WHITE,
    );
}

fn draw_primary_meter(
    ui: &egui::Ui,
    rect: egui::Rect,
    label: &str,
    reading: &str,
    fraction: f32,
    color: Color32,
) {
    let painter = ui.painter();
    let outer = rect.expand(1.0);
    painter.rect_filled(
        outer,
        egui::CornerRadius::same(7),
        Color32::from_rgb(10, 20, 29),
    );
    painter.rect_stroke(
        outer,
        egui::CornerRadius::same(7),
        egui::Stroke::new(1.0_f32, color.gamma_multiply(0.65)),
        egui::StrokeKind::Inside,
    );

    let inner = rect.shrink2(egui::vec2(5.0, 6.0));
    let segments = 30;
    let gap = 2.0;
    let segment_width = ((inner.width() - gap * (segments - 1) as f32) / segments as f32).max(1.0);
    let lit = (fraction.clamp(0.0, 1.0) * segments as f32).ceil() as usize;
    for index in 0..segments {
        let left = inner.left() + index as f32 * (segment_width + gap);
        let segment = egui::Rect::from_min_max(
            egui::pos2(left, inner.top()),
            egui::pos2(left + segment_width, inner.bottom()),
        );
        let fill = if index < lit {
            color
        } else {
            Color32::from_rgb(28, 45, 57)
        };
        painter.rect_filled(segment, egui::CornerRadius::same(2), fill);
    }
    if !label.is_empty() {
        painter.rect_filled(
            egui::Rect::from_min_max(
                egui::pos2(rect.left() + 4.0, rect.top() + 2.0),
                egui::pos2(rect.left() + 57.0, rect.bottom() - 2.0),
            ),
            egui::CornerRadius::same(3),
            Color32::from_rgba_unmultiplied(10, 20, 29, 225),
        );
        painter.text(
            egui::pos2(rect.left() + 8.0, rect.center().y),
            egui::Align2::LEFT_CENTER,
            label,
            egui::FontId::monospace(11.0),
            Color32::WHITE,
        );
    }
    painter.rect_filled(
        egui::Rect::from_min_max(
            egui::pos2(rect.right() - 143.0, rect.top() + 2.0),
            egui::pos2(rect.right() - 4.0, rect.bottom() - 2.0),
        ),
        egui::CornerRadius::same(3),
        Color32::from_rgba_unmultiplied(10, 20, 29, 225),
    );
    painter.text(
        egui::pos2(rect.right() - 8.0, rect.center().y),
        egui::Align2::RIGHT_CENTER,
        reading,
        egui::FontId::monospace(11.0),
        Color32::WHITE,
    );
}

#[cfg(test)]
fn format_meter_value(value: Option<u8>) -> String {
    value
        .map(|value| format!("{:.0}%", f32::from(value) * 100.0 / 255.0))
        .unwrap_or_else(|| "—".to_string())
}

fn meter_label(id: MeterId) -> &'static str {
    match id {
        MeterId::Signal => "S-METER",
        MeterId::Power => "POWER",
        MeterId::Swr => "SWR",
        MeterId::Alc => "ALC",
        MeterId::Compression => "COMP",
        MeterId::Current => "CURRENT",
        MeterId::Voltage => "VOLTAGE",
        MeterId::Temperature => "TEMP",
    }
}

fn meter_reading(id: MeterId, value: Option<u8>) -> String {
    let Some(value) = value else {
        return "—".to_string();
    };
    if id == MeterId::Signal {
        let s_units = u16::from(value) / 12;
        if s_units <= 9 {
            format!("S{s_units} · {}%", u16::from(value) * 100 / 255)
        } else {
            format!(
                "S9 +{} dB · {}%",
                (s_units - 9) * 6,
                u16::from(value) * 100 / 255
            )
        }
    } else if id == MeterId::Voltage {
        format!("REL {value}/255")
    } else {
        format!("{}%", u16::from(value) * 100 / 255)
    }
}

fn meter_reading_for_model(id: MeterId, value: Option<u8>, model: &str) -> String {
    if id == MeterId::Voltage {
        if let (Some(profile), Some(raw)) = (native_radio_profile("native", model), value) {
            if let Some(voltage) = profile.calibrated_meter_value(id, raw) {
                return format!("{voltage:.1} V");
            }
        }
    }
    meter_reading(id, value)
}

fn meter_tooltip(id: MeterId) -> &'static str {
    match id {
        MeterId::Signal => {
            "Receive signal level; S-unit display is derived from the normalized driver level"
        }
        MeterId::Power => "Measured relative RF output level",
        MeterId::Swr => "Transmit SWR meter level; exact ratio is model-specific",
        MeterId::Alc => "Transmit ALC level",
        MeterId::Compression => "Transmit speech/data compression level",
        MeterId::Current => "PA drain/current meter level",
        MeterId::Voltage => {
            "PA voltage level; IC-7300 is shown in volts, other radios are relative"
        }
        MeterId::Temperature => "PA temperature meter; exact units depend on the driver",
    }
}

fn meter_color(id: MeterId, value: Option<u8>) -> Color32 {
    if value.is_none() {
        return Color32::GRAY;
    }
    let value = value.unwrap_or_default();
    match id {
        MeterId::Swr if value >= 190 => Color32::RED,
        MeterId::Swr if value >= 130 => Color32::YELLOW,
        MeterId::Power | MeterId::Alc | MeterId::Compression if value >= 220 => {
            Color32::from_rgb(255, 145, 100)
        }
        _ => Color32::from_rgb(100, 210, 150),
    }
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
    fn null_profiles_always_use_virtual_audio_devices() {
        assert_eq!(
            effective_audio_input_device("null", Some("Physical microphone".to_string())),
            Some(NULL_INPUT_DEVICE.to_string())
        );
        assert_eq!(
            effective_audio_output_device("mock", Some("Physical speakers".to_string())),
            Some(NULL_OUTPUT_DEVICE.to_string())
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
        assert_eq!(radio_mode_label("LSB-D", Some(false)), "LSB");
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
}
