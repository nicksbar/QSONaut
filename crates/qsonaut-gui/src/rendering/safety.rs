use super::super::*;

impl QsonautGuiApp {
    pub(crate) fn draw_tx_safety_card(&mut self, ui: &mut egui::Ui, snapshot: &GuiState) {
        let armed = self.any_tx_armed(snapshot);
        let (fill, border, status, detail) = if armed {
            (
                Color32::from_rgb(73, 35, 24),
                Color32::from_rgb(255, 137, 61),
                "🔥 TRANSMIT ARMED",
                "Digital automation, SSTV, queued audio, or PTT can transmit",
            )
        } else {
            (
                Color32::from_rgb(22, 48, 59),
                Color32::from_rgb(77, 184, 211),
                "🔒 ALL TX DISARMED",
                "Safe state · arm explicitly from a transmit workspace",
            )
        };

        egui::Frame::group(ui.style())
            .fill(fill)
            .stroke(egui::Stroke::new(2.0_f32, border))
            .show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new(status).strong().size(17.0).color(border));
                    ui.label(RichText::new(detail).small().color(Color32::LIGHT_GRAY));
                    ui.add_space(3.0);
                    if ui
                        .add_sized(
                            [ui.available_width(), 34.0],
                            egui::Button::new(
                                RichText::new("⛔ STOP + DISARM ALL TX")
                                    .strong()
                                    .color(Color32::WHITE),
                            )
                            .fill(Color32::from_rgb(126, 25, 39))
                            .stroke(egui::Stroke::new(
                                1.5_f32,
                                Color32::from_rgb(255, 105, 115),
                            )),
                        )
                        .on_hover_text(
                            "Drop PTT, abort queued/active audio, and disarm every automatic sequence",
                        )
                        .clicked()
                    {
                        self.disarm_all_tx("All TX stopped and disarmed by global safety control");
                    }
                });
            });
    }
}
