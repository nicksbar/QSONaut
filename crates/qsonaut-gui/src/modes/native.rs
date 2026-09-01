use super::super::*;
use super::digital_conversation::draw_digital_conversation;

pub(crate) const MSK144_BAND_PLAN: &[(&str, u64)] = &[
    ("160m", 1_840_000),
    ("80m", 3_579_000),
    ("60m", 5_357_000),
    ("40m", 7_099_000),
    ("30m", 10_136_000),
    ("20m", 14_099_000),
    ("17m", 18_100_000),
    ("15m", 21_099_000),
    ("12m", 24_924_000),
    ("10m", 28_099_000),
    ("6m", 50_313_000),
    ("2m", 144_138_000),
    ("70cm", 432_075_000),
];
pub(crate) const FLDIGI_BAND_PLAN: &[(&str, u64)] = &[
    ("160m", 1_840_000),
    ("80m", 3_573_000),
    ("60m", 5_357_000),
    ("40m", 7_074_000),
    ("30m", 10_136_000),
    ("20m", 14_074_000),
    ("17m", 18_100_000),
    ("15m", 21_074_000),
    ("12m", 24_924_000),
    ("10m", 28_074_000),
    ("6m", 50_313_000),
];

fn native_radio_mode_label(preset: crate::band_plan::WorkspaceRadioPreset) -> &'static str {
    match (preset.base_mode, preset.data_mode) {
        (BaseMode::Usb, true) => "USB-D",
        (BaseMode::Usb, false) => "USB",
        (BaseMode::Lsb, true) => "LSB-D",
        (BaseMode::Lsb, false) => "LSB",
        (BaseMode::Cw | BaseMode::CwR, _) => "CW",
        (BaseMode::Rtty | BaseMode::RttyR, true) => "RTTY-D",
        (BaseMode::Rtty | BaseMode::RttyR, false) => "RTTY",
        _ => "DIGITAL",
    }
}

fn native_slot_label(mode: WorkspaceMode, fst4_submode: crate::modes::fst4::Submode) -> String {
    mode.slot_seconds(fst4_submode).map_or_else(
        || "Continuous".to_string(),
        |seconds| {
            if seconds.fract() == 0.0 {
                format!("{seconds:.0} s")
            } else {
                format!("{seconds:.1} s")
            }
        },
    )
}

fn native_mode_guidance(mode: WorkspaceMode) -> &'static str {
    match mode {
        WorkspaceMode::Wspr => {
            "WSPR is a 120-second propagation beacon. TX format: CALL GRID POWER_DBM; use one-shot transmissions only."
        }
        WorkspaceMode::Fst4 => {
            "FST4-60 is currently selected by the decoder. TX is one timed frame; verify the slot countdown before sending."
        }
        WorkspaceMode::Jt9 => {
            "JT9 uses 60-second slots. The native panel provides manual one-shot TX; sequencing is not enabled yet."
        }
        WorkspaceMode::Jt65 => {
            "JT65 uses 60-second slots. The native panel provides manual one-shot TX; sequencing is not enabled yet."
        }
        WorkspaceMode::Q65 => {
            "Q65-A30 is currently selected by the decoder. The native panel provides manual one-shot TX."
        }
        _ => "Native digital TX is one-shot and slot-timed; use STOP TX to disarm a queued frame.",
    }
}

impl QsonautGuiApp {
    pub(crate) fn draw_mfsk_mode_workspace(
        &mut self,
        ui: &mut egui::Ui,
        snapshot: &GuiState,
        mode: WorkspaceMode,
    ) {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_min_height(360.0);
            let deck_rect = ui.available_rect_before_wrap();
            ui.allocate_rect(deck_rect, egui::Sense::hover());
            let mut deck_ui = ui.new_child(
                egui::UiBuilder::new()
                    .id_salt(("native-mode-deck", mode.label()))
                    .max_rect(deck_rect)
                    .layout(egui::Layout::top_down(egui::Align::Min)),
            );
            deck_ui.set_clip_rect(deck_rect);
            deck_ui.columns(2, |columns| {
                let (left_columns, right_columns) = columns.split_at_mut(1);
                let left = &mut left_columns[0];
                let right = &mut right_columns[0];
                left.set_min_width(0.0);
                right.set_min_width(0.0);
                egui::Frame::dark_canvas(left.style()).show(left, |ui| {
                    self.draw_mfsk_mode_details(ui, snapshot, mode);
                });
                egui::Frame::group(right.style()).show(right, |ui| {
                    let lines = self
                        .digital_tx_chat
                        .iter()
                        .filter(|entry| entry.mode == mode)
                        .map(|entry| Ft8ChatLine {
                            period: entry.period,
                            utc: entry.utc.clone(),
                            message: entry.message.clone(),
                            detail: "TX".to_string(),
                            direction: Ft8ChatDirection::Tx,
                        })
                        .collect();
                    draw_digital_conversation(
                        ui,
                        ui.available_height(),
                        "native-conversation",
                        format!("💬 {} CONVERSATION", mode.label()),
                        self.native_sessions
                            .get(&mode)
                            .map(|session| qso_stage_label(session.stage)),
                        "Select a decode to track that callsign here.",
                        self.station_callsign_or_default(),
                        self.rx_tone_hz,
                        self.tx_tone_hz,
                        audio_cursor_level(&snapshot.audio_waterfall_rows, self.rx_tone_hz),
                        audio_cursor_level(&snapshot.audio_waterfall_rows, self.tx_tone_hz),
                        lines,
                    );
                });
            });
        });
    }

    fn draw_mfsk_mode_details(
        &mut self,
        ui: &mut egui::Ui,
        snapshot: &GuiState,
        mode: WorkspaceMode,
    ) {
        let preset = workspace_radio_preset(mode);
        let preset_label = native_radio_mode_label(preset);
        let slot_s = native_slot_label(mode, self.fst4_submode);

        ui.heading(mode.label());
        ui.separator();

        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Backend:").strong());
            let backend = if mode.has_native_decoder() {
                "shared WSJT adapter"
            } else if mode == WorkspaceMode::Fldigi {
                "external FLDIGI bridge"
            } else {
                "CW backend pending"
            };
            ui.label(
                RichText::new(backend)
                    .monospace()
                    .color(if mode.has_native_decoder() {
                        Color32::LIGHT_GREEN
                    } else {
                        theme_warning(ui)
                    }),
            );
            ui.separator();
            ui.label(format!("Slot: {slot_s}"));
            ui.separator();
            ui.label(format!("Radio preset: {preset_label} FIL{}", preset.filter));
            ui.separator();
            ui.label(format!(
                "Radio: {:.3} MHz  {}",
                snapshot.frequency_hz.unwrap_or_default() as f64 / 1_000_000.0,
                snapshot.mode
            ));
        });
        ui.add_space(8.0);
        ui.label(
            RichText::new(native_mode_guidance(mode))
                .small()
                .color(Color32::GRAY),
        );
        if matches!(
            mode,
            WorkspaceMode::Fst4 | WorkspaceMode::Jt9 | WorkspaceMode::Jt65 | WorkspaceMode::Q65
        ) {
            ui.horizontal_wrapped(|ui| {
                let mut armed = self.native_autoseq_mode == Some(mode);
                if ui.checkbox(&mut armed, "Automatic exchange").changed() {
                    self.native_autoseq_mode = armed.then_some(mode);
                    if !armed {
                        self.native_stop_policy = AutoTxStopPolicy::Continuous;
                    }
                }
                ui.label("Reply priority");
                egui::ComboBox::from_id_salt(("native_reply_policy", mode.label()))
                    .selected_text(self.native_auto_reply_policy.label())
                    .show_ui(ui, |ui| {
                        for policy in AutoReplyPolicy::ALL {
                            ui.selectable_value(
                                &mut self.native_auto_reply_policy,
                                policy,
                                policy.label(),
                            );
                        }
                    });
                ui.label("Stop");
                egui::ComboBox::from_id_salt(("native_stop_policy", mode.label()))
                    .selected_text(self.native_stop_policy.label())
                    .show_ui(ui, |ui| {
                        for policy in AutoTxStopPolicy::ALL {
                            ui.selectable_value(
                                &mut self.native_stop_policy,
                                policy,
                                policy.label(),
                            );
                        }
                    });
            });
        }
        ui.add_space(4.0);
        if let Some(slot_seconds) = mode.slot_seconds(self.fst4_submode) {
            let now_s = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_secs_f64())
                .unwrap_or(0.0);
            let progress = (now_s % slot_seconds) / slot_seconds;
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(&snapshot.digital_decode_status)
                        .monospace()
                        .color(Color32::LIGHT_GREEN),
                );
                ui.add(
                    egui::ProgressBar::new(progress as f32)
                        .desired_width(180.0)
                        .text(format!("{:.1}s", slot_seconds * (1.0 - progress))),
                );
            });
            ui.separator();
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("UTC          SNR     dT      Hz  Message")
                        .monospace()
                        .strong(),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("Clear").clicked() {
                        self.state
                            .lock()
                            .expect("ui state lock poisoned")
                            .digital_decodes
                            .retain(|entry| entry.mode != mode);
                    }
                });
            });
            ui.separator();
            egui::ScrollArea::vertical()
                .id_salt(("digital-decodes", mode.label()))
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    let mut shown = 0usize;
                    for entry in snapshot
                        .digital_decodes
                        .iter()
                        .filter(|entry| entry.mode == mode)
                    {
                        shown += 1;
                        ui.label(
                            RichText::new(format!(
                                "{:12}  {:+5.1}  {:+6.2}  {:>5}  {}",
                                entry.utc, entry.snr_db, entry.dt_s, entry.freq_hz, entry.message
                            ))
                            .monospace(),
                        )
                        .on_hover_text(format!("decode period {}", entry.period));
                    }
                    if shown == 0 {
                        ui.label(
                            RichText::new(format!(
                                "Collecting the first complete {} slot",
                                mode.label()
                            ))
                            .color(Color32::GRAY),
                        );
                    }
                });

            ui.separator();
            let can_transmit = matches!(
                mode,
                WorkspaceMode::Ft4
                    | WorkspaceMode::Fst4
                    | WorkspaceMode::Jt9
                    | WorkspaceMode::Jt65
                    | WorkspaceMode::Q65
            );
            ui.horizontal(|ui| {
                ui.label(RichText::new("TX").strong());
                let hint = if mode == WorkspaceMode::Wspr {
                    "CALL GRID POWER_DBM (for example N0CALL FN20 37)"
                } else {
                    "CQ W1AW FN20"
                };
                ui.add_enabled(
                    can_transmit && !self.digital_tx_active.load(Ordering::Acquire),
                    egui::TextEdit::singleline(&mut self.digital_compose)
                        .desired_width((ui.available_width() - 250.0).max(180.0))
                        .hint_text(hint)
                        .font(egui::TextStyle::Monospace),
                );
                if mode == WorkspaceMode::Wspr {
                    if ui
                        .add_enabled(can_transmit, egui::Button::new("FILL BEACON"))
                        .clicked()
                    {
                        self.digital_compose = format!(
                            "{} {} 37",
                            self.station_callsign_or_default(),
                            self.station_grid_or_default()
                        );
                    }
                } else if ui
                    .add_enabled(can_transmit, egui::Button::new("CQ"))
                    .clicked()
                {
                    self.digital_compose = format!(
                        "CQ {} {}",
                        self.station_callsign_or_default(),
                        self.station_grid_or_default()
                    );
                }
                if ui
                    .add_enabled(
                        can_transmit
                            && !self.digital_compose.trim().is_empty()
                            && !self.digital_tx_active.load(Ordering::Acquire),
                        egui::Button::new("SEND NEXT SLOT"),
                    )
                    .clicked()
                {
                    self.queue_native_digital_tx(mode);
                }
                if ui
                    .add_enabled(
                        self.digital_tx_active.load(Ordering::Acquire),
                        egui::Button::new("STOP TX"),
                    )
                    .clicked()
                {
                    self.stop_native_digital_tx();
                }
            });
            ui.label(
                RichText::new(if can_transmit {
                    self.digital_tx_status.as_str()
                } else if mode == WorkspaceMode::Wspr {
                    "WSPR transmit setup requires callsign, locator, power, and beacon duty-cycle controls"
                } else {
                    "MSK144 receive is active; transmit framing is not yet exposed by the core audio API"
                })
                .color(if can_transmit {
                    Color32::GRAY
                } else {
                    theme_warning(ui)
                }),
            );
        } else {
            ui.separator();
            ui.label(
                RichText::new(
                    "FLDIGI is currently a radio preset and waterfall view. No XML-RPC modem connection is active yet.",
                )
                .color(theme_warning(ui)),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{native_mode_guidance, native_radio_mode_label, native_slot_label};
    use crate::{
        band_plan::WorkspaceRadioPreset, modes::fst4::Submode, AppConfig, BaseMode,
        GraphicsPreferences, GuiState, QsonautGuiApp, WorkspaceMode, QSONAUT_ICON_PNG,
    };
    use std::sync::{Arc, Mutex};

    fn preset(base_mode: BaseMode, data_mode: bool) -> WorkspaceRadioPreset {
        WorkspaceRadioPreset {
            base_mode,
            data_mode,
            filter: 1,
        }
    }

    #[test]
    fn labels_all_supported_native_radio_modes() {
        assert_eq!(
            native_radio_mode_label(preset(BaseMode::Usb, true)),
            "USB-D"
        );
        assert_eq!(native_radio_mode_label(preset(BaseMode::Usb, false)), "USB");
        assert_eq!(
            native_radio_mode_label(preset(BaseMode::Lsb, true)),
            "LSB-D"
        );
        assert_eq!(native_radio_mode_label(preset(BaseMode::Lsb, false)), "LSB");
        assert_eq!(native_radio_mode_label(preset(BaseMode::Cw, false)), "CW");
        assert_eq!(native_radio_mode_label(preset(BaseMode::CwR, true)), "CW");
        assert_eq!(
            native_radio_mode_label(preset(BaseMode::Rtty, true)),
            "RTTY-D"
        );
        assert_eq!(
            native_radio_mode_label(preset(BaseMode::RttyR, false)),
            "RTTY"
        );
        assert_eq!(
            native_radio_mode_label(preset(BaseMode::Am, false)),
            "DIGITAL"
        );
    }

    #[test]
    fn formats_continuous_and_timed_native_slots() {
        assert_eq!(
            native_slot_label(WorkspaceMode::Wspr, Submode::S60),
            "120 s"
        );
        assert_eq!(native_slot_label(WorkspaceMode::Jt9, Submode::S60), "60 s");
        assert_eq!(
            native_slot_label(WorkspaceMode::Cw, Submode::S60),
            "Continuous"
        );
    }

    #[test]
    fn keeps_mode_specific_operator_guidance() {
        assert!(native_mode_guidance(WorkspaceMode::Wspr).contains("120-second"));
        assert!(native_mode_guidance(WorkspaceMode::Fst4).contains("FST4-60"));
        assert!(native_mode_guidance(WorkspaceMode::Jt65).contains("JT65"));
        assert!(native_mode_guidance(WorkspaceMode::Q65).contains("Q65-A30"));
        assert!(native_mode_guidance(WorkspaceMode::Fldigi).contains("one-shot"));
    }

    #[test]
    fn draws_native_details_for_every_supported_mode_without_hardware() {
        let icon = eframe::icon_data::from_png_bytes(QSONAUT_ICON_PNG).expect("test icon");
        let context = eframe::egui::Context::default();
        let mut app = QsonautGuiApp::new_with_context(
            AppConfig::default(),
            false,
            false,
            &context,
            &icon,
            eframe::Renderer::Wgpu,
            None,
            GraphicsPreferences::from_environment(),
            None,
            Vec::new(),
            Arc::new(Mutex::new(None)),
        );
        let snapshot = GuiState {
            frequency_hz: Some(14_074_000),
            mode: "USB-D".to_string(),
            ..GuiState::default()
        };
        for mode in [
            WorkspaceMode::Ft4,
            WorkspaceMode::Fst4,
            WorkspaceMode::Jt9,
            WorkspaceMode::Jt65,
            WorkspaceMode::Q65,
            WorkspaceMode::Wspr,
            WorkspaceMode::Msk144,
            WorkspaceMode::Cw,
            WorkspaceMode::Fldigi,
        ] {
            let _ = context.run(Default::default(), |context| {
                eframe::egui::CentralPanel::default().show(context, |ui| {
                    app.draw_mfsk_mode_details(ui, &snapshot, mode);
                });
            });
        }
        for mode in [WorkspaceMode::Ft4, WorkspaceMode::Wspr, WorkspaceMode::Cw] {
            let _ = context.run(Default::default(), |context| {
                eframe::egui::CentralPanel::default().show(context, |ui| {
                    app.draw_mfsk_mode_workspace(ui, &snapshot, mode);
                });
            });
        }
    }
}
