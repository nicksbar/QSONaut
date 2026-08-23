use super::super::*;

pub(crate) const BAND_PLAN: &[(&str, u64)] = &[
    ("160m", 1_880_000),
    ("80m", 3_900_000),
    ("60m", 5_357_000),
    ("40m", 7_200_000),
    ("30m", 10_125_000),
    ("20m", 14_300_000),
    ("17m", 18_150_000),
    ("15m", 21_400_000),
    ("12m", 24_950_000),
    ("10m", 28_400_000),
    ("6m", 52_525_000),
    ("2m", 146_520_000),
    ("70cm", 446_000_000),
];

#[derive(Debug, Clone)]
pub(crate) struct VoiceContestField {
    pub(crate) name: String,
    pub(crate) sent: String,
    pub(crate) received: String,
}

impl VoiceContestField {
    pub(crate) fn new(name: impl Into<String>) -> Self {
        Self { name: name.into(), sent: String::new(), received: String::new() }
    }
}

impl QsonautGuiApp {
    pub(crate) fn draw_voice_workspace(&mut self, ui: &mut egui::Ui, snapshot: &GuiState) {
        let frequency = snapshot.frequency_hz
            .map(|hz| format!("{:.6} MHz", hz as f64 / 1_000_000.0))
            .unwrap_or_else(|| "RADIO OFFLINE".to_string());
        let band = snapshot.frequency_hz.map(band_for_frequency)
            .filter(|band| !band.is_empty()).unwrap_or("--");
        let valid_call = is_probable_callsign(self.voice_callsign.trim());

        ui.horizontal(|ui| {
            ui.heading("Voice Logger");
            ui.separator();
            ui.label(RichText::new(frequency).monospace().strong());
            ui.label(RichText::new(band).color(Color32::from_rgb(220, 190, 100)));
            ui.label(RichText::new(radio_mode_label(&snapshot.mode, snapshot.data_mode)).monospace());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new("Radio controls are in the top bar").small().color(theme_muted(ui)));
            });
        });
        ui.separator();

        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("CURRENT CONTACT").strong().color(theme_success(ui)));
                if self.voice_qso_started_at.is_some() {
                    ui.label(RichText::new("IN PROGRESS").small().color(theme_warning(ui)));
                }
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Callsign").strong());
                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.voice_callsign)
                        .desired_width(220.0)
                        .hint_text("Enter callsign")
                        .font(egui::TextStyle::Heading),
                );
                if response.changed() {
                    self.voice_callsign = self.voice_callsign.to_ascii_uppercase();
                    if is_probable_callsign(self.voice_callsign.trim())
                        && self.voice_qso_started_at.is_none()
                    {
                        self.voice_qso_started_at = Some(unix_now());
                    }
                }
                if response.lost_focus() {
                    self.lookup_voice_callsign();
                }
                if ui.button("Clear").clicked() {
                    self.voice_callsign.clear();
                    self.voice_grid.clear();
                    self.voice_state.clear();
                    self.voice_contest_serial_sent.clear();
                    self.voice_contest_serial_received.clear();
                    self.voice_qso_started_at = None;
                    self.voice_lookup_requested.clear();
                    self.voice_lookup_status.clear();
                    self.voice_hamdb = None;
                }
                ui.label(if valid_call {
                    RichText::new("READY").small().color(theme_success(ui))
                } else {
                    RichText::new("CALLSIGN REQUIRED").small().color(theme_muted(ui))
                });
            });
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                ui.label("RST sent");
                ui.add(egui::TextEdit::singleline(&mut self.voice_rst_sent).desired_width(58.0));
                ui.label("RST received");
                ui.add(egui::TextEdit::singleline(&mut self.voice_rst_received).desired_width(58.0));
                ui.separator();
                ui.label("Serial sent");
                ui.add(egui::TextEdit::singleline(&mut self.voice_contest_serial_sent).desired_width(64.0));
                ui.label("Serial received");
                ui.add(egui::TextEdit::singleline(&mut self.voice_contest_serial_received).desired_width(64.0));
                ui.separator();
                ui.label("Grid");
                ui.add(egui::TextEdit::singleline(&mut self.voice_grid).desired_width(84.0).hint_text("FN31pr"));
                ui.label("State / province");
                ui.add(egui::TextEdit::singleline(&mut self.voice_state).desired_width(100.0).hint_text("Optional"));
            });
            if let Some(hamdb) = &self.voice_hamdb {
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
                ui.horizontal(|ui| {
                    ui.label(RichText::new("HamDB").strong().color(theme_accent(ui)));
                    ui.label(if operator_name.is_empty() {
                        "Operator name unavailable".to_string()
                    } else {
                        operator_name
                    });
                    if !hamdb.country.trim().is_empty() {
                        ui.label(RichText::new(format!("{} / {}", hamdb.state, hamdb.country)).small().color(theme_muted(ui)));
                    }
                });
            } else if !self.voice_lookup_status.is_empty() {
                ui.label(RichText::new(&self.voice_lookup_status).small().color(theme_muted(ui)));
            }
        });

        ui.add_space(8.0);
        ui.columns(2, |columns| {
            columns[0].vertical(|ui| {
                ui.heading("Exchange");
                ui.label(RichText::new("Contest, net, or event exchange fields").small().color(theme_muted(ui)));
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.add_space(92.0);
                    ui.label(RichText::new("Sent").small().color(theme_muted(ui)));
                    ui.add_space(72.0);
                    ui.label(RichText::new("Received").small().color(theme_muted(ui)));
                });
                let mut remove = None;
                for (index, field) in self.voice_contest_fields.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut field.name).desired_width(90.0));
                        ui.add(egui::TextEdit::singleline(&mut field.sent).desired_width(82.0));
                        ui.add(egui::TextEdit::singleline(&mut field.received).desired_width(92.0));
                        if ui.small_button("x").on_hover_text("Remove exchange field").clicked() {
                            remove = Some(index);
                        }
                    });
                }
                if let Some(index) = remove { self.voice_contest_fields.remove(index); }
                if ui.small_button("+ Add exchange field").clicked() {
                    self.voice_contest_fields.push(VoiceContestField::new("Field"));
                }
            });
            columns[1].vertical(|ui| {
                ui.heading("Notes");
                ui.add(egui::TextEdit::multiline(&mut self.voice_notes)
                    .desired_rows(6).desired_width(ui.available_width())
                    .hint_text("Name, QTH, equipment, event notes..."));
                ui.add_space(6.0);
                ui.label(RichText::new("HamDB details attach when the QSO is logged.").small().color(theme_muted(ui)));
            });
        });

        ui.add_space(10.0);
        ui.horizontal(|ui| {
            let log_response = ui.add_enabled(valid_call,
                egui::Button::new(RichText::new("LOG QSO").strong()).min_size(egui::vec2(150.0, 38.0)));
            if log_response.clicked() { self.log_voice_qso(snapshot); }
            ui.label(RichText::new("Records the current frequency, band, mode, reports, exchange, and notes.").small().color(theme_muted(ui)));
        });

    }

    fn lookup_voice_callsign(&mut self) {
        let callsign = self.voice_callsign.trim().to_ascii_uppercase();
        if !is_probable_callsign(&callsign)
            || callsign.eq_ignore_ascii_case(&self.voice_lookup_requested)
        {
            return;
        }
        self.voice_lookup_requested = callsign.clone();
        self.voice_hamdb = None;
        self.voice_lookup_status = "HamDB: looking up...".to_string();
        info!(callsign = %callsign, "HamDB Voice callsign lookup started");
        let now = unix_now();
        if let Ok(Some(entry)) = HamDbCache::open(&hamdb_cache_path())
            .and_then(|cache| cache.get_fresh(&callsign, now, HAMDB_CACHE_TTL_SECONDS))
        {
            if self.voice_grid.trim().is_empty() {
                self.voice_grid = entry.grid.clone();
            }
            if self.voice_state.trim().is_empty() {
                self.voice_state = entry.state.clone();
            }
            self.voice_hamdb = Some(entry);
            self.voice_lookup_status = "HamDB: cached operator found".to_string();
        } else {
            self.hamdb_lookup_rx = Some(spawn_hamdb_lookup(callsign, now));
        }
    }

    fn log_voice_qso(&mut self, snapshot: &GuiState) {
        let now = unix_now();
        let frequency_hz = snapshot.frequency_hz.unwrap_or_default();
        let mut record = QsoRecord::new(
            self.voice_callsign.trim().to_ascii_uppercase(), "SSB",
            band_for_frequency(frequency_hz), frequency_hz,
            self.voice_qso_started_at.unwrap_or(now), now,
        );
        record.operation_mode = self.activity.label().to_string();
        record.grid = self.voice_grid.trim().to_ascii_uppercase();
        record.state = self.voice_state.trim().to_ascii_uppercase();
        record.report_sent = self.voice_rst_sent.trim().to_string();
        record.report_received = self.voice_rst_received.trim().to_string();
        record.contest_serial_sent = self.voice_contest_serial_sent.trim().parse().ok();
        record.contest_serial_received = self.voice_contest_serial_received.trim().parse().ok();
        record.contest_exchange_sent = self.voice_contest_fields.iter()
            .filter(|field| !field.sent.trim().is_empty())
            .map(|field| format!("{}={}", field.name.trim(), field.sent.trim()))
            .collect::<Vec<_>>().join(" ");
        record.contest_exchange_received = self.voice_contest_fields.iter()
            .filter(|field| !field.received.trim().is_empty())
            .map(|field| format!("{}={}", field.name.trim(), field.received.trim()))
            .collect::<Vec<_>>().join(" ");
        record.notes = self.voice_notes.trim().to_string();
        info!(
            callsign = %record.callsign,
            band = %record.band,
            frequency_hz = record.frequency_hz,
            activity = %record.operation_mode,
            "Voice QSO logging requested"
        );
        self.append_qso(record, "Voice QSO saved");
        self.voice_callsign.clear();
        self.voice_grid.clear();
        self.voice_state.clear();
        self.voice_contest_serial_sent.clear();
        self.voice_contest_serial_received.clear();
        self.voice_notes.clear();
        self.voice_qso_started_at = None;
    }
}

fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs()).unwrap_or_default()
}
