use std::{fs, path::Path};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub station: StationConfig,
    pub audio: AudioConfig,
    pub radio: RadioConfig,
    pub ai: AiConfig,
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
    pub sample_rate_hz: u32,
    pub channels: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RadioConfig {
    pub enabled: bool,
    pub backend: String,
    pub serial_port: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiConfig {
    pub enabled: bool,
    pub provider: String,
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
                sample_rate_hz: 48_000,
                channels: 1,
            },
            radio: RadioConfig {
                enabled: true,
                backend: "none".to_string(),
                serial_port: None,
            },
            ai: AiConfig {
                enabled: false,
                provider: "none".to_string(),
            },
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

        if let Ok(callsign) = std::env::var("RIGFORGE_STATION_CALLSIGN") {
            cfg.station.callsign = Some(callsign);
        }
        if let Ok(grid) = std::env::var("RIGFORGE_STATION_GRID") {
            cfg.station.grid = Some(grid);
        }
        if let Ok(provider) = std::env::var("RIGFORGE_AI_PROVIDER") {
            cfg.ai.provider = provider;
        }
        if let Ok(enabled) = std::env::var("RIGFORGE_AI_ENABLED") {
            cfg.ai.enabled = parse_bool(&enabled);
        }

        if let Ok(enabled) = std::env::var("RIGFORGE_AUDIO_ENABLED") {
            cfg.audio.enabled = parse_bool(&enabled);
        }
        if let Ok(device) = std::env::var("RIGFORGE_AUDIO_INPUT_DEVICE") {
            cfg.audio.input_device = Some(device);
        }
        if let Ok(rate) = std::env::var("RIGFORGE_AUDIO_SAMPLE_RATE_HZ") {
            if let Ok(parsed) = rate.parse::<u32>() {
                cfg.audio.sample_rate_hz = parsed;
            }
        }
        if let Ok(channels) = std::env::var("RIGFORGE_AUDIO_CHANNELS") {
            if let Ok(parsed) = channels.parse::<u8>() {
                cfg.audio.channels = parsed;
            }
        }

        if let Ok(enabled) = std::env::var("RIGFORGE_RADIO_ENABLED") {
            cfg.radio.enabled = parse_bool(&enabled);
        }
        if let Ok(backend) = std::env::var("RIGFORGE_RADIO_BACKEND") {
            cfg.radio.backend = backend;
        }
        if let Ok(port) = std::env::var("RIGFORGE_RADIO_SERIAL_PORT") {
            cfg.radio.serial_port = Some(port);
        }

        Ok(cfg)
    }
}

fn parse_bool(value: &str) -> bool {
    matches!(value.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on")
}
