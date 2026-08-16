use super::super::*;

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

        ui.horizontal(|ui| {
            ui.label(
                RichText::new("UTC          Decoded text")
                    .monospace()
                    .strong(),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("Clear").clicked() {
                    self.state
                        .lock()
                        .expect("ui state lock poisoned")
                        .digital_decodes
                        .retain(|entry| entry.mode != WorkspaceMode::Cw);
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
                        RichText::new("Collecting the first 8-second CW window…")
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
            ui.label("CW tone");
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
                "Software CW uses USB-D and the configured audio tone for wide-passband receive and subband TX placement. DitDah supports A-Z, 0-9, and spaces; prosigns, punctuation, paddle/keyed-carrier CW, and automatic QSO sequencing are not yet available.",
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
