use super::super::*;

pub(crate) const BAND_PLAN: &[(&str, u64)] = &[
    ("160m", 1_838_000),
    ("80m", 3_573_000),
    ("60m", 5_357_000),
    ("40m", 7_076_000),
    ("30m", 10_136_000),
    ("20m", 14_076_000),
    ("17m", 18_100_000),
    ("15m", 21_076_000),
    ("12m", 24_924_000),
    ("10m", 28_076_000),
    ("6m", 50_323_000),
];

impl QsonautGuiApp {
    pub(crate) fn draw_q65_workspace(&mut self, ui: &mut egui::Ui, snapshot: &GuiState) {
        self.draw_mfsk_mode_workspace(ui, snapshot, WorkspaceMode::Q65);
        ui.label(
            RichText::new(
                "Q65-A30 is the configured mfsk-core submode. QSONaut currently exposes the backend's 30-second decode and standard waveform TX with manual one-shot operation.",
            )
            .small()
            .color(theme_muted(ui)),
        );
    }
}
