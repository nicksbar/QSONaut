use super::super::*;

pub(crate) const BAND_PLAN: &[(&str, u64)] = &[
    ("160m", 1_842_000),
    ("80m", 3_578_000),
    ("60m", 5_357_000),
    ("40m", 7_078_000),
    ("30m", 10_130_000),
    ("20m", 14_078_000),
    ("17m", 18_104_000),
    ("15m", 21_078_000),
    ("12m", 24_922_000),
    ("10m", 28_078_000),
    ("6m", 50_318_000),
];

pub(crate) fn parse_js8_compose(compose: &str) -> Option<qsonaut_js8::Js8Message> {
    let tokens: Vec<&str> = compose.split_whitespace().collect();
    let first = tokens.first()?.to_ascii_uppercase();
    if first == "CQ" || first == "HEARTBEAT" {
        let callsign = tokens.get(1)?.to_ascii_uppercase();
        let grid = tokens.get(2).map(|value| value.to_ascii_uppercase());
        return Some(qsonaut_js8::Js8Message::Heartbeat {
            callsign,
            grid,
            cq: first == "CQ",
            subtype: 0,
        });
    }
    if tokens.len() >= 3 && is_probable_callsign(&first) && is_probable_callsign(tokens[1]) {
        let command_name = tokens[2].to_ascii_uppercase();
        let (name, code) = js8_command_code(&command_name)?;
        let number = if code == 25 {
            tokens.get(3)?.parse::<i8>().ok()
        } else {
            None
        };
        return Some(qsonaut_js8::Js8Message::Directed {
            from: first,
            to: tokens[1].to_ascii_uppercase(),
            command: qsonaut_js8::Js8Command {
                code,
                name: name.to_string(),
                number,
            },
        });
    }
    if is_probable_callsign(&first) {
        if let Some(command_name) = tokens.get(1..) {
            let command_name = command_name.join(" ").to_ascii_uppercase();
            if let Some((name, code)) = js8_command_code(&command_name) {
                return Some(qsonaut_js8::Js8Message::Compound {
                    callsign: first,
                    grid: None,
                    command: Some(qsonaut_js8::Js8Command {
                        code,
                        name: name.to_string(),
                        number: None,
                    }),
                    directed: true,
                });
            }
        }
        return Some(qsonaut_js8::Js8Message::Compound {
            callsign: first,
            grid: tokens.get(1).map(|value| value.to_ascii_uppercase()),
            command: None,
            directed: false,
        });
    }
    if tokens.len() == 1 && compose.trim().chars().count() == 12 {
        return Some(qsonaut_js8::Js8Message::Raw {
            frame_type: qsonaut_js8::Js8FrameType::Unknown(0),
            payload: compose.trim().to_ascii_uppercase(),
        });
    }
    None
}

fn js8_command_code(command: &str) -> Option<(&'static str, u8)> {
    Some(match command {
        "SNR?" => ("SNR?", 0),
        "GRID?" => ("GRID?", 4),
        "STATUS?" => ("STATUS?", 6),
        "STATUS" => ("STATUS", 7),
        "HEARING?" => ("HEARING?", 3),
        "ACK" => ("ACK", 14),
        "GRID" => ("GRID", 15),
        "INFO?" => ("INFO?", 16),
        "INFO" => ("INFO", 17),
        "QSL?" => ("QSL?", 22),
        "QSL" => ("QSL", 23),
        "73" => ("73", 28),
        "SNR" => ("SNR", 25),
        _ => return None,
    })
}

pub(crate) fn format_js8_message(message: &qsonaut_js8::Js8Message) -> String {
    match message {
        qsonaut_js8::Js8Message::Heartbeat {
            callsign, grid, cq, ..
        } => {
            let prefix = if *cq { "CQ" } else { "HEARTBEAT" };
            match grid {
                Some(grid) => format!("{prefix} {callsign} {grid}"),
                None => format!("{prefix} {callsign}"),
            }
        }
        qsonaut_js8::Js8Message::Compound {
            callsign,
            grid,
            command,
            directed,
        } => {
            let mut text = callsign.clone();
            if let Some(grid) = grid {
                text.push(' ');
                text.push_str(grid);
            }
            if let Some(command) = command {
                text.push(' ');
                text.push_str(&command.name);
            }
            if *directed {
                format!("DIRECTED {text}")
            } else {
                text
            }
        }
        qsonaut_js8::Js8Message::Directed { from, to, command } => {
            let number = command
                .number
                .map(|number| format!(" {number}"))
                .unwrap_or_default();
            format!("{from} {to} {}{number}", command.name)
        }
        qsonaut_js8::Js8Message::Data {
            encoded,
            compressed,
        } => {
            let kind = if *compressed {
                "DATA-COMPRESSED"
            } else {
                "DATA"
            };
            format!("{kind} {encoded}")
        }
        qsonaut_js8::Js8Message::Raw { payload, .. } => payload.clone(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Js8Controls {
    pub(crate) mode: qsonaut_js8::Js8Mode,
    pub(crate) frame_type: u8,
    pub(crate) rx_frequency_half_width_hz: f32,
    pub(crate) rx_frequency_step_hz: f32,
    pub(crate) max_fec_iterations: usize,
    pub(crate) waterfall: bool,
    pub(crate) scan_start_sample: usize,
    pub(crate) scan_step_samples: usize,
    pub(crate) scan_max_candidates: usize,
    pub(crate) scan_dedup_samples: usize,
    pub(crate) minimum_sync_quality: f32,
    pub(crate) sync_frequency_half_width_hz: f32,
    pub(crate) sync_frequency_step_hz: f32,
    pub(crate) waterfall_low_hz: f32,
    pub(crate) waterfall_high_hz: f32,
    pub(crate) max_frequency_hypotheses: usize,
    pub(crate) max_signals_per_window: usize,
}

impl Default for Js8Controls {
    fn default() -> Self {
        let scan = qsonaut_js8::Js8ScanConfig::waterfall();
        Self {
            mode: qsonaut_js8::Js8Mode::Normal,
            frame_type: 0,
            rx_frequency_half_width_hz: 12.0,
            rx_frequency_step_hz: 0.5,
            max_fec_iterations: 10,
            waterfall: true,
            scan_start_sample: scan.start_sample,
            scan_step_samples: scan.step_samples,
            scan_max_candidates: scan.max_candidates,
            scan_dedup_samples: scan.dedup_samples,
            minimum_sync_quality: scan.minimum_sync_quality,
            sync_frequency_half_width_hz: scan.sync_frequency_half_width_hz,
            sync_frequency_step_hz: scan.sync_frequency_step_hz,
            waterfall_low_hz: 200.0,
            waterfall_high_hz: 3_000.0,
            // Two ranked hypotheses cover the strongest waterfall carriers
            // while avoiding a large number of expensive exact timing/FEC
            // attempts for every residual signal in a live slot. The scan
            // still extracts multiple carriers; this only bounds retries per
            // carrier when coarse acquisition produces close contenders.
            max_frequency_hypotheses: scan.max_frequency_hypotheses.min(2),
            max_signals_per_window: scan.max_signals_per_window,
        }
    }
}

impl Js8Controls {
    pub(crate) fn rx_config(self, center_frequency_hz: f32) -> qsonaut_js8::Js8RxConfig {
        qsonaut_js8::Js8RxConfig {
            mode: self.mode,
            center_frequency_hz,
            frequency_half_width_hz: self.rx_frequency_half_width_hz,
            frequency_step_hz: self.rx_frequency_step_hz,
            max_fec_iterations: self.max_fec_iterations,
        }
    }

    pub(crate) fn scan_config(self) -> qsonaut_js8::Js8ScanConfig {
        let (waterfall_low_hz, waterfall_high_hz) = (
            self.waterfall_low_hz.min(self.waterfall_high_hz),
            self.waterfall_low_hz.max(self.waterfall_high_hz),
        );
        let mut config = if self.waterfall {
            qsonaut_js8::Js8ScanConfig::waterfall()
        } else {
            qsonaut_js8::Js8ScanConfig::default()
        };
        config.start_sample = self.scan_start_sample;
        config.step_samples = self.scan_step_samples.max(1);
        config.max_candidates = self.scan_max_candidates.max(1);
        config.dedup_samples = self.scan_dedup_samples;
        config.minimum_sync_quality = self.minimum_sync_quality;
        config.sync_frequency_half_width_hz = self.sync_frequency_half_width_hz;
        config.sync_frequency_step_hz = self.sync_frequency_step_hz.max(0.1);
        config.waterfall_frequency_range_hz = self
            .waterfall
            .then_some((waterfall_low_hz, waterfall_high_hz));
        config.max_frequency_hypotheses = self.max_frequency_hypotheses.max(1);
        config.max_signals_per_window = self.max_signals_per_window.max(1);
        config
    }
}

fn js8_status(app: &QsonautGuiApp) -> &'static str {
    if app
        .digital_tx_active
        .load(std::sync::atomic::Ordering::Acquire)
    {
        "TX IN PROGRESS"
    } else if app.digital_tx_started.is_some()
        || app
            .digital_tx_status
            .to_ascii_uppercase()
            .contains("QUEUED")
    {
        "TX ARMED"
    } else {
        "RX · TX DISARMED"
    }
}

fn js8_heard_callsign(message: &str) -> Option<String> {
    let upper = message.trim().to_ascii_uppercase();
    if upper.split_whitespace().any(|token| token == "@ALLCALL") {
        return Some("@ALLCALL".to_string());
    }
    if let Some(call) = message
        .trim()
        .to_ascii_uppercase()
        .strip_prefix("CQ+")
        .and_then(|payload| payload.split('+').next())
    {
        if is_probable_callsign(call) {
            return Some(call.to_string());
        }
    }
    message
        .trim()
        .to_ascii_uppercase()
        .split('+')
        .next()
        .filter(|call| is_probable_callsign(call))
        .map(str::to_string)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Js8TrafficKind {
    Conversation,
    Announcement,
    Presence,
    Maintenance,
}

fn js8_traffic_kind(message: &str) -> Js8TrafficKind {
    let upper = message.trim().to_ascii_uppercase();
    if upper.starts_with("CQ ") {
        return Js8TrafficKind::Announcement;
    }
    if upper.starts_with("HEARTBEAT ") || upper.ends_with(" 73") {
        return Js8TrafficKind::Presence;
    }
    if [
        "GRID?", "STATUS?", "STATUS", "HEARING?", "INFO?", "INFO", "SNR?",
    ]
    .iter()
    .any(|command| upper.split_whitespace().last() == Some(*command))
    {
        return Js8TrafficKind::Maintenance;
    }
    Js8TrafficKind::Conversation
}

fn js8_traffic_label(kind: Js8TrafficKind) -> &'static str {
    match kind {
        Js8TrafficKind::Conversation => "conversation",
        Js8TrafficKind::Announcement => "announcement",
        Js8TrafficKind::Presence => "presence",
        Js8TrafficKind::Maintenance => "maintenance",
    }
}

fn js8_entry_callsign(entry: &DigitalDecodeEntry) -> Option<String> {
    parse_message(&entry.message)
        .map(|message| message.from)
        .or_else(|| js8_heard_callsign(&entry.message))
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::{
        format_js8_message, js8_heard_callsign, js8_status, js8_traffic_kind, parse_js8_compose,
        Js8Controls, Js8TrafficKind, BAND_PLAN,
    };
    use crate::{DigitalDecodeEntry, DigitalTxChatEntry, QsonautGuiApp, WorkspaceMode};
    use std::sync::{Arc, Mutex};

    #[test]
    fn exposes_the_initial_hf_js8_calling_plan() {
        assert_eq!(BAND_PLAN.first(), Some(&("160m", 1_842_000)));
        assert!(BAND_PLAN.iter().any(|(band, _)| *band == "20m"));
        assert!(BAND_PLAN.windows(2).all(|window| window[0].1 < window[1].1));
    }

    #[test]
    fn builds_human_readable_semantic_conversation_messages() {
        let message = parse_js8_compose("CQ N7UF CN87").expect("CQ message");
        assert_eq!(format_js8_message(&message), "CQ N7UF CN87");

        let message = parse_js8_compose("W1AW FN31").expect("station message");
        assert_eq!(format_js8_message(&message), "W1AW FN31");

        let message = parse_js8_compose("HEARTBEAT N7UF CN87").expect("heartbeat");
        assert_eq!(format_js8_message(&message), "HEARTBEAT N7UF CN87");

        let message = parse_js8_compose("W1AW N7UF GRID?").expect("directed command");
        assert_eq!(format_js8_message(&message), "W1AW N7UF GRID?");

        let message = parse_js8_compose("N7UF W1AW SNR -10").expect("numbered command");
        assert_eq!(format_js8_message(&message), "N7UF W1AW SNR -10");
        assert_eq!(
            js8_traffic_kind("CQ QZ0NA CN87"),
            Js8TrafficKind::Announcement
        );
        assert_eq!(
            js8_traffic_kind("HEARTBEAT QZ0NA CN87"),
            Js8TrafficKind::Presence
        );
        assert_eq!(
            js8_traffic_kind("QZ0NA QZ1NB GRID?"),
            Js8TrafficKind::Maintenance
        );
        assert_eq!(
            js8_traffic_kind("QZ0NA QZ1NB QSL"),
            Js8TrafficKind::Conversation
        );
        assert_eq!(
            format_js8_message(&qsonaut_js8::Js8Message::Data {
                encoded: "ABC".into(),
                compressed: false,
            }),
            "DATA ABC"
        );
        assert_eq!(
            format_js8_message(&qsonaut_js8::Js8Message::Data {
                encoded: "ABC".into(),
                compressed: true,
            }),
            "DATA-COMPRESSED ABC"
        );
        assert_eq!(
            format_js8_message(&qsonaut_js8::Js8Message::Raw {
                frame_type: qsonaut_js8::Js8FrameType::Unknown(1),
                payload: "RAW".into(),
            }),
            "RAW"
        );
    }

    #[test]
    fn parses_all_initial_compose_shapes_and_heard_callsigns() {
        assert!(parse_js8_compose("HEARTBEAT QZ0NA").is_some());
        assert!(parse_js8_compose("QZ0NA QZ1NB ACK").is_some());
        assert!(parse_js8_compose("QZ0NA QZ1NB SNR -12").is_some());
        assert!(parse_js8_compose("QZ0NA QZ1NB QSL").is_some());
        assert!(parse_js8_compose("QZ0NA CN87").is_some());
        assert!(parse_js8_compose("0123456789AB").is_some());
        assert!(parse_js8_compose("not a JS8 message").is_none());
        assert_eq!(js8_heard_callsign("CQ+QZ0NA+CN87"), Some("QZ0NA".into()));
        assert_eq!(
            js8_heard_callsign("QZ0NA+QZ1NB+GRID?"),
            Some("QZ0NA".into())
        );
        assert_eq!(
            js8_heard_callsign("@ALLCALL HEARTBEAT"),
            Some("@ALLCALL".into())
        );
        assert_eq!(js8_heard_callsign("noise"), None);
    }

    #[test]
    fn normalizes_scan_controls_and_renders_activity_workspace() {
        let mut controls = Js8Controls::default();
        controls.waterfall_low_hz = 3_000.0;
        controls.waterfall_high_hz = 200.0;
        controls.scan_step_samples = 0;
        controls.scan_max_candidates = 0;
        controls.sync_frequency_step_hz = 0.0;
        controls.max_frequency_hypotheses = 0;
        controls.max_signals_per_window = 0;
        let scan = controls.scan_config();
        assert_eq!(scan.waterfall_frequency_range_hz, Some((200.0, 3_000.0)));
        assert_eq!(scan.step_samples, 1);
        assert_eq!(scan.max_candidates, 1);
        assert_eq!(scan.sync_frequency_step_hz, 0.1);
        assert_eq!(scan.max_frequency_hypotheses, 1);
        assert_eq!(scan.max_signals_per_window, 1);
        assert_eq!(controls.rx_config(1_500.0).center_frequency_hz, 1_500.0);

        let icon = eframe::icon_data::from_png_bytes(crate::QSONAUT_ICON_PNG).unwrap();
        let context = crate::egui::Context::default();
        let mut config = crate::AppConfig::default();
        config.radio.enabled = false;
        let mut app = QsonautGuiApp::new_with_context(
            config,
            false,
            false,
            &context,
            &icon,
            eframe::Renderer::Wgpu,
            None,
            crate::GraphicsPreferences::from_environment(),
            None,
            Vec::new(),
            Arc::new(Mutex::new(None)),
        );
        app.js8_controls = controls;
        app.js8_target = Some("QZ0NA".to_string());
        assert_eq!(js8_status(&app), "RX · TX DISARMED");
        app.digital_tx_status = "queued for next slot".to_string();
        assert_eq!(js8_status(&app), "TX ARMED");
        app.digital_tx_active
            .store(true, std::sync::atomic::Ordering::Release);
        assert_eq!(js8_status(&app), "TX IN PROGRESS");
        app.digital_tx_chat.push_back(DigitalTxChatEntry {
            mode: WorkspaceMode::Js8,
            period: 2,
            utc: "12:00:02".to_string(),
            message: "CQ QZ0NA CN87".to_string(),
        });
        let mut snapshot = crate::GuiState::default();
        snapshot.digital_decode_status = "JS8 listening".to_string();
        snapshot.digital_decodes.push_back(DigitalDecodeEntry {
            mode: WorkspaceMode::Js8,
            period: 1,
            utc: "12:00:01".to_string(),
            snr_db: -8.0,
            dt_s: 0.2,
            freq_hz: 1_500,
            message: "CQ QZ0NA CN87".to_string(),
        });
        snapshot.digital_decodes.push_back(DigitalDecodeEntry {
            mode: WorkspaceMode::Js8,
            period: 2,
            utc: "12:00:02".to_string(),
            snr_db: 3.0,
            dt_s: 0.1,
            freq_hz: 1_700,
            message: "QZ0NA QZ1NB GRID?".to_string(),
        });
        let _ = context.run(Default::default(), |ctx| {
            crate::egui::CentralPanel::default().show(ctx, |ui| {
                app.draw_js8_workspace(ui, &snapshot);
            });
        });
        snapshot.digital_decodes.clear();
        let _ = context.run(Default::default(), |ctx| {
            crate::egui::CentralPanel::default().show(ctx, |ui| {
                app.draw_js8_workspace(ui, &snapshot);
            });
        });
    }
}

impl QsonautGuiApp {
    pub(crate) fn draw_js8_workspace(&mut self, ui: &mut egui::Ui, snapshot: &GuiState) {
        // This workspace lives below the optional waterfall deck. Do not add
        // a minimum height here: doing so pushes the composer below the
        // viewport whenever the waterfall is tall or the window is short.
        let workspace_height = ui.available_height().max(0.0);
        let conversation_height = (workspace_height - 142.0).max(72.0);
        let mut heard = std::collections::BTreeMap::new();
        let mut lines = Vec::new();
        for entry in snapshot
            .digital_decodes
            .iter()
            .filter(|entry| entry.mode == WorkspaceMode::Js8)
        {
            let call = js8_entry_callsign(entry);
            if let Some(call) = call.as_ref() {
                heard.insert(call.clone(), entry.freq_hz);
            }
            let kind = js8_traffic_kind(&entry.message);
            lines.push(Ft8ChatLine {
                period: entry.period,
                utc: entry.utc.clone(),
                message: entry.message.clone(),
                detail: format!(
                    "{} · RX {:+.1} dB · {} Hz",
                    js8_traffic_label(kind),
                    entry.snr_db,
                    entry.freq_hz
                ),
                direction: Ft8ChatDirection::Rx,
            });
        }
        for entry in self
            .digital_tx_chat
            .iter()
            .filter(|entry| entry.mode == WorkspaceMode::Js8)
        {
            lines.push(Ft8ChatLine {
                period: entry.period,
                utc: entry.utc.clone(),
                message: entry.message.clone(),
                detail: "TX".to_string(),
                direction: Ft8ChatDirection::Tx,
            });
        }
        lines.sort_by_key(|line| (line.period, line.direction == Ft8ChatDirection::Tx));

        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_min_height(workspace_height);
            ui.set_max_height(workspace_height);
            ui.horizontal(|ui| {
                ui.heading("JS8");
                ui.separator();
                ui.label(RichText::new(js8_status(self)).strong());
                ui.separator();
                ui.label(format!(
                    "{:?} · {} s",
                    self.js8_controls.mode,
                    self.js8_controls.mode.tx_seconds()
                ));
                ui.separator();
                ui.label(format!("{} stations", heard.len()));
                ui.separator();
                ui.label(
                    RichText::new(&snapshot.digital_decode_status)
                        .small()
                        .color(theme_muted(ui)),
                );
            });
            ui.horizontal(|ui| {
                ui.label(RichText::new("Tune behavior").strong());
                let label = if self.js8_tune_tx_with_rx {
                    "RX + TX follow"
                } else {
                    "RX only"
                };
                if ui
                    .selectable_label(self.js8_tune_tx_with_rx, label)
                    .on_hover_text(
                        "When enabled, Tune moves the receive and transmit audio cursors together.",
                    )
                    .clicked()
                {
                    self.js8_tune_tx_with_rx = !self.js8_tune_tx_with_rx;
                }
                ui.label(
                    RichText::new(format!(
                        "RX {} Hz · TX {} Hz",
                        self.rx_tone_hz, self.tx_tone_hz
                    ))
                    .small()
                    .color(theme_muted(ui)),
                );
            });
            let slot_seconds = self.js8_controls.mode.tx_seconds() as f64;
            let now_s = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_secs_f64())
                .unwrap_or(0.0);
            let progress = (now_s.rem_euclid(slot_seconds) / slot_seconds) as f32;
            ui.add(
                egui::ProgressBar::new(progress)
                    .desired_width(240.0)
                    .text(format!(
                        "next slot in {:.1}s",
                        slot_seconds * (1.0 - f64::from(progress))
                    )),
            );
            ui.separator();
            egui::SidePanel::right("js8-heard-panel")
                .resizable(true)
                .default_width((ui.available_width() * 0.28).clamp(190.0, 300.0))
                .width_range(170.0..=360.0)
                .show_inside(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .max_height(conversation_height)
                        .show(ui, |ui| {
                            egui::Frame::group(ui.style()).show(ui, |ui| {
                                ui.heading("Heard recently");
                                ui.separator();
                                if heard.is_empty() {
                                    ui.label(
                                        RichText::new("No stations heard yet").color(theme_muted(ui)),
                                    );
                                } else {
                                    for (call, frequency_hz) in &heard {
                                        if ui
                                            .selectable_label(
                                                self.js8_target.as_deref() == Some(call),
                                                RichText::new(call).monospace().strong(),
                                            )
                                            .on_hover_text(format!(
                                                "Select {call} and tune RX to {frequency_hz} Hz"
                                            ))
                                            .clicked()
                                        {
                                            self.js8_target = Some(call.clone());
                                            self.rx_tone_hz = *frequency_hz;
                                            if self.js8_tune_tx_with_rx {
                                                self.tx_tone_hz = *frequency_hz;
                                            }
                                        }
                                    }
                                }
                                ui.separator();
                                ui.label(RichText::new("Conversation target").strong());
                                egui::ComboBox::from_id_salt("js8-target")
                                    .selected_text(
                                        self.js8_target
                                            .as_deref()
                                            .unwrap_or("Everyone in passband"),
                                    )
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(
                                            &mut self.js8_target,
                                            None,
                                            "Everyone in passband",
                                        );
                                        ui.selectable_value(
                                            &mut self.js8_target,
                                            Some("@ALLCALL".to_string()),
                                            "@ALLCALL group",
                                        );
                                        for call in heard.keys() {
                                            ui.selectable_value(
                                                &mut self.js8_target,
                                                Some(call.clone()),
                                                call,
                                            );
                                        }
                                    });
                                if let Some(target) = self.js8_target.as_deref() {
                                    if target == "@ALLCALL" {
                                        ui.label(
                                            RichText::new(
                                                "Group target selected; group-directed TX support is pending in the modem message API.",
                                            )
                                            .small()
                                            .color(theme_muted(ui)),
                                        );
                                    } else {
                                        ui.label(
                                            RichText::new(format!(
                                                "Target {target} · select an offset before TX"
                                            ))
                                            .small()
                                            .color(theme_muted(ui)),
                                        );
                                    }
                                }
                                ui.separator();
                                ui.label(RichText::new("Protocol actions").strong());
                                ui.label(
                                    RichText::new("CQ, heartbeat, directed commands, and inbox actions will use the typed JS8 message layer.")
                                        .small()
                                        .color(theme_muted(ui)),
                                );
                            });
                            ui.collapsing("JS8 controls", |ui| {
                                ui.horizontal(|ui| {
                                    ui.label("Speed");
                                    egui::ComboBox::from_id_salt("js8-mode")
                                        .selected_text(format!("{:?}", self.js8_controls.mode))
                                        .show_ui(ui, |ui| {
                                            for mode in [
                                                qsonaut_js8::Js8Mode::Normal,
                                                qsonaut_js8::Js8Mode::Fast,
                                                qsonaut_js8::Js8Mode::Turbo,
                                                qsonaut_js8::Js8Mode::Slow,
                                                qsonaut_js8::Js8Mode::Ultra,
                                            ] {
                                                ui.selectable_value(
                                                    &mut self.js8_controls.mode,
                                                    mode,
                                                    format!("{:?} · {} s", mode, mode.tx_seconds()),
                                                );
                                            }
                                        });
                                });
                                ui.horizontal(|ui| {
                                    ui.label("TX frame");
                                    ui.add(egui::Slider::new(
                                        &mut self.js8_controls.frame_type,
                                        0..=7,
                                    ));
                                    ui.label(format!(
                                        "{:?}",
                                        qsonaut_js8::Js8FrameType::from(
                                            self.js8_controls.frame_type,
                                        )
                                    ));
                                });
                                ui.label(RichText::new("Receive scope").strong());
                                ui.horizontal(|ui| {
                                    ui.selectable_value(
                                        &mut self.js8_controls.waterfall,
                                        true,
                                        "Full passband",
                                    );
                                    ui.selectable_value(
                                        &mut self.js8_controls.waterfall,
                                        false,
                                        "Selected offset",
                                    );
                                });
                                ui.add(
                                    egui::Slider::new(
                                        &mut self.js8_controls.rx_frequency_half_width_hz,
                                        1.0..=50.0,
                                    )
                                    .text("RX width ±Hz"),
                                );
                                ui.add(
                                    egui::Slider::new(
                                        &mut self.js8_controls.rx_frequency_step_hz,
                                        0.1..=5.0,
                                    )
                                    .text("RX step Hz"),
                                );
                                ui.add(
                                    egui::Slider::new(
                                        &mut self.js8_controls.max_fec_iterations,
                                        1..=50,
                                    )
                                    .text("FEC iterations"),
                                );
                                if self.js8_controls.waterfall {
                                    ui.separator();
                                    ui.label(RichText::new("Waterfall scan").strong());
                                    ui.add(
                                        egui::Slider::new(
                                            &mut self.js8_controls.waterfall_low_hz,
                                            0.0..=3_500.0,
                                        )
                                        .text("Low Hz"),
                                    );
                                    ui.add(
                                        egui::Slider::new(
                                            &mut self.js8_controls.waterfall_high_hz,
                                            500.0..=4_000.0,
                                        )
                                        .text("High Hz"),
                                    );
                                    ui.add(
                                        egui::Slider::new(
                                            &mut self.js8_controls.scan_start_sample,
                                            0..=48_000,
                                        )
                                        .text("Start sample"),
                                    );
                                    ui.add(
                                        egui::Slider::new(
                                            &mut self.js8_controls.sync_frequency_step_hz,
                                            1.0..=25.0,
                                        )
                                        .text("Coarse step Hz"),
                                    );
                                    ui.add(
                                        egui::Slider::new(
                                            &mut self.js8_controls.sync_frequency_half_width_hz,
                                            0.0..=100.0,
                                        )
                                        .text("Coarse width ±Hz"),
                                    );
                                    ui.add(
                                        egui::Slider::new(
                                            &mut self.js8_controls.minimum_sync_quality,
                                            0.0..=1.0,
                                        )
                                        .text("Minimum sync"),
                                    );
                                    ui.add(
                                        egui::Slider::new(
                                            &mut self.js8_controls.max_frequency_hypotheses,
                                            1..=16,
                                        )
                                        .text("Frequency hypotheses"),
                                    );
                                    ui.add(
                                        egui::Slider::new(
                                            &mut self.js8_controls.max_signals_per_window,
                                            1..=8,
                                        )
                                        .text("Signals per window"),
                                    );
                                    ui.add(
                                        egui::Slider::new(
                                            &mut self.js8_controls.scan_max_candidates,
                                            1..=512,
                                        )
                                        .text("Candidate windows"),
                                    );
                                    ui.add(
                                        egui::Slider::new(
                                            &mut self.js8_controls.scan_step_samples,
                                            1_000..=48_000,
                                        )
                                        .text("Candidate step samples"),
                                    );
                                    ui.add(
                                        egui::Slider::new(
                                            &mut self.js8_controls.scan_dedup_samples,
                                            0..=240_000,
                                        )
                                        .text("Dedup distance samples"),
                                    );
                                }
                            });
                        });
                });
            egui::Frame::dark_canvas(ui.style()).show(ui, |ui| {
                ui.set_max_height(conversation_height);
                ui.heading("Conversation");
                ui.label(
                    RichText::new("Everyone heard on this frequency appears in the room.")
                        .small()
                        .color(theme_muted(ui)),
                );
                ui.separator();
                egui::ScrollArea::vertical()
                    .id_salt("js8-conversation")
                    .max_height((conversation_height - 55.0).max(24.0))
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        if lines.is_empty() {
                            ui.centered_and_justified(|ui| {
                                ui.label(
                                    RichText::new(
                                        "Listening for JS8 activity… decoded messages will appear here.",
                                    )
                                    .color(theme_muted(ui)),
                                );
                            });
                        }
                        for line in &lines {
                            let is_tx = line.direction == Ft8ChatDirection::Tx;
                            let layout = if is_tx {
                                egui::Layout::right_to_left(egui::Align::Min)
                            } else {
                                egui::Layout::left_to_right(egui::Align::Min)
                            };
                            ui.with_layout(layout, |ui| {
                                let fill = if is_tx {
                                    Color32::from_rgb(53, 43, 25)
                                } else {
                                    Color32::from_rgb(25, 49, 38)
                                };
                                egui::Frame::group(ui.style()).fill(fill).show(ui, |ui| {
                                    ui.label(RichText::new(&line.message).strong());
                                    ui.label(
                                        RichText::new(format!("{} · {}", line.utc, line.detail))
                                            .small()
                                            .color(theme_muted(ui)),
                                    );
                                });
                            });
                            ui.add_space(2.0);
                        }
                    });
            });
            ui.separator();
            ui.vertical(|ui| {
                ui.label("Message");
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("Quick compose").small().color(theme_muted(ui)));
                    if ui.button("CQ").clicked() {
                        self.digital_compose = format!(
                            "CQ {} {}",
                            self.station_callsign_or_default(),
                            self.station_grid_or_default()
                        );
                    }
                    if ui.button("Heartbeat").clicked() {
                        self.digital_compose = format!(
                            "HEARTBEAT {} {}",
                            self.station_callsign_or_default(),
                            self.station_grid_or_default()
                        );
                    }
                });
                ui.add(
                    egui::TextEdit::singleline(&mut self.digital_compose)
                        .desired_width(ui.available_width())
                        .hint_text("CQ CALL GRID, HEARTBEAT CALL GRID, CALL GRID, or raw 12-character payload")
                        .font(egui::TextStyle::Monospace),
                );
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .add_enabled(
                            !self.digital_compose.trim().is_empty()
                                && !self
                                    .digital_tx_active
                                    .load(std::sync::atomic::Ordering::Acquire),
                            egui::Button::new("Send next slot"),
                        )
                        .clicked()
                    {
                        self.queue_native_digital_tx(WorkspaceMode::Js8);
                    }
                    if ui
                        .add_enabled(
                            self.digital_tx_active
                                .load(std::sync::atomic::Ordering::Acquire),
                            egui::Button::new("Stop TX"),
                        )
                        .clicked()
                    {
                        self.stop_native_digital_tx();
                    }
                });
            });
            ui.label(
                RichText::new(
                    "Semantic CQ and heartbeat messages use the modem message layer; raw payloads remain an advanced fallback. TX is globally covered by QSONaut disarm.",
                )
                .small()
                .color(theme_muted(ui)),
            );
        });
    }
}
