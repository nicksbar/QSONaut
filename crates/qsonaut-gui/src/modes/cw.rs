use super::super::*;

pub(crate) const BAND_PLAN: &[(&str, u64)] = &[
    ("160m", 1_836_000),
    ("80m", 3_570_000),
    ("60m", 5_351_500),
    ("40m", 7_030_000),
    ("30m", 10_120_000),
    ("20m", 14_050_000),
    ("17m", 18_086_000),
    ("15m", 21_050_000),
    ("12m", 24_930_000),
    ("10m", 28_050_000),
    ("6m", 50_100_000),
    ("2m", 146_520_000),
    ("70cm", 432_100_000),
];

impl QsonautGuiApp {
    pub(crate) fn draw_cw_workspace(&mut self, ui: &mut egui::Ui, snapshot: &GuiState) {
        let preset = workspace_radio_preset(WorkspaceMode::Cw);
        ui.horizontal_wrapped(|ui| {
            ui.heading("CW · Software Audio");
            ui.separator();
            ui.label(
                RichText::new("Backend: DitDah")
                    .strong()
                    .color(Color32::LIGHT_GREEN),
            );
            ui.separator();
            ui.label("Timing: Continuous");
            ui.separator();
            ui.label(format!("Radio preset: USB-D FIL{}", preset.filter));
            ui.separator();
            ui.label(format!(
                "Radio: {:.3} MHz · {}",
                snapshot.frequency_hz.unwrap_or_default() as f64 / 1_000_000.0,
                snapshot.mode
            ));
        });

        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new(&snapshot.digital_decode_status)
                    .monospace()
                    .color(Color32::LIGHT_GREEN),
            );
            if let Some(level) = snapshot.audio_level_dbfs {
                ui.separator();
                ui.label(format!("Input {level:.0} dBFS"));
            }
        });
        ui.separator();

        egui::Frame::group(ui.style())
            .fill(Color32::from_rgb(20, 34, 28))
            .stroke(egui::Stroke::new(1.0_f32, Color32::from_rgb(90, 210, 130)))
            .show(ui, |ui| {
                ui.label(
                    RichText::new("LIVE DECODE")
                        .small()
                        .strong()
                        .color(Color32::LIGHT_GREEN),
                );
                ui.label(
                    RichText::new(if snapshot.cw_live_text.is_empty() {
                        "Listening…"
                    } else {
                        &snapshot.cw_live_text
                    })
                    .monospace()
                    .strong()
                    .size(18.0),
                );
            });
        ui.add_space(6.0);

        ui.horizontal(|ui| {
            ui.label(
                RichText::new("UTC          New transcript text")
                    .monospace()
                    .strong(),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("Clear").clicked() {
                    let mut state = self.state.lock().expect("ui state lock poisoned");
                    state
                        .digital_decodes
                        .retain(|entry| entry.mode != WorkspaceMode::Cw);
                    state.cw_live_text.clear();
                    state.cw_last_window_text.clear();
                }
            });
        });
        egui::ScrollArea::vertical()
            .id_salt("cw-decodes")
            .max_height((ui.available_height() * 0.45).max(140.0))
            .stick_to_bottom(true)
            .show(ui, |ui| {
                let mut shown = 0;
                for entry in snapshot
                    .digital_decodes
                    .iter()
                    .filter(|entry| entry.mode == WorkspaceMode::Cw)
                {
                    shown += 1;
                    ui.label(
                        RichText::new(format!("{:12}  {}", entry.utc, entry.message)).monospace(),
                    );
                }
                if shown == 0 {
                    ui.label(
                        RichText::new("Listening for the first stable CW characters…")
                            .color(Color32::GRAY),
                    );
                }
            });

        ui.separator();
        ui.horizontal_wrapped(|ui| {
            ui.label("TX speed");
            if ui
                .add(egui::Slider::new(&mut self.cw_wpm, 5..=40).suffix(" WPM"))
                .changed()
            {
                self.profile_dirty = true;
                self.persist_profile("CW speed saved to");
            }
            ui.separator();
            ui.label("RX/TX CW tone");
            if ui
                .add(egui::Slider::new(&mut self.cw_tone_hz, 200..=1_200).suffix(" Hz"))
                .changed()
            {
                self.profile_dirty = true;
                self.persist_profile("CW tone saved to");
            }
        });

        ui.horizontal(|ui| {
            ui.label(RichText::new("TX").strong());
            ui.add(
                egui::TextEdit::singleline(&mut self.digital_compose)
                    .desired_width((ui.available_width() - 180.0).max(180.0))
                    .hint_text("CQ CQ DE W1AW (A-Z, 0-9, spaces)")
                    .font(egui::TextStyle::Monospace),
            );
            if ui
                .add_enabled(
                    !self.digital_compose.trim().is_empty()
                        && !self.digital_tx_active.load(Ordering::Acquire),
                    egui::Button::new("SEND CW"),
                )
                .clicked()
            {
                self.queue_native_digital_tx(WorkspaceMode::Cw);
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
        ui.label(RichText::new(&self.digital_tx_status).color(Color32::GRAY));
        ui.label(
            RichText::new(
                "Software CW uses USB-D. The selected CW tone centers a narrow RX decoder gate and sets the TX audio pitch; broadband noise and steady carriers are rejected before DitDah. DitDah supports A-Z, 0-9, and spaces; prosigns, punctuation, paddle/keyed-carrier CW, and automatic QSO sequencing are not yet available.",
            )
            .small()
            .color(Color32::YELLOW),
        );

        let cw_tx: Vec<_> = self
            .digital_tx_chat
            .iter()
            .rev()
            .filter(|entry| entry.mode == WorkspaceMode::Cw)
            .take(8)
            .collect();
        if !cw_tx.is_empty() {
            ui.separator();
            ui.label(RichText::new("Recent CW TX").strong());
            for entry in cw_tx.into_iter().rev() {
                ui.label(RichText::new(format!("{}  {}", entry.utc, entry.message)).monospace());
            }
        }
    }
}
