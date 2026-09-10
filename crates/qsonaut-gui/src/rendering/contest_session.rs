use super::super::*;

impl QsonautGuiApp {
    pub(crate) fn draw_contest_session(&mut self, ui: &mut egui::Ui, snapshot: &GuiState) {
        if let Some((_, event_name)) = &self.server_active_event {
            ui.strong(format!(
                "Server Contest · {event_name} · {}",
                self.station_callsign_or_default()
            ));
            if let Some((event_id, _)) = &self.server_active_event {
                if let Some(score) = self.server_client.as_ref().and_then(|client| {
                    client
                        .status()
                        .event_scores
                        .into_iter()
                        .find(|score| score.event_id == *event_id)
                }) {
                    ui.label(format!(
                        "Score: {} · {} QSOs · {} dupes",
                        score.total_points, score.qso_count, score.duplicate_count
                    ));
                }
            }
            ui.colored_label(
                theme_warning(ui),
                "Server scoring is authoritative; TX requires an active station assignment",
            );
            return;
        }
        if !self.contest_enabled {
            return;
        }
        let Some(definition) = crate::contest_catalog::find(&self.contest_type) else {
            ui.colored_label(
                theme_warning(ui),
                "Unknown contest definition; choose a contest in Station Settings",
            );
            return;
        };
        let mut changed = false;
        ui.horizontal_wrapped(|ui| {
            ui.strong(format!("{} · {}", definition.name, self.station_callsign_or_default()));
            ui.label(if self.server_active_event.is_some() { "Server event selected" } else { "Local session · scoring not yet calculated" });
            if ui.small_button("New local session").on_hover_text("Start a new contest occurrence with a fresh duplicate history and serial counter").clicked()
                && self.server_active_event.is_none() {
                self.disarm_all_tx_with_persistence("New contest session", false);
                self.contest_session_id = Uuid::new_v4().to_string();
                self.contest_serial_current = self.contest_serial_start.max(1);
                self.contest_exchange_fields.clear();
                self.cw_qso_exchange_received.clear();
                changed = true;
            }
            ui.hyperlink_to("Rules", &definition.rules_url);
        });
        egui::CollapsingHeader::new("Contest setup and exchange")
            .id_salt("shared_contest_setup")
            .default_open(true)
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    for field in &definition.fields {
                        ui.label(&field.label);
                        let value = self
                            .contest_field_values
                            .entry(field.key.clone())
                            .or_default();
                        if field.options.is_empty() {
                            changed |= ui
                                .add(egui::TextEdit::singleline(value).desired_width(85.0))
                                .changed();
                        } else {
                            egui::ComboBox::from_id_salt(("shared_setup", &field.key))
                                .selected_text(if value.is_empty() {
                                    "Select"
                                } else {
                                    value.as_str()
                                })
                                .show_ui(ui, |ui| {
                                    for option in &field.options {
                                        changed |= ui
                                            .selectable_value(value, option.clone(), option)
                                            .changed();
                                    }
                                });
                        }
                    }
                });
                for key in &definition.exchange {
                    if !self
                        .contest_exchange_fields
                        .iter()
                        .any(|field| field.name.eq_ignore_ascii_case(key))
                    {
                        self.contest_exchange_fields
                            .push(ContestExchangeField::new(key));
                    }
                    let field = self
                        .contest_exchange_fields
                        .iter_mut()
                        .find(|field| field.name.eq_ignore_ascii_case(key))
                        .unwrap();
                    if let Some(value) = self.contest_field_values.get(key) {
                        field.sent = value.clone();
                    }
                    ui.horizontal(|ui| {
                        ui.label(key);
                        ui.label("Sent");
                        ui.add_enabled(
                            !self.contest_field_values.contains_key(key),
                            egui::TextEdit::singleline(&mut field.sent).desired_width(90.0),
                        );
                        ui.label("Received");
                        ui.add(egui::TextEdit::singleline(&mut field.received).desired_width(90.0));
                    });
                }
                let mut errors = definition.validate_setup(&self.contest_field_values);
                let band = snapshot.frequency_hz.map(band_for_frequency).unwrap_or("");
                errors.extend(definition.validate_band_mode(band, self.workspace_mode.label()));
                if errors.is_empty() {
                    ui.colored_label(
                        theme_success(ui),
                        "Setup and band/mode valid · verify the received exchange before logging",
                    );
                } else {
                    for error in errors {
                        ui.colored_label(theme_warning(ui), error);
                    }
                }
            });
        if changed {
            self.profile_dirty = true;
            self.persist_profile("Contest setup saved");
        }
        ui.separator();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_editor_uses_exchange_keys_not_setup_fields() {
        let fd = crate::contest_catalog::find("ARRL_FD").unwrap();
        assert!(fd.fields.iter().any(|field| field.key == "power"));
        assert!(!fd.exchange.iter().any(|field| field == "power"));
        let context = egui::Context::default();
        let icon = eframe::icon_data::from_png_bytes(QSONAUT_ICON_PNG).unwrap();
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
        app.contest_enabled = true;
        app.contest_type = "ARRL_FD".into();
        app.contest_field_values.insert("class".into(), "1A".into());
        for mode in [
            WorkspaceMode::Voice,
            WorkspaceMode::Cw,
            WorkspaceMode::Ft8,
            WorkspaceMode::Ft4,
        ] {
            app.workspace_mode = mode;
            let snapshot = app.state.lock().unwrap().clone();
            let _ = context.run(Default::default(), |ctx| {
                egui::CentralPanel::default()
                    .show(ctx, |ui| app.draw_contest_session(ui, &snapshot));
            });
            assert_eq!(app.contest_exchange_fields.len(), 2);
            assert_eq!(app.contest_exchange_fields[0].sent, "1A");
        }
    }
}
