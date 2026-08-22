use super::super::*;

impl QsonautGuiApp {
    pub(in super::super) fn reconnect_radio(&mut self) {
        if let Some(tx) = &self.command_tx {
            let _ = tx.send(GuiCommand::Quit);
        }
        self.radio_worker_stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.radio_worker_handle.take() {
            let _ = handle.join();
        }
        self.command_tx = None;
        self.radio_worker_stop = Arc::new(AtomicBool::new(false));

        if !self.config.radio.enabled {
            self.radio_init_rx = None;
            self.radio_init_attempted = true;
            let mut state = self.state.lock().expect("ui state lock poisoned");
            state.radio_waterfall_status = "UNAVAILABLE (radio disabled)".to_string();
            return;
        }

        let port = self.config.radio.serial_port.clone().unwrap_or_default();
        self.radio_init_rx = Some(spawn_radio_init(
            self.config.radio.backend.clone(),
            self.config.radio.model.clone(),
            port,
            self.config.radio.endpoint.clone(),
            self.config.radio.baud_rate,
            self.config.radio.controller_civ_address,
            self.config.radio.civ_address,
        ));
        self.radio_init_attempted = false;
        self.device_restart_required = false;
        let mut state = self.state.lock().expect("ui state lock poisoned");
        state.radio_waterfall_status = "CONNECTING…".to_string();
        state.last_error = None;
    }

    pub(in super::super) fn restart_audio(&mut self) {
        self.audio_worker_stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.audio_worker_handle.take() {
            let _ = handle.join();
        }
        self.audio_worker_stop = Arc::new(AtomicBool::new(false));
        self.audio_worker_handle = Some(spawn_audio_spectrum_worker(
            self.state.clone(),
            self.audio_worker_stop.clone(),
            self.ft8_tx_active.clone(),
            self.digital_tx_active.clone(),
            self.config.audio.enabled,
            self.config.audio.sample_rate_hz,
            self.config.audio.channels,
            self.config.audio.input_device.clone(),
            self.config.audio.monitor_enabled,
            self.config
                .audio
                .monitor_output_device
                .clone()
                .or_else(|| self.config.audio.output_device.clone()),
            self.monitor_volume.clone(),
            self.repaint_ctx.clone(),
            self.display_tuning.clone(),
        ));
        self.audio_restart_required = false;
    }

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
                .color(theme_muted(ui)),
            );
        });
        ui.separator();

        let input_devices = self.audio_input_devices.clone();
        let output_devices = self.audio_output_devices.clone();
        let serial_ports = self.radio_serial_ports.clone();
        let old_input = self.config.audio.input_device.clone();
        let old_output = self.config.audio.output_device.clone();
        let old_port = self.config.radio.serial_port.clone();
        let old_backend = self.config.radio.backend.clone();
        let old_endpoint = self.config.radio.endpoint.clone();
        let old_model = self.config.radio.model.clone();
        let old_baud = self.config.radio.baud_rate;
        let old_monitor = self.config.audio.monitor_enabled;
        let old_monitor_device = self.config.audio.monitor_output_device.clone();

        egui::Grid::new("device_settings_grid")
            .num_columns(2)
            .spacing([10.0, 6.0])
            .show(ui, |ui| {
                ui.label("Radio backend");
                egui::ComboBox::from_id_salt("radio_backend")
                    .selected_text(radio_backend_label(&self.config.radio.backend))
                    .width(ui.available_width().max(180.0))
                    .show_ui(ui, |ui| {
                        for (backend, label) in RADIO_BACKENDS {
                            ui.selectable_value(
                                &mut self.config.radio.backend,
                                backend.to_string(),
                                *label,
                            );
                        }
                    });
                ui.end_row();

                if matches!(
                    self.config.radio.backend.to_ascii_lowercase().as_str(),
                    "rigctld" | "rigctl" | "dxlab" | "dxlab-commander" | "commander"
                ) {
                    ui.label("Backend endpoint");
                    ui.text_edit_singleline(&mut self.config.radio.endpoint);
                    ui.end_row();
                }

                if self.config.radio.backend.eq_ignore_ascii_case("native") {
                    ui.label("Radio model");
                    egui::ComboBox::from_id_salt("radio_model")
                        .selected_text(selected_radio_label(&self.config.radio.model))
                        .width(ui.available_width().max(180.0))
                        .show_ui(ui, |ui| {
                            for manufacturer in Manufacturer::ALL {
                                ui.label(RichText::new(manufacturer.label()).strong());
                                for profile in POPULAR_RADIOS
                                    .iter()
                                    .filter(|profile| profile.manufacturer == manufacturer)
                                {
                                    ui.selectable_value(
                                        &mut self.config.radio.model,
                                        profile.model.to_string(),
                                        format!(
                                            "{} — {}",
                                            profile.model,
                                            profile.support.short_label()
                                        ),
                                    );
                                }
                                ui.separator();
                            }
                        });
                    ui.end_row();
                }

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
            RichText::new("The monitor plays captured audio from the selected monitor output.")
                .small()
                .color(theme_muted(ui)),
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
        });

        ui.add_space(6.0);
        if matches!(
            self.config.radio.backend.to_ascii_lowercase().as_str(),
            "null" | "mock"
        ) {
            ui.label(
                RichText::new("Offline test radio · no hardware connection")
                    .small()
                    .color(Color32::LIGHT_GREEN),
            );
        } else if !self.config.radio.backend.eq_ignore_ascii_case("native") {
            ui.label(
                RichText::new(format!(
                    "External backend: {} · capabilities are negotiated by that service",
                    radio_backend_label(&self.config.radio.backend)
                ))
                .small()
                .color(Color32::YELLOW),
            );
        } else if let Some(profile) = find_model(&self.config.radio.model) {
            let scope = if profile.capabilities.spectrum {
                "radio scope available"
            } else {
                "audio waterfall only"
            };
            ui.label(
                RichText::new(format!(
                    "{} {} · {} · {} · {}",
                    profile.manufacturer.label(),
                    profile.model,
                    profile.protocol.label(),
                    profile.support.detail_label(),
                    scope,
                ))
                .small()
                .color(if profile.support == SupportLevel::HardwareValidated {
                    theme_success(ui)
                } else {
                    theme_warning(ui)
                }),
            );
            egui::CollapsingHeader::new("Radio capability details")
                .default_open(false)
                .show(ui, |ui| {
                    ui.label(
                        RichText::new(format!(
                            "Rigwright controls: {}",
                            radio_capability_summary(*profile)
                        ))
                        .small()
                        .color(theme_muted(ui)),
                    );
                    if !self.radio_detected_models.is_empty() {
                        ui.label(
                            RichText::new(format!(
                                "Detected radios: {}",
                                self.radio_detected_models.join(", ")
                            ))
                            .small()
                            .color(theme_success(ui)),
                        );
                    }
                });
        } else {
            ui.label(
                RichText::new(format!(
                    "Unknown native radio profile '{}'; choose a Rigwright model",
                    self.config.radio.model
                ))
                .small()
                .color(theme_warning(ui)),
            );
        }
        let backend = self.config.radio.backend.to_ascii_lowercase();
        let backend_details = if matches!(backend.as_str(), "native" | "null" | "mock") {
            format!(
                "Backend: {}",
                radio_backend_label(&self.config.radio.backend)
            )
        } else {
            format!(
                "Backend: {} · {}",
                radio_backend_label(&self.config.radio.backend),
                self.config.radio.endpoint
            )
        };
        ui.label(
            RichText::new(format!(
                "{} · serial: {}",
                backend_details,
                self.config
                    .radio
                    .serial_port
                    .as_deref()
                    .unwrap_or("auto-detect")
            ))
            .small()
            .color(theme_muted(ui)),
        );

        let audio_changed = old_input != self.config.audio.input_device
            || old_output != self.config.audio.output_device
            || old_monitor != self.config.audio.monitor_enabled
            || old_monitor_device != self.config.audio.monitor_output_device;
        let radio_changed = old_port != self.config.radio.serial_port
            || old_backend != self.config.radio.backend
            || old_endpoint != self.config.radio.endpoint
            || old_model != self.config.radio.model
            || old_baud != self.config.radio.baud_rate;
        if audio_changed || radio_changed {
            if old_model != self.config.radio.model {
                if let Some(profile) = find_model(&self.config.radio.model) {
                    self.config.radio.baud_rate = profile.preferred_baud_rate();
                    if let Protocol::IcomCiV { default_address } = profile.protocol {
                        self.config.radio.civ_address = default_address;
                    }
                    if !profile.capabilities.spectrum {
                        self.civ_spectrum_on = false;
                    }
                }
            }
            self.audio_restart_required |= audio_changed;
            self.device_restart_required |= radio_changed;
            self.profile_dirty = true;
            self.persist_profile("Saved devices to");
        }

        if self.audio_restart_required {
            ui.add_space(4.0);
            if ui.button("Restart audio now").clicked() {
                self.restart_audio();
            }
            ui.label(
                RichText::new(
                    "Audio settings are saved. Restart audio now to apply the selected input and monitor output.",
                )
                .small()
                .color(theme_warning(ui)),
            );
        }

        if self.device_restart_required {
            ui.add_space(4.0);
            if ui.button("Reconnect radio now").clicked() {
                self.reconnect_radio();
            }
            ui.label(
                RichText::new(
                    "Radio settings are saved. Reconnect now to apply the selected backend and endpoint.",
                )
                .small()
                .color(theme_warning(ui)),
            );
        } else if input_devices.is_empty() && output_devices.is_empty() {
            ui.label(
                RichText::new("No audio devices were reported by the operating system.")
                    .small()
                    .color(theme_warning(ui)),
            );
        }
    }
}

const RADIO_BACKENDS: &[(&str, &str)] = &[
    ("native", "Native Rigwright"),
    ("rigctld", "Hamlib rigctld"),
    ("dxlab", "DX Lab Commander"),
    ("null", "Offline test radio"),
];

fn radio_backend_label(backend: &str) -> &str {
    RADIO_BACKENDS
        .iter()
        .find(|(value, _)| value.eq_ignore_ascii_case(backend))
        .map(|(_, label)| *label)
        .unwrap_or(backend)
}

fn selected_radio_label(model: &str) -> String {
    find_model(model)
        .map(|profile| {
            format!(
                "{} {} — {}",
                profile.manufacturer.label(),
                profile.model,
                profile.support.short_label()
            )
        })
        .unwrap_or_else(|| format!("Unknown profile: {model}"))
}

fn radio_capability_summary(profile: qsonaut_radio::models::RadioModelProfile) -> String {
    let capabilities = profile.driver_capabilities();
    let mut labels = Vec::new();
    if capabilities.can_get_frequency && capabilities.can_set_frequency {
        labels.push("frequency");
    }
    if capabilities.can_get_mode && capabilities.can_set_mode {
        labels.push("mode");
    }
    if capabilities.can_set_ptt {
        labels.push(if capabilities.can_get_ptt {
            "read/write PTT"
        } else {
            "write-only PTT"
        });
    }
    if profile.supports_control(ControlId::AfGain) {
        labels.push("AF gain");
    }
    if profile.supports_control(ControlId::RfPower) {
        labels.push("RF power");
    }
    if profile.supports_control(ControlId::Filter) {
        labels.push("filter");
    }
    if profile.supports_control(ControlId::Split) {
        labels.push("split");
    }
    if profile.capabilities.spectrum {
        labels.push("spectrum");
    }
    labels.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_radio_labels_and_controls_follow_rigwright_profiles() {
        assert_eq!(
            selected_radio_label("FTDX10"),
            "Yaesu FTDX10 — experimental"
        );
        assert!(radio_capability_summary(*find_model("FTDX10").unwrap()).contains("split"));
        assert!(!radio_capability_summary(*find_model("FT-991A").unwrap()).contains("split"));
        assert!(
            radio_capability_summary(*find_model("TS-890S").unwrap()).contains("write-only PTT")
        );
    }

    #[test]
    fn native_radio_exposes_protocol_only_profiles() {
        for (model, label) in [
            ("CI-V (generic)", "Icom CI-V (generic) — experimental"),
            ("CAT (generic)", "Yaesu CAT (generic) — experimental"),
            (
                "classic CAT (generic)",
                "Yaesu classic CAT (generic) — experimental",
            ),
            (
                "PC control (generic)",
                "Kenwood PC control (generic) — experimental",
            ),
        ] {
            assert_eq!(selected_radio_label(model), label);
            assert!(radio_capability_summary(*find_model(model).unwrap()).contains("frequency"));
        }
    }

    #[test]
    fn backend_labels_are_ui_owned_connection_labels() {
        assert_eq!(radio_backend_label("native"), "Native Rigwright");
        assert_eq!(radio_backend_label("RIGCTLD"), "Hamlib rigctld");
        assert_eq!(radio_backend_label("custom"), "custom");
    }
}
