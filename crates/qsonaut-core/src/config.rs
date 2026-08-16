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
    pub sample_rate_hz: u32,
    pub channels: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RadioConfig {
    pub enabled: bool,
    pub backend: String,
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
                sample_rate_hz: 48_000,
                channels: 1,
            },
            radio: RadioConfig {
                enabled: true,
                backend: "none".to_string(),
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
            cfg.audio.input_device = Some(device);
        }
        if let Ok(device) = std::env::var("QSONAUT_AUDIO_OUTPUT_DEVICE") {
            cfg.audio.output_device = Some(device);
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

fn default_radio_baud_rate() -> u32 {
    115_200
}

fn default_radio_model() -> String {
    "IC-7300".to_string()
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
    }

    #[test]
    fn app_config_serializes_contest_profile_section() {
        let cfg = AppConfig::default();
        let body = toml::to_string(&cfg).expect("serialize config");
        assert!(body.contains("[contest]"));
        assert!(body.contains("serial_start = 1"));
        assert!(body.contains("dupe_check = true"));
    }
}
