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
                self.qso_selected = self.qso_log.contacts.last().map(|contact| contact.id);
                self.qso_log_dirty = true;
                self.qso_log_status = "New contact; edit and save".to_string();
            }
            if ui
                .add_enabled(self.qso_log_dirty, egui::Button::new("Save"))
                .clicked()
            {
                let publish = self
                    .qso_selected
                    .and_then(|id| {
                        self.qso_log
                            .contacts
                            .iter()
                            .find(|contact| contact.id == id)
                    })
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
                            self.qso_selected =
                                self.qso_log.contacts.last().map(|contact| contact.id);
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
        egui::SidePanel::left("qso_log_list")
            .resizable(true)
            .default_width(560.0)
            .width_range(280.0..=(ui.available_width() - 280.0).max(280.0))
            .show_inside(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("qso_log_rows")
                    .auto_shrink([false, false])
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

                                for (_index, contact) in
                                    self.qso_log.contacts.iter().enumerate().rev()
                                {
                                    let is_selected = selected == Some(contact.id);
                                    let row_response = ui.selectable_label(
                                        is_selected,
                                        format!("{}  {}", contact.qso_date, contact.time_on),
                                    );
                                    if row_response.clicked() {
                                        selected = Some(contact.id);
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
            });
        self.qso_selected = selected;

        let mut changed = false;
        let mut delete_selected = false;
        let mut close_editor = false;
        if let Some(id) = self.qso_selected {
            let Some(index) = self
                .qso_log
                .contacts
                .iter()
                .position(|contact| contact.id == id)
            else {
                self.qso_selected = None;
                return;
            };
            let refresh_requested = ui.input(|input| input.key_pressed(egui::Key::F5));
            if refresh_requested {
                self.refresh_hamdb_for_contact(index);
            }
            let mut refresh_clicked = false;
            if let Some(contact) = self.qso_log.contacts.get_mut(index) {
                ui.separator();
                ui.horizontal(|ui| {
                    ui.heading("Selected Contact");
                    if ui
                        .small_button("Close Editor")
                        .on_hover_text("Close the contact editor; the contact list remains open")
                        .clicked()
                    {
                        close_editor = true;
                    }
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
                            .color(theme_accent(ui)),
                        );
                        if !operator_name.is_empty() {
                            ui.label(RichText::new(operator_name).strong().color(Color32::WHITE));
                        }
                        ui.label(
                            RichText::new(format!("{} · {}", hamdb.state, hamdb.country))
                                .small()
                                .color(theme_accent(ui)),
                        );
                    } else {
                        ui.label(
                            RichText::new("HamDB: not loaded")
                                .small()
                                .color(theme_muted(ui)),
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
                                        .color(theme_muted(ui)),
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
                            theme_warning(ui)
                        } else {
                            theme_muted(ui)
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
                    .color(theme_muted(ui)),
            );
        }

        if close_editor {
            self.qso_selected = None;
        }
        if changed {
            if let Some(id) = self.qso_selected {
                if let Some(contact) = self
                    .qso_log
                    .contacts
                    .iter_mut()
                    .find(|contact| contact.id == id)
                {
                    contact.callsign = contact.callsign.trim().to_ascii_uppercase();
                    contact.grid = contact.grid.trim().to_ascii_uppercase();
                    contact.mode = contact.mode.trim().to_ascii_uppercase();
                }
            }
            self.qso_log_dirty = true;
            self.qso_log_status = "Unsaved changes".to_string();
        }
        if delete_selected {
            if let Some(id) = self.qso_selected.take() {
                if let Some(index) = self
                    .qso_log
                    .contacts
                    .iter()
                    .position(|contact| contact.id == id)
                {
                    let callsign = self.qso_log.contacts[index].callsign.clone();
                    self.qso_log.contacts.remove(index);
                    info!(contact_id = id, callsign = %callsign, "QSO contact deleted");
                }
                self.qso_log_dirty = true;
                self.persist_qso_log("Deleted contact from");
            }
        }
    }
}
