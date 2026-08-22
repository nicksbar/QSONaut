use super::super::*;

#[allow(clippy::too_many_arguments)]
pub(super) fn draw_digital_conversation(
    ui: &mut egui::Ui,
    height: f32,
    id: &'static str,
    title: String,
    stage: Option<&str>,
    empty_text: &str,
    operator_call: &str,
    rx_tone_hz: u32,
    tx_tone_hz: u32,
    rx_level: u8,
    tx_level: u8,
    lines: Vec<Ft8ChatLine>,
) {
    egui::Frame::group(ui.style())
        .fill(Color32::from_rgb(20, 23, 28))
        .show(ui, |ui| {
            ui.set_min_height(height);
            ui.set_max_height(height);
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(title).strong().color(theme_accent(ui)));
                if let Some(stage) = stage {
                    ui.label(
                        RichText::new(stage)
                            .small()
                            .color(Color32::from_rgb(220, 180, 90)),
                    );
                }
                ui.separator();
                ui.label(
                    RichText::new(format!("RX CURSOR {rx_tone_hz} Hz"))
                        .monospace()
                        .color(Color32::from_rgb(120, 220, 120)),
                );
                ui.add(egui::ProgressBar::new(rx_level as f32 / 255.0).desired_width(70.0));
                ui.label(
                    RichText::new(format!("TX CURSOR {tx_tone_hz} Hz"))
                        .monospace()
                        .color(Color32::from_rgb(220, 160, 80)),
                );
                ui.add(egui::ProgressBar::new(tx_level as f32 / 255.0).desired_width(70.0));
            });
            ui.separator();
            egui::ScrollArea::vertical()
                .id_salt(id)
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    if lines.is_empty() {
                        ui.centered_and_justified(|ui| {
                            ui.label(RichText::new(empty_text).color(theme_muted(ui)));
                        });
                    }
                    for line in lines {
                        let is_tx = line.direction == Ft8ChatDirection::Tx;
                        let call_hit = (!is_tx)
                            .then(|| operator_call_hit(&line.message, operator_call))
                            .flatten();
                        let layout = if is_tx {
                            egui::Layout::right_to_left(egui::Align::Min)
                        } else {
                            egui::Layout::left_to_right(egui::Align::Min)
                        };
                        ui.with_layout(layout, |ui| {
                            let fill = if is_tx {
                                Color32::from_rgb(53, 43, 25)
                            } else {
                                Color32::from_rgb(25, 49, 38)
                            };
                            egui::Frame::group(ui.style()).fill(fill).show(ui, |ui| {
                                if let Some(hit) = call_hit {
                                    let (badge, accent, _) = call_hit_badge(hit);
                                    ui.label(RichText::new(badge).strong().color(accent));
                                }
                                ui.label(RichText::new(&line.message).monospace().strong());
                                ui.label(
                                    RichText::new(format!("{} · {}", line.utc, line.detail))
                                        .small()
                                        .color(theme_muted(ui)),
                                );
                            });
                        });
                        ui.add_space(2.0);
                    }
                });
        });
}
