use std::{path::PathBuf, time::Duration};

use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};
use qsonaut_core::AppConfig;
use qsonaut_gui::run_gui;
use qsonaut_radio::{
    enumerate_serial_ports, ControlId, ControlValue, IcomCiVRadio, Mode, Radio, RadioHal,
};
use tracing::info;

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CliMode {
    Lsb,
    Usb,
    Cw,
    Data,
}

impl From<CliMode> for Mode {
    fn from(value: CliMode) -> Self {
        match value {
            CliMode::Lsb => Mode::Lsb,
            CliMode::Usb => Mode::Usb,
            CliMode::Cw => Mode::Cw,
            CliMode::Data => Mode::Data,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CliPtt {
    On,
    Off,
}

impl CliPtt {
    fn as_enabled(self) -> bool {
        matches!(self, CliPtt::On)
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CliControl {
    AfGain,
    RfGain,
    Squelch,
    RfPower,
    Preamp,
    Attenuator,
    Nb,
    Nr,
    Agc,
    Split,
    DataMode,
    Filter,
}

impl From<CliControl> for ControlId {
    fn from(value: CliControl) -> Self {
        match value {
            CliControl::AfGain => ControlId::AfGain,
            CliControl::RfGain => ControlId::RfGain,
            CliControl::Squelch => ControlId::Squelch,
            CliControl::RfPower => ControlId::RfPower,
            CliControl::Preamp => ControlId::Preamp,
            CliControl::Attenuator => ControlId::Attenuator,
            CliControl::Nb => ControlId::NoiseBlanker,
            CliControl::Nr => ControlId::NoiseReduction,
            CliControl::Agc => ControlId::Agc,
            CliControl::Split => ControlId::Split,
            CliControl::DataMode => ControlId::DataMode,
            CliControl::Filter => ControlId::Filter,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum CliProfile {
    Ft8_20m,
}

#[derive(Debug, Parser)]
#[command(name = "qsonaut")]
#[command(about = "QSONaut radio platform shell", long_about = None)]
struct Cli {
    /// Optional path to config file (TOML)
    #[arg(long)]
    config: Option<PathBuf>,

    /// Print discovered serial radio ports and exit
    #[arg(long)]
    list_radio: bool,

    /// Read the radio status over CI-V and print it
    #[arg(long)]
    radio_status: bool,

    /// Serial radio endpoint to use (defaults to config or /dev/ttyUSB0)
    #[arg(long)]
    radio_port: Option<String>,

    /// Serial baud rate for CI-V
    #[arg(long)]
    radio_baud: Option<u32>,

    /// CI-V target radio address (examples: 148, 0x94, 94h)
    #[arg(long)]
    radio_civ_address: Option<String>,

    /// CI-V controller/source address (examples: 224, 0xE0, E0h)
    #[arg(long)]
    controller_civ_address: Option<String>,

    /// Set VFO frequency in Hz using CI-V
    #[arg(long)]
    set_frequency_hz: Option<u64>,

    /// Set operating mode using CI-V
    #[arg(long, value_enum)]
    set_mode: Option<CliMode>,

    /// Set PTT state using CI-V
    #[arg(long, value_enum)]
    ptt: Option<CliPtt>,

    /// Send a raw CI-V frame as hex (for example: "FE FE 94 E0 03 FD")
    #[arg(long)]
    civ_raw: Option<String>,

    /// Apply a validated operating profile
    #[arg(long, value_enum)]
    apply_profile: Option<CliProfile>,

    /// Set a typed radio control by name
    #[arg(long, value_enum)]
    set_control: Option<CliControl>,

    /// Value for --set-control (bool: on/off/true/false, integer: 0..255)
    #[arg(long)]
    control_value: Option<String>,

    /// Read a typed radio control by name
    #[arg(long, value_enum)]
    get_control: Option<CliControl>,

    /// Enable IC-7300 spectrum/waterfall stream and wait for first data frame
    #[arg(long)]
    enable_spectrum_stream: bool,

    /// Disable IC-7300 spectrum/waterfall stream
    #[arg(long)]
    disable_spectrum_stream: bool,

    /// Timeout waiting for first spectrum data frame after enable
    #[arg(long, default_value_t = 2500)]
    spectrum_timeout_ms: u64,

    /// Verify by reading back frequency/mode after set operations
    #[arg(long)]
    verify_after_set: bool,

    /// Launch native desktop GUI (egui/eframe)
    #[arg(long)]
    gui: bool,
}

fn main() -> Result<()> {
    prepare_wsl_gui_environment()?;
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("failed to start QSONaut runtime")?
        .block_on(async_main())
}

async fn async_main() -> Result<()> {
    let launch_gui_by_default = std::env::args_os().len() == 1;
    let cli = Cli::parse();

    qsonaut_log::init("info")?;

    let config_path = cli.config.as_deref();
    let mut config = AppConfig::load(config_path)?;

    if let Some(addr) = cli.radio_civ_address.as_deref() {
        config.radio.civ_address = parse_civ_address(addr)
            .with_context(|| format!("invalid --radio-civ-address value: {addr}"))?;
    }
    if let Some(addr) = cli.controller_civ_address.as_deref() {
        config.radio.controller_civ_address = parse_civ_address(addr)
            .with_context(|| format!("invalid --controller-civ-address value: {addr}"))?;
    }
    info!(?config.station.callsign, ?config.station.grid, "QSONaut starting");
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

    let has_radio_ops = cli.radio_status
        || cli.set_frequency_hz.is_some()
        || cli.set_mode.is_some()
        || cli.ptt.is_some()
        || cli.civ_raw.is_some()
        || cli.apply_profile.is_some()
        || cli.set_control.is_some()
        || cli.get_control.is_some()
        || cli.enable_spectrum_stream
        || cli.disable_spectrum_stream;

    if has_radio_ops {
        let port = cli
            .radio_port
            .clone()
            .or_else(|| config.radio.serial_port.clone())
            .unwrap_or_else(|| "/dev/ttyUSB0".to_string());
        let radio_baud = cli.radio_baud.unwrap_or(config.radio.baud_rate);
        info!(port = %port, radio_baud, "Using CI-V serial settings");
        let radio = IcomCiVRadio::new(
            port.clone(),
            radio_baud,
            config.radio.controller_civ_address,
        )
        .with_radio_address(config.radio.civ_address);

        if cli.enable_spectrum_stream && cli.disable_spectrum_stream {
            anyhow::bail!(
                "--enable-spectrum-stream and --disable-spectrum-stream are mutually exclusive"
            );
        }

        if let Some(profile) = cli.apply_profile {
            apply_profile(&radio, profile)
                .await
                .with_context(|| format!("failed to apply profile {profile:?} on {port}"))?;
            println!("Applied profile: {profile:?}");
        }

        if let Some(hz) = cli.set_frequency_hz {
            radio
                .set_frequency(hz)
                .await
                .with_context(|| format!("failed to set frequency to {hz} Hz on {port}"))?;
            println!("Set frequency_hz: {hz}");
        }

        if let Some(mode) = cli.set_mode {
            let target: Mode = mode.into();
            Radio::set_mode(&radio, target)
                .await
                .with_context(|| format!("failed to set mode on {port}"))?;
            println!("Set mode: {mode:?}");
        }

        if let Some(ptt) = cli.ptt {
            let enabled = ptt.as_enabled();
            radio
                .ptt(enabled)
                .await
                .with_context(|| format!("failed to set ptt={} on {port}", enabled))?;
            println!("Set ptt: {}", if enabled { "ON" } else { "OFF" });
        }

        if let Some(raw) = cli.civ_raw.as_deref() {
            let request = parse_hex_bytes(raw)?;
            let response = radio
                .protocol_write_read(&request)
                .await
                .with_context(|| format!("raw CI-V request failed on {port}"))?;

            println!("Raw CI-V request: {}", format_hex_bytes(&request));
            println!("Raw CI-V response: {}", format_hex_bytes(&response));
        }

        if let Some(control) = cli.set_control {
            let id: ControlId = control.into();
            let raw_value = cli.control_value.as_deref().with_context(|| {
                format!("--control-value is required for --set-control {control:?}")
            })?;
            let value = parse_control_value(id, raw_value)?;
            radio
                .set_control(id, value)
                .await
                .with_context(|| format!("failed to set control {control:?} on {port}"))?;
            println!("Set control: {control:?} = {raw_value}");
        }

        if let Some(control) = cli.get_control {
            let id: ControlId = control.into();
            let value = radio
                .get_control(id)
                .await
                .with_context(|| format!("failed to get control {control:?} on {port}"))?;
            match value {
                Some(v) => println!("Control {control:?}: {}", format_control_value(&v)),
                None => println!("Control {control:?}: (unsupported or unavailable)"),
            }
        }

        if cli.enable_spectrum_stream {
            let timeout = Duration::from_millis(cli.spectrum_timeout_ms);
            let first_frame = radio
                .enable_spectrum_stream(timeout)
                .await
                .with_context(|| format!("failed to enable spectrum stream on {port}"))?;

            println!(
                "Spectrum stream: READY (first frame: {})",
                format_hex_bytes(&first_frame)
            );
        }

        if cli.disable_spectrum_stream {
            radio
                .disable_spectrum_stream()
                .await
                .with_context(|| format!("failed to disable spectrum stream on {port}"))?;
            println!("Spectrum stream: DISABLED");
        }

        let should_verify = cli.verify_after_set
            || cli.apply_profile.is_some()
            || cli.set_frequency_hz.is_some()
            || cli.set_mode.is_some()
            || cli.set_control.is_some();

        if should_verify {
            let verify_mode = radio.mode().await.ok();
            let verify_freq = radio.frequency().await.ok();
            println!("Verification readback:");
            if let Some(freq) = verify_freq {
                println!("  frequency_hz: {freq}");
            } else {
                println!("  frequency_hz: (unavailable)");
            }
            if let Some(mode) = verify_mode {
                println!("  mode_core: {}", format_core_mode(mode));
            } else {
                println!("  mode_core: (unavailable)");
            }
        }

        if cli.radio_status {
            match radio.probe() {
                Ok(status) => {
                    println!("Radio status:");
                    if let Some(freq) = status.frequency_hz {
                        println!("  frequency_hz: {freq}");
                    }
                    if let Some(ref mode) = status.mode {
                        println!("  mode: {mode}");
                    }
                    if let Some(details) = status.mode_details {
                        println!("  mode_base: {:?}", details.base);
                        println!("  data_mode: {}", details.data_mode);
                        if let Some(filter) = details.filter {
                            println!("  filter: FIL{filter}");
                        }
                    }
                    if status.frequency_hz.is_none() && status.mode.is_none() {
                        println!("  (no CI-V response received; verify the serial port and command framing)");
                    }
                }
                Err(err) => {
                    println!("Radio status query failed: {err}");
                }
            }
        }

        return Ok(());
    }

    if cli.gui || launch_gui_by_default {
        let display_value = std::env::var("DISPLAY").ok();
        let wayland_value = std::env::var("WAYLAND_DISPLAY").ok();

        info!(
            display_value = ?display_value,
            wayland_value = ?wayland_value,
            "GUI launch requested"
        );

        let gui_result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run_gui(config.clone())));
        match gui_result {
            Ok(Ok(())) => {}
            Ok(Err(err)) => {
                println!("GUI launch failed: {err}");
                println!(
                    "If you are on a headless shell, run from a desktop session with DISPLAY/Wayland configured."
                );
                println!(
                    "WSL tip: ensure WSLg is active and WAYLAND_DISPLAY is set; if needed install wl-clipboard and xclip."
                );
                return Ok(());
            }
            Err(_) => {
                println!(
                    "GUI backend panicked during startup. This is commonly caused by missing/invalid desktop graphics context."
                );
                println!(
                    "Run from a desktop session with DISPLAY/Wayland configured, or check GPU/GL driver availability."
                );
                println!(
                    "WSL tip: ensure WSLg is active and WAYLAND_DISPLAY is set; if needed install wl-clipboard and xclip."
                );
                return Ok(());
            }
        }
        info!("QSONaut GUI closed");
        return Ok(());
    }

    println!("No operation requested. Use --help for CLI commands or --gui for desktop console.");

    info!("QSONaut clean shutdown");
    Ok(())
}

/// Mesa reads its renderer selection during process startup. Under WSL, set
/// the D3D12 policy and re-exec once so EGL never starts in llvmpipe and then
/// observes a late driver change. Explicit operator choices remain untouched.
fn prepare_wsl_gui_environment() -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::CommandExt;
        use std::process::Command;

        let args = std::env::args_os().collect::<Vec<_>>();
        let wants_gui = args.len() == 1 || args.iter().skip(1).any(|arg| arg == "--gui");
        if !wants_gui || std::env::var_os("QSONAUT_GUI_ENV_READY").is_some() {
            return Ok(());
        }

        let is_wsl = std::fs::read_to_string("/proc/version")
            .map(|version| version.to_ascii_lowercase().contains("microsoft"))
            .unwrap_or(false);
        let d3d12_driver = PathBuf::from("/usr/lib/x86_64-linux-gnu/dri/d3d12_dri.so").exists();
        if !is_wsl || !PathBuf::from("/dev/dxg").exists() || !d3d12_driver {
            return Ok(());
        }

        let gallium = std::env::var_os("GALLIUM_DRIVER");
        let use_d3d12 = gallium
            .as_deref()
            .map(|driver| driver == "d3d12")
            .unwrap_or(true);
        let needs_driver = gallium.is_none();
        let needs_adapter =
            use_d3d12 && std::env::var_os("MESA_D3D12_DEFAULT_ADAPTER_NAME").is_none();
        if !needs_driver && !needs_adapter {
            return Ok(());
        }

        let executable = std::env::current_exe().context("failed to locate QSONaut executable")?;
        let mut command = Command::new(executable);
        command.args(args.iter().skip(1));
        command.env("QSONAUT_GUI_ENV_READY", "1");
        if needs_driver {
            command.env("GALLIUM_DRIVER", "d3d12");
        }
        if needs_adapter {
            command.env("MESA_D3D12_DEFAULT_ADAPTER_NAME", "AMD");
        }
        let error = command.exec();
        return Err(error).context("failed to restart QSONaut with WSL GPU rendering");
    }

    #[cfg(not(target_os = "linux"))]
    Ok(())
}

fn parse_hex_bytes(input: &str) -> Result<Vec<u8>> {
    let compact: String = input.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    if compact.is_empty() {
        anyhow::bail!("hex input is empty")
    }
    if compact.len() % 2 != 0 {
        anyhow::bail!("hex input must contain an even number of hex digits")
    }

    let mut bytes = Vec::with_capacity(compact.len() / 2);
    for i in (0..compact.len()).step_by(2) {
        let byte = u8::from_str_radix(&compact[i..i + 2], 16)
            .with_context(|| format!("invalid hex byte at position {i}"))?;
        bytes.push(byte);
    }

    Ok(bytes)
}

fn format_hex_bytes(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn parse_civ_address(input: &str) -> Result<u8> {
    let raw = input.trim();
    if raw.is_empty() {
        anyhow::bail!("empty address");
    }

    if let Ok(v) = raw.parse::<u8>() {
        return Ok(v);
    }

    let lower = raw.to_ascii_lowercase();
    if let Some(rest) = lower.strip_prefix("0x") {
        return u8::from_str_radix(rest, 16)
            .with_context(|| format!("invalid hex address: {input}"));
    }
    if let Some(rest) = lower.strip_suffix('h') {
        return u8::from_str_radix(rest, 16)
            .with_context(|| format!("invalid hex address: {input}"));
    }

    u8::from_str_radix(&lower, 16).with_context(|| format!("invalid address: {input}"))
}

fn format_core_mode(mode: Mode) -> &'static str {
    match mode {
        Mode::Usb => "USB",
        Mode::Lsb => "LSB",
        Mode::Cw => "CW",
        Mode::Data => "DATA",
    }
}

async fn apply_profile(radio: &IcomCiVRadio, profile: CliProfile) -> Result<()> {
    match profile {
        CliProfile::Ft8_20m => {
            radio.set_frequency(14_074_000).await?;
            Radio::set_mode(radio, Mode::Data).await?;
            radio.set_ptt(false).await?;
        }
    }
    Ok(())
}

fn parse_control_value(id: ControlId, raw: &str) -> Result<ControlValue> {
    match id {
        ControlId::Preamp
        | ControlId::Attenuator
        | ControlId::NoiseBlanker
        | ControlId::NoiseReduction
        | ControlId::Split
        | ControlId::DataMode => {
            let v = parse_bool_like(raw)?;
            Ok(ControlValue::Bool(v))
        }
        ControlId::AfGain
        | ControlId::RfGain
        | ControlId::Squelch
        | ControlId::RfPower
        | ControlId::Agc
        | ControlId::Filter => {
            let v: u8 = raw
                .parse()
                .with_context(|| format!("invalid numeric control value: {raw}"))?;
            Ok(ControlValue::U8(v))
        }
        _ => anyhow::bail!("unsupported control for CLI value parsing: {id:?}"),
    }
}

fn parse_bool_like(raw: &str) -> Result<bool> {
    match raw.to_ascii_lowercase().as_str() {
        "1" | "on" | "true" | "yes" => Ok(true),
        "0" | "off" | "false" | "no" => Ok(false),
        _ => anyhow::bail!("invalid boolean value: {raw} (expected on/off/true/false/1/0)"),
    }
}

fn format_control_value(value: &ControlValue) -> String {
    match value {
        ControlValue::Bool(v) => format!("{}", if *v { "ON" } else { "OFF" }),
        ControlValue::U8(v) => v.to_string(),
        ControlValue::I32(v) => v.to_string(),
        ControlValue::U64(v) => v.to_string(),
        ControlValue::Mode(v) => format_core_mode(*v).to_string(),
        ControlValue::Text(v) => v.clone(),
        ControlValue::Raw(v) => format_hex_bytes(v),
    }
}
