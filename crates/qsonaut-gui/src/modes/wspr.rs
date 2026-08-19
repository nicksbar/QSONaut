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

impl QsonautGuiApp {
    pub(crate) fn draw_wspr_workspace(&mut self, ui: &mut egui::Ui, snapshot: &GuiState) {
        ui.heading("WSPR · Propagation Beacon");
        ui.separator();
        ui.label(
            RichText::new(
                "WSPR uses 2-minute UTC slots. This first operating workflow transmits one Type-1 beacon at a time; it does not create QSO records or run FT8/FT4 sequencing.",
            )
            .color(Color32::LIGHT_BLUE),
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
                            .color(Color32::GRAY),
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
                        && !self.digital_compose.trim().is_empty(),
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
        ui.label(RichText::new(&self.digital_tx_status).color(Color32::GRAY));
        ui.label(
            RichText::new("Format: CALL GRID POWER_DBM, for example K1ABC FN42 37. TX is one-shot and starts on the next valid 2-minute slot.")
                .small()
                .color(Color32::YELLOW),
        );
    }
}
