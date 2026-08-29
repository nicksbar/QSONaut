use super::super::*;

impl QsonautGuiApp {
    pub(in super::super) fn draw_pota_panel(&mut self, ui: &mut egui::Ui) {
        ui.set_min_width(430.0);
        ui.heading("🌲 POTA activity");
        ui.separator();

        if !self.pota_enabled {
            ui.label(
                RichText::new("POTA data querying is disabled in Reporting.")
                    .color(theme_warning(ui)),
            );
            return;
        }

        let activators = self
            .pota_spots
            .iter()
            .map(|spot| spot.activator.as_str())
            .collect::<HashSet<_>>()
            .len();
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{activators} active activators")).strong());
            ui.separator();
            ui.label(format!("{} spots", self.pota_spots.len()));
            ui.separator();
            ui.label(
                self.pota_last_updated
                    .map(|updated| format!("updated {}s ago", updated.elapsed().as_secs()))
                    .unwrap_or_else(|| "waiting for first update".to_string()),
            );
        });
        if let Some(error) = &self.pota_last_error {
            ui.label(
                RichText::new(format!("POTA query error: {error}"))
                    .small()
                    .color(theme_warning(ui)),
            );
        }

        ui.add_space(6.0);
        ui.label(RichText::new("Activators over recent updates").strong());
        draw_pota_history_graph(ui, &self.pota_history);
        ui.separator();
        ui.label(RichText::new("Latest activator spots").strong());

        if self.pota_spots.is_empty() {
            ui.label(RichText::new("No current POTA spots.").color(theme_muted(ui)));
        } else {
            let spots = self.pota_spots.clone();
            egui::ScrollArea::vertical()
                .id_salt("pota_latest_spots")
                .max_height(330.0)
                .show(ui, |ui| {
                    for spot in spots.iter().take(50) {
                        let label = format!(
                            "{} · {} · {} · {} · {}",
                            spot.activator,
                            spot.reference,
                            band_for_frequency(spot.frequency_hz),
                            format_frequency(spot.frequency_hz),
                            spot.mode
                        );
                        if ui
                            .selectable_label(false, RichText::new(label).size(12.0))
                            .on_hover_text(format!(
                                "{}\nClick to tune {} on {} and switch to {} / POTA",
                                spot.name,
                                format_frequency(spot.frequency_hz),
                                band_for_frequency(spot.frequency_hz),
                                spot.mode
                            ))
                            .clicked()
                        {
                            self.tune_to_pota_spot(spot);
                        }
                        ui.label(
                            RichText::new(format!("{} · {}", spot.name, spot.reference))
                                .small()
                                .color(theme_muted(ui)),
                        );
                    }
                });
        }
    }

    fn tune_to_pota_spot(&mut self, spot: &PotaSpot) {
        let mode = parse_workspace_mode_token(&spot.mode);
        if let Some(mode) = mode {
            self.workspace_mode = mode;
            self.profile_dirty = true;
            self.persist_profile("POTA spot mode saved to");
            self.send_command(GuiCommand::ApplyWorkspace {
                mode,
                frequency_hz: spot.frequency_hz,
            });
        } else {
            self.send_command(GuiCommand::TuneTo(spot.frequency_hz));
        }
        info!(
            activator = %spot.activator,
            reference = %spot.reference,
            park = %spot.name,
            band = %band_for_frequency(spot.frequency_hz),
            frequency_hz = spot.frequency_hz,
            mode = %spot.mode,
            "POTA spot selected for tuning"
        );
    }
}

fn format_frequency(frequency_hz: u64) -> String {
    if frequency_hz >= 1_000_000 {
        format!("{:.6} MHz", frequency_hz as f64 / 1_000_000.0)
    } else {
        format!("{:.1} kHz", frequency_hz as f64 / 1_000.0)
    }
}

fn draw_pota_history_graph(ui: &mut egui::Ui, history: &VecDeque<(Instant, usize)>) {
    let desired = egui::vec2(ui.available_width().max(260.0), 110.0);
    let (rect, _) = ui.allocate_exact_size(desired, egui::Sense::hover());
    let painter = ui.painter_at(rect);
    let grid_color = ui.visuals().widgets.noninteractive.bg_stroke.color;
    painter.rect_stroke(
        rect,
        3.0,
        egui::Stroke::new(1.0_f32, grid_color),
        egui::StrokeKind::Inside,
    );
    if history.is_empty() {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "Waiting for POTA data",
            egui::FontId::proportional(12.0),
            theme_muted(ui),
        );
        return;
    }
    let max_value = history
        .iter()
        .map(|(_, count)| *count)
        .max()
        .unwrap_or(1)
        .max(1) as f32;
    let left = rect.left() + 6.0;
    let right = rect.right() - 6.0;
    let top = rect.top() + 8.0;
    let bottom = rect.bottom() - 8.0;
    let points = history
        .iter()
        .enumerate()
        .map(|(index, (_, count))| {
            let x = if history.len() == 1 {
                (left + right) / 2.0
            } else {
                left + (right - left) * index as f32 / (history.len() - 1) as f32
            };
            let y = bottom - (bottom - top) * *count as f32 / max_value;
            egui::pos2(x, y)
        })
        .collect::<Vec<_>>();
    painter.add(egui::Shape::line(
        points,
        egui::Stroke::new(2.0_f32, Color32::from_rgb(110, 220, 150)),
    ));
    painter.text(
        egui::pos2(left, top),
        egui::Align2::LEFT_TOP,
        format!("{max_value:.0}"),
        egui::FontId::proportional(10.0),
        theme_muted(ui),
    );
}

#[cfg(test)]
mod tests {
    use super::{draw_pota_history_graph, format_frequency};
    use std::collections::VecDeque;

    #[test]
    fn formats_pota_frequencies_in_human_readable_units() {
        assert_eq!(format_frequency(14_074_000), "14.074000 MHz");
        assert_eq!(format_frequency(144_300_000), "144.300000 MHz");
        assert_eq!(format_frequency(999_900), "999.9 kHz");
    }

    #[test]
    fn draws_an_empty_history_graph_without_data() {
        let context = eframe::egui::Context::default();
        let history = VecDeque::new();
        let _ = context.run(Default::default(), |context| {
            eframe::egui::CentralPanel::default().show(context, |ui| {
                draw_pota_history_graph(ui, &history);
            });
        });
    }
}
