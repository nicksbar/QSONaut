use anyhow::{anyhow, Result};
use eframe::egui::{self, Color32, ColorImage, RichText, TextureHandle, TextureOptions};
use rigforge_audio::AudioService;
use rigforge_core::AppConfig;
use rigforge_radio::{ControlId, ControlValue, IcomCiVRadio, Mode, Radio, RadioHal};
use rustfft::{FftPlanner, num_complex::Complex};
use std::collections::VecDeque;
use std::f32::consts::PI;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};
use tracing::info;

const RADIO_WF_WIDTH: usize = 360;
const RADIO_WF_HEIGHT: usize = 180;
const AUDIO_BINS: usize = 512;
const AUDIO_WF_HEIGHT: usize = 120;
const AUDIO_MAX_FREQ_HZ: u32 = 4_000;
// 8192 samples @ 48 kHz = 170 ms window, ~5.9 Hz/bin, ~683 useful bins for 0-4 kHz.
const FFT_SIZE: usize = 8192;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WaterfallSpeed {
    Slow,
    Mid,
    Fast,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkspaceMode {
    Ft8,
    Js8Call,
    Wefax,
}

impl WorkspaceMode {
    fn label(self) -> &'static str {
        match self {
            WorkspaceMode::Ft8 => "FT8",
            WorkspaceMode::Js8Call => "JS8Call",
            WorkspaceMode::Wefax => "WEFAX",
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
    radio_spectrum_enabled: bool,
    radio_waterfall_status: String,
    radio_waterfall_rows: VecDeque<Vec<u8>>,
    audio_spectrum_status: String,
    audio_waterfall_rows: VecDeque<Vec<u8>>,
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
            radio_spectrum_enabled: false,
            radio_waterfall_status: "AUTO-ARM PENDING".to_string(),
            radio_waterfall_rows: VecDeque::with_capacity(RADIO_WF_HEIGHT),
            audio_spectrum_status: "INIT".to_string(),
            audio_waterfall_rows: VecDeque::with_capacity(AUDIO_WF_HEIGHT),
            last_error: None,
            last_update: None,
        }
    }
}

#[derive(Debug, Clone)]
enum GuiCommand {
    TuneDelta(i64),
    CycleMode,
    TogglePtt,
    AfGainDelta(i16),
    Quit,
}

pub fn run_gui(config: AppConfig) -> Result<()> {
    info!(
        display_value = ?std::env::var("DISPLAY").ok(),
        wayland_value = ?std::env::var("WAYLAND_DISPLAY").ok(),
        winit_backend_value = ?std::env::var("WINIT_UNIX_BACKEND").ok(),
        wgpu_backend_value = ?std::env::var("WGPU_BACKEND").ok(),
        "Preparing native GUI launch"
    );

    configure_unix_gui_environment();

    info!(
        display_value = ?std::env::var("DISPLAY").ok(),
        wayland_value = ?std::env::var("WAYLAND_DISPLAY").ok(),
        winit_backend_value = ?std::env::var("WINIT_UNIX_BACKEND").ok(),
        wgpu_backend_value = ?std::env::var("WGPU_BACKEND").ok(),
        "GUI environment after configuration"
    );

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1280.0, 860.0])
            .with_min_inner_size([980.0, 680.0])
            .with_title("RigForge — Radio Console")
            .with_resizable(true),
        ..Default::default()
    };

    let app_config = config.clone();
    info!(title = "RigForge", "Calling eframe::run_native");
    let result = eframe::run_native(
        "RigForge",
        options,
        Box::new(move |_cc| Ok(Box::new(RigforgeGuiApp::new(app_config.clone())))),
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

fn configure_unix_gui_environment() {
    #[cfg(target_os = "linux")]
    {
        if std::env::var_os("NO_AT_BRIDGE").is_none() {
            std::env::set_var("NO_AT_BRIDGE", "1");
        }
    }
}

struct RigforgeGuiApp {
    config: AppConfig,
    state: Arc<Mutex<GuiState>>,
    command_tx: Option<mpsc::Sender<GuiCommand>>,
    worker_stop: Arc<AtomicBool>,
    radio_worker_handle: Option<std::thread::JoinHandle<()>>,
    audio_worker_handle: Option<std::thread::JoinHandle<()>>,
    radio_waterfall_texture: Option<TextureHandle>,
    audio_waterfall_texture: Option<TextureHandle>,
    workspace_mode: WorkspaceMode,
    display_tuning: Arc<Mutex<DisplayTuning>>,
    repaint_ctx: Arc<OnceLock<egui::Context>>,
}

impl RigforgeGuiApp {
    fn new(config: AppConfig) -> Self {
        let state = Arc::new(Mutex::new(GuiState::default()));
        let worker_stop = Arc::new(AtomicBool::new(false));
        let display_tuning = Arc::new(Mutex::new(DisplayTuning::default()));

        let repaint_ctx: Arc<OnceLock<egui::Context>> = Arc::new(OnceLock::new());

        let (command_tx, radio_worker_handle) = if config.radio.enabled {
            let port = config.radio.serial_port.clone().unwrap_or_default();
            let radio = IcomCiVRadio::new(
                port.clone(),
                config.radio.baud_rate,
                config.radio.controller_civ_address,
            )
            .with_radio_address(config.radio.civ_address);
            let (tx, rx) = mpsc::channel::<GuiCommand>();
            let display_port = if port.is_empty() {
                "auto".to_string()
            } else {
                port.clone()
            };
            info!(port = %display_port, baud = config.radio.baud_rate, "Starting GUI radio worker");
            let handle = spawn_radio_worker(
                radio,
                state.clone(),
                worker_stop.clone(),
                display_tuning.clone(),
                rx,
                repaint_ctx.clone(),
            );
            (Some(tx), Some(handle))
        } else {
            {
                let mut s = state.lock().expect("ui state lock poisoned");
                s.last_error = Some("Radio is disabled in config; UI running in monitor-only mode".to_string());
                s.radio_waterfall_status = "UNAVAILABLE (radio disabled)".to_string();
            }
            (None, None)
        };

        let audio_worker_handle = Some(spawn_audio_spectrum_worker(
            state.clone(),
            worker_stop.clone(),
            config.audio.enabled,
            config.audio.sample_rate_hz,
            config.audio.channels,
            config.audio.input_device.clone(),
            repaint_ctx.clone(),
            display_tuning.clone(),
        ));

        Self {
            config,
            state,
            command_tx,
            worker_stop,
            radio_worker_handle,
            audio_worker_handle,
            radio_waterfall_texture: None,
            audio_waterfall_texture: None,
            workspace_mode: WorkspaceMode::Ft8,
            display_tuning,
            repaint_ctx,
        }
    }

    fn send_command(&self, cmd: GuiCommand) {
        if let Some(tx) = &self.command_tx {
            let _ = tx.send(cmd);
        }
    }

    fn draw_status(&self, ui: &mut egui::Ui, snapshot: &GuiState) {
        ui.heading("Live Status");
        ui.separator();

        let freq = snapshot
            .frequency_hz
            .map(|v| format!("{v} Hz"))
            .unwrap_or_else(|| "(unavailable)".to_string());

        ui.label(format!("Frequency: {freq}"));
        ui.label(format!("Mode: {}", snapshot.mode));
        ui.label(format!(
            "PTT: {}",
            if snapshot.ptt_on { "ON" } else { "OFF" }
        ));
        ui.label(format!(
            "Data mode: {}",
            snapshot
                .data_mode
                .map(|v| if v { "ON" } else { "OFF" })
                .unwrap_or("?")
        ));
        ui.label(format!(
            "Filter: {}",
            snapshot
                .filter
                .map(|v| format!("FIL{v}"))
                .unwrap_or_else(|| "?".to_string())
        ));

        ui.add_space(6.0);

        ui.label(format!(
            "AF: {}   RF: {}   Power: {}",
            fmt_opt_u8(snapshot.af_gain),
            fmt_opt_u8(snapshot.rf_gain),
            fmt_opt_u8(snapshot.rf_power)
        ));

        let wf_color = if snapshot.radio_spectrum_enabled {
            Color32::LIGHT_GREEN
        } else {
            Color32::YELLOW
        };
        ui.label(RichText::new(format!(
            "Radio waterfall: {} ({})",
            snapshot.radio_waterfall_status,
            if snapshot.radio_spectrum_enabled {
                "auto-on"
            } else {
                "auto-arm retry"
            }
        ))
        .color(wf_color));

        ui.label(RichText::new(format!(
            "Audio spectrum: {}",
            snapshot.audio_spectrum_status
        ))
        .color(if snapshot.audio_spectrum_status.contains("LIVE") {
            Color32::LIGHT_GREEN
        } else {
            Color32::YELLOW
        }));

        if let Some(last) = snapshot.last_update {
            ui.label(format!("Last update: {:.1}s ago", last.elapsed().as_secs_f32()));
        }

        if let Some(err) = &snapshot.last_error {
            ui.add_space(6.0);
            ui.label(RichText::new(format!("Last error: {err}")).color(Color32::YELLOW));
        }
    }

    fn draw_radio_waterfall(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, snapshot: &GuiState) {
        ui.heading("Radio Waterfall (CI-V Scope)");
        ui.separator();

        let display_size = egui::vec2(ui.available_width(), RADIO_WF_HEIGHT as f32 * 1.9);

        let image = build_waterfall_image(
            &snapshot.radio_waterfall_rows,
            RADIO_WF_WIDTH,
            RADIO_WF_HEIGHT,
            0.7,
        );

        if let Some(tex) = &mut self.radio_waterfall_texture {
            tex.set(image, TextureOptions::LINEAR);
        } else {
            self.radio_waterfall_texture = Some(ctx.load_texture(
                "rigforge-radio-waterfall",
                image,
                TextureOptions::LINEAR,
            ));
        }

        if let Some(tex) = &self.radio_waterfall_texture {
            ui.image((tex.id(), display_size));
        }

        ui.label("Auto-enabled on startup when supported. Palette: blue\u{2192}cyan\u{2192}yellow\u{2192}white");
    }

    fn draw_audio_waterfall(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, snapshot: &GuiState) {
        ui.heading("Audio Waterfall (DSP Input)");
        ui.separator();

        let bw_hz = filter_bandwidth_hz(&snapshot.mode, snapshot.filter);
        let display_bins = ((bw_hz.min(AUDIO_MAX_FREQ_HZ) as f32 / AUDIO_MAX_FREQ_HZ as f32)
            * AUDIO_BINS as f32)
            .round() as usize;
        let display_bins = display_bins.clamp(16, AUDIO_BINS);

        // Capture layout geometry before texture ops — available_width() can change mid-frame.
        let display_size = egui::vec2(ui.available_width(), AUDIO_WF_HEIGHT as f32 * 1.9);

        let image = build_waterfall_image(&snapshot.audio_waterfall_rows, display_bins, AUDIO_WF_HEIGHT, 1.0);
        if let Some(tex) = &mut self.audio_waterfall_texture {
            tex.set(image, TextureOptions::LINEAR);
        } else {
            self.audio_waterfall_texture = Some(ctx.load_texture(
                "rigforge-audio-waterfall",
                image,
                TextureOptions::LINEAR,
            ));
        }
        if let Some(tex) = &self.audio_waterfall_texture {
            ui.image((tex.id(), display_size));
        }
        ui.label(format!(
            "Audio: {}  |  0\u{2013}{} Hz ({} {})",
            snapshot.audio_spectrum_status,
            bw_hz.min(AUDIO_MAX_FREQ_HZ),
            snapshot.mode,
            snapshot.filter.map(|f| format!("FIL{f}")).unwrap_or_default(),
        ));
    }

    fn draw_workspace(&mut self, ui: &mut egui::Ui, snapshot: &GuiState) {
        ui.horizontal(|ui| {
            ui.heading("Mode Workspace");
            ui.separator();
            ui.selectable_value(&mut self.workspace_mode, WorkspaceMode::Ft8, "FT8");
            ui.selectable_value(&mut self.workspace_mode, WorkspaceMode::Js8Call, "JS8Call");
            ui.selectable_value(&mut self.workspace_mode, WorkspaceMode::Wefax, "WEFAX");
        });
        ui.separator();

        match self.workspace_mode {
            WorkspaceMode::Ft8 => {
                ui.group(|ui| {
                    ui.label(RichText::new("FT8 / Multi-tone Narrowband Layout").strong());
                    ui.label("• Left visual stack: radio waterfall + audio spectrum");
                    ui.label("• This panel: decode list, RX/TX lanes, free text, macros");
                    ui.label("• Status now: mode, filter, data mode already live");
                });
            }
            WorkspaceMode::Js8Call => {
                ui.group(|ui| {
                    ui.label(RichText::new("JS8Call / Messaging-centric Layout").strong());
                    ui.label("• Conversation timeline + directed call inbox");
                    ui.label("• Message composer + retry queue + heartbeat controls");
                    ui.label("• Radio/audio visuals remain pinned on the left stack");
                });
            }
            WorkspaceMode::Wefax => {
                ui.group(|ui| {
                    ui.label(RichText::new("WEFAX / Continuous image decode Layout").strong());
                    ui.label("• Slant correction, IOC settings, sync/contrast tools");
                    ui.label("• Decoded image pane replaces message/decode grid");
                    ui.label("• Shared control bar remains minimal and horizontal");
                });
            }
        }

        ui.add_space(8.0);
        ui.label(format!(
            "Active workspace: {} • Radio mode: {} • Freq: {}",
            self.workspace_mode.label(),
            snapshot.mode,
            snapshot
                .frequency_hz
                .map(|v| format!("{v} Hz"))
                .unwrap_or_else(|| "(unavailable)".to_string())
        ));
    }

    fn draw_radio_control_strip(&self, ui: &mut egui::Ui, snapshot: &GuiState) {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Radio").strong());

            if ui.small_button("-1 kHz").clicked() {
                self.send_command(GuiCommand::TuneDelta(-1_000));
            }
            if ui.small_button("+1 kHz").clicked() {
                self.send_command(GuiCommand::TuneDelta(1_000));
            }
            if ui.small_button("Mode").clicked() {
                self.send_command(GuiCommand::CycleMode);
            }
            if ui
                .small_button(if snapshot.ptt_on { "PTT OFF" } else { "PTT ON" })
                .clicked()
            {
                self.send_command(GuiCommand::TogglePtt);
            }
            if ui.small_button("AF-").clicked() {
                self.send_command(GuiCommand::AfGainDelta(-5));
            }
            if ui.small_button("AF+").clicked() {
                self.send_command(GuiCommand::AfGainDelta(5));
            }

            ui.separator();

            let mut tuning = self.display_tuning.lock().expect("tuning lock poisoned");
            let auto_clicked = ui
                .selectable_label(tuning.auto_visual, "AUTO")
                .on_hover_text("Auto-select waterfall speed and audio detail for current mode")
                .clicked();
            if auto_clicked {
                tuning.auto_visual = !tuning.auto_visual;
            }

            ui.label("WF");
            if ui
                .selectable_label(!tuning.auto_visual && tuning.waterfall_speed == WaterfallSpeed::Slow, "Slow")
                .clicked()
            {
                tuning.auto_visual = false;
                tuning.waterfall_speed = WaterfallSpeed::Slow;
            }
            if ui
                .selectable_label(!tuning.auto_visual && tuning.waterfall_speed == WaterfallSpeed::Mid, "Mid")
                .clicked()
            {
                tuning.auto_visual = false;
                tuning.waterfall_speed = WaterfallSpeed::Mid;
            }
            if ui
                .selectable_label(!tuning.auto_visual && tuning.waterfall_speed == WaterfallSpeed::Fast, "Fast")
                .clicked()
            {
                tuning.auto_visual = false;
                tuning.waterfall_speed = WaterfallSpeed::Fast;
            }

            drop(tuning);

            ui.separator();
            ui.label(format!(
                "{} | {} | {}",
                snapshot.mode,
                snapshot
                    .frequency_hz
                    .map(|v| format!("{v} Hz"))
                    .unwrap_or_else(|| "freq ?".to_string()),
                if snapshot.radio_spectrum_enabled { "WF LIVE" } else { "WF ARMING" },
            ));
        });
    }
}

impl eframe::App for RigforgeGuiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Give background workers a handle so they can trigger repaints directly.
        let _ = self.repaint_ctx.get_or_init(|| ctx.clone());
        // Safety-net repaint in case no worker data arrives for a long time.
        ctx.request_repaint_after(Duration::from_secs(1));

        let snapshot = self.state.lock().expect("ui state lock poisoned").clone();

        egui::TopBottomPanel::top("header").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("RigForge Operator Console");
                ui.separator();
                ui.label(format!(
                    "Callsign: {}",
                    self.config.station.callsign.as_deref().unwrap_or("(unset)")
                ));
                ui.label(format!(
                    "Grid: {}",
                    self.config.station.grid.as_deref().unwrap_or("(unset)")
                ));
            });
        });

        egui::TopBottomPanel::bottom("radio_strip")
            .resizable(false)
            .default_height(38.0)
            .show(ctx, |ui| {
                self.draw_radio_control_strip(ui, &snapshot);
            });

        egui::SidePanel::left("signals")
            .resizable(true)
            .default_width(760.0)
            .min_width(560.0)
            .show(ctx, |ui| {
                ui.group(|ui| self.draw_status(ui, &snapshot));
                ui.add_space(6.0);
                ui.group(|ui| self.draw_radio_waterfall(ui, ctx, &snapshot));
                ui.add_space(6.0);
                ui.group(|ui| self.draw_audio_waterfall(ui, ctx, &snapshot));
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.group(|ui| self.draw_workspace(ui, &snapshot));
        });
    }
}

impl Drop for RigforgeGuiApp {
    fn drop(&mut self) {
        self.worker_stop.store(true, Ordering::Relaxed);
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

fn spawn_radio_worker(
    radio: IcomCiVRadio,
    state: Arc<Mutex<GuiState>>,
    stop: Arc<AtomicBool>,
    display_tuning: Arc<Mutex<DisplayTuning>>,
    rx: mpsc::Receiver<GuiCommand>,
    repaint_ctx: Arc<OnceLock<egui::Context>>,
) -> std::thread::JoinHandle<()> {
    thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(err) => {
                let mut s = state.lock().expect("ui state lock poisoned");
                s.last_error = Some(format!("failed to start GUI runtime: {err}"));
                return;
            }
        };

        // Shared cell: the frame reader writes here, the display ticker reads from here.
        let latest_scope: Arc<Mutex<Option<Vec<u8>>>> = Arc::new(Mutex::new(None));

        let stream_state = state.clone();
        let stream_radio = radio.clone();
        let stream_stop = stop.clone();
        let scope_writer = latest_scope.clone();
        let _stream_handle = thread::spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(err) => {
                    let mut s = stream_state.lock().expect("ui state lock poisoned");
                    s.last_error = Some(format!("failed to start streaming runtime: {err}"));
                    return;
                }
            };

            while !stream_stop.load(Ordering::Relaxed) {
                let spectrum_enabled = stream_state.lock().expect("ui state lock poisoned").radio_spectrum_enabled;
                if !spectrum_enabled {
                    match rt.block_on(stream_radio.enable_spectrum_stream(Duration::from_millis(2_500))) {
                        Ok(frame) => {
                            let mut s = stream_state.lock().expect("ui state lock poisoned");
                            s.radio_spectrum_enabled = true;
                            s.radio_waterfall_status = if frame.len() >= 6 && frame[4] == 0xFB {
                                "ARMED (ACK)".to_string()
                            } else {
                                "READY".to_string()
                            };
                            s.last_error = None;
                        }
                        Err(err) => {
                            let mut s = stream_state.lock().expect("ui state lock poisoned");
                            s.radio_spectrum_enabled = false;
                            s.radio_waterfall_status = "AUTO-ARM RETRY".to_string();
                            s.last_error = Some(err.to_string());
                            thread::sleep(Duration::from_millis(1_000));
                        }
                    }
                    continue;
                }

                // 300 ms gives enough headroom for two-segment frame assembly.
                match rt.block_on(stream_radio.try_scope_waveform_bins_stream(Duration::from_millis(300))) {
                    Ok(Some(bins)) if !bins.is_empty() => {
                        *scope_writer.lock().expect("scope lock") = Some(bins);
                    }
                    Err(err) => {
                        let msg = err.to_string();
                        let mut s = stream_state.lock().expect("ui state lock poisoned");
                        if is_transient_civ_read_error(&msg) {
                            s.radio_waterfall_status = "WAITING FRAME".to_string();
                        } else {
                            s.radio_spectrum_enabled = false;
                            s.radio_waterfall_status = "NO FRAME".to_string();
                            s.last_error = Some(msg);
                        }
                    }
                    _ => {}
                }
            }
        });

        // Display ticker: pushes one row at the target interval, matching the audio worker rate.
        let ticker_state = state.clone();
        let ticker_stop = stop.clone();
        let ticker_repaint = repaint_ctx.clone();
        let ticker_tuning = display_tuning.clone();
        let scope_reader = latest_scope;
        let _ticker_handle = thread::spawn(move || {
            while !ticker_stop.load(Ordering::Relaxed) {
                let interval_ms: u64 = {
                    let t = ticker_tuning.lock().expect("tuning lock poisoned");
                    let mode = ticker_state.lock().expect("ui state lock poisoned").mode.clone();
                    let speed = if t.auto_visual {
                        let m = mode.to_ascii_uppercase();
                        if m.contains("DATA") || m.contains("FT8") || m.contains("JS8") || m.contains("RTTY") || m.contains("CW") {
                            WaterfallSpeed::Fast
                        } else {
                            WaterfallSpeed::Mid
                        }
                    } else {
                        t.waterfall_speed
                    };
                    match speed {
                        WaterfallSpeed::Fast => 42,
                        WaterfallSpeed::Mid  => 83,
                        WaterfallSpeed::Slow => 167,
                    }
                };
                thread::sleep(Duration::from_millis(interval_ms));

                let bins = scope_reader.lock().expect("scope lock").clone();
                if let Some(bins) = bins {
                    let mut s = ticker_state.lock().expect("ui state lock poisoned");
                    apply_waterfall_bins(&mut s, &bins);
                    s.radio_waterfall_status = "READY".to_string();
                    drop(s);
                    if let Some(ctx) = ticker_repaint.get() {
                        ctx.request_repaint();
                    }
                }
            }
        });

        poll_radio_core_state(&rt, &radio, &state);

        while !stop.load(Ordering::Relaxed) {
            let cmd = match rx.recv() {
                Ok(cmd) => Some(cmd),
                Err(_) => break,
            };

            if let Some(cmd) = cmd {
                match cmd {
                    GuiCommand::Quit => return,
                    GuiCommand::TuneDelta(delta) => {
                        let freq = rt.block_on(radio.frequency()).ok();
                        if let Some(freq) = freq {
                            let target = if delta.is_negative() {
                                freq.saturating_sub(delta.unsigned_abs())
                            } else {
                                freq.saturating_add(delta as u64)
                            };
                            if let Err(err) = rt.block_on(radio.set_frequency(target)) {
                                let mut s = state.lock().expect("ui state lock poisoned");
                                s.last_error = Some(err.to_string());
                            }
                        }
                        poll_radio_core_state(&rt, &radio, &state);
                    }
                    GuiCommand::CycleMode => {
                        let current = rt.block_on(radio.mode()).unwrap_or(Mode::Usb);
                        let next = match current {
                            Mode::Usb => Mode::Lsb,
                            Mode::Lsb => Mode::Cw,
                            Mode::Cw => Mode::Data,
                            Mode::Data => Mode::Usb,
                        };
                        if let Err(err) = rt.block_on(Radio::set_mode(&radio, next)) {
                            let mut s = state.lock().expect("ui state lock poisoned");
                            s.last_error = Some(err.to_string());
                        }
                        poll_radio_core_state(&rt, &radio, &state);
                    }
                    GuiCommand::TogglePtt => {
                        let ptt_target = {
                            let s = state.lock().expect("ui state lock poisoned");
                            !s.ptt_on
                        };
                        let mut s = state.lock().expect("ui state lock poisoned");
                        match rt.block_on(radio.set_ptt(ptt_target)) {
                            Ok(_) => {
                                s.ptt_on = ptt_target;
                                s.last_error = None;
                            }
                            Err(err) => s.last_error = Some(err.to_string()),
                        }
                        poll_radio_core_state(&rt, &radio, &state);
                    }
                    GuiCommand::AfGainDelta(delta) => {
                        let current = match rt.block_on(radio.get_control(ControlId::AfGain)).ok().flatten() {
                            Some(ControlValue::U8(v)) => v,
                            _ => 100,
                        };
                        let target = if delta.is_negative() {
                            current.saturating_sub(delta.unsigned_abs() as u8)
                        } else {
                            current.saturating_add(delta as u8).min(255)
                        };
                        if let Err(err) = rt.block_on(radio.set_control(ControlId::AfGain, ControlValue::U8(target))) {
                            let mut s = state.lock().expect("ui state lock poisoned");
                            s.last_error = Some(err.to_string());
                        }
                        poll_radio_core_state(&rt, &radio, &state);
                    }
                }
            }

            if let Ok(mut s) = state.lock() {
                let t = display_tuning.lock().expect("tuning lock poisoned").clone();
                let (_, sweep_code) = effective_visual_profile(&t, &s.mode);
                if let Err(err) = rt.block_on(radio.set_scope_sweep_speed(sweep_code)) {
                    s.last_error = Some(err.to_string());
                }
            }
        }
    })
}

fn poll_radio_core_state(
    rt: &tokio::runtime::Runtime,
    radio: &IcomCiVRadio,
    state: &Arc<Mutex<GuiState>>,
) {
    // Read what we need under a brief lock, then do all I/O outside it.
    let spectrum_enabled = state.lock().expect("ui state lock poisoned").radio_spectrum_enabled;
    let status_result = if spectrum_enabled {
        rt.block_on(radio.probe_stream_status())
    } else {
        radio.probe()
    };
    let af = read_u8_control(rt, radio, ControlId::AfGain);
    let rf = read_u8_control(rt, radio, ControlId::RfGain);
    let pwr = read_u8_control(rt, radio, ControlId::RfPower);
    let filt = read_u8_control(rt, radio, ControlId::Filter);

    let mut s = state.lock().expect("ui state lock poisoned");
    if let Ok(status) = status_result {
        if let Some(freq) = status.frequency_hz { s.frequency_hz = Some(freq); }
        if let Some(mode) = status.mode { s.mode = mode; }
        if let Some(details) = status.mode_details {
            s.data_mode = Some(details.data_mode);
            s.filter = details.filter;
        }
        s.last_update = Some(Instant::now());
    }
    if let Some(v) = af { s.af_gain = Some(v); }
    if let Some(v) = rf { s.rf_gain = Some(v); }
    if let Some(v) = pwr { s.rf_power = Some(v); }
    if let Some(v) = filt { s.filter = Some(v); }
}

fn apply_waterfall_bins(next: &mut GuiState, bins: &[u8]) {
    let row = downsample_bins(bins, RADIO_WF_WIDTH);
    if next.radio_waterfall_rows.len() >= RADIO_WF_HEIGHT {
        next.radio_waterfall_rows.pop_front();
    }
    next.radio_waterfall_rows.push_back(row);
}

fn read_u8_control(rt: &tokio::runtime::Runtime, radio: &IcomCiVRadio, id: ControlId) -> Option<u8> {
    match rt.block_on(radio.get_control(id)).ok().flatten() {
        Some(ControlValue::U8(v)) => Some(v),
        _ => None,
    }
}

fn downsample_bins(bins: &[u8], width: usize) -> Vec<u8> {
    if bins.is_empty() {
        return vec![0; width];
    }

    if bins.len() == width {
        return bins.to_vec();
    }

    if bins.len() == 1 {
        return vec![bins[0]; width];
    }

    if bins.len() > width {
        let mut out = Vec::with_capacity(width);
        for x in 0..width {
            let start = x * bins.len() / width;
            let end = ((x + 1) * bins.len() / width).max(start + 1);
            let mut peak = 0u8;
            for &v in &bins[start..end] {
                if v > peak {
                    peak = v;
                }
            }
            out.push(peak);
        }
        return out;
    }

    // If source is narrower than target, upsample with linear interpolation to reduce blockiness.
    let mut out = Vec::with_capacity(width);
    let src_last = (bins.len() - 1) as f32;
    let dst_last = (width - 1).max(1) as f32;
    for x in 0..width {
        let pos = (x as f32 / dst_last) * src_last;
        let i0 = pos.floor() as usize;
        let i1 = pos.ceil() as usize;
        if i0 == i1 {
            out.push(bins[i0]);
        } else {
            let t = pos - i0 as f32;
            let a = bins[i0] as f32;
            let b = bins[i1] as f32;
            out.push((a + (b - a) * t).round() as u8);
        }
    }
    out
}

fn spawn_audio_spectrum_worker(
    state: Arc<Mutex<GuiState>>,
    stop: Arc<AtomicBool>,
    enabled: bool,
    sample_rate_hz: u32,
    channels: u8,
    preferred_device: Option<String>,
    repaint_ctx: Arc<OnceLock<egui::Context>>,
    display_tuning: Arc<Mutex<DisplayTuning>>,
) -> std::thread::JoinHandle<()> {
    thread::spawn(move || {
        if !enabled {
            let mut s = state.lock().expect("ui state lock poisoned");
            s.audio_spectrum_status = "DISABLED".to_string();
            return;
        }

        if !command_exists("arecord") {
            let mut s = state.lock().expect("ui state lock poisoned");
            s.audio_spectrum_status = "UNAVAILABLE (arecord missing)".to_string();
            return;
        }

        let audio_service = AudioService::new(preferred_device, true);
        let mut stream = match audio_service.open_stream(sample_rate_hz, channels as u16) {
            Ok(stream) => stream,
            Err(err) => {
                let mut s = state.lock().expect("ui state lock poisoned");
                s.audio_spectrum_status = format!("NO INPUT ({err})");
                return;
            }
        };

        let mut fft_planner = FftPlanner::<f32>::new();
        let audio_fft = fft_planner.plan_fft_forward(FFT_SIZE);
        let mut fft_buf = vec![Complex::<f32>::new(0.0, 0.0); FFT_SIZE];
        let mut ring: VecDeque<f32> = VecDeque::with_capacity(FFT_SIZE);

        while !stop.load(Ordering::Relaxed) {
            let chunk_samples = {
                let t = display_tuning.lock().expect("tuning lock poisoned");
                let mode = state.lock().expect("ui state lock poisoned").mode.clone();
                let speed = if t.auto_visual {
                    let m = mode.to_ascii_uppercase();
                    if m.contains("DATA") || m.contains("FT8") || m.contains("JS8") || m.contains("RTTY") || m.contains("CW") {
                        WaterfallSpeed::Fast
                    } else {
                        WaterfallSpeed::Mid
                    }
                } else {
                    t.waterfall_speed
                };
                match speed {
                    WaterfallSpeed::Fast => (sample_rate_hz / 24) as usize,
                    WaterfallSpeed::Mid  => (sample_rate_hz / 12) as usize,
                    WaterfallSpeed::Slow => (sample_rate_hz / 6)  as usize,
                }
            };
            let chunk_bytes = (chunk_samples * 2).max(512);
            match stream.read_chunk(chunk_bytes) {
                Ok(samples) => {
                    for &s in &samples {
                        ring.push_back(s as f32 / i16::MAX as f32);
                    }
                    while ring.len() > FFT_SIZE {
                        ring.pop_front();
                    }
                    let nfill = ring.len();
                    for (i, b) in fft_buf.iter_mut().enumerate() {
                        *b = if i < nfill {
                            let w = 0.5 - 0.5 * (2.0 * PI * i as f32 / (nfill.max(2) - 1) as f32).cos();
                            Complex::new(ring[i] * w, 0.0)
                        } else {
                            Complex::new(0.0, 0.0)
                        };
                    }
                    audio_fft.process(&mut fft_buf);
                    let bins = fft_buffer_to_display_bins(&fft_buf, AUDIO_BINS, sample_rate_hz);
                    let mut s = state.lock().expect("ui state lock poisoned");
                    if s.audio_waterfall_rows.len() >= AUDIO_WF_HEIGHT {
                        s.audio_waterfall_rows.pop_front();
                    }
                    s.audio_waterfall_rows.push_back(bins);
                    s.audio_spectrum_status = "LIVE".to_string();
                    drop(s);
                    if let Some(ctx) = repaint_ctx.get() {
                        ctx.request_repaint();
                    }
                }
                Err(err) => {
                    state.lock().expect("ui state lock poisoned").audio_spectrum_status =
                        format!("NO INPUT ({err})");
                    thread::sleep(Duration::from_millis(100));
                }
            }
        }
    })
}

fn fft_buffer_to_display_bins(buf: &[Complex<f32>], bins: usize, sample_rate_hz: u32) -> Vec<u8> {
    let n = buf.len();
    if n == 0 || bins == 0 {
        return vec![0; bins];
    }
    let max_k = (n as f32 * AUDIO_MAX_FREQ_HZ as f32 / sample_rate_hz as f32).round() as usize;
    let max_k = max_k.clamp(2, n / 2);
    (0..bins)
        .map(|i| {
            // Fractional bin position with linear magnitude interpolation.
            let pos = 1.0 + (i as f32 / (bins.max(2) - 1) as f32) * (max_k - 1) as f32;
            let k0 = pos.floor() as usize;
            let k1 = (k0 + 1).min(max_k);
            let t = pos - k0 as f32;
            let m0 = buf[k0].norm() / n as f32;
            let m1 = buf[k1].norm() / n as f32;
            let mag = m0 + (m1 - m0) * t;
            let db = (20.0 * mag.max(1e-9_f32).log10()).clamp(-65.0, 0.0);
            ((db + 65.0) / 65.0 * 255.0).round().clamp(0.0, 255.0) as u8
        })
        .collect()
}

fn filter_bandwidth_hz(mode: &str, filter: Option<u8>) -> u32 {
    let f = filter.unwrap_or(1);
    let m = mode.to_ascii_uppercase();
    if m.contains("CW") {
        match f { 1 => 500, 2 => 250, 3 => 100, _ => 500 }
    } else if m.contains("FM") {
        match f { 1 => 15_000, 2 => 10_000, 3 => 7_000, _ => 15_000 }
    } else if m.contains("RTTY") {
        match f { 1 => 500, 2 => 350, 3 => 250, _ => 500 }
    } else {
        // USB / LSB / Data — IC-7300 defaults
        match f { 1 => 3_000, 2 => 2_400, 3 => 1_800, _ => 3_000 }
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
    if m.contains("DATA") || m.contains("FT8") || m.contains("JS8") || m.contains("RTTY") || m.contains("CW") {
        (45, 0)
    } else if m.contains("FM") {
        (120, 1)
    } else {
        (90, 1)
    }
}

fn is_transient_civ_read_error(message: &str) -> bool {
    let m = message.to_ascii_lowercase();
    m.contains("failed to read ci-v response")
        || m.contains("timed out")
        || m.contains("timeout")
}

fn command_exists(cmd: &str) -> bool {
    Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {} >/dev/null 2>&1", cmd))
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn build_waterfall_image(rows: &VecDeque<Vec<u8>>, width: usize, height: usize, gamma: f32) -> ColorImage {
    let mut pixels = vec![Color32::BLACK; width * height];
    let empty_row = vec![0u8; width];
    let missing = height.saturating_sub(rows.len());
    for y in 0..height {
        let src_row = if y < missing { &empty_row } else { rows.get(y - missing).unwrap_or(&empty_row) };
        for x in 0..width {
            let value = src_row.get(x).copied().unwrap_or(0);
            pixels[y * width + x] = waterfall_color(value, gamma);
        }
    }
    ColorImage::new([width, height], pixels)
}

fn waterfall_color(v: u8, gamma: f32) -> Color32 {
    let t = (v as f32 / 255.0).powf(gamma);
    let (r, g, b) = if t < 0.33 {
        let k = t / 0.33;
        (0.0, k * 180.0, 80.0 + k * 175.0)
    } else if t < 0.66 {
        let k = (t - 0.33) / 0.33;
        (k * 220.0, 180.0 + k * 60.0, 255.0 - k * 220.0)
    } else {
        let k = (t - 0.66) / 0.34;
        (220.0 + k * 35.0, 240.0 + k * 15.0, 35.0 + k * 220.0)
    };
    Color32::from_rgb(r as u8, g as u8, b as u8)
}

fn fmt_opt_u8(v: Option<u8>) -> String {
    v.map(|n| n.to_string()).unwrap_or_else(|| "?".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode_pcm_samples(bytes: &[u8]) -> Vec<i16> {
        bytes.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect()
    }

    #[test]
    fn decode_pcm_samples_handles_little_endian_i16() {
        let bytes = [0x00, 0x00, 0x01, 0x00, 0xFF, 0xFF, 0xFE, 0xFF];
        let samples = decode_pcm_samples(&bytes);
        assert_eq!(samples, vec![0i16, 1, -1, -2]);
    }

    #[test]
    fn apply_waterfall_bins_caps_rows_and_preserves_latest() {
        let mut state = GuiState::default();
        for i in 0..RADIO_WF_HEIGHT + 3 {
            apply_waterfall_bins(&mut state, &[i as u8; 8]);
        }

        assert_eq!(state.radio_waterfall_rows.len(), RADIO_WF_HEIGHT);
        assert_eq!(state.radio_waterfall_rows.back().unwrap()[0], (RADIO_WF_HEIGHT + 2) as u8);
    }

    #[test]
    fn compute_audio_spectrum_bins_returns_expected_length() {
        fn compute_audio_spectrum_bins(samples: &[i16], bins: usize, sample_rate_hz: u32) -> Vec<u8> {
            let n = samples.len().min(FFT_SIZE).max(2);
            let mut planner = FftPlanner::<f32>::new();
            let fft = planner.plan_fft_forward(n);
            let mut buf: Vec<Complex<f32>> = samples.iter().take(n).enumerate().map(|(i, &s)| {
                let w = 0.5 - 0.5 * (2.0 * PI * i as f32 / (n - 1) as f32).cos();
                Complex::new(s as f32 / i16::MAX as f32 * w, 0.0)
            }).collect();
            fft.process(&mut buf);
            fft_buffer_to_display_bins(&buf, bins, sample_rate_hz)
        }
        let bins = compute_audio_spectrum_bins(&[0i16; 256], AUDIO_BINS, 48_000);
        assert_eq!(bins.len(), AUDIO_BINS);
        assert!(bins.iter().all(|&v| v <= u8::MAX));
    }

}
