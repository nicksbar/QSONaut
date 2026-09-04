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
    "Q65 uses the selected shared WSJT submode for matching decode timing and waveform synthesis."
}

impl QsonautGuiApp {
    pub(crate) fn draw_q65_workspace(&mut self, ui: &mut egui::Ui, snapshot: &GuiState) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Submode").strong());
            egui::ComboBox::from_id_salt("q65_submode")
                .selected_text(self.q65_submode.name())
                .show_ui(ui, |ui| {
                    for submode in [
                        qsonaut_third_party::wsjt::Q65Submode::A15,
                        qsonaut_third_party::wsjt::Q65Submode::A30,
                        qsonaut_third_party::wsjt::Q65Submode::A60,
                        qsonaut_third_party::wsjt::Q65Submode::B60,
                        qsonaut_third_party::wsjt::Q65Submode::C60,
                        qsonaut_third_party::wsjt::Q65Submode::D60,
                        qsonaut_third_party::wsjt::Q65Submode::E60,
                        qsonaut_third_party::wsjt::Q65Submode::D120,
                        qsonaut_third_party::wsjt::Q65Submode::E120,
                        qsonaut_third_party::wsjt::Q65Submode::A300,
                    ] {
                        ui.selectable_value(&mut self.q65_submode, submode, submode.name());
                    }
                });
            ui.label(format!("{} second T/R period", self.q65_submode.seconds()));
        });
        self.draw_mfsk_mode_workspace(ui, snapshot, WorkspaceMode::Q65);
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
    fn exposes_the_q65_band_plan_and_timing_contract() {
        assert_eq!(BAND_PLAN.len(), 11);
        assert_eq!(BAND_PLAN.first(), Some(&("160m", 1_838_000)));
        assert_eq!(BAND_PLAN.last(), Some(&("6m", 50_323_000)));
        assert!(workspace_description().contains("selected shared WSJT submode"));
    }

    #[test]
    fn exposes_every_adapter_q65_submode() {
        let submodes = [
            qsonaut_third_party::wsjt::Q65Submode::A15,
            qsonaut_third_party::wsjt::Q65Submode::A30,
            qsonaut_third_party::wsjt::Q65Submode::A60,
            qsonaut_third_party::wsjt::Q65Submode::B60,
            qsonaut_third_party::wsjt::Q65Submode::C60,
            qsonaut_third_party::wsjt::Q65Submode::D60,
            qsonaut_third_party::wsjt::Q65Submode::E60,
            qsonaut_third_party::wsjt::Q65Submode::D120,
            qsonaut_third_party::wsjt::Q65Submode::E120,
            qsonaut_third_party::wsjt::Q65Submode::A300,
        ];
        assert_eq!(
            submodes
                .iter()
                .map(|submode| submode.name())
                .collect::<Vec<_>>(),
            [
                "q65-a15", "q65-a30", "q65-a60", "q65-b60", "q65-c60", "q65-d60", "q65-e60",
                "q65-d120", "q65-e120", "q65-a300",
            ]
        );
        assert_eq!(
            submodes
                .iter()
                .map(|submode| submode.seconds())
                .collect::<Vec<_>>(),
            [15, 30, 60, 60, 60, 60, 60, 120, 120, 300]
        );
    }
}
