use super::super::*;

impl QsonautGuiApp {
    pub(crate) fn draw_meter_display(&mut self, ui: &mut egui::Ui, snapshot: &GuiState) {
        let primary_id = if snapshot.ptt_on {
            MeterId::Power
        } else {
            MeterId::Signal
        };
        let primary_value = meter_value(snapshot, primary_id);
        let primary_label = if snapshot.ptt_on { "POWER" } else { "" };
        let primary_reading = meter_reading(primary_id, primary_value);
        let radio_model = self.config.radio.model.as_str();
        let (primary_rect, primary_response) =
            ui.allocate_exact_size(egui::vec2(280.0, 24.0), egui::Sense::click());
        draw_primary_meter(
            ui,
            primary_rect,
            primary_label,
            &primary_reading,
            primary_value.map(meter_percent).unwrap_or_default(),
            meter_color(primary_id, primary_value),
        );
        let s_click = primary_response.on_hover_text("Click to open the live radio meter drawer");
        if s_click.clicked() {
            self.show_meter_panel = !self.show_meter_panel;
            self.meter_panel_close_deadline = None;
        }

        if self.show_meter_panel {
            let drawer_position = egui::pos2(primary_rect.left(), primary_rect.bottom() + 5.0);
            egui::Area::new(ui.id().with("meter_drawer_overlay"))
                .order(egui::Order::Foreground)
                .fixed_pos(drawer_position)
                .show(ui.ctx(), |ui| {
                    egui::Frame::group(ui.style())
                        .fill(Color32::from_rgba_unmultiplied(18, 30, 42, 245))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new(format!(
                                        "TX / PA METERS · {}",
                                        radio_mode_label(&snapshot.mode, snapshot.data_mode)
                                    ))
                                    .strong()
                                    .color(
                                        if snapshot.ptt_on {
                                            Color32::from_rgb(255, 145, 120)
                                        } else {
                                            Color32::from_rgb(120, 225, 255)
                                        },
                                    ),
                                );
                                if snapshot.ptt_on {
                                    ui.label(
                                        RichText::new("● TRANSMIT").strong().color(Color32::RED),
                                    );
                                }
                            });
                            ui.separator();
                            if snapshot.ptt_on
                                && snapshot.supported_controls.contains(&ControlId::RfPower)
                            {
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new("TX SET").monospace().strong())
                                        .on_hover_text(
                                            "Configured RF transmit power, not measured output",
                                        );
                                    let value = snapshot.rf_power;
                                    ui.label(meter_reading(MeterId::Power, value));
                                });
                            }
                            for id in mode_meter_order(snapshot.ptt_on) {
                                if id == MeterId::Signal {
                                    continue;
                                }
                                if !snapshot.supported_meters.contains(&id) {
                                    continue;
                                }
                                let value = meter_value(snapshot, id);
                                ui.horizontal(|ui| {
                                    let reading = meter_reading_for_model(id, value, radio_model);
                                    let label_height =
                                        if id == MeterId::Voltage { 28.0 } else { 18.0 };
                                    let label_color = if id == MeterId::Current && snapshot.ptt_on {
                                        Color32::from_rgb(150, 255, 225)
                                    } else {
                                        Color32::WHITE
                                    };
                                    ui.add_sized(
                                        egui::vec2(METER_LABEL_WIDTH, label_height),
                                        egui::Label::new(
                                            RichText::new(meter_label(id))
                                                .monospace()
                                                .strong()
                                                .color(label_color),
                                        ),
                                    )
                                    .on_hover_text(meter_tooltip(id));
                                    if id == MeterId::Voltage {
                                        draw_voltage_graph(ui, &snapshot.voltage_history, &reading);
                                        return;
                                    }
                                    let meter_response = ui.add(
                                        egui::ProgressBar::new(
                                            value.map(meter_percent).unwrap_or_default(),
                                        )
                                        .desired_width(ui.available_width().max(100.0))
                                        .desired_height(14.0)
                                        .fill(meter_color_for_context(id, value, snapshot.ptt_on)),
                                    );
                                    let reading_width = 136.0;
                                    let reading_rect = egui::Rect::from_min_max(
                                        egui::pos2(
                                            meter_response.rect.right() - reading_width,
                                            meter_response.rect.top() + 1.0,
                                        ),
                                        egui::pos2(
                                            meter_response.rect.right() - 3.0,
                                            meter_response.rect.bottom() - 1.0,
                                        ),
                                    );
                                    ui.painter().rect_filled(
                                        reading_rect,
                                        egui::CornerRadius::same(3),
                                        Color32::from_rgba_unmultiplied(10, 20, 29, 225),
                                    );
                                    ui.painter().text(
                                        reading_rect.right_center() - egui::vec2(5.0, 0.0),
                                        egui::Align2::RIGHT_CENTER,
                                        reading,
                                        egui::FontId::monospace(11.0),
                                        Color32::WHITE,
                                    );
                                });
                            }
                        });
                });
        }
    }
}
