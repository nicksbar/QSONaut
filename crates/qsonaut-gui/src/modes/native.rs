use super::super::*;

impl QsonautGuiApp {
    pub(crate) fn draw_mfsk_mode_workspace(
        &mut self,
        ui: &mut egui::Ui,
        snapshot: &GuiState,
        mode: WorkspaceMode,
    ) {
        let preset = workspace_radio_preset(mode);
        let preset_label = match (preset.base_mode, preset.data_mode) {
            (BaseMode::Usb, true) => "USB-D",
            (BaseMode::Usb, false) => "USB",
            (BaseMode::Lsb, true) => "LSB-D",
            (BaseMode::Lsb, false) => "LSB",
            (BaseMode::Cw | BaseMode::CwR, _) => "CW",
            (BaseMode::Rtty | BaseMode::RttyR, true) => "RTTY-D",
            (BaseMode::Rtty | BaseMode::RttyR, false) => "RTTY",
            _ => "DIGITAL",
        };
        let slot_s = mode.core_slot_seconds().map_or_else(
            || "Continuous".to_string(),
            |seconds| {
                if seconds.fract() == 0.0 {
                    format!("{seconds:.0} s")
                } else {
                    format!("{seconds:.1} s")
                }
            },
        );

        ui.heading(mode.label());
        ui.separator();

        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Backend:").strong());
            let backend = if mode.has_native_decoder() {
                "mfsk-core"
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
                        Color32::YELLOW
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
        if let Some(slot_seconds) = mode.core_slot_seconds() {
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
                ui.add_enabled(
                    can_transmit && !self.digital_tx_active.load(Ordering::Acquire),
                    egui::TextEdit::singleline(&mut self.digital_compose)
                        .desired_width((ui.available_width() - 250.0).max(180.0))
                        .hint_text("CQ W1AW FN20")
                        .font(egui::TextStyle::Monospace),
                );
                if ui
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
                    Color32::YELLOW
                }),
            );
        } else {
            ui.separator();
            ui.label(
                RichText::new(
                    "FLDIGI is currently a radio preset and waterfall view. No XML-RPC modem connection is active yet.",
                )
                .color(Color32::YELLOW),
            );
        }
    }
}
