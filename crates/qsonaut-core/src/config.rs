use std::{fs, path::Path};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub station: StationConfig,
    pub audio: AudioConfig,
    pub radio: RadioConfig,
    #[serde(default)]
    pub server: ServerConfig,
    #[serde(default)]
    pub contest: ContestProfile,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StationConfig {
    pub callsign: Option<String>,
    pub grid: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioConfig {
    pub enabled: bool,
    pub input_device: Option<String>,
    #[serde(default)]
    pub output_device: Option<String>,
    #[serde(default)]
    pub monitor_enabled: bool,
    #[serde(default)]
    pub monitor_output_device: Option<String>,
    #[serde(default = "default_monitor_volume")]
    pub monitor_volume: f32,
    pub sample_rate_hz: u32,
    pub channels: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RadioConfig {
    pub enabled: bool,
    pub backend: String,
    #[serde(default = "default_radio_endpoint")]
    pub endpoint: String,
    #[serde(default = "default_radio_model")]
    pub model: String,
    pub serial_port: Option<String>,
    #[serde(default = "default_radio_baud_rate")]
    pub baud_rate: u32,
    #[serde(default = "default_radio_civ_address")]
    pub civ_address: u8,
    #[serde(default = "default_controller_civ_address")]
    pub controller_civ_address: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ServerConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub device_token: String,
    #[serde(default)]
    pub share_presence: bool,
    #[serde(default)]
    pub share_radio_details: bool,
    #[serde(default)]
    pub share_logs: bool,
    #[serde(default)]
    pub share_diagnostics: bool,
    /// Include a bounded, redacted recent application-log excerpt in manual
    /// diagnostic snapshots. This never enables automatic uploads.
    #[serde(default)]
    pub share_debug_logs: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum ContestOperatingMode {
    #[default]
    Run,
    SearchAndPounce,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum SplitPolicy {
    #[default]
    Off,
    Fake,
    Rig,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum FoxHoundRole {
    #[default]
    Disabled,
    Fox,
    Hound,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContestProfile {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub operating_mode: ContestOperatingMode,
    #[serde(default)]
    pub split_policy: SplitPolicy,
    #[serde(default)]
    pub fox_hound_role: FoxHoundRole,
    #[serde(default)]
    pub exchange_template: Option<String>,
    #[serde(default = "default_serial_start")]
    pub serial_start: u32,
    #[serde(default = "default_serial_step")]
    pub serial_step: u32,
    #[serde(default = "default_dupe_check")]
    pub dupe_check: bool,
}

impl Default for ContestProfile {
    fn default() -> Self {
        Self {
            enabled: false,
            operating_mode: ContestOperatingMode::Run,
            split_policy: SplitPolicy::Off,
            fox_hound_role: FoxHoundRole::Disabled,
            exchange_template: None,
            serial_start: default_serial_start(),
            serial_step: default_serial_step(),
            dupe_check: default_dupe_check(),
        }
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            station: StationConfig {
                callsign: None,
                grid: None,
            },
            audio: AudioConfig {
                enabled: true,
                input_device: None,
                output_device: None,
                monitor_enabled: false,
                monitor_output_device: None,
                monitor_volume: default_monitor_volume(),
                sample_rate_hz: 48_000,
                channels: 1,
            },
            radio: RadioConfig {
                enabled: true,
                backend: "native".to_string(),
                endpoint: default_radio_endpoint(),
                model: default_radio_model(),
                serial_port: None,
                baud_rate: default_radio_baud_rate(),
                civ_address: default_radio_civ_address(),
                controller_civ_address: default_controller_civ_address(),
            },
            server: ServerConfig::default(),
            contest: ContestProfile::default(),
        }
    }
}

fn default_monitor_volume() -> f32 {
    1.0
}

impl AppConfig {
    pub fn load(path: Option<&Path>) -> Result<Self> {
        let _ = dotenvy::dotenv();

        let mut cfg = if let Some(path) = path {
            let src = fs::read_to_string(path)
                .with_context(|| format!("failed reading config file {}", path.display()))?;
            toml::from_str::<AppConfig>(&src)
                .with_context(|| format!("failed parsing config file {}", path.display()))?
        } else {
            Self::default()
        };

        if let Ok(callsign) = std::env::var("QSONAUT_STATION_CALLSIGN") {
            cfg.station.callsign = Some(callsign);
        }
        if let Ok(grid) = std::env::var("QSONAUT_STATION_GRID") {
            cfg.station.grid = Some(grid);
        }
        if let Ok(enabled) = std::env::var("QSONAUT_CONTEST_ENABLED") {
            cfg.contest.enabled = parse_bool(&enabled);
        }

        if let Ok(enabled) = std::env::var("QSONAUT_SERVER_ENABLED") {
            cfg.server.enabled = parse_bool(&enabled);
        }
        if let Ok(url) = std::env::var("QSONAUT_SERVER_URL") {
            cfg.server.url = url;
        }
        if let Ok(token) = std::env::var("QSONAUT_SERVER_DEVICE_TOKEN") {
            cfg.server.device_token = token;
        }
        if let Ok(enabled) = std::env::var("QSONAUT_SERVER_SHARE_PRESENCE") {
            cfg.server.share_presence = parse_bool(&enabled);
        }
        if let Ok(enabled) = std::env::var("QSONAUT_SERVER_SHARE_RADIO_DETAILS") {
            cfg.server.share_radio_details = parse_bool(&enabled);
        }
        if let Ok(enabled) = std::env::var("QSONAUT_SERVER_SHARE_LOGS") {
            cfg.server.share_logs = parse_bool(&enabled);
        }
        if let Ok(enabled) = std::env::var("QSONAUT_SERVER_SHARE_DIAGNOSTICS") {
            cfg.server.share_diagnostics = parse_bool(&enabled);
        }
        if let Ok(enabled) = std::env::var("QSONAUT_SERVER_SHARE_DEBUG_LOGS") {
            cfg.server.share_debug_logs = parse_bool(&enabled);
        }
        if let Ok(mode) = std::env::var("QSONAUT_CONTEST_OPERATING_MODE") {
            if let Some(parsed) = parse_contest_operating_mode(&mode) {
                cfg.contest.operating_mode = parsed;
            }
        }
        if let Ok(split) = std::env::var("QSONAUT_CONTEST_SPLIT_POLICY") {
            if let Some(parsed) = parse_split_policy(&split) {
                cfg.contest.split_policy = parsed;
            }
        }
        if let Ok(role) = std::env::var("QSONAUT_CONTEST_FOX_HOUND_ROLE") {
            if let Some(parsed) = parse_fox_hound_role(&role) {
                cfg.contest.fox_hound_role = parsed;
            }
        }
        if let Ok(template) = std::env::var("QSONAUT_CONTEST_EXCHANGE_TEMPLATE") {
            let trimmed = template.trim();
            cfg.contest.exchange_template = if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            };
        }
        if let Ok(start) = std::env::var("QSONAUT_CONTEST_SERIAL_START") {
            if let Ok(parsed) = start.parse::<u32>() {
                cfg.contest.serial_start = parsed.max(1);
            }
        }
        if let Ok(step) = std::env::var("QSONAUT_CONTEST_SERIAL_STEP") {
            if let Ok(parsed) = step.parse::<u32>() {
                cfg.contest.serial_step = parsed.max(1);
            }
        }
        if let Ok(dupe_check) = std::env::var("QSONAUT_CONTEST_DUPE_CHECK") {
            cfg.contest.dupe_check = parse_bool(&dupe_check);
        }

        if let Ok(enabled) = std::env::var("QSONAUT_AUDIO_ENABLED") {
            cfg.audio.enabled = parse_bool(&enabled);
        }
        if let Ok(device) = std::env::var("QSONAUT_AUDIO_INPUT_DEVICE") {
            cfg.audio.input_device = nonempty(device);
        }
        if let Ok(device) = std::env::var("QSONAUT_AUDIO_OUTPUT_DEVICE") {
            cfg.audio.output_device = nonempty(device);
        }
        if let Ok(enabled) = std::env::var("QSONAUT_AUDIO_MONITOR_ENABLED") {
            cfg.audio.monitor_enabled = parse_bool(&enabled);
        }
        if let Ok(device) = std::env::var("QSONAUT_AUDIO_MONITOR_OUTPUT_DEVICE") {
            cfg.audio.monitor_output_device = nonempty(device);
        }
        if let Ok(volume) = std::env::var("QSONAUT_AUDIO_MONITOR_VOLUME") {
            if let Ok(parsed) = volume.parse::<f32>() {
                cfg.audio.monitor_volume = parsed.clamp(0.0, 2.0);
            }
        }
        if let Ok(rate) = std::env::var("QSONAUT_AUDIO_SAMPLE_RATE_HZ") {
            if let Ok(parsed) = rate.parse::<u32>() {
                cfg.audio.sample_rate_hz = parsed;
            }
        }
        if let Ok(channels) = std::env::var("QSONAUT_AUDIO_CHANNELS") {
            if let Ok(parsed) = channels.parse::<u8>() {
                cfg.audio.channels = parsed;
            }
        }

        if let Ok(enabled) = std::env::var("QSONAUT_RADIO_ENABLED") {
            cfg.radio.enabled = parse_bool(&enabled);
        }
        if let Ok(backend) = std::env::var("QSONAUT_RADIO_BACKEND") {
            cfg.radio.backend = backend;
        }
        // `none` was an internal placeholder. Radio enablement is controlled
        // by `radio.enabled`; migrate older configurations to the real
        // default backend instead of silently disabling radio control.
        if cfg.radio.backend.trim().eq_ignore_ascii_case("none") {
            cfg.radio.backend = "native".to_string();
        }
        if let Ok(endpoint) = std::env::var("QSONAUT_RADIO_ENDPOINT") {
            cfg.radio.endpoint = endpoint;
        }
        if let Ok(model) = std::env::var("QSONAUT_RADIO_MODEL") {
            cfg.radio.model = model;
        }
        if let Ok(port) = std::env::var("QSONAUT_RADIO_SERIAL_PORT") {
            cfg.radio.serial_port = Some(port);
        }
        if let Ok(baud) = std::env::var("QSONAUT_RADIO_BAUD_RATE") {
            if let Ok(parsed) = baud.parse::<u32>() {
                cfg.radio.baud_rate = parsed;
            }
        }
        if let Ok(addr) = std::env::var("QSONAUT_RADIO_CIV_ADDRESS") {
            if let Some(parsed) = parse_u8_flexible(&addr) {
                cfg.radio.civ_address = parsed;
            }
        }
        if let Ok(addr) = std::env::var("QSONAUT_RADIO_CONTROLLER_CIV_ADDRESS") {
            if let Some(parsed) = parse_u8_flexible(&addr) {
                cfg.radio.controller_civ_address = parsed;
            }
        }

        Ok(cfg)
    }
}

fn parse_bool(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn nonempty(value: String) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn default_radio_baud_rate() -> u32 {
    115_200
}

fn default_radio_model() -> String {
    "IC-7300".to_string()
}

fn default_radio_endpoint() -> String {
    "127.0.0.1:4532".to_string()
}

fn default_radio_civ_address() -> u8 {
    0x94
}

fn default_controller_civ_address() -> u8 {
    0xE0
}

fn default_serial_start() -> u32 {
    1
}

fn default_serial_step() -> u32 {
    1
}

fn default_dupe_check() -> bool {
    true
}

fn parse_contest_operating_mode(value: &str) -> Option<ContestOperatingMode> {
    match value.trim().to_ascii_lowercase().as_str() {
        "run" => Some(ContestOperatingMode::Run),
        "snp" | "search" | "search_and_pounce" | "search-and-pounce" => {
            Some(ContestOperatingMode::SearchAndPounce)
        }
        _ => None,
    }
}

fn parse_split_policy(value: &str) -> Option<SplitPolicy> {
    match value.trim().to_ascii_lowercase().as_str() {
        "off" | "none" => Some(SplitPolicy::Off),
        "fake" | "fake_split" | "fake-split" => Some(SplitPolicy::Fake),
        "rig" | "rig_split" | "rig-split" => Some(SplitPolicy::Rig),
        _ => None,
    }
}

fn parse_fox_hound_role(value: &str) -> Option<FoxHoundRole> {
    match value.trim().to_ascii_lowercase().as_str() {
        "disabled" | "off" | "none" => Some(FoxHoundRole::Disabled),
        "fox" => Some(FoxHoundRole::Fox),
        "hound" => Some(FoxHoundRole::Hound),
        _ => None,
    }
}

fn parse_u8_flexible(input: &str) -> Option<u8> {
    let raw = input.trim();
    if raw.is_empty() {
        return None;
    }

    if let Ok(v) = raw.parse::<u8>() {
        return Some(v);
    }

    let lower = raw.to_ascii_lowercase();
    if let Some(rest) = lower.strip_prefix("0x") {
        return u8::from_str_radix(rest, 16).ok();
    }
    if let Some(rest) = lower.strip_suffix('h') {
        return u8::from_str_radix(rest, 16).ok();
    }
    u8::from_str_radix(&lower, 16).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contest_profile_defaults_when_missing_from_toml() {
        let src = r#"
[station]
callsign = "N0CALL"
grid = "AA00"

[audio]
enabled = true
sample_rate_hz = 48000
channels = 1

[radio]
enabled = true
backend = "none"
"#;

        let cfg: AppConfig = toml::from_str(src).expect("config parse");
        assert_eq!(cfg.contest, ContestProfile::default());
        assert_eq!(cfg.radio.model, "IC-7300");
        assert!(!cfg.audio.monitor_enabled);
        assert_eq!(cfg.audio.monitor_output_device, None);
        assert_eq!(cfg.audio.monitor_volume, 1.0);
    }

    #[test]
    fn app_config_serializes_contest_profile_section() {
        let cfg = AppConfig::default();
        let body = toml::to_string(&cfg).expect("serialize config");
        assert!(body.contains("[contest]"));
        assert!(body.contains("serial_start = 1"));
        assert!(body.contains("dupe_check = true"));
    }

    #[test]
    fn native_radio_is_the_default_backend() {
        assert_eq!(AppConfig::default().radio.backend, "native");
        assert_eq!(AppConfig::default().radio.endpoint, "127.0.0.1:4532");
    }

    #[test]
    fn parses_boolean_and_optional_string_values_conservatively() {
        for value in ["1", "true", "YES", " on "] {
            assert!(parse_bool(value));
        }
        for value in ["0", "false", "no", "off", "maybe", ""] {
            assert!(!parse_bool(value));
        }
        assert_eq!(
            nonempty("  device  ".to_string()),
            Some("device".to_string())
        );
        assert_eq!(nonempty("   ".to_string()), None);
    }

    #[test]
    fn parses_contest_mode_aliases_and_rejects_unknown_values() {
        assert_eq!(
            parse_contest_operating_mode("run"),
            Some(ContestOperatingMode::Run)
        );
        for value in ["snp", "search", "search_and_pounce", "search-and-pounce"] {
            assert_eq!(
                parse_contest_operating_mode(value),
                Some(ContestOperatingMode::SearchAndPounce)
            );
        }
        assert_eq!(parse_contest_operating_mode("other"), None);
    }

    #[test]
    fn parses_split_and_fox_hound_aliases() {
        for value in ["off", "none"] {
            assert_eq!(parse_split_policy(value), Some(SplitPolicy::Off));
        }
        for value in ["fake", "fake_split", "fake-split"] {
            assert_eq!(parse_split_policy(value), Some(SplitPolicy::Fake));
        }
        for value in ["rig", "rig_split", "rig-split"] {
            assert_eq!(parse_split_policy(value), Some(SplitPolicy::Rig));
        }
        assert_eq!(parse_split_policy("invalid"), None);

        for value in ["disabled", "off", "none"] {
            assert_eq!(parse_fox_hound_role(value), Some(FoxHoundRole::Disabled));
        }
        assert_eq!(parse_fox_hound_role("fox"), Some(FoxHoundRole::Fox));
        assert_eq!(parse_fox_hound_role("hound"), Some(FoxHoundRole::Hound));
        assert_eq!(parse_fox_hound_role("invalid"), None);
    }

    #[test]
    fn parses_decimal_and_hex_radio_addresses() {
        assert_eq!(parse_u8_flexible("148"), Some(148));
        assert_eq!(parse_u8_flexible("0x94"), Some(148));
        assert_eq!(parse_u8_flexible("94h"), Some(148));
        assert_eq!(parse_u8_flexible("E0"), Some(224));
        for value in ["", "  ", "0x", "100h", "not-an-address"] {
            assert_eq!(parse_u8_flexible(value), None);
        }
    }

    #[test]
    fn config_defaults_are_stable_and_safe() {
        assert_eq!(default_monitor_volume(), 1.0);
        assert_eq!(default_radio_baud_rate(), 115_200);
        assert_eq!(default_radio_model(), "IC-7300");
        assert_eq!(default_radio_endpoint(), "127.0.0.1:4532");
        assert_eq!(default_radio_civ_address(), 0x94);
        assert_eq!(default_controller_civ_address(), 0xE0);
        assert_eq!(default_serial_start(), 1);
        assert_eq!(default_serial_step(), 1);
        assert!(default_dupe_check());
    }

    #[test]
    fn loads_file_config_and_migrates_legacy_none_backend() {
        let path = std::env::temp_dir().join(format!(
            "qsonaut-core-config-test-{}.toml",
            std::process::id()
        ));
        let src = r#"
[station]
callsign = "N7UF"
grid = "CN84JU"

[audio]
enabled = false
sample_rate_hz = 44100
channels = 2

[radio]
enabled = true
backend = "none"
serial_port = "COM3"
baud_rate = 9600
civ_address = 148
controller_civ_address = 224
"#;
        std::fs::write(&path, src).expect("write isolated config fixture");
        let cfg = AppConfig::load(Some(&path)).expect("load config fixture");
        let _ = std::fs::remove_file(&path);

        assert_eq!(cfg.station.callsign.as_deref(), Some("N7UF"));
        assert!(!cfg.audio.enabled);
        assert_eq!(cfg.audio.sample_rate_hz, 44_100);
        assert_eq!(cfg.audio.channels, 2);
        assert_eq!(cfg.radio.backend, "native");
        assert_eq!(cfg.radio.serial_port.as_deref(), Some("COM3"));
        assert_eq!(cfg.radio.baud_rate, 9_600);
        assert_eq!(cfg.radio.civ_address, 148);
        assert_eq!(cfg.radio.controller_civ_address, 224);
    }

    #[test]
    fn reports_missing_and_invalid_config_files() {
        let missing = std::env::temp_dir().join(format!(
            "qsonaut-core-config-missing-{}.toml",
            std::process::id()
        ));
        let error = AppConfig::load(Some(&missing)).expect_err("missing config must fail");
        assert!(error.to_string().contains("failed reading config file"));

        let invalid = std::env::temp_dir().join(format!(
            "qsonaut-core-config-invalid-{}.toml",
            std::process::id()
        ));
        std::fs::write(&invalid, "not = valid = toml").expect("write invalid fixture");
        let error = AppConfig::load(Some(&invalid)).expect_err("invalid config must fail");
        let _ = std::fs::remove_file(&invalid);
        assert!(error.to_string().contains("failed parsing config file"));
    }
}
