use std::{fs, path::PathBuf};

use anyhow::Result;
use qsonaut_accelerate::ComputePreference;
use qsonaut_core::{ContestOperatingMode, FoxHoundRole, ServerConfig, SplitPolicy};
use qsonaut_log::app_config_dir;
use serde::{Deserialize, Serialize};

use super::{
    ft8_ops::{AutoReplyPolicy, DEFAULT_PTT_LEAD_SECONDS, MAX_ATTEMPTS_PER_EXCHANGE},
    AchievementKind, CustomAchievementRule, WaterfallTheme, GUI_SCALE_BASE,
};

pub(super) const OPERATOR_PROFILE_FILE: &str = "profile.toml";
pub(super) const OPERATOR_PROFILE_VERSION: u32 = 9;
const LEGACY_OPERATOR_PROFILE_FILE: &str = ".rigforge_profile.toml";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct OperatorProfile {
    #[serde(default)]
    pub(super) profile_version: u32,
    pub(super) callsign: String,
    pub(super) grid: String,
    pub(super) qth: String,
    pub(super) follow_log: bool,
    pub(super) max_log_entries: usize,
    pub(super) deep_decode: bool,
    #[serde(default)]
    pub(super) ft4_deep_decode: bool,
    #[serde(default)]
    pub(super) ft4_autoseq: bool,
    #[serde(default)]
    pub(super) ft4_auto_reply_policy: AutoReplyPolicy,
    #[serde(default)]
    pub(super) ft4_cq_only_view: bool,
    #[serde(default = "default_follow_log")]
    pub(super) ft4_follow_log: bool,
    #[serde(default = "default_max_log_entries")]
    pub(super) ft4_max_log_entries: usize,
    #[serde(default = "default_max_attempts")]
    pub(super) ft4_max_attempts: u8,
    #[serde(default)]
    pub(super) autoseq: bool,
    #[serde(default)]
    pub(super) auto_reply_policy: AutoReplyPolicy,
    #[serde(default)]
    pub(super) auto_answer_cq: bool,
    #[serde(default)]
    pub(super) cq_only_view: bool,
    #[serde(default)]
    pub(super) civ_spectrum_on: bool,
    #[serde(default)]
    pub(super) waterfall_theme: WaterfallTheme,
    #[serde(default = "default_waterfall_deck_height")]
    pub(super) waterfall_deck_height: f32,
    #[serde(default)]
    pub(super) halt_after_tx: bool,
    #[serde(default = "default_max_attempts")]
    pub(super) ft8_max_attempts: u8,
    #[serde(default)]
    pub(super) hold_tx_freq: bool,
    #[serde(default = "default_rx_tone_hz")]
    pub(super) rx_tone_hz: u32,
    #[serde(default = "default_tx_tone_hz")]
    pub(super) tx_tone_hz: u32,
    #[serde(default = "default_ptt_lead_ms")]
    pub(super) ptt_lead_ms: u64,
    #[serde(default = "default_ptt_tail_ms")]
    pub(super) ptt_tail_ms: u64,
    #[serde(default)]
    pub(super) audio_input_device: Option<String>,
    #[serde(default)]
    pub(super) audio_output_device: Option<String>,
    #[serde(default)]
    pub(super) radio_serial_port: Option<String>,
    #[serde(default = "default_radio_model")]
    pub(super) radio_model: String,
    #[serde(default = "default_radio_baud_rate")]
    pub(super) radio_baud_rate: u32,
    #[serde(default = "default_gui_scale")]
    pub(super) gui_scale: f32,
    #[serde(default)]
    pub(super) compute_preference: ComputePreference,
    #[serde(default)]
    pub(super) psk_reporter_enabled: bool,
    #[serde(default)]
    pub(super) server_instance_id: String,
    #[serde(default)]
    pub(super) server: Option<ServerConfig>,
    #[serde(default)]
    pub(super) contest_enabled: bool,
    #[serde(default)]
    pub(super) contest_operating_mode: ContestOperatingMode,
    #[serde(default)]
    pub(super) contest_split_policy: SplitPolicy,
    #[serde(default)]
    pub(super) contest_fox_hound_role: FoxHoundRole,
    #[serde(default)]
    pub(super) contest_exchange_template: String,
    #[serde(default = "default_contest_serial_start")]
    pub(super) contest_serial_start: u32,
    #[serde(default = "default_contest_serial_step")]
    pub(super) contest_serial_step: u32,
    #[serde(default = "default_contest_dupe_check")]
    pub(super) contest_dupe_check: bool,
    #[serde(default = "default_contest_serial_current")]
    pub(super) contest_serial_current: u32,
    #[serde(default = "default_contest_fake_split_offset_hz")]
    pub(super) contest_fake_split_offset_hz: u32,
    #[serde(default)]
    pub(super) hunter_unlocked: Vec<AchievementKind>,
    #[serde(default)]
    pub(super) hunter_custom_rules: Vec<CustomAchievementRule>,
}

pub(super) fn default_gui_scale() -> f32 {
    GUI_SCALE_BASE
}

pub(super) fn default_waterfall_deck_height() -> f32 {
    320.0
}

pub(super) fn default_rx_tone_hz() -> u32 {
    1_500
}

pub(super) fn default_tx_tone_hz() -> u32 {
    1_500
}

pub(super) fn default_max_attempts() -> u8 {
    MAX_ATTEMPTS_PER_EXCHANGE
}

pub(super) fn default_ptt_lead_ms() -> u64 {
    (DEFAULT_PTT_LEAD_SECONDS * 1_000.0) as u64
}

pub(super) fn default_ptt_tail_ms() -> u64 {
    100
}

pub(super) fn default_contest_serial_start() -> u32 {
    1
}

pub(super) fn default_contest_serial_step() -> u32 {
    1
}

pub(super) fn default_contest_dupe_check() -> bool {
    true
}

pub(super) fn default_contest_fake_split_offset_hz() -> u32 {
    250
}

fn default_follow_log() -> bool {
    true
}

fn default_max_log_entries() -> usize {
    300
}

fn default_radio_model() -> String {
    "IC-7300".to_string()
}

fn default_radio_baud_rate() -> u32 {
    115_200
}

fn default_contest_serial_current() -> u32 {
    default_contest_serial_start()
}

fn operator_profile_path() -> PathBuf {
    app_config_dir().join(OPERATOR_PROFILE_FILE)
}

pub(super) fn load_operator_profile() -> Option<OperatorProfile> {
    let preferred = operator_profile_path();
    let legacy = std::env::current_dir()
        .ok()
        .map(|directory| directory.join(LEGACY_OPERATOR_PROFILE_FILE));
    let source = fs::read_to_string(&preferred)
        .ok()
        .or_else(|| legacy.and_then(|path| fs::read_to_string(path).ok()))?;
    toml::from_str(&source).ok()
}

pub(super) fn save_operator_profile(profile: &OperatorProfile) -> Result<()> {
    let path = operator_profile_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, toml::to_string_pretty(profile)?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}
