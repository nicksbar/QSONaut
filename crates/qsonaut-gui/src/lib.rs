mod automation_hunter;
mod band_plan;
mod contest;
mod decode_model;
mod local_ai;
mod modes;
mod panels;
mod profile;
mod server_integration;
mod tx_audio;
mod ui_format;
mod visuals;
mod workers;

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
use qsonaut_accelerate::{
    AccelerationReport, ActiveBackend, ComputePreference, DecodeTelemetry, DecodeTrace,
};
use qsonaut_audio::{play_pcm_blocking, AudioService};
use qsonaut_automation::{
    Action, AutomationEvent, AutomationHost, Capability, CapabilitySet, EventKind,
    ExternalSourceConfig, RuleComponent, RuleComponentConfig,
};
use qsonaut_core::{
    AppConfig, AppEvent, AppEventBus, ContestOperatingMode, ContestProfile, FoxHoundRole,
    SplitPolicy,
};
use qsonaut_log::{
    app_config_dir, hamdb_cache_path, log_file_path, read_log_tail, AdifExportFilter, HamDbCache,
    HamDbCacheEntry, QsoLog, QsoRecord,
};
use qsonaut_pskreporter::{
    ReceptionReport, ReportSender, Reporter, ReporterConfig, ReporterTuning,
};
use qsonaut_radio::{
    drivers::{
        open_dxlab, open_model_with_radio_address, open_null, open_rigctld, ConfiguredRadio,
    },
    enumerate_serial_port_descriptors,
    models::{find_model, Manufacturer, Protocol, SupportLevel, POPULAR_RADIOS},
    BaseMode, ControlId, ControlValue, IcomCiVRadio, Mode, Radio, SerialPortDescriptor,
};
use qsonaut_server_client::{
    log_idempotency_key, new_instance_id, ConnectionConfig as ServerConnectionConfig,
    ConnectionState as ServerConnectionState, Presence as ServerPresence, ServerClient,
};
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
use tracing::{debug, error, info};

const QSONAUT_ICON_PNG: &[u8] = include_bytes!("../../../assets/branding/qsonaut-icon.png");

use automation_hunter::{
    AchievementKind, CustomAchievementRule, ExternalSendRecord, HunterAlert, HunterMetric,
};
use band_plan::{
    band_for_frequency, workspace_band_plan, workspace_radio_preset, WorkspaceMode,
    HF_WORKSPACE_MODES, OTHER_WORKSPACE_MODES, WORKSPACE_MODES,
};
use decode_model::{
    digital_activity_stats, ft8_activity_stats, operator_call_hit, DigitalDecodeEntry,
    DigitalSlotGate, Ft8DecodeEntry, Ft8SlotGate, OperatorCallHit, PendingFt8Decode, PotaSpot,
};
use local_ai::{LocalImageEvent, LocalImageProvider, LocalImageSettings};
use modes::exchange::{
    callsign_eq, is_probable_callsign, next_reply_period, next_tx_period, parse_message,
    select_candidate, should_finalize_after_tx, should_repeat_cq, should_retry_after_decode,
    AutoReplyPolicy, AutoTxStopPolicy, Exchange, ParsedMessage, QsoSession, QsoStage,
    ReplyCandidate, SLOT_SECONDS,
};
use profile::{
    active_operator_profile_name, default_contest_fake_split_offset_hz, default_cw_tone_hz,
    default_cw_wpm, default_gui_scale, default_max_attempts as default_ft8_max_attempts,
    default_psk_batch_interval_secs, default_psk_max_pending, default_psk_repeat_cache_secs,
    default_ptt_lead_ms, default_ptt_tail_ms, default_rx_tone_hz, default_tx_tone_hz,
    default_waterfall_deck_height, list_operator_profiles, load_operator_profile,
    load_operator_profile_named, save_operator_profile, save_operator_profile_named,
    select_operator_profile, OperatorProfile, RadioProfile, OPERATOR_PROFILE_FILE,
    OPERATOR_PROFILE_VERSION,
};
#[cfg(test)]
use tx_audio::FT8_TX_AUDIO_START_S;
use tx_audio::{
    build_ft8_tx_pcm, build_native_digital_tx_pcm, run_digital_tx_job, run_ft8_tx_job,
    DigitalTxChatEntry, DigitalTxEvent, DigitalTxJob, Ft8ChatDirection, Ft8ChatLine,
    Ft8TxChatEntry, Ft8TxEvent, Ft8TxJob,
};
use ui_format::{format_signal_report, ft8_period_progress, qso_stage_label, utc_hhmmss_millis};
use visuals::{
    audio_cursor_level, build_scope_waterfall_image, downsample_bins, fft_buffer_to_display_bins,
    scale_scope_levels,
};
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
const GUI_SCALE_MIN: f32 = 0.9;
const QSO_LOG_FILE: &str = "log.toml";
const QSO_ADIF_FILE: &str = "log.adi";
const HAMDB_CACHE_TTL_SECONDS: u64 = 30 * 24 * 60 * 60;
// The generated waveform starts at +0.5 s and ends at about +13.14 s.
const FT8_EARLY_DECODE_S: f64 = 13.2;
const FT8_SLOT_SAMPLES: usize = 12_000 * 15;
// mfsk-core's WSJT-X depth/recall ladder is calibrated at 1.3. In particular,
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
    Profile,
    Contest,
    Reporting,
    Waterfall,
    Settings,
    Server,
    RadioTuning,
    AppLog,
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

fn gui_scale_from_percent(percent: u32) -> f32 {
    (GUI_SCALE_BASE * percent as f32 / 100.0).clamp(GUI_SCALE_MIN, GUI_SCALE_MAX)
}

fn gui_scale_percent(scale: f32) -> f32 {
    scale / GUI_SCALE_BASE * 100.0
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
        "SSTV" => Some(WorkspaceMode::Sstv),
        "FLDIGI" => Some(WorkspaceMode::Fldigi),
        _ => None,
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
        return None;
    }
    let mut config = ReporterConfig::production(callsign, grid);
    config.tuning = tuning;
    let reporter = Reporter::start(config);
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
    sender.submit(ReceptionReport {
        sender_callsign: callsign,
        frequency_hz,
        snr_db: snr_db.round().clamp(-127.0, 127.0) as i8,
        mode: mode.to_string(),
        sender_locator,
        received_at,
    });
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WaterfallSpeed {
    Slow,
    Mid,
    Fast,
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
    radio_scope_contrast: f32,
    radio_scope_span_code: u8,
    radio_scope_vbw_wide: bool,
    radio_scope_hold: bool,
    radio_scope_reference_tenths_db: i16,
    radio_scope_view: RadioScopeView,
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
    cw_live_text: String,
    cw_record_rx: bool,
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
    sstv_rx_mode: Option<qsonaut_sstv::SstvMode>,
    sstv_detected_mode: Option<qsonaut_sstv::SstvMode>,
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
            radio_scope_contrast: 1.2,
            radio_scope_span_code: 0,
            radio_scope_vbw_wide: false,
            radio_scope_hold: false,
            radio_scope_reference_tenths_db: 0,
            radio_scope_view: RadioScopeView::Narrow,
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
            cw_live_text: String::new(),
            cw_record_rx: false,
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
            sstv_rx_mode: None,
            sstv_detected_mode: None,
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
    AfGainDelta(i16),
    ApplyWorkspace {
        mode: WorkspaceMode,
        frequency_hz: u64,
    },
    SetFilter(u8),
    SetControl(ControlId, ControlValue),
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
    info!(build_profile, "QSONaut GUI startup");
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
        .with_resizable(true);
    if let Some(geometry) = stored_geometry {
        viewport = geometry.apply(viewport);
    }
    let options = eframe::NativeOptions {
        viewport,
        renderer,
        // QSONaut restores geometry through the builder above so winit applies
        // it once, before the window is ever shown.
        persist_window: false,
        ..Default::default()
    };

    let app_config = config.clone();
    info!(title = "QSONaut", renderer = %renderer, "Calling eframe::run_native");
    let result = eframe::run_native(
        "QSONaut",
        options,
        Box::new(move |cc| {
            info!(renderer = %renderer, "eframe app creation callback entered");
            Ok(Box::new(QsonautGuiApp::new(
                app_config.clone(),
                cc,
                &app_icon,
                renderer,
                stored_geometry,
            )))
        }),
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

fn native_radio_profile(
    backend: &str,
    model: &str,
) -> Option<&'static qsonaut_radio::models::RadioModelProfile> {
    backend
        .trim()
        .eq_ignore_ascii_case("native")
        .then(|| find_model(model))
        .flatten()
}

/// Native window geometry restored by QSONaut instead of eframe. Applying it to
/// the `ViewportBuilder` means winit configures the window once, while it is
/// still hidden, instead of showing and re-hiding it for each late change.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
struct WindowGeometry {
    #[serde(default)]
    maximized: bool,
    #[serde(default)]
    position: Option<[f32; 2]>,
    #[serde(default)]
    size: Option<[f32; 2]>,
}

impl WindowGeometry {
    fn path() -> PathBuf {
        qsonaut_log::app_config_dir().join(WINDOW_GEOMETRY_FILE)
    }

    fn load() -> Option<Self> {
        let raw = std::fs::read_to_string(Self::path()).ok()?;
        match serde_json::from_str::<Self>(&raw) {
            Ok(geometry) => Some(geometry.sanitized()),
            Err(error) => {
                info!(%error, "Ignoring unreadable window geometry");
                None
            }
        }
    }

    /// A stale profile can carry a monitor that no longer exists or values from
    /// a crashed session, which would otherwise open the window off-screen.
    fn sanitized(mut self) -> Self {
        self.size = self
            .size
            .filter(|s| s.iter().all(|v| v.is_finite()))
            .map(|s| {
                [
                    s[0].clamp(WINDOW_MIN_SIZE[0], 16_000.0),
                    s[1].clamp(WINDOW_MIN_SIZE[1], 16_000.0),
                ]
            });
        self.position = self
            .position
            .filter(|p| p.iter().all(|v| v.is_finite() && v.abs() <= 32_000.0));
        self
    }

    fn read(ctx: &egui::Context, previous: Option<Self>) -> Option<Self> {
        ctx.input(|input| {
            let viewport = input.viewport();
            let maximized = viewport.maximized.unwrap_or(false);
            // Restore bounds are meaningless while maximized, so keep the last
            // known un-maximized rect instead of overwriting it.
            if maximized {
                let previous = previous.unwrap_or_default();
                return Some(Self {
                    maximized: true,
                    position: previous.position,
                    size: previous.size,
                });
            }
            let position = viewport.outer_rect.map(|rect| [rect.min.x, rect.min.y])?;
            let size = viewport
                .inner_rect
                .map(|rect| [rect.width(), rect.height()])?;
            Some(Self {
                maximized: false,
                position: Some(position),
                size: Some(size),
            })
        })
    }

    fn save(self) {
        let path = Self::path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match serde_json::to_string_pretty(&self) {
            Ok(json) => {
                if let Err(error) = std::fs::write(&path, json) {
                    info!(%error, path = %path.display(), "Failed to save window geometry");
                }
            }
            Err(error) => info!(%error, "Failed to serialize window geometry"),
        }
    }

    fn apply(self, mut builder: egui::ViewportBuilder) -> egui::ViewportBuilder {
        if let Some(size) = self.size {
            builder = builder.with_inner_size(size);
        }
        if let Some(position) = self.position {
            builder = builder.with_position(position);
        }
        // Maximized is deliberately not set here: winit would `SW_MAXIMIZE` the
        // still-unpainted window and immediately `SW_HIDE` it again, which is
        // the white flash. It is applied after the first frame instead.
        builder
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
        let (serial_ports, serial_port_labels, detected_models) =
            radio_port_inventory(enumerate_serial_port_descriptors().unwrap_or_default());
        let _ = tx.send(DeviceInventory {
            audio_inputs: AudioService::input_devices().unwrap_or_default(),
            audio_outputs: AudioService::output_devices().unwrap_or_default(),
            serial_ports,
            serial_port_labels,
            detected_models,
        });
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

fn spawn_acceleration_probe(preference: ComputePreference) -> mpsc::Receiver<AccelerationReport> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(AccelerationReport::probe(preference));
    });
    rx
}

fn preferred_renderer() -> eframe::Renderer {
    // QSONaut renders a 2D operator console. glow (OpenGL) is the lightest
    // eframe backend and behaves identically on Windows, Linux, and WSL.
    if let Some(raw) = std::env::var_os("QSONAUT_RENDERER") {
        let raw = raw.to_string_lossy();
        if !raw.eq_ignore_ascii_case("glow") {
            info!(
                requested = %raw,
                "QSONAUT_RENDERER override ignored: only 'glow' is built in"
            );
        }
    }
    eframe::Renderer::Glow
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
    audio_worker_stop: Arc<AtomicBool>,
    radio_init_rx: Option<mpsc::Receiver<Option<ConfiguredRadio>>>,
    hamdb_lookup_rx: Option<mpsc::Receiver<Option<HamDbCacheEntry>>>,
    hamdb_profile_lookup_rx: Option<mpsc::Receiver<Option<HamDbCacheEntry>>>,
    pota_spots: Vec<PotaSpot>,
    pota_lookup_rx: Option<mpsc::Receiver<Vec<PotaSpot>>>,
    pota_last_lookup: Instant,
    radio_init_attempted: bool,
    radio_worker_handle: Option<std::thread::JoinHandle<()>>,
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
    sstv_tx_mode: qsonaut_sstv::SstvMode,
    sstv_file_dialog: egui_file_dialog::FileDialog,
    sstv_image_path: String,
    sstv_ai_prompt: String,
    local_image_settings: LocalImageSettings,
    local_image_models: Vec<String>,
    local_image_status: String,
    local_image_event_tx: mpsc::Sender<LocalImageEvent>,
    local_image_event_rx: mpsc::Receiver<LocalImageEvent>,
    workspace_mode: WorkspaceMode,
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
    selected_profile_name: String,
    new_profile_name: String,
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
    waterfall_deck_height: f32,
    waterfall_deck_resize_pending: bool,
    show_signal_panel: bool,
    signal_panel_tab: SignalPanelTab,
    device_restart_required: bool,
    audio_restart_required: bool,
    gui_scale: f32,
    compute_preference: ComputePreference,
    acceleration_report: AccelerationReport,
    acceleration_probe: Option<mpsc::Receiver<AccelerationReport>>,
    psk_reporter_enabled: bool,
    psk_batch_interval_secs: u64,
    psk_repeat_cache_secs: u64,
    psk_max_pending: usize,
    psk_reporter: Option<Reporter>,
    server_client: Option<ServerClient>,
    server_instance_id: String,
    server_last_presence: Instant,
    brand_icon: TextureHandle,
    selected_renderer: eframe::Renderer,
    first_frame_logged: bool,
    window_geometry: Option<WindowGeometry>,
    pending_maximize: bool,
}

impl QsonautGuiApp {
    fn new(
        mut config: AppConfig,
        cc: &eframe::CreationContext<'_>,
        app_icon: &egui::IconData,
        selected_renderer: eframe::Renderer,
        stored_geometry: Option<WindowGeometry>,
    ) -> Self {
        let ctx = &cc.egui_ctx;
        let brand_image = ColorImage::from_rgba_unmultiplied(
            [app_icon.width as usize, app_icon.height as usize],
            &app_icon.rgba,
        );
        let brand_icon =
            ctx.load_texture("qsonaut-brand-icon", brand_image, TextureOptions::LINEAR);
        if let Some(profile) = load_operator_profile() {
            if profile.profile_version >= 3 {
                config.audio.input_device = profile.audio_input_device;
                config.audio.output_device = profile.audio_output_device;
                if profile.profile_version >= AUDIO_MONITOR_PROFILE_VERSION {
                    config.audio.monitor_enabled = profile.audio_monitor_enabled;
                    config.audio.monitor_output_device = profile.audio_monitor_output_device;
                    config.audio.monitor_volume = profile.audio_monitor_volume.clamp(0.0, 2.0);
                }
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
            }
        }

        let state = Arc::new(Mutex::new(GuiState::default()));
        let app_events = AppEventBus::new(256);
        let automation_event_rx = app_events.subscribe();
        let (automation_host, automation_status, automation_external_transports) =
            bootstrap_automation_host();
        let radio_worker_stop = Arc::new(AtomicBool::new(false));
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

        let ft8_tx_active = Arc::new(AtomicBool::new(false));
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
            config.audio.input_device.clone(),
            config.audio.monitor_enabled,
            config
                .audio
                .monitor_output_device
                .clone()
                .or_else(|| config.audio.output_device.clone()),
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
        let mut gui_scale = default_gui_scale();
        let mut compute_preference = ComputePreference::Auto;
        let mut psk_reporter_enabled = false;
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
        let mut radio_profiles = Vec::new();
        let mut mode_radio_profile = std::collections::BTreeMap::new();
        let profile_io_status: String;

        if let Some(p) = load_operator_profile() {
            station_callsign = p.callsign;
            station_grid = p.grid;
            station_qth = p.qth;
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
            gui_scale = if p.profile_version >= GUI_SCALE_PROFILE_VERSION {
                p.gui_scale.clamp(GUI_SCALE_MIN, GUI_SCALE_MAX)
            } else {
                // v3 called this physical size 160%; it is the v4 100% baseline.
                default_gui_scale()
            };
            compute_preference = p.compute_preference;
            psk_reporter_enabled = p.psk_reporter_enabled;
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
            radio_profiles = p.radio_profiles;
            mode_radio_profile = p.mode_radio_profile;
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
                audio_input_device: config.audio.input_device.clone(),
                audio_output_device: config.audio.output_device.clone(),
                audio_monitor_enabled: config.audio.monitor_enabled,
                audio_monitor_output_device: config.audio.monitor_output_device.clone(),
                audio_monitor_volume: config.audio.monitor_volume.clamp(0.0, 2.0),
                radio_serial_port: config.radio.serial_port.clone(),
                radio_backend: config.radio.backend.clone(),
                radio_endpoint: config.radio.endpoint.clone(),
                radio_model: config.radio.model.clone(),
                radio_baud_rate: config.radio.baud_rate,
                gui_scale,
                compute_preference,
                psk_reporter_enabled,
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

        let available_profiles = list_operator_profiles();
        let active_profile_name = active_operator_profile_name();
        let selected_profile_name = available_profiles
            .iter()
            .find(|name| name.eq_ignore_ascii_case(&active_profile_name))
            .cloned()
            .unwrap_or_else(|| "Default".to_string());

        let server_client = (config.server.enabled
            && !config.server.url.trim().is_empty()
            && !config.server.device_token.trim().is_empty())
        .then(|| {
            ServerClient::spawn(ServerConnectionConfig {
                server_url: config.server.url.trim().to_string(),
                device_token: config.server.device_token.trim().to_string(),
                client_version: env!("CARGO_PKG_VERSION").to_string(),
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
        // Applied before the first paint so the window is never laid out at one
        // scale and immediately re-laid out at another.
        ctx.set_zoom_factor(gui_scale);
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
            audio_worker_stop,
            radio_init_rx,
            hamdb_lookup_rx: None,
            hamdb_profile_lookup_rx: None,
            pota_spots: Vec::new(),
            pota_lookup_rx: None,
            pota_last_lookup: Instant::now() - Duration::from_secs(60),
            logo_clicks: VecDeque::new(),
            logo_spin_until: None,
            radio_init_attempted: false,
            radio_worker_handle,
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
            sstv_tx_mode: qsonaut_sstv::SstvMode::MartinM1,
            sstv_file_dialog: egui_file_dialog::FileDialog::new(),
            sstv_image_path: String::new(),
            sstv_ai_prompt: String::new(),
            local_image_settings: LocalImageSettings::load(),
            local_image_models: Vec::new(),
            local_image_status: "Local image server not checked".to_string(),
            local_image_event_tx,
            local_image_event_rx,
            workspace_mode: WorkspaceMode::Ft8,
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
            selected_profile_name,
            new_profile_name: String::new(),
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
            waterfall_deck_height,
            waterfall_deck_resize_pending: false,
            show_signal_panel: true,
            signal_panel_tab: SignalPanelTab::Achievements,
            device_restart_required: false,
            audio_restart_required: false,
            gui_scale,
            compute_preference,
            acceleration_report,
            acceleration_probe,
            psk_reporter_enabled,
            psk_batch_interval_secs,
            psk_repeat_cache_secs,
            psk_max_pending,
            psk_reporter,
            server_client,
            server_instance_id,
            server_last_presence: Instant::now() - Duration::from_secs(60),
            brand_icon,
            selected_renderer,
            first_frame_logged: false,
            window_geometry: stored_geometry,
            pending_maximize: stored_geometry.is_some_and(|geometry| geometry.maximized),
        }
    }

    fn refresh_acceleration_report(&mut self) {
        self.acceleration_report = AccelerationReport::pending(self.compute_preference);
        self.acceleration_probe = Some(spawn_acceleration_probe(self.compute_preference));
    }

    fn persist_profile(&mut self, status_prefix: &str) {
        match save_operator_profile_named(
            &self.selected_profile_name,
            &self.current_operator_profile(),
        ) {
            Ok(_) => {
                self.profile_io_status =
                    format!("{status_prefix} profile ‘{}’", self.selected_profile_name);
                self.available_profiles = list_operator_profiles();
                self.profile_dirty = false;
            }
            Err(err) => {
                self.profile_io_status = format!("Save failed: {err}");
            }
        }
    }

    fn apply_operator_profile(&mut self, profile: OperatorProfile) {
        let previous_audio = (
            self.config.audio.input_device.clone(),
            self.config.audio.output_device.clone(),
            self.config.audio.monitor_enabled,
            self.config.audio.monitor_output_device.clone(),
        );
        let previous_radio = (
            self.config.radio.serial_port.clone(),
            self.config.radio.backend.clone(),
            self.config.radio.endpoint.clone(),
            self.config.radio.model.clone(),
            self.config.radio.baud_rate,
        );
        self.station_callsign = profile.callsign;
        self.station_grid = profile.grid;
        self.station_qth = profile.qth;
        self.ft8_follow_log = profile.follow_log;
        self.ft8_max_log_entries = profile.max_log_entries.clamp(80, 1000);
        self.ft8_deep_decode = profile.deep_decode;
        self.ft4_deep_decode = profile.ft4_deep_decode;
        // Loading or switching profiles must never arm transmit automation.
        self.ft4_autoseq = false;
        self.ft4_auto_reply_policy = profile.ft4_auto_reply_policy;
        self.ft4_cq_only_view = profile.ft4_cq_only_view;
        self.ft4_follow_log = profile.ft4_follow_log;
        self.ft4_max_log_entries = profile.ft4_max_log_entries.clamp(80, 300);
        self.ft4_max_attempts = profile.ft4_max_attempts.clamp(1, 20);
        self.ft4_stop_policy = AutoTxStopPolicy::Continuous;
        // Loading or switching profiles must never arm transmit automation.
        self.ft8_autoseq = false;
        self.ft8_auto_reply_policy = profile.auto_reply_policy;
        // Unattended CQ answering must be explicitly enabled for this run.
        self.ft8_auto_answer_cq = false;
        self.automation_unlocked = profile.automation_unlocked;
        self.ft8_cq_only_view = profile.cq_only_view;
        self.civ_spectrum_on = profile.civ_spectrum_on;
        self.radio_scope_vbw_wide = profile.radio_scope_vbw_wide;
        self.radio_scope_view = profile.radio_scope_view;
        self.waterfall_theme = profile.waterfall_theme;
        self.waterfall_deck_height = profile.waterfall_deck_height.clamp(170.0, 560.0);
        self.ft8_stop_policy = AutoTxStopPolicy::Continuous;
        self.ft8_max_attempts = profile.ft8_max_attempts.clamp(1, 20);
        self.ft8_hold_tx_freq = profile.profile_version >= 3 && profile.hold_tx_freq;
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
        self.contest_enabled = profile.contest_enabled;
        self.contest_operating_mode = profile.contest_operating_mode;
        self.contest_split_policy = profile.contest_split_policy;
        self.contest_fox_hound_role = profile.contest_fox_hound_role;
        self.contest_exchange_template = profile.contest_exchange_template;
        self.contest_serial_start = profile.contest_serial_start.max(1);
        self.contest_serial_step = profile.contest_serial_step.max(1);
        self.contest_dupe_check = profile.contest_dupe_check;
        self.contest_serial_current = profile
            .contest_serial_current
            .max(self.contest_serial_start)
            .max(1);
        self.contest_fake_split_offset_hz = profile.contest_fake_split_offset_hz.clamp(0, 2_000);
        self.hunter_unlocked = profile.hunter_unlocked.into_iter().collect();
        self.hunter_acknowledged = profile.hunter_acknowledged.into_iter().collect();
        self.hunter_alerts_enabled = profile.hunter_alerts_enabled;
        self.hunter_custom_rules = profile.hunter_custom_rules;
        self.radio_profiles = profile.radio_profiles;
        self.mode_radio_profile = profile.mode_radio_profile;
        self.gui_scale = if profile.profile_version >= GUI_SCALE_PROFILE_VERSION {
            profile.gui_scale.clamp(GUI_SCALE_MIN, GUI_SCALE_MAX)
        } else {
            default_gui_scale()
        };
        self.compute_preference = profile.compute_preference;
        self.psk_reporter_enabled = profile.psk_reporter_enabled;
        self.psk_batch_interval_secs = profile.psk_batch_interval_secs.clamp(60, 3_600);
        self.psk_repeat_cache_secs = profile.psk_repeat_cache_secs.clamp(60, 3_600);
        self.psk_max_pending = profile.psk_max_pending.clamp(1, 2_048);
        if !profile.server_instance_id.is_empty() {
            self.server_instance_id = profile.server_instance_id;
        }
        if let Some(server) = profile.server {
            self.config.server = server;
            self.reconnect_server();
        }
        self.refresh_acceleration_report();
        if profile.profile_version >= 3 {
            self.config.audio.input_device = profile.audio_input_device;
            self.config.audio.output_device = profile.audio_output_device;
            if profile.profile_version >= AUDIO_MONITOR_PROFILE_VERSION {
                self.config.audio.monitor_enabled = profile.audio_monitor_enabled;
                self.config.audio.monitor_output_device = profile.audio_monitor_output_device;
                self.config.audio.monitor_volume = profile.audio_monitor_volume.clamp(0.0, 2.0);
                self.monitor_volume.store(
                    self.config.audio.monitor_volume.to_bits(),
                    Ordering::Relaxed,
                );
            }
            self.config.radio.serial_port = profile.radio_serial_port;
            self.config.radio.backend = profile.radio_backend;
            self.config.radio.endpoint = profile.radio_endpoint;
            if profile.profile_version >= 8 {
                self.config.radio.model = profile.radio_model;
                self.config.radio.baud_rate = profile.radio_baud_rate;
            }
        }
        self.config.station.callsign = Some(self.station_callsign.clone());
        self.config.station.grid = Some(self.station_grid.clone());
        self.config.contest = ContestProfile {
            enabled: self.contest_enabled,
            operating_mode: self.contest_operating_mode,
            split_policy: self.contest_split_policy,
            fox_hound_role: self.contest_fox_hound_role,
            exchange_template: if self.contest_exchange_template.trim().is_empty() {
                None
            } else {
                Some(self.contest_exchange_template.trim().to_string())
            },
            serial_start: self.contest_serial_start,
            serial_step: self.contest_serial_step,
            dupe_check: self.contest_dupe_check,
        };
        self.restart_psk_reporter();
        let current_audio = (
            self.config.audio.input_device.clone(),
            self.config.audio.output_device.clone(),
            self.config.audio.monitor_enabled,
            self.config.audio.monitor_output_device.clone(),
        );
        let current_radio = (
            self.config.radio.serial_port.clone(),
            self.config.radio.backend.clone(),
            self.config.radio.endpoint.clone(),
            self.config.radio.model.clone(),
            self.config.radio.baud_rate,
        );
        if current_audio != previous_audio {
            self.restart_audio();
        }
        if current_radio != previous_radio {
            self.reconnect_radio();
        }
        self.profile_dirty = false;
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

    fn pump_hamdb_lookup(&mut self) {
        let Some(rx) = self.hamdb_lookup_rx.as_ref() else {
            return;
        };
        let Ok(Some(entry)) = rx.try_recv() else {
            return;
        };
        let cache = HamDbCache::open(&hamdb_cache_path()).ok();
        for record in self
            .qso_log
            .contacts
            .iter_mut()
            .filter(|record| record.callsign.eq_ignore_ascii_case(&entry.callsign))
        {
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
        self.qso_log_dirty = true;
        self.persist_qso_log("HamDB details saved to");
        self.hamdb_lookup_rx = None;
    }

    fn pump_hamdb_profile_lookup(&mut self) {
        let Some(rx) = self.hamdb_profile_lookup_rx.as_ref() else {
            return;
        };
        let entry = match rx.try_recv() {
            Ok(Some(entry)) => entry,
            Ok(None) | Err(mpsc::TryRecvError::Disconnected) => {
                self.profile_io_status = "HamDB did not return a license record".to_string();
                self.hamdb_profile_lookup_rx = None;
                return;
            }
            Err(mpsc::TryRecvError::Empty) => return,
        };
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
        if let Some(rx) = &self.pota_lookup_rx {
            if let Ok(spots) = rx.try_recv() {
                self.pota_spots = spots;
                self.pota_lookup_rx = None;
            }
        }
        if self.pota_lookup_rx.is_some()
            || self.pota_last_lookup.elapsed() < Duration::from_secs(30)
        {
            return;
        }
        self.pota_last_lookup = Instant::now();
        let (tx, rx) = mpsc::channel();
        self.pota_lookup_rx = Some(rx);
        thread::spawn(move || {
            let spots = reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .ok()
                .and_then(|client| {
                    client
                        .get("https://api.pota.app/spot/activator")
                        .send()
                        .ok()
                })
                .and_then(|response| response.json::<Vec<PotaApiSpot>>().ok())
                .unwrap_or_default()
                .into_iter()
                .filter_map(|spot| {
                    Some(PotaSpot {
                        activator: spot.activator?.trim().to_ascii_uppercase(),
                        reference: spot.reference?.trim().to_string(),
                        name: spot.name?.trim().to_string(),
                        frequency_hz: spot.frequency?.parse::<f64>().ok()?.round() as u64 * 1_000,
                        mode: spot.mode?.trim().to_ascii_uppercase(),
                    })
                })
                .collect();
            let _ = tx.send(spots);
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
        self.profile_dirty = true;
        self.persist_profile("All TX disarmed");
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
            audio_input_device: self.config.audio.input_device.clone(),
            audio_output_device: self.config.audio.output_device.clone(),
            audio_monitor_enabled: self.config.audio.monitor_enabled,
            audio_monitor_output_device: self.config.audio.monitor_output_device.clone(),
            audio_monitor_volume: self.config.audio.monitor_volume.clamp(0.0, 2.0),
            radio_serial_port: self.config.radio.serial_port.clone(),
            radio_backend: self.config.radio.backend.clone(),
            radio_endpoint: self.config.radio.endpoint.clone(),
            radio_model: self.config.radio.model.clone(),
            radio_baud_rate: self.config.radio.baud_rate,
            gui_scale: self.gui_scale.clamp(GUI_SCALE_MIN, GUI_SCALE_MAX),
            compute_preference: self.compute_preference,
            psk_reporter_enabled: self.psk_reporter_enabled,
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
            radio_profiles: self.radio_profiles.clone(),
            mode_radio_profile: self.mode_radio_profile.clone(),
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
            self.automation_status =
                "🤖 External ingress blocked: source, author, and message are required".to_string();
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
        self.automation_status =
            format!("🤖 External message injected from {source} as {author}: {message}");
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
    }

    fn send_command(&self, cmd: GuiCommand) {
        if let Some(tx) = &self.command_tx {
            let _ = tx.send(cmd);
        }
    }

    fn refresh_device_lists(&mut self) {
        self.device_scan = Some(spawn_device_scan());
    }

    fn apply_device_inventory(&mut self, inventory: DeviceInventory) {
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
            WorkspaceMode::Ft8 | WorkspaceMode::Ft4 | WorkspaceMode::Sstv
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

    fn draw_connection_status(&self, ui: &mut egui::Ui, snapshot: &GuiState) {
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
            for label in ["IRC", "Discord"] {
                ui.label(RichText::new(format!("{label} NOT IMPLEMENTED")).color(Color32::GRAY));
                ui.separator();
            }
            ui.label(
                RichText::new(format!("Compute {}", self.acceleration_report.summary()))
                    .color(Color32::from_rgb(180, 150, 255)),
            )
            .on_hover_text(self.acceleration_report.hardware_detail());
            if let Some(error) = &snapshot.last_error {
                ui.separator();
                ui.label(RichText::new("⚠ NEEDS ATTENTION").color(theme_warning(ui)))
                    .on_hover_text(error);
            }
            ui.separator();
            ui.label(RichText::new("Reporting").strong());
            if !self.psk_reporter_enabled {
                ui.label(RichText::new("PSK Reporter OFF").color(Color32::GRAY))
                    .on_hover_text(
                        "Enable in the Reporting panel to batch decoded stations to PSK Reporter",
                    );
            } else if let Some(reporter) = &self.psk_reporter {
                let status = reporter.status();
                let (label, color) = if status.last_error.is_some() {
                    ("PSK Reporter ERROR".to_string(), Color32::from_rgb(255, 110, 100))
                } else if !status.active {
                    ("PSK Reporter STOPPED".to_string(), theme_warning(ui))
                } else {
                    (
                        format!("PSK Reporter {} queued · {} sent", status.queued, status.sent),
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
                                "Batching every ~{} s · same callsign re-reported after {} s · {} max pending",
                                self.psk_batch_interval_secs,
                                self.psk_repeat_cache_secs,
                                self.psk_max_pending
                            )
                        }),
                );
            } else {
                ui.label(RichText::new("PSK Reporter WAITING").color(theme_warning(ui)))
                    .on_hover_text("Set a real callsign and grid before reporting");
            }
        });
    }

    fn draw_banner_radio_controls(&mut self, ui: &mut egui::Ui, snapshot: &GuiState) {
        let native_profile =
            native_radio_profile(&self.config.radio.backend, &self.config.radio.model);
        let supports_levels =
            native_profile.is_some_and(|profile| profile.supports_control(ControlId::AfGain));
        let supports_filter =
            native_profile.is_some_and(|profile| profile.supports_control(ControlId::Filter));
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Radio").strong());
            if ui.small_button("-1 kHz").clicked() {
                self.send_command(GuiCommand::TuneDelta(-1_000));
            }
            if ui.small_button("+1 kHz").clicked() {
                self.send_command(GuiCommand::TuneDelta(1_000));
            }
            if ui
                .add_enabled(supports_levels, egui::Button::new("AF-").small())
                .clicked()
            {
                self.send_command(GuiCommand::AfGainDelta(-5));
            }
            if ui
                .add_enabled(supports_levels, egui::Button::new("AF+").small())
                .clicked()
            {
                self.send_command(GuiCommand::AfGainDelta(5));
            }
            ui.separator();
            ui.label(RichText::new("Mode").strong());
            ui.label(RichText::new("HF / primary").strong());
            for mode in HF_WORKSPACE_MODES {
                if ui
                    .selectable_label(self.workspace_mode == mode, mode.label())
                    .clicked()
                {
                    self.workspace_mode = mode;
                    if let Some(frequency_hz) =
                        workspace_frequency_for_current_band(mode, snapshot.frequency_hz)
                    {
                        self.send_command(GuiCommand::ApplyWorkspace { mode, frequency_hz });
                    }
                }
            }
            ui.separator();
            ui.label(
                RichText::new("Other / experimental")
                    .strong()
                    .color(Color32::GRAY),
            );
            for mode in OTHER_WORKSPACE_MODES {
                let enabled = !mode.is_uhf();
                let response = ui.add_enabled(
                    enabled,
                    egui::Button::selectable(self.workspace_mode == mode, mode.label()),
                );
                if response.clicked() && enabled {
                    self.workspace_mode = mode;
                    if let Some(frequency_hz) =
                        workspace_frequency_for_current_band(mode, snapshot.frequency_hz)
                    {
                        self.send_command(GuiCommand::ApplyWorkspace { mode, frequency_hz });
                    }
                }
                if !enabled {
                    response.on_hover_text("Disabled: no UHF radio is configured for this station");
                }
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Band").strong());
            let current_hz = snapshot.frequency_hz.unwrap_or(0);
            for &(label, frequency_hz) in workspace_band_plan(self.workspace_mode) {
                let on_band = current_hz.abs_diff(frequency_hz) < 200_000;
                if ui
                    .selectable_label(on_band, label)
                    .on_hover_text(format!("{:.6} MHz", frequency_hz as f64 / 1_000_000.0))
                    .clicked()
                {
                    self.send_command(GuiCommand::ApplyWorkspace {
                        mode: self.workspace_mode,
                        frequency_hz,
                    });
                }
            }
            ui.separator();
            ui.label(RichText::new("Filter").strong());
            for filter in 1_u8..=3 {
                if ui
                    .add_enabled(
                        supports_filter,
                        egui::Button::new(format!("FIL{filter}"))
                            .selected(snapshot.filter == Some(filter)),
                    )
                    .clicked()
                {
                    self.send_command(GuiCommand::SetFilter(filter));
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
        if self.pending_maximize {
            self.pending_maximize = false;
            ctx.send_viewport_cmd(egui::ViewportCommand::Maximized(true));
        }
        if let Some(geometry) = WindowGeometry::read(ctx, self.window_geometry) {
            self.window_geometry = Some(geometry);
        }
        if !self.first_frame_logged {
            self.first_frame_logged = true;
            info!(
                renderer = %self.selected_renderer,
                zoom_factor = ctx.zoom_factor(),
                pixels_per_point = ctx.pixels_per_point(),
                "QSONaut first GUI frame reached"
            );
        }
        // Zoom is layered on top of the OS DPI scale, so text, controls,
        // spacing, hit targets, and custom drawings stay in proportion.
        if (ctx.zoom_factor() - self.gui_scale).abs() > 0.001 {
            ctx.set_zoom_factor(self.gui_scale);
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
                Err(mpsc::TryRecvError::Disconnected) => self.device_scan = None,
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }

        // Poll for radio initialization result from background thread
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
                            self.display_tuning.clone(),
                            rx,
                            self.repaint_ctx.clone(),
                        );
                        self.command_tx = Some(tx);
                        self.radio_worker_handle = Some(handle);
                    }
                    Ok(None) => {
                        // Radio initialization failed
                        self.radio_init_attempted = true;
                        let mut s = self.state.lock().expect("ui state lock poisoned");
                        s.radio_waterfall_status = "UNAVAILABLE (connection failed)".to_string();
                        s.last_error = Some(format!(
                            "Failed to initialize radio backend '{}' (model '{}', endpoint '{}', serial port '{}')",
                            self.config.radio.backend,
                            self.config.radio.model,
                            self.config.radio.endpoint,
                            self.config.radio.serial_port.as_deref().unwrap_or("auto"),
                        ));
                    }
                    Err(mpsc::TryRecvError::Disconnected) => {
                        // Thread panicked or dropped
                        self.radio_init_attempted = true;
                        let mut s = self.state.lock().expect("ui state lock poisoned");
                        s.radio_waterfall_status = "UNAVAILABLE (init thread crashed)".to_string();
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
        self.emit_radio_state_hook_if_changed(&snapshot);
        self.publish_server_presence(&snapshot);

        egui::TopBottomPanel::top("header")
            .resizable(false)
            .min_height(112.0)
            .max_height(240.0)
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    let spin_angle = self.logo_spin_until.map_or(0.0, |until| {
                        let remaining = until.saturating_duration_since(Instant::now());
                        (1.0 - remaining.as_secs_f32() / 0.7).clamp(0.0, 1.0)
                            * std::f32::consts::TAU
                    });
                    let logo = egui::Image::new((self.brand_icon.id(), egui::vec2(46.0, 46.0)))
                        .corner_radius(8.0)
                        .rotate(spin_angle, egui::Vec2::splat(0.5))
                        .sense(egui::Sense::click());
                    let logo_response = ui.add(logo);
                    if logo_response.clicked() {
                        self.handle_logo_click();
                    }
                    if self.logo_spin_until.is_some_and(|until| Instant::now() < until) {
                        ui.ctx().request_repaint();
                    } else {
                        self.logo_spin_until = None;
                    }
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new("QSONaut")
                                .strong()
                                .size(24.0)
                                .color(Color32::from_rgb(109, 224, 255)),
                        );
                        ui.label(
                            RichText::new("AMATEUR RADIO MISSION CONTROL")
                                .strong()
                                .size(10.0)
                                .color(Color32::from_rgb(255, 137, 108)),
                        );
                    });
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
                                theme_warning(ui)
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
                    let active_profile = self.active_radio_profile_name().unwrap_or("None");
                    ui.label(
                        RichText::new(format!("🎛 {active_profile}"))
                            .small()
                            .color(if active_profile == "None" {
                                Color32::GRAY
                            } else {
                                Color32::from_rgb(255, 201, 92)
                            }),
                    )
                    .on_hover_text(
                        "Enabled radio tuning profile for this QSONaut mode; edit it in RADIO TUNING",
                    );
                    ui.label(
                        RichText::new(format!(
                            "AF {} · RF {} · PWR {}",
                            snapshot.af_gain.map_or("—".to_string(), |value| value.to_string()),
                            snapshot.rf_gain.map_or("—".to_string(), |value| value.to_string()),
                            snapshot.rf_power.map_or("—".to_string(), |value| value.to_string()),
                        ))
                        .small()
                        .monospace()
                        .color(Color32::GRAY),
                    )
                    .on_hover_text("Current values reported by the radio");
                    ui.separator();
                    ui.label(
                        RichText::new(format!(
                            "RX {} · TX {} Hz",
                            self.rx_tone_hz, self.tx_tone_hz
                        ))
                        .monospace()
                        .color(Color32::from_rgb(135, 220, 145)),
                    );
                    let monitor_label = if self.config.audio.monitor_enabled {
                        "🎧 RX MONITOR ON"
                    } else {
                        "🎧 RX MONITOR OFF"
                    };
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
                    let old_monitor_output = self.config.audio.monitor_output_device.clone();
                    egui::ComboBox::from_id_salt("top_audio_monitor_output")
                        .selected_text(
                            self.config
                                .audio
                                .monitor_output_device
                                .as_deref()
                                .or(self.config.audio.output_device.as_deref())
                                .unwrap_or("Audio output"),
                        )
                        .width(180.0)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.config.audio.monitor_output_device,
                                None,
                                "Audio output",
                            );
                            for name in &self.audio_output_devices {
                                ui.selectable_value(
                                    &mut self.config.audio.monitor_output_device,
                                    Some(name.clone()),
                                    name,
                                );
                            }
                        });
                    if old_monitor_output != self.config.audio.monitor_output_device {
                        self.profile_dirty = true;
                        self.persist_profile("Audio monitor output saved to");
                        self.restart_audio();
                    }
                    let mut monitor_percent =
                        (self.config.audio.monitor_volume.clamp(0.0, 2.0) * 100.0).round() as u16;
                    if ui
                        .add(
                            egui::DragValue::new(&mut monitor_percent)
                                .range(0..=200)
                                .speed(1)
                                .suffix("%"),
                        )
                        .on_hover_text("RX monitor volume · applies immediately")
                        .changed()
                    {
                        self.config.audio.monitor_volume = f32::from(monitor_percent) / 100.0;
                        self.monitor_volume.store(
                            self.config.audio.monitor_volume.to_bits(),
                            Ordering::Relaxed,
                        );
                        self.profile_dirty = true;
                        self.persist_profile("RX monitor volume saved to");
                    }
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
                self.draw_banner_radio_controls(ui, &snapshot);
            });

        let supports_radio_scope =
            native_radio_profile(&self.config.radio.backend, &self.config.radio.model)
                .is_some_and(|profile| profile.capabilities.spectrum);
        let radio_scope_visible = self.civ_spectrum_on
            && supports_radio_scope
            && !snapshot.radio_waterfall_status.starts_with("UNAVAILABLE");
        let monitor_min_height = 170.0;
        let monitor_max_height = 560.0_f32.min(ctx.content_rect().height() * 0.75).max(170.0);
        self.waterfall_deck_height = self
            .waterfall_deck_height
            .clamp(monitor_min_height, monitor_max_height);
        let waterfall_panel_id = egui::Id::new("waterfall_monitor_deck");
        let previous_deck_height = self.waterfall_deck_height;
        egui::TopBottomPanel::top(waterfall_panel_id)
            .resizable(true)
            .show_separator_line(true)
            .default_height(self.waterfall_deck_height)
            .height_range(monitor_min_height..=monitor_max_height)
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("Waterfalls").strong());
                    let wf_color = if snapshot.radio_spectrum_enabled {
                        Color32::LIGHT_GREEN
                    } else if self.civ_spectrum_on {
                        theme_warning(ui)
                    } else {
                        Color32::GRAY
                    };
                    ui.label(
                        RichText::new(&snapshot.radio_waterfall_status)
                            .small()
                            .color(wf_color),
                    );
                    ui.label(
                        RichText::new("drag lower edge to resize")
                            .small()
                            .color(Color32::GRAY),
                    );
                });
                // Own exactly the remainder of the panel. Waterfall controls and
                // images may clip inside this child, but they must never enlarge
                // the parent response and ratchet the saved panel height upward.
                let deck_rect = ui.available_rect_before_wrap();
                ui.allocate_rect(deck_rect, egui::Sense::hover());
                let mut deck_ui = ui.new_child(
                    egui::UiBuilder::new()
                        .id_salt("waterfall_deck_contents")
                        .max_rect(deck_rect)
                        .layout(egui::Layout::top_down(egui::Align::Min)),
                );
                deck_ui.set_clip_rect(deck_rect);
                if radio_scope_visible {
                    let total_width = deck_ui.available_width();
                    let radio_default_width = total_width * 0.5;
                    let radio_max_width = (total_width - 260.0).max(280.0);
                    egui::SidePanel::left("radio_waterfall_split")
                        .resizable(true)
                        .default_width(radio_default_width)
                        .width_range(280.0..=radio_max_width)
                        .show_inside(&mut deck_ui, |ui| {
                            self.draw_radio_waterfall(ui, ctx, &snapshot);
                        });
                    self.draw_audio_waterfall(&mut deck_ui, ctx, &snapshot);
                } else {
                    self.draw_audio_waterfall(&mut deck_ui, ctx, &snapshot);
                }
            });
        let actual_deck_height = egui::containers::panel::PanelState::load(ctx, waterfall_panel_id)
            .map(|state| state.size().y)
            .unwrap_or(previous_deck_height)
            .clamp(monitor_min_height, monitor_max_height);
        if (actual_deck_height - previous_deck_height).abs() > 0.5 {
            self.waterfall_deck_height = actual_deck_height;
            self.profile_dirty = true;
            self.waterfall_deck_resize_pending = true;
        }
        // Native panel state owns live dragging. Persist only on release so
        // profile I/O never fights the pointer or forces an old height back.
        if self.waterfall_deck_resize_pending && ctx.input(|input| input.pointer.any_released()) {
            self.waterfall_deck_resize_pending = false;
            self.persist_profile("Auto-saved");
        }

        // Bottom panels are stacked in declaration order: the first one owns
        // the outermost bottom strip. Declare the compact status strip first
        // so it remains below the resizable contact log.
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
                        for (tab, label) in [
                            (SignalPanelTab::Achievements, "ACHIEVEMENTS"),
                            (SignalPanelTab::Profile, "PROFILE"),
                            (SignalPanelTab::Contest, "CONTEST"),
                            (SignalPanelTab::Reporting, "REPORTING"),
                            (SignalPanelTab::Waterfall, "WATERFALL"),
                            (SignalPanelTab::Settings, "SETTINGS"),
                            (SignalPanelTab::Server, "SERVER"),
                            (SignalPanelTab::RadioTuning, "RADIO TUNING"),
                            (SignalPanelTab::AppLog, "APP LOG"),
                        ] {
                            let selected = self.signal_panel_tab == tab;
                            let text = if selected {
                                RichText::new(label)
                                    .strong()
                                    .color(Color32::from_rgb(120, 225, 255))
                            } else {
                                RichText::new(label).color(Color32::GRAY)
                            };
                            if ui.selectable_label(selected, text).clicked() {
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
                                SignalPanelTab::Profile => self.draw_profile_panel(ui),
                                SignalPanelTab::Contest => self.draw_contest_panel(ui),
                                SignalPanelTab::Reporting => self.draw_reporting_panel(ui),
                                SignalPanelTab::Waterfall => {
                                    self.draw_waterfall_panel(ui, &snapshot)
                                }
                                SignalPanelTab::Settings => self.draw_settings_panel(ui),
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

        egui::CentralPanel::default().show(ctx, |ui| {
            self.draw_bounded_workspace(ui, ctx, &snapshot);
        });
    }
}

impl Drop for QsonautGuiApp {
    fn drop(&mut self) {
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
        if let Some(handle) = self.radio_worker_handle.take() {
            let _ = handle.join();
        }
        if let Some(handle) = self.audio_worker_handle.take() {
            let _ = handle.join();
        }
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
        // A dense, fast waterfall makes short digital signals easier to tune
        // and preserves the radio's native scope cadence.
        (35, 0)
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

#[cfg(test)]
mod tests {
    use super::*;

    const LEGACY_GUI_SCALE_BASE: f32 = 1.6;

    #[test]
    fn radio_profiles_apply_only_to_the_native_backend() {
        assert_eq!(
            native_radio_profile("native", "IC-7300").map(|profile| profile.model),
            Some("IC-7300")
        );
        assert!(native_radio_profile("rigctld", "IC-7300").is_none());
        assert!(native_radio_profile("null", "IC-7300").is_none());
    }

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
        assert_eq!(effective_visual_profile(&tuning, "USB-D"), (35, 0));
        assert_eq!(effective_visual_profile(&tuning, "FT8"), (35, 0));
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
    fn gui_scale_baseline_is_rebased_so_legacy_75_is_now_100() {
        let legacy_75 = LEGACY_GUI_SCALE_BASE * 0.75;
        assert!((gui_scale_percent(legacy_75) - 100.0).abs() < 0.01);
        assert!((gui_scale_from_percent(100) - legacy_75).abs() < 0.001);
    }

    #[test]
    fn gui_scale_percent_mapping_clamps_to_supported_range() {
        assert_eq!(gui_scale_from_percent(10), GUI_SCALE_MIN);
        assert_eq!(gui_scale_from_percent(500), GUI_SCALE_MAX);
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
    fn workspace_mode_supports_native_tx_matches_current_backends() {
        assert!(workspace_mode_supports_native_tx(WorkspaceMode::Ft4));
        assert!(workspace_mode_supports_native_tx(WorkspaceMode::Jt9));
        assert!(workspace_mode_supports_native_tx(WorkspaceMode::Cw));
        assert!(workspace_mode_supports_native_tx(WorkspaceMode::Sstv));
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
