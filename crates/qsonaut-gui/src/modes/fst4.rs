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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Submode {
    S15,
    S30,
    #[default]
    S60,
    S120,
    S300,
}

impl Submode {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::S15 => "FST4-15",
            Self::S30 => "FST4-30",
            Self::S60 => "FST4-60",
            Self::S120 => "FST4-120",
            Self::S300 => "FST4-300",
        }
    }

    pub(crate) fn seconds(self) -> f64 {
        match self {
            Self::S15 => 15.0,
            Self::S30 => 30.0,
            Self::S60 => 60.0,
            Self::S120 => 120.0,
            Self::S300 => 300.0,
        }
    }
}

impl QsonautGuiApp {
    pub(crate) fn draw_fst4_workspace(&mut self, ui: &mut egui::Ui, snapshot: &GuiState) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Submode").strong());
            egui::ComboBox::from_id_salt("fst4_submode")
                .selected_text(self.fst4_submode.label())
                .show_ui(ui, |ui| {
                    for submode in [
                        Submode::S15,
                        Submode::S30,
                        Submode::S60,
                        Submode::S120,
                        Submode::S300,
                    ] {
                        ui.selectable_value(&mut self.fst4_submode, submode, submode.label());
                    }
                });
            ui.label(format!("{} second T/R period", self.fst4_submode.seconds()));
        });
        self.draw_mfsk_mode_workspace(ui, snapshot, WorkspaceMode::Fst4);
        ui.label(
            RichText::new(
                "FST4 submodes use the matching mfsk-core decoder and waveform configuration.",
            )
            .small()
            .color(theme_muted(ui)),
        );
    }
}
