use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use clap::Parser;
use rigforge_ai::NullLanguageModel;
use rigforge_audio::{AudioService, CaptureOptions};
use rigforge_core::{AppConfig, AppEventBus};
use rigforge_radio::{enumerate_serial_ports, NullRadio};
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

    /// Print discovered audio inputs and exit
    #[arg(long)]
    list_audio: bool,

    /// Print discovered serial radio ports and exit
    #[arg(long)]
    list_radio: bool,

    /// Capture audio to WAV (Stage 1)
    #[arg(long)]
    record_wav: Option<PathBuf>,

    /// Capture duration in seconds for --record-wav
    #[arg(long, default_value_t = 10)]
    duration_secs: u64,

    /// Preferred audio input device name (substring match)
    #[arg(long)]
    audio_device: Option<String>,
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
    let audio = AudioService::new(config.audio.input_device.clone(), config.audio.enabled);

    info!(?config.station.callsign, ?config.station.grid, "RigForge starting");

    if let Some(path) = cli.input {
        warn!(?path, "WAV input mode is not implemented yet (planned milestone)");
    }

    let devices = audio.enumerate_devices().await?;
    for d in &devices {
        events.publish(rigforge_core::AppEvent::DeviceDiscovered {
            subsystem: "audio".to_string(),
            name: d.name.clone(),
            detail: format!("{} Hz, {} ch", d.default_sample_rate_hz, d.channels),
        });
    }

    let mut serial_ports = if config.radio.enabled {
        enumerate_serial_ports()?
    } else {
        Vec::new()
    };
    if let Some(port) = config.radio.serial_port.as_deref() {
        if !serial_ports.iter().any(|existing| existing == port) {
            serial_ports.push(port.to_string());
        }
    }
    for p in &serial_ports {
        events.publish(rigforge_core::AppEvent::DeviceDiscovered {
            subsystem: "radio-serial".to_string(),
            name: p.clone(),
            detail: "serial endpoint".to_string(),
        });
    }

    if cli.list_audio {
        println!("Audio input devices:");
        for (idx, d) in devices.iter().enumerate() {
            println!("  [{idx}] {} ({} Hz, {} ch)", d.name, d.default_sample_rate_hz, d.channels);
        }
        if devices.is_empty() {
            if config.audio.enabled {
                println!("  (none discovered; hardware may be unplugged or unavailable)");
            } else {
                println!("  (audio disabled by config)");
            }
        }
        return Ok(());
    }

    if cli.list_radio {
        println!("Serial radio endpoints:");
        for (idx, p) in serial_ports.iter().enumerate() {
            println!("  [{idx}] {p}");
        }
        if serial_ports.is_empty() {
            if config.radio.enabled {
                println!("  (none discovered; hardware may be unplugged or unavailable)");
            } else {
                println!("  (radio disabled by config)");
            }
        }
        return Ok(());
    }

    if let Some(path) = cli.record_wav {
        let preferred_device_name = cli.audio_device.or_else(|| config.audio.input_device.clone());
        let opts = CaptureOptions {
            preferred_device_name,
            sample_rate_hz: Some(config.audio.sample_rate_hz),
            channels: Some(config.audio.channels as u16),
            duration: Duration::from_secs(cli.duration_secs),
        };
        println!("Recording WAV for {} seconds...", cli.duration_secs);
        let summary = audio.capture_wav(path.clone(), opts)?;
        println!("Saved: {}", path.display());
        println!("Sample rate: {} Hz", summary.sample_rate_hz);
        println!("Channels: {}", summary.channels);
        println!("Samples: {}", summary.samples_written);
        println!("Peak: {:.1} dBFS", summary.peak_dbfs);
        println!("RMS: {:.1} dBFS", summary.rms_dbfs);
        return Ok(());
    }

    rigforge_tui::render_shell(&config, &events, &radio, &ai).await?;

    info!("RigForge clean shutdown");
    Ok(())
}
