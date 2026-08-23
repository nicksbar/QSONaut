use std::{fs, path::PathBuf};

use anyhow::Result;
use qsonaut_accelerate::ComputePreference;
use qsonaut_core::{ContestOperatingMode, FoxHoundRole, ServerConfig, SplitPolicy};
use qsonaut_log::app_config_dir;
use serde::{Deserialize, Serialize};

use super::{
    default_true,
    modes::exchange::{AutoReplyPolicy, DEFAULT_PTT_LEAD_SECONDS, MAX_ATTEMPTS_PER_EXCHANGE},
    AchievementKind, CustomAchievementRule, RadioScopeView, WaterfallTheme, GUI_SCALE_BASE,
};

pub(super) const OPERATOR_PROFILE_FILE: &str = "profile.toml";
pub(super) const OPERATOR_PROFILE_VERSION: u32 = 13;
const LEGACY_OPERATOR_PROFILE_FILE: &str = ".rigforge_profile.toml";
const DEFAULT_PROFILE_NAME: &str = "Default";
const OPERATOR_PROFILES_DIR: &str = "profiles";
const ACTIVE_PROFILE_FILE: &str = "active-profile";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct OperatorProfile {
    #[serde(default)]
    pub(super) profile_version: u32,
    pub(super) callsign: String,
    pub(super) grid: String,
    pub(super) qth: String,
    #[serde(default)]
    pub(super) station_rig: String,
    #[serde(default)]
    pub(super) station_antenna: String,
    #[serde(default)]
    pub(super) station_notes: String,
    #[serde(default)]
    pub(super) llm_prompt_context: String,
    #[serde(default)]
    pub(super) sstv_image_requirements: String,
    #[serde(default)]
    pub(super) llm_model_notes: String,
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
    pub(super) automation_unlocked: bool,
    #[serde(default)]
    pub(super) cq_only_view: bool,
    #[serde(default)]
    pub(super) civ_spectrum_on: bool,
    #[serde(default)]
    pub(super) radio_scope_vbw_wide: bool,
    #[serde(default)]
    pub(super) radio_scope_view: RadioScopeView,
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
    #[serde(default = "default_cw_wpm")]
    pub(super) cw_wpm: u8,
    #[serde(default = "default_cw_tone_hz")]
    pub(super) cw_tone_hz: u16,
    #[serde(default)]
    pub(super) audio_input_device: Option<String>,
    #[serde(default)]
    pub(super) audio_output_device: Option<String>,
    #[serde(default)]
    pub(super) audio_monitor_enabled: bool,
    #[serde(default)]
    pub(super) audio_monitor_output_device: Option<String>,
    #[serde(default = "default_audio_monitor_volume")]
    pub(super) audio_monitor_volume: f32,
    #[serde(default)]
    pub(super) radio_serial_port: Option<String>,
    #[serde(default = "default_radio_backend")]
    pub(super) radio_backend: String,
    #[serde(default = "default_radio_endpoint")]
    pub(super) radio_endpoint: String,
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
    #[serde(default = "default_psk_batch_interval_secs")]
    pub(super) psk_batch_interval_secs: u64,
    #[serde(default = "default_psk_repeat_cache_secs")]
    pub(super) psk_repeat_cache_secs: u64,
    #[serde(default = "default_psk_max_pending")]
    pub(super) psk_max_pending: usize,
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
    pub(super) hunter_acknowledged: Vec<AchievementKind>,
    #[serde(default = "default_true")]
    pub(super) hunter_alerts_enabled: bool,
    #[serde(default)]
    pub(super) hunter_custom_rules: Vec<CustomAchievementRule>,
    #[serde(default)]
    pub(super) radio_profiles: Vec<RadioProfile>,
    #[serde(default)]
    pub(super) mode_radio_profile: std::collections::BTreeMap<String, String>,
    #[serde(default = "default_workspace_mode")]
    pub(super) workspace_mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct RadioProfile {
    pub(super) name: String,
    #[serde(default)]
    pub(super) mode: Option<String>,
    #[serde(default)]
    pub(super) data_mode: Option<bool>,
    #[serde(default)]
    pub(super) filter: Option<u8>,
    #[serde(default)]
    pub(super) af_gain: Option<u8>,
    #[serde(default)]
    pub(super) rf_gain: Option<u8>,
    #[serde(default)]
    pub(super) rf_power: Option<u8>,
    #[serde(default)]
    pub(super) preamp: Option<bool>,
    #[serde(default)]
    pub(super) attenuator: Option<bool>,
    #[serde(default)]
    pub(super) noise_blank: Option<bool>,
    #[serde(default)]
    pub(super) noise_reduction: Option<bool>,
    #[serde(default)]
    pub(super) agc: Option<u8>,
}

pub(super) fn default_gui_scale() -> f32 {
    GUI_SCALE_BASE
}

fn default_workspace_mode() -> String {
    "FT8".to_string()
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
    0
}

pub(super) fn default_cw_wpm() -> u8 {
    20
}

pub(super) fn default_cw_tone_hz() -> u16 {
    600
}

fn default_audio_monitor_volume() -> f32 {
    1.0
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

fn default_radio_backend() -> String {
    "native".to_string()
}

fn default_radio_endpoint() -> String {
    "127.0.0.1:4532".to_string()
}

pub(super) fn default_psk_batch_interval_secs() -> u64 {
    300
}

pub(super) fn default_psk_repeat_cache_secs() -> u64 {
    300
}

pub(super) fn default_psk_max_pending() -> usize {
    80
}

fn default_contest_serial_current() -> u32 {
    default_contest_serial_start()
}

fn operator_profile_path() -> PathBuf {
    app_config_dir().join(OPERATOR_PROFILE_FILE)
}

fn operator_profiles_dir() -> PathBuf {
    app_config_dir().join(OPERATOR_PROFILES_DIR)
}

fn active_profile_path() -> PathBuf {
    app_config_dir().join(ACTIVE_PROFILE_FILE)
}

fn validate_profile_name(name: &str) -> Result<&str> {
    let name = name.trim();
    anyhow::ensure!(!name.is_empty(), "profile name cannot be empty");
    anyhow::ensure!(
        name.chars()
            .all(|character| character.is_ascii_alphanumeric()
                || matches!(character, ' ' | '-' | '_')),
        "profile names may contain only letters, numbers, spaces, '-' and '_'"
    );
    Ok(name)
}

fn named_operator_profile_path(name: &str) -> Result<PathBuf> {
    let name = validate_profile_name(name)?;
    if name.eq_ignore_ascii_case(DEFAULT_PROFILE_NAME) {
        Ok(operator_profile_path())
    } else {
        Ok(operator_profiles_dir().join(format!("{name}.toml")))
    }
}

pub(super) fn active_operator_profile_name() -> String {
    fs::read_to_string(active_profile_path())
        .ok()
        .and_then(|name| validate_profile_name(&name).ok().map(str::to_string))
        .unwrap_or_else(|| DEFAULT_PROFILE_NAME.to_string())
}

pub(super) fn select_operator_profile(name: &str) -> Result<()> {
    let name = validate_profile_name(name)?;
    anyhow::ensure!(
        named_operator_profile_path(name)?.is_file(),
        "profile ‘{name}’ does not exist"
    );
    let path = active_profile_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, name)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

pub(super) fn list_operator_profiles() -> Vec<String> {
    let mut profiles = vec![DEFAULT_PROFILE_NAME.to_string()];
    if let Ok(entries) = fs::read_dir(operator_profiles_dir()) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("toml") {
                continue;
            }
            if let Some(name) = path.file_stem().and_then(|name| name.to_str()) {
                if validate_profile_name(name).is_ok() {
                    profiles.push(name.to_string());
                }
            }
        }
    }
    profiles.sort_by_key(|name| name.to_ascii_lowercase());
    profiles.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
    profiles
}

pub(super) fn load_operator_profile_named(name: &str) -> Option<OperatorProfile> {
    let path = named_operator_profile_path(name).ok()?;
    let source = fs::read_to_string(path).ok()?;
    toml::from_str(&source).ok()
}

pub(super) fn load_operator_profile() -> Option<OperatorProfile> {
    let selected = active_operator_profile_name();
    if let Some(profile) = load_operator_profile_named(&selected) {
        return Some(profile);
    }
    let legacy = std::env::current_dir()
        .ok()
        .map(|directory| directory.join(LEGACY_OPERATOR_PROFILE_FILE));
    let source = fs::read_to_string(operator_profile_path())
        .ok()
        .or_else(|| legacy.and_then(|path| fs::read_to_string(path).ok()))?;
    toml::from_str(&source).ok()
}

pub(super) fn save_operator_profile_named(name: &str, profile: &OperatorProfile) -> Result<()> {
    let name = validate_profile_name(name)?;
    let path = named_operator_profile_path(name)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, toml::to_string_pretty(profile)?)?;
    fs::write(active_profile_path(), name)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        fs::set_permissions(active_profile_path(), fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

pub(super) fn save_operator_profile(profile: &OperatorProfile) -> Result<()> {
    save_operator_profile_named(&active_operator_profile_name(), profile)
}

#[cfg(test)]
mod tests {
    use super::validate_profile_name;

    #[test]
    fn profile_names_allow_human_readable_safe_names() {
        assert_eq!(
            validate_profile_name(" Field Day 2026 ").unwrap(),
            "Field Day 2026"
        );
        assert!(validate_profile_name("portable_vhf-2").is_ok());
    }

    #[test]
    fn profile_names_reject_paths_and_empty_values() {
        assert!(validate_profile_name("").is_err());
        assert!(validate_profile_name("../other").is_err());
        assert!(validate_profile_name("club/profile").is_err());
    }
}
