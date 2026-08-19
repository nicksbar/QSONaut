use super::super::*;

impl QsonautGuiApp {
    pub(in super::super) fn draw_device_settings(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("Devices");
            if ui.small_button("Refresh").clicked() {
                self.refresh_device_lists();
            }
            ui.label(
                RichText::new(format!(
                    "{} / {}",
                    std::env::consts::OS,
                    std::env::consts::ARCH
                ))
                .small()
                .color(Color32::GRAY),
            );
        });
        ui.separator();

        let input_devices = self.audio_input_devices.clone();
        let output_devices = self.audio_output_devices.clone();
        let serial_ports = self.radio_serial_ports.clone();
        let old_input = self.config.audio.input_device.clone();
        let old_output = self.config.audio.output_device.clone();
        let old_port = self.config.radio.serial_port.clone();
        let old_model = self.config.radio.model.clone();
        let old_baud = self.config.radio.baud_rate;
        let old_monitor = self.config.audio.monitor_enabled;
        let old_monitor_device = self.config.audio.monitor_output_device.clone();

        egui::Grid::new("device_settings_grid")
            .num_columns(2)
            .spacing([10.0, 6.0])
            .show(ui, |ui| {
                ui.label("Radio model");
                egui::ComboBox::from_id_salt("radio_model")
                    .selected_text(&self.config.radio.model)
                    .width(ui.available_width().max(180.0))
                    .show_ui(ui, |ui| {
                        for profile in POPULAR_RADIOS {
                            let maturity = if profile.support == SupportLevel::HardwareValidated {
                                "validated"
                            } else {
                                "experimental"
                            };
                            ui.selectable_value(
                                &mut self.config.radio.model,
                                profile.model.to_string(),
                                format!("{} — {maturity}", profile.model),
                            );
                        }
                    });
                ui.end_row();

                ui.label("Audio input");
                egui::ComboBox::from_id_salt("audio_input_device")
                    .selected_text(
                        self.config
                            .audio
                            .input_device
                            .as_deref()
                            .unwrap_or("System default"),
                    )
                    .width(ui.available_width().max(180.0))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.config.audio.input_device,
                            None,
                            "System default",
                        );
                        for name in &input_devices {
                            ui.selectable_value(
                                &mut self.config.audio.input_device,
                                Some(name.clone()),
                                name,
                            );
                        }
                    });
                ui.end_row();

                ui.label("Audio output");
                egui::ComboBox::from_id_salt("audio_output_device")
                    .selected_text(
                        self.config
                            .audio
                            .output_device
                            .as_deref()
                            .unwrap_or("System default"),
                    )
                    .width(ui.available_width().max(180.0))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.config.audio.output_device,
                            None,
                            "System default",
                        );
                        for name in &output_devices {
                            ui.selectable_value(
                                &mut self.config.audio.output_device,
                                Some(name.clone()),
                                name,
                            );
                        }
                    });
                ui.end_row();

                ui.label("Radio / USB serial");
                egui::ComboBox::from_id_salt("radio_serial_port")
                    .selected_text(match self.config.radio.serial_port.as_deref() {
                        Some(port) => self
                            .radio_serial_port_labels
                            .get(port)
                            .map(String::as_str)
                            .unwrap_or(port),
                        None => "Auto detect",
                    })
                    .width(ui.available_width().max(180.0))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.config.radio.serial_port,
                            None,
                            "Auto detect",
                        );
                        for name in &serial_ports {
                            let label = self
                                .radio_serial_port_labels
                                .get(name)
                                .map(String::as_str)
                                .unwrap_or(name.as_str());
                            ui.selectable_value(
                                &mut self.config.radio.serial_port,
                                Some(name.clone()),
                                label,
                            );
                        }
                    });
                ui.end_row();

                ui.label("CAT baud rate");
                ui.add(
                    egui::DragValue::new(&mut self.config.radio.baud_rate)
                        .range(1_200..=230_400)
                        .speed(1_200),
                );
                ui.end_row();
            });

        ui.add_space(6.0);
        ui.label(RichText::new("RX monitor diagnostics").strong());
        ui.label(
            RichText::new("The monitor plays captured audio from the selected monitor output. Use the test tone to verify that output device, stream, and system volume are working independently of the radio input.")
                .small()
                .color(Color32::GRAY),
        );
        ui.horizontal(|ui| {
            ui.label("Monitor volume");
            let mut volume = self.config.audio.monitor_volume;
            if ui
                .add(egui::Slider::new(&mut volume, 0.0..=2.0).suffix("×"))
                .changed()
            {
                self.config.audio.monitor_volume = volume;
                self.monitor_volume
                    .store(volume.to_bits(), Ordering::Relaxed);
                self.profile_dirty = true;
                self.persist_profile("RX monitor volume saved to");
            }
            if ui.button("Play test tone").clicked() {
                self.state
                    .lock()
                    .expect("ui state lock poisoned")
                    .monitor_test_tone = true;
            }
        });
        ui.label(
            RichText::new("Test tone: 700 Hz for 350 ms · increase the control only as needed; system output volume still applies.")
                .small()
                .color(Color32::GRAY),
        );

        ui.add_space(6.0);
        if let Some(profile) = find_model(&self.config.radio.model) {
            let maturity = if profile.support == SupportLevel::HardwareValidated {
                "hardware validated"
            } else {
                "experimental — hardware validation pending"
            };
            let scope = if profile.capabilities.spectrum {
                "radio scope available"
            } else {
                "audio waterfall only"
            };
            ui.label(
                RichText::new(format!(
                    "Selected profile: {} · {maturity} · {scope}",
                    profile.model
                ))
                .small()
                .color(if profile.support == SupportLevel::HardwareValidated {
                    Color32::LIGHT_GREEN
                } else {
                    Color32::YELLOW
                }),
            );
        }
        if self.radio_detected_models.is_empty() {
            ui.label(
                RichText::new("Detected radios: none recognized yet (serial bridge only or unsupported model)")
                    .small()
                    .color(Color32::YELLOW),
            );
        } else {
            ui.label(
                RichText::new(format!(
                    "Detected radios: {}",
                    self.radio_detected_models.join(", ")
                ))
                .small()
                .color(Color32::LIGHT_GREEN),
            );
        }

        if old_input != self.config.audio.input_device
            || old_output != self.config.audio.output_device
            || old_port != self.config.radio.serial_port
            || old_model != self.config.radio.model
            || old_baud != self.config.radio.baud_rate
            || old_monitor != self.config.audio.monitor_enabled
            || old_monitor_device != self.config.audio.monitor_output_device
        {
            if old_model != self.config.radio.model {
                if let Some(profile) = find_model(&self.config.radio.model) {
                    self.config.radio.baud_rate = match profile.protocol {
                        Protocol::YaesuLegacyCat => 4_800,
                        Protocol::YaesuCat => 38_400,
                        Protocol::IcomCiV { default_address } => {
                            self.config.radio.civ_address = default_address;
                            115_200
                        }
                        Protocol::KenwoodCat => 115_200,
                    };
                    if !profile.capabilities.spectrum {
                        self.civ_spectrum_on = false;
                    }
                }
            }
            self.device_restart_required = true;
            self.profile_dirty = true;
            self.persist_profile("Saved devices to");
        }

        if self.device_restart_required {
            ui.add_space(4.0);
            ui.label(
                RichText::new(
                    "Output changes apply to the next transmission. Restart QSONaut to reconnect input or radio devices.",
                )
                .small()
                .color(Color32::YELLOW),
            );
        } else if input_devices.is_empty() && output_devices.is_empty() {
            ui.label(
                RichText::new("No audio devices were reported by the operating system.")
                    .small()
                    .color(Color32::YELLOW),
            );
        }
    }
}
