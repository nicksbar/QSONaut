use super::super::*;

impl QsonautGuiApp {
    pub(crate) fn draw_power_button(&mut self, ui: &mut egui::Ui, snapshot: &GuiState) {
        let power_known = snapshot.radio_power_on.is_some();
        let power_on = snapshot.radio_power_on.unwrap_or(false);
        let (power_rect, power_button) = ui.allocate_exact_size(
            egui::vec2(28.0, 28.0),
            if snapshot.radio_power_supported && !snapshot.radio_power_command_pending {
                egui::Sense::click()
            } else {
                egui::Sense::hover()
            },
        );
        let power_color = if !snapshot.radio_power_supported {
            Color32::DARK_GRAY
        } else if !power_known {
            Color32::GRAY
        } else if power_on {
            Color32::LIGHT_GREEN
        } else {
            Color32::GRAY
        };
        let painter = ui.painter_at(power_rect);
        let center = power_rect.center();
        painter.circle_stroke(center, 8.0, egui::Stroke::new(2.0_f32, power_color));
        painter.line_segment(
            [
                egui::pos2(center.x, center.y - 11.0),
                egui::pos2(center.x, center.y + 1.0),
            ],
            egui::Stroke::new(2.0_f32, power_color),
        );
        if power_button.clicked() {
            self.state
                .lock()
                .expect("ui state lock poisoned")
                .radio_power_command_pending = true;
            self.send_command(GuiCommand::SetPower(!power_on));
        }
        power_button.on_hover_text(if !power_known {
            "Radio power: unknown · click to turn on"
        } else if power_on {
            "Radio power: ON · click to turn off"
        } else {
            "Radio power: OFF · click to turn on"
        });
    }

    pub(crate) fn draw_rf_power_control(&mut self, ui: &mut egui::Ui, snapshot: &GuiState) {
        let supported = snapshot.supported_controls.contains(&ControlId::RfPower);
        let ready = snapshot.radio_power_on == Some(true)
            && !snapshot.radio_power_command_pending
            && !snapshot.radio_power_settling;
        let color = if supported && ready {
            Color32::from_rgb(255, 190, 105)
        } else {
            Color32::GRAY
        };
        let power_button = ui
            .add_enabled(
                supported && ready,
                egui::Button::new(RichText::new("⚡").size(17.0).color(color)),
            )
            .on_hover_text(if !supported {
                "RF transmit power control is not supported by this radio profile"
            } else if !ready {
                "RF transmit power is unavailable while the radio is offline or waking"
            } else {
                "Open RF transmit power control"
            });
        egui::Popup::menu(&power_button)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
                ui.vertical_centered(|ui| {
                    let mut percent = snapshot
                        .rf_power
                        .map(|value| f32::from(value) * 100.0 / 255.0)
                        .unwrap_or_default();
                    let response = ui.add(
                        egui::Slider::new(&mut percent, 0.0..=100.0)
                            .vertical()
                            .show_value(false),
                    );
                    ui.label(format!("{percent:.0}%"));
                    if response.changed() && response.drag_stopped() {
                        let normalized = (percent.clamp(0.0, 100.0) * 255.0 / 100.0).round() as u8;
                        {
                            let mut state = self.state.lock().expect("ui state lock poisoned");
                            state.rf_power = Some(normalized);
                            state.rf_power_write_pending = Some(normalized);
                        }
                        self.send_command(GuiCommand::SetControl(
                            ControlId::RfPower,
                            ControlValue::U8(normalized),
                        ));
                    }
                });
            });
    }
}
