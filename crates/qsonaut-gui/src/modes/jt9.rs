use super::super::*;

pub(crate) const BAND_PLAN: &[(&str, u64)] = &[
    ("160m", 1_836_000),
    ("80m", 3_579_000),
    ("60m", 5_357_000),
    ("40m", 7_078_000),
    ("30m", 10_136_000),
    ("20m", 14_078_000),
    ("17m", 18_100_000),
    ("15m", 21_078_000),
    ("12m", 24_928_000),
    ("10m", 28_078_000),
    ("6m", 50_313_000),
];

fn workspace_description() -> &'static str {
    "JT9 uses the shared WSJT 60-second decoder and standard message synthesizer. QSONaut currently provides manual one-shot slot TX; no JT9 sequencing is implied."
}

impl QsonautGuiApp {
    pub(crate) fn draw_jt9_workspace(&mut self, ui: &mut egui::Ui, snapshot: &GuiState) {
        self.draw_mfsk_mode_workspace(ui, snapshot, WorkspaceMode::Jt9);
        ui.label(
            RichText::new(workspace_description())
                .small()
                .color(theme_muted(ui)),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{workspace_description, BAND_PLAN};

    #[test]
    fn exposes_the_jt9_band_plan_and_manual_tx_contract() {
        assert_eq!(BAND_PLAN.len(), 11);
        assert_eq!(BAND_PLAN.first(), Some(&("160m", 1_836_000)));
        assert_eq!(BAND_PLAN.last(), Some(&("6m", 50_313_000)));
        assert!(workspace_description().contains("60-second decoder"));
        assert!(workspace_description().contains("no JT9 sequencing"));
    }
}
