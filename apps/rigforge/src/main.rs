use std::path::PathBuf;

use anyhow::Result;
use clap::Parser;
use rigforge_ai::NullLanguageModel;
use rigforge_audio::AudioService;
use rigforge_core::{AppConfig, AppEventBus};
use rigforge_radio::NullRadio;
use tracing::{info, warn};

#[derive(Debug, Parser)]
#[command(name = "rigforge")]
#[command(about = "RigForge radio platform shell", long_about = None)]
struct Cli {
    /// Optional path to config file (TOML)
    #[arg(long)]
    config: Option<PathBuf>,

    /// Optional WAV file input (future M2/M4 pipeline)
    #[arg(long)]
    input: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    rigforge_log::init("info")?;

    let config_path = cli.config.as_deref();
    let config = AppConfig::load(config_path)?;
    let events = AppEventBus::new(256);

    let radio = NullRadio::default();
    let ai = NullLanguageModel::default();
    let audio = AudioService::default();

    info!(?config.station.callsign, ?config.station.grid, "RigForge starting");

    if let Some(path) = cli.input {
        warn!(?path, "WAV input mode is not implemented yet (planned milestone)");
    }

    let devices = audio.enumerate_devices().await?;
    for d in devices {
        events.publish(rigforge_core::AppEvent::DeviceDiscovered {
            subsystem: "audio".to_string(),
            name: d.name,
            detail: format!("{} Hz, {} ch", d.default_sample_rate_hz, d.channels),
        });
    }

    rigforge_tui::render_shell(&config, &events, &radio, &ai).await?;

    info!("RigForge clean shutdown");
    Ok(())
}
