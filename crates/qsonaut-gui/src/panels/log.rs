use super::super::*;

impl QsonautGuiApp {
    pub(in super::super) fn draw_contact_log(&mut self, ui: &mut egui::Ui, snapshot: &GuiState) {
        ui.horizontal(|ui| {
            ui.heading("Contact Log");
            ui.separator();
            if ui.small_button("+").on_hover_text("New contact").clicked() {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|duration| duration.as_secs())
                    .unwrap_or_default();
                let frequency_hz = snapshot.frequency_hz.unwrap_or_default();
                let record = QsoRecord::new(
                    "",
                    self.workspace_mode.label(),
                    band_for_frequency(frequency_hz),
                    frequency_hz,
                    now,
                    now,
                );
                self.qso_log.contacts.push(record);
                self.qso_selected = Some(self.qso_log.contacts.len() - 1);
                self.qso_log_dirty = true;
                self.qso_log_status = "New contact; edit and save".to_string();
            }
            if ui
                .add_enabled(self.qso_log_dirty, egui::Button::new("Save"))
                .clicked()
            {
                let publish = self
                    .qso_selected
                    .and_then(|index| self.qso_log.contacts.get(index))
                    .cloned();
                self.persist_qso_log("Saved");
                if let Some(record) = &publish {
                    self.publish_qso_to_server(record);
                }
            }
            if ui.small_button("Export ADIF").clicked() {
                match self.qso_log.export_adif(&qso_adif_path()) {
                    Ok(()) => self.qso_log_status = format!("Exported {}", QSO_ADIF_FILE),
                    Err(error) => self.qso_log_status = format!("ADIF export failed: {error}"),
                }
            }
            if ui.small_button("Export ADIF (view)").clicked() {
                let band = band_for_frequency(snapshot.frequency_hz.unwrap_or_default());
                let filter = AdifExportFilter {
                    date_from: None,
                    date_to: None,
                    mode: Some(snapshot.mode.clone()),
                    band: (!band.is_empty()).then(|| band.to_string()),
                };
                let filtered = qso_adif_path().with_file_name("qsonaut-view.adif");
                match self.qso_log.export_adif_filtered(&filtered, &filter) {
                    Ok(()) => self.qso_log_status = format!("Exported {}", filtered.display()),
                    Err(error) => {
                        self.qso_log_status = format!("Filtered ADIF export failed: {error}")
                    }
                }
            }
            if ui.small_button("Import ADIF").clicked() {
                match self.qso_log.import_adif(&qso_adif_path()) {
                    Ok(summary) => {
                        self.qso_log_status = format!(
                            "ADIF import: {} added · {} duplicate(s) · {} invalid",
                            summary.imported, summary.duplicates, summary.invalid
                        );
                        if summary.imported > 0 {
                            self.qso_selected = Some(self.qso_log.contacts.len() - 1);
                            self.qso_log_dirty = true;
                            self.persist_qso_log("Imported + saved");
                        }
                    }
                    Err(error) => self.qso_log_status = format!("ADIF import failed: {error}"),
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(format!("{} QSOs", self.qso_log.contacts.len()));
            });
        });
        ui.separator();

        let mut selected = self.qso_selected;
        egui::ScrollArea::vertical()
            .id_salt("qso_log_rows")
            // Let the table consume the space made available by the resizable
            // bottom panel instead of leaving an empty/black region beneath it.
            .max_height((ui.available_height() - 150.0).max(80.0))
            .show(ui, |ui| {
                egui::Grid::new("qso_log_grid")
                    .striped(true)
                    .min_col_width(38.0)
                    .show(ui, |ui| {
                        for heading in [
                            "Date", "UTC", "Call", "Name", "Grid", "State", "Band", "Mode",
                        ] {
                            ui.label(RichText::new(heading).strong().small());
                        }
                        ui.end_row();

                        for (index, contact) in self.qso_log.contacts.iter().enumerate().rev() {
                            let is_selected = selected == Some(index);
                            if ui
                                .selectable_label(is_selected, &contact.qso_date)
                                .clicked()
                            {
                                selected = Some(index);
                            }
                            ui.label(&contact.time_on);
                            ui.label(RichText::new(&contact.callsign).monospace().strong());
                            let name = contact
                                .hamdb
                                .as_ref()
                                .map(|hamdb| {
                                    [
                                        hamdb.first_name.as_str(),
                                        hamdb.middle_name.as_str(),
                                        hamdb.name.as_str(),
                                        hamdb.suffix.as_str(),
                                    ]
                                    .into_iter()
                                    .map(str::trim)
                                    .filter(|part| !part.is_empty())
                                    .collect::<Vec<_>>()
                                    .join(" ")
                                })
                                .filter(|name| !name.is_empty())
                                .unwrap_or_else(|| "—".to_string());
                            ui.label(RichText::new(name).small());
                            ui.label(RichText::new(&contact.grid).small());
                            ui.label(RichText::new(&contact.state).small());
                            ui.label(&contact.band);
                            ui.label(&contact.mode);
                            ui.end_row();
                        }
                    });
            });
        self.qso_selected = selected;

        let mut changed = false;
        let mut delete_selected = false;
        if let Some(index) = self.qso_selected {
            let refresh_requested = ui.input(|input| input.key_pressed(egui::Key::F5));
            if refresh_requested {
                self.refresh_hamdb_for_contact(index);
            }
            let mut refresh_clicked = false;
            if let Some(contact) = self.qso_log.contacts.get_mut(index) {
                ui.separator();
                ui.horizontal(|ui| {
                    if ui
                        .small_button("Refresh HamDB")
                        .on_hover_text("Refresh all HamDB fields for this callsign")
                        .clicked()
                    {
                        refresh_clicked = true;
                    }
                    if let Some(hamdb) = &contact.hamdb {
                        let operator_name = [
                            hamdb.first_name.as_str(),
                            hamdb.middle_name.as_str(),
                            hamdb.name.as_str(),
                            hamdb.suffix.as_str(),
                        ]
                        .into_iter()
                        .map(str::trim)
                        .filter(|part| !part.is_empty())
                        .collect::<Vec<_>>()
                        .join(" ");
                        ui.label(
                            RichText::new(if operator_name.is_empty() {
                                "HamDB: name unavailable"
                            } else {
                                "HamDB operator"
                            })
                            .small()
                            .color(Color32::LIGHT_BLUE),
                        );
                        if !operator_name.is_empty() {
                            ui.label(RichText::new(operator_name).strong().color(Color32::WHITE));
                        }
                        ui.label(
                            RichText::new(format!("{} · {}", hamdb.state, hamdb.country))
                                .small()
                                .color(Color32::LIGHT_BLUE),
                        );
                    } else {
                        ui.label(
                            RichText::new("HamDB: not loaded")
                                .small()
                                .color(Color32::GRAY),
                        );
                    }
                });
                if let Some(hamdb) = &contact.hamdb {
                    ui.horizontal_wrapped(|ui| {
                        for (label, value) in [
                            ("Class", &hamdb.class),
                            ("Status", &hamdb.status),
                            ("Expires", &hamdb.expires),
                            ("Name", &hamdb.name),
                            ("Grid", &hamdb.grid),
                            ("State", &hamdb.state),
                            ("ZIP", &hamdb.zip),
                            ("Lat", &hamdb.latitude),
                            ("Lon", &hamdb.longitude),
                            ("Country", &hamdb.country),
                            ("Addr", &hamdb.address_line_1),
                            ("Addr 2", &hamdb.address_line_2),
                        ] {
                            if !value.trim().is_empty() {
                                ui.label(
                                    RichText::new(format!("{label}: {value}"))
                                        .small()
                                        .color(Color32::GRAY),
                                );
                            }
                        }
                    });
                }
                ui.horizontal(|ui| {
                    ui.label("Call");
                    changed |= ui
                        .add(
                            egui::TextEdit::singleline(&mut contact.callsign)
                                .desired_width(95.0)
                                .font(egui::TextStyle::Monospace),
                        )
                        .changed();
                    ui.label("Grid");
                    changed |= ui
                        .add(
                            egui::TextEdit::singleline(&mut contact.grid)
                                .desired_width(72.0)
                                .font(egui::TextStyle::Monospace),
                        )
                        .changed();
                    ui.label("State");
                    changed |= ui
                        .add(
                            egui::TextEdit::singleline(&mut contact.state)
                                .desired_width(42.0)
                                .font(egui::TextStyle::Monospace),
                        )
                        .changed();
                    ui.label("Mode");
                    changed |= ui
                        .add(egui::TextEdit::singleline(&mut contact.mode).desired_width(62.0))
                        .changed();
                    ui.label("Band");
                    changed |= ui
                        .add(egui::TextEdit::singleline(&mut contact.band).desired_width(48.0))
                        .changed();
                });
                ui.horizontal(|ui| {
                    ui.label("Date");
                    changed |= ui
                        .add(
                            egui::TextEdit::singleline(&mut contact.qso_date)
                                .desired_width(74.0)
                                .font(egui::TextStyle::Monospace),
                        )
                        .changed();
                    ui.label("On");
                    changed |= ui
                        .add(
                            egui::TextEdit::singleline(&mut contact.time_on)
                                .desired_width(58.0)
                                .font(egui::TextStyle::Monospace),
                        )
                        .changed();
                    ui.label("Off");
                    changed |= ui
                        .add(
                            egui::TextEdit::singleline(&mut contact.time_off)
                                .desired_width(58.0)
                                .font(egui::TextStyle::Monospace),
                        )
                        .changed();
                    let mut frequency_mhz = contact.frequency_hz as f64 / 1_000_000.0;
                    ui.label("MHz");
                    if ui
                        .add(
                            egui::DragValue::new(&mut frequency_mhz)
                                .range(0.0..=10_000.0)
                                .speed(0.001)
                                .max_decimals(6),
                        )
                        .changed()
                    {
                        contact.frequency_hz = (frequency_mhz * 1_000_000.0).round() as u64;
                        changed = true;
                    }
                });
                ui.horizontal(|ui| {
                    ui.label("Sent");
                    changed |= ui
                        .add(
                            egui::TextEdit::singleline(&mut contact.report_sent)
                                .desired_width(48.0)
                                .font(egui::TextStyle::Monospace),
                        )
                        .changed();
                    ui.label("Rcvd");
                    changed |= ui
                        .add(
                            egui::TextEdit::singleline(&mut contact.report_received)
                                .desired_width(48.0)
                                .font(egui::TextStyle::Monospace),
                        )
                        .changed();
                    ui.label("Contest sent");
                    changed |= ui
                        .add(
                            egui::TextEdit::singleline(&mut contact.contest_exchange_sent)
                                .desired_width(82.0)
                                .font(egui::TextStyle::Monospace),
                        )
                        .changed();
                    ui.label("Contest rcvd");
                    changed |= ui
                        .add(
                            egui::TextEdit::singleline(&mut contact.contest_exchange_received)
                                .desired_width(82.0)
                                .font(egui::TextStyle::Monospace),
                        )
                        .changed();
                    ui.label("STX");
                    let mut stx = contact.contest_serial_sent.unwrap_or_default() as i64;
                    if ui
                        .add(egui::DragValue::new(&mut stx).range(0..=999_999).speed(1.0))
                        .changed()
                    {
                        contact.contest_serial_sent = (stx > 0).then_some(stx as u32);
                        changed = true;
                    }
                    ui.label("SRX");
                    let mut srx = contact.contest_serial_received.unwrap_or_default() as i64;
                    if ui
                        .add(egui::DragValue::new(&mut srx).range(0..=999_999).speed(1.0))
                        .changed()
                    {
                        contact.contest_serial_received = (srx > 0).then_some(srx as u32);
                        changed = true;
                    }
                    ui.label("Notes");
                    changed |= ui
                        .add(
                            egui::TextEdit::singleline(&mut contact.notes)
                                .desired_width(ui.available_width().max(80.0)),
                        )
                        .changed();
                });
                ui.horizontal(|ui| {
                    if ui.small_button("Delete").clicked() {
                        delete_selected = true;
                    }
                    ui.label(RichText::new(&self.qso_log_status).small().color(
                        if self.qso_log_dirty {
                            Color32::YELLOW
                        } else {
                            Color32::GRAY
                        },
                    ));
                });
            }
            if refresh_clicked {
                self.refresh_hamdb_for_contact(index);
            }
        } else {
            ui.label(
                RichText::new(&self.qso_log_status)
                    .small()
                    .color(Color32::GRAY),
            );
        }

        if changed {
            if let Some(index) = self.qso_selected {
                if let Some(contact) = self.qso_log.contacts.get_mut(index) {
                    contact.callsign = contact.callsign.trim().to_ascii_uppercase();
                    contact.grid = contact.grid.trim().to_ascii_uppercase();
                    contact.mode = contact.mode.trim().to_ascii_uppercase();
                }
            }
            self.qso_log_dirty = true;
            self.qso_log_status = "Unsaved changes".to_string();
        }
        if delete_selected {
            if let Some(index) = self.qso_selected.take() {
                self.qso_log.contacts.remove(index);
                self.qso_log_dirty = true;
                self.persist_qso_log("Deleted contact from");
            }
        }
    }
}
