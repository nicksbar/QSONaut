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

    pub(crate) fn draw_extended_radio_controls(&mut self, ui: &mut egui::Ui, snapshot: &GuiState) {
        let radio_ready = snapshot.radio_power_on == Some(true)
            && !snapshot.radio_power_command_pending
            && !snapshot.radio_power_settling;
        let supports = |id| snapshot.supported_controls.contains(&id);

        {
            let hardware_step = supports(ControlId::TuningStep);
            let values = self
                .driver_metadata
                .control_values
                .get(&ControlId::TuningStep)
                .copied()
                .map(<[u8]>::to_vec)
                .unwrap_or_else(|| (0..=5).collect());
            let response = ui.menu_button(RichText::new("STEP").monospace(), |ui| {
                ui.label(RichText::new("TUNING STEP").strong());
                if !hardware_step {
                    ui.label(RichText::new("Application tuning increment").small());
                }
                ui.separator();
                for value in values {
                    if ui
                        .selectable_label(
                            snapshot.tuning_step == Some(value),
                            tuning_step_label(value),
                        )
                        .clicked()
                    {
                        if hardware_step && !radio_ready {
                            continue;
                        }
                        if hardware_step {
                            self.send_command(GuiCommand::SetControl(
                                ControlId::TuningStep,
                                ControlValue::U8(value),
                            ));
                        } else {
                            self.state
                                .lock()
                                .expect("ui state lock poisoned")
                                .tuning_step = Some(value);
                        }
                        ui.close();
                    }
                }
            });
            response.response.on_hover_text(if hardware_step {
                "Select the radio tuning step"
            } else {
                "Select the application tuning increment"
            });
        }

        if supports(ControlId::Antenna) {
            let values = self
                .driver_metadata
                .control_values
                .get(&ControlId::Antenna)
                .copied()
                .map(<[u8]>::to_vec)
                .unwrap_or_else(|| {
                    let maximum = self
                        .driver_metadata
                        .control_maxes
                        .get(&ControlId::Antenna)
                        .copied()
                        .unwrap_or(2);
                    (1..=maximum).collect()
                });
            let response = ui.menu_button(RichText::new("ANT").monospace(), |ui| {
                ui.label(RichText::new("ANTENNA").strong());
                ui.separator();
                for value in values {
                    if ui
                        .selectable_label(snapshot.antenna == Some(value), format!("ANT {value}"))
                        .clicked()
                    {
                        if !radio_ready {
                            continue;
                        }
                        self.send_command(GuiCommand::SetControl(
                            ControlId::Antenna,
                            ControlValue::U8(value),
                        ));
                        ui.close();
                    }
                }
            });
            response
                .response
                .on_hover_text("Select the active antenna connector");
        }

        self.draw_normalized_control_menu(
            ui,
            snapshot,
            radio_ready,
            ControlId::MicGain,
            "MIC",
            snapshot.mic_gain,
            "Transmit microphone gain",
        );
        self.draw_normalized_control_menu(
            ui,
            snapshot,
            radio_ready,
            ControlId::MonitorLevel,
            "MON",
            snapshot.monitor_level,
            "Transmit audio monitor level",
        );
        self.draw_speech_processor_control(ui, snapshot, radio_ready);

        if supports(ControlId::Lock) {
            let locked = snapshot.lock == Some(true);
            let response = ui
                .add_enabled(
                    radio_ready,
                    egui::Button::new(if locked { "LOCKED" } else { "LOCK" }),
                )
                .on_hover_text(if locked {
                    "Unlock the radio controls"
                } else {
                    "Lock the radio controls"
                });
            if response.clicked() {
                self.send_command(GuiCommand::SetControl(
                    ControlId::Lock,
                    ControlValue::Bool(!locked),
                ));
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn draw_normalized_control_menu(
        &mut self,
        ui: &mut egui::Ui,
        snapshot: &GuiState,
        radio_ready: bool,
        id: ControlId,
        label: &str,
        value: Option<u8>,
        tooltip: &str,
    ) {
        if !snapshot.supported_controls.contains(&id) {
            return;
        }
        let response = ui.menu_button(RichText::new(label).monospace(), |ui| {
            let mut percent = value
                .map(|raw| f32::from(raw) * 100.0 / 255.0)
                .unwrap_or_default();
            let slider = ui.add_enabled(
                radio_ready,
                egui::Slider::new(&mut percent, 0.0..=100.0)
                    .vertical()
                    .show_value(false),
            );
            ui.label(format!("{percent:.0}%"));
            if slider.changed() && slider.drag_stopped() {
                let normalized = (percent.clamp(0.0, 100.0) * 255.0 / 100.0).round() as u8;
                self.send_command(GuiCommand::SetControl(id, ControlValue::U8(normalized)));
            }
        });
        response.response.on_hover_text(tooltip);
    }

    fn draw_speech_processor_control(
        &mut self,
        ui: &mut egui::Ui,
        snapshot: &GuiState,
        radio_ready: bool,
    ) {
        let supports_processor = snapshot
            .supported_controls
            .contains(&ControlId::SpeechProcessor);
        let supports_level = snapshot
            .supported_controls
            .contains(&ControlId::SpeechProcessorLevel);
        if !supports_processor && !supports_level {
            return;
        }
        let response = ui.menu_button(
            RichText::new(if snapshot.speech_processor == Some(true) {
                "PROC ON"
            } else {
                "PROC"
            })
            .monospace(),
            |ui| {
                if supports_processor {
                    let mut enabled = snapshot.speech_processor == Some(true);
                    if ui
                        .add_enabled(
                            radio_ready,
                            egui::Checkbox::new(&mut enabled, "Speech processor"),
                        )
                        .changed()
                    {
                        self.send_command(GuiCommand::SetControl(
                            ControlId::SpeechProcessor,
                            ControlValue::Bool(enabled),
                        ));
                    }
                }
                if supports_level {
                    let mut percent = snapshot
                        .speech_processor_level
                        .map(|raw| f32::from(raw) * 100.0 / 255.0)
                        .unwrap_or_default();
                    let slider = ui.add_enabled(
                        radio_ready,
                        egui::Slider::new(&mut percent, 0.0..=100.0).text("Level"),
                    );
                    if slider.changed() && slider.drag_stopped() {
                        let normalized = (percent.clamp(0.0, 100.0) * 255.0 / 100.0).round() as u8;
                        self.send_command(GuiCommand::SetControl(
                            ControlId::SpeechProcessorLevel,
                            ControlValue::U8(normalized),
                        ));
                    }
                }
            },
        );
        response
            .response
            .on_hover_text("Enable speech processing and adjust its level");
    }
}

fn tuning_step_label(value: u8) -> String {
    const STEPS_HZ: [u32; 6] = [1, 5, 10, 50, 100, 1000];
    STEPS_HZ
        .get(usize::from(value))
        .map(|step| format!("{step} Hz"))
        .unwrap_or_else(|| format!("Step {value}"))
}
