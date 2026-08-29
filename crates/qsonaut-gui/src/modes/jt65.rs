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

fn workspace_description() -> &'static str {
    "JT65A is the mfsk-core submode currently available here. The workspace uses the backend's standard 60-second decoder and synthesizer with manual one-shot TX."
}

impl QsonautGuiApp {
    pub(crate) fn draw_jt65_workspace(&mut self, ui: &mut egui::Ui, snapshot: &GuiState) {
        self.draw_mfsk_mode_workspace(ui, snapshot, WorkspaceMode::Jt65);
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
    fn exposes_the_jt65_band_plan_and_workspace_contract() {
        assert_eq!(BAND_PLAN.len(), 11);
        assert_eq!(BAND_PLAN.first(), Some(&("160m", 1_838_000)));
        assert_eq!(BAND_PLAN.last(), Some(&("6m", 50_323_000)));
        assert!(workspace_description().contains("60-second decoder"));
        assert!(workspace_description().contains("manual one-shot TX"));
    }
}
