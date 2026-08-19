use super::super::*;

pub(crate) const BAND_PLAN: &[(&str, u64)] = &[
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

impl QsonautGuiApp {
    pub(crate) fn draw_fst4_workspace(&mut self, ui: &mut egui::Ui, snapshot: &GuiState) {
        self.draw_mfsk_mode_workspace(ui, snapshot, WorkspaceMode::Fst4);
        ui.label(
            RichText::new(
                "FST4-60 is the configured mfsk-core submode. Other FST4 periods are not exposed until the workspace can select a matching decoder and waveform configuration.",
            )
            .small()
            .color(Color32::GRAY),
        );
    }
}
