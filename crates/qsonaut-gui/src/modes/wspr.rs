use super::super::*;

pub(crate) const BAND_PLAN: &[(&str, u64)] = &[
    ("160m", 1_836_600),
    ("80m", 3_592_600),
    ("60m", 5_208_600),
    ("40m", 7_038_600),
    ("30m", 10_138_700),
    ("20m", 14_095_600),
    ("17m", 18_104_600),
    ("15m", 21_094_600),
    ("12m", 24_924_600),
    ("10m", 28_124_600),
    ("6m", 50_293_000),
];

const WSPR_POWER_DBM: [i32; 19] = [
    0, 3, 7, 10, 13, 17, 20, 23, 27, 30, 33, 37, 40, 43, 47, 50, 53, 57, 60,
];

fn parse_beacon(message: &str) -> Option<(&str, &str, i32)> {
    let mut fields = message.split_whitespace();
    let callsign = fields.next()?;
    let grid = fields.next()?;
    let power = fields.next()?.parse().ok()?;
    if fields.next().is_some() {
        return None;
    }
    Some((callsign, grid, power))
}

impl QsonautGuiApp {
    pub(crate) fn draw_wspr_workspace(&mut self, ui: &mut egui::Ui, snapshot: &GuiState) {
        ui.heading("WSPR · Propagation Beacon");
        ui.separator();
        ui.label(
            RichText::new(
                "WSPR uses 2-minute UTC slots. This first operating workflow transmits one Type-1 beacon at a time; it does not create QSO records or run FT8/FT4 sequencing.",
            )
            .color(theme_accent(ui)),
        );
        ui.horizontal_wrapped(|ui| {
            ui.label(format!(
                "Radio: {:.3} MHz · {}",
                snapshot.frequency_hz.unwrap_or_default() as f64 / 1_000_000.0,
                snapshot.mode
            ));
            ui.separator();
            ui.label(RichText::new(&snapshot.digital_decode_status).monospace());
        });
        ui.separator();
        ui.label(RichText::new("Beacon settings").strong());
        ui.label(
            RichText::new("The backend currently supports WSPR Type-1: callsign, four-character locator, and one of the standard WSPR dBm power values.")
                .small()
                .color(theme_muted(ui)),
        );
        ui.horizontal_wrapped(|ui| {
            if ui.button("FILL FROM STATION").clicked() {
                self.digital_compose = format!(
                    "{} {} 37",
                    self.station_callsign_or_default(),
                    self.station_grid_or_default()
                );
            }
            ui.label("Power");
            let mut power = parse_beacon(&self.digital_compose)
                .map(|(_, _, power)| power)
                .unwrap_or(37);
            egui::ComboBox::from_id_salt("wspr_power_dbm")
                .selected_text(format!("{power} dBm"))
                .show_ui(ui, |ui| {
                    for candidate in WSPR_POWER_DBM {
                        if ui
                            .selectable_value(&mut power, candidate, format!("{candidate} dBm"))
                            .clicked()
                        {
                            if let Some((callsign, grid, _)) = parse_beacon(&self.digital_compose) {
                                self.digital_compose = format!("{callsign} {grid} {power}");
                            }
                        }
                    }
                });
            ui.label("120-second slot · one-shot only");
        });
        ui.separator();
        ui.label(RichText::new("Recent spots").strong());
        egui::ScrollArea::vertical()
            .id_salt("wspr-spots")
            .max_height((ui.available_height() * 0.45).max(160.0))
            .stick_to_bottom(true)
            .show(ui, |ui| {
                let mut shown = 0;
                for entry in snapshot
                    .digital_decodes
                    .iter()
                    .filter(|entry| entry.mode == WorkspaceMode::Wspr)
                {
                    shown += 1;
                    ui.label(
                        RichText::new(format!(
                            "{:12}  {:+5.1} dB  dT {:+5.1}  {:>5} Hz  {}",
                            entry.utc, entry.snr_db, entry.dt_s, entry.freq_hz, entry.message
                        ))
                        .monospace(),
                    );
                }
                if shown == 0 {
                    ui.label(
                        RichText::new("Waiting for a complete 120-second WSPR slot…")
                            .color(theme_muted(ui)),
                    );
                }
            });
        ui.separator();
        ui.horizontal(|ui| {
            ui.label("Beacon");
            ui.add_enabled(
                !self.digital_tx_active.load(Ordering::Acquire),
                egui::TextEdit::singleline(&mut self.digital_compose)
                    .desired_width((ui.available_width() - 170.0).max(220.0))
                    .hint_text("CALL GRID POWER_DBM"),
            );
            if ui
                .add_enabled(
                    !self.digital_tx_active.load(Ordering::Acquire)
                        && parse_beacon(&self.digital_compose).is_some_and(
                            |(callsign, grid, power)| {
                                callsign.len() <= 6
                                    && grid.len() == 4
                                    && WSPR_POWER_DBM.contains(&power)
                            },
                        ),
                    egui::Button::new("TRANSMIT ONCE"),
                )
                .clicked()
            {
                self.queue_native_digital_tx(WorkspaceMode::Wspr);
            }
            if ui
                .add_enabled(
                    self.digital_tx_active.load(Ordering::Acquire),
                    egui::Button::new("STOP + DISARM"),
                )
                .clicked()
            {
                self.stop_native_digital_tx();
            }
        });
        ui.label(RichText::new(&self.digital_tx_status).color(theme_muted(ui)));
        if let Some((callsign, grid, power)) = parse_beacon(&self.digital_compose) {
            let valid = callsign.len() <= 6 && grid.len() == 4 && WSPR_POWER_DBM.contains(&power);
            ui.label(
                RichText::new(if valid {
                    format!("Ready: {callsign} {grid} at {power} dBm")
                } else {
                    "Invalid WSPR Type-1 beacon fields".to_string()
                })
                .color(if valid {
                    theme_success(ui)
                } else {
                    theme_warning(ui)
                }),
            );
        } else {
            ui.label(
                RichText::new("Enter CALL GRID POWER_DBM to enable transmit")
                    .color(theme_warning(ui)),
            );
        }
        ui.label(
            RichText::new("Format: CALL GRID POWER_DBM, for example K1ABC FN42 37. TX is one-shot and starts on the next valid 2-minute slot.")
                .small()
                .color(theme_warning(ui)),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::parse_beacon;

    #[test]
    fn parses_a_complete_type_one_beacon() {
        assert_eq!(parse_beacon("K1ABC FN42 37"), Some(("K1ABC", "FN42", 37)));
    }

    #[test]
    fn rejects_incomplete_invalid_and_extra_beacon_fields() {
        assert_eq!(parse_beacon("K1ABC FN42"), None);
        assert_eq!(parse_beacon("K1ABC FN42 nope"), None);
        assert_eq!(parse_beacon("K1ABC FN42 37 extra"), None);
        assert_eq!(parse_beacon(""), None);
    }
}
