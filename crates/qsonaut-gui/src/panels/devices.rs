use super::super::*;

impl QsonautGuiApp {
    fn profile_device_users(
        &self,
        selector: impl Fn(&OperatorProfile) -> Option<&String>,
        enabled: impl Fn(&OperatorProfile) -> bool,
    ) -> HashMap<String, Vec<String>> {
        let mut users = HashMap::new();
        for name in &self.available_profiles {
            let Some(profile) = load_operator_profile_named(name) else {
                continue;
            };
            if enabled(&profile) {
                if let Some(device) = selector(&profile) {
                    users
                        .entry(device.clone())
                        .or_insert_with(Vec::new)
                        .push(name.clone());
                }
            }
        }
        users
    }

    fn device_choice_label(
        device: &str,
        users: &HashMap<String, Vec<String>>,
        fallback: &str,
    ) -> String {
        match users.get(device) {
            Some(profiles) if !profiles.is_empty() => {
                format!("{device} · used by {}", profiles.join(", "))
            }
            _ => format!("{device} · {fallback}"),
        }
    }

    pub(in super::super) fn restart_audio(&mut self) {
        info!(enabled = self.config.audio.enabled, input = ?self.config.audio.input_device, "Audio worker restart requested");
        self.audio_worker_stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.audio_worker_handle.take() {
            let _ = handle.join();
        }
        if !self.config.audio.enabled {
            if let Ok(mut state) = self.state.lock() {
                state.audio_spectrum_status = "DISABLED".to_string();
            }
            self.audio_restart_required = false;
            info!("Audio worker stopped by operator");
            return;
        }
        self.audio_worker_stop = Arc::new(AtomicBool::new(false));
        self.audio_worker_handle = Some(spawn_audio_spectrum_worker(
            self.state.clone(),
            self.audio_worker_stop.clone(),
            self.ft8_tx_active.clone(),
            self.digital_tx_active.clone(),
            self.config.audio.enabled,
            true,
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
        info!("Audio worker restart queued");
    }

    pub(in super::super) fn draw_device_settings(
        &mut self,
        ui: &mut egui::Ui,
        include_monitor: bool,
    ) {
        ui.horizontal(|ui| {
            ui.heading("Devices");
            if ui.small_button("Refresh").clicked() {
                self.refresh_device_lists();
            }
            if self.device_scan.is_some() {
                ui.spinner();
                ui.label(RichText::new("Scanning…").small().color(theme_muted(ui)));
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
        let old_audio_enabled = self.config.audio.enabled;
        let old_output = self.config.audio.output_device.clone();
        let old_port = self.config.radio.serial_port.clone();
        let old_enabled = self.config.radio.enabled;
        let old_backend = self.config.radio.backend.clone();
        let old_endpoint = self.config.radio.endpoint.clone();
        let old_model = self.config.radio.model.clone();
        let old_baud = self.config.radio.baud_rate;
        let old_hostbridge_key = self.config.radio.hostbridge_access_key.clone();
        let old_hostbridge_password = self.config.radio.hostbridge_password.clone();
        let old_hostbridge_radio_id = self.config.radio.hostbridge_radio_id.clone();
        let old_hostbridge_audio_source_id = self.config.radio.hostbridge_audio_source_id.clone();
        let old_hostbridge_audio_output_id = self.config.radio.hostbridge_audio_output_id.clone();
        let old_monitor = self.config.audio.monitor_enabled;
        let old_monitor_device = self.config.audio.monitor_output_device.clone();
        let input_users = self.profile_device_users(
            |profile| profile.audio.input_device.as_ref(),
            |profile| profile.audio.enabled,
        );
        let output_users = self.profile_device_users(
            |profile| profile.audio.output_device.as_ref(),
            |profile| profile.audio.enabled,
        );
        let monitor_users = self.profile_device_users(
            |profile| profile.audio.monitor_output_device.as_ref(),
            |profile| profile.audio.enabled && profile.audio.monitor_enabled,
        );
        let serial_users = self.profile_device_users(
            |profile| profile.radio.serial_port.as_ref(),
            |profile| profile.radio.enabled,
        );

        egui::Grid::new("device_settings_grid")
            .num_columns(2)
            .spacing([10.0, 6.0])
            .show(ui, |ui| {
                ui.label("Radio connection");
                ui.checkbox(&mut self.config.radio.enabled, "Enabled");
                ui.end_row();

                ui.label("Audio worker");
                ui.checkbox(&mut self.config.audio.enabled, "Enabled");
                ui.end_row();

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

                if self.config.radio.backend.eq_ignore_ascii_case("hostbridge") {
                    ui.label("HostBridge endpoint");
                    ui.text_edit_singleline(&mut self.config.radio.endpoint);
                    ui.end_row();
                    ui.label("Access key");
                    ui.text_edit_singleline(&mut self.config.radio.hostbridge_access_key);
                    ui.end_row();
                    ui.label("Password");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.config.radio.hostbridge_password)
                            .password(true),
                    );
                    ui.end_row();
                    ui.label("Radio device ID");
                    ui.text_edit_singleline(
                        self.config
                            .radio
                            .hostbridge_radio_id
                            .get_or_insert_with(String::new),
                    );
                    ui.end_row();
                    ui.label("Audio source ID");
                    ui.text_edit_singleline(
                        self.config
                            .radio
                            .hostbridge_audio_source_id
                            .get_or_insert_with(String::new),
                    );
                    ui.end_row();
                    ui.label("Audio output ID");
                    ui.text_edit_singleline(
                        self.config
                            .radio
                            .hostbridge_audio_output_id
                            .get_or_insert_with(String::new),
                    );
                    ui.end_row();
                    ui.label(
                        RichText::new(
                            "Leave the ID blank to select the first available host radio.",
                        )
                        .small()
                        .color(theme_muted(ui)),
                    );
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
            });

        if self.config.radio.backend.eq_ignore_ascii_case("native") {
            let help = help_for_model(&self.config.radio.model);
            ui.add_space(4.0);
            ui.group(|ui| {
                ui.label(RichText::new(format!("Recommended setup · {}", help.title)).strong());
                ui.add(egui::Label::new(help.blurb).wrap());
                ui.horizontal(|ui| {
                    if ui.link("Model FAQ").clicked() {
                        self.radio_help_window_model = self.config.radio.model.clone();
                        self.radio_faq_document = RadioHelpDocument::Model;
                        self.radio_faq_window_open = true;
                    }
                    if ui.link("Manufacturer FAQ").clicked() {
                        self.radio_help_window_model = self.config.radio.model.clone();
                        self.radio_faq_document = RadioHelpDocument::Manufacturer;
                        self.radio_faq_window_open = true;
                    }
                    if ui.link("Model Guide").clicked() {
                        self.radio_help_window_model = self.config.radio.model.clone();
                        self.radio_guide_document = RadioHelpDocument::Model;
                        self.radio_guide_window_open = true;
                    }
                    if ui.link("Manufacturer Guide").clicked() {
                        self.radio_help_window_model = self.config.radio.model.clone();
                        self.radio_guide_document = RadioHelpDocument::Manufacturer;
                        self.radio_guide_window_open = true;
                    }
                    if ui.link("Audio FAQ").clicked() {
                        self.radio_help_window_model = self.config.radio.model.clone();
                        self.radio_faq_document = RadioHelpDocument::Audio;
                        self.radio_faq_window_open = true;
                    }
                });
            });
            ui.add_space(4.0);
        }

        egui::Grid::new("device_audio_serial_grid")
            .num_columns(2)
            .spacing([10.0, 6.0])
            .show(ui, |ui| {
                ui.label("Audio input");
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt("audio_input_device")
                        .selected_text(
                            self.config
                                .audio
                                .input_device
                                .as_deref()
                                .unwrap_or("System default"),
                        )
                        .width((ui.available_width() - 34.0).max(180.0))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.config.audio.input_device,
                                None,
                                "System default",
                            );
                            for name in &input_devices {
                                let label =
                                    Self::device_choice_label(name, &input_users, "available");
                                ui.selectable_value(
                                    &mut self.config.audio.input_device,
                                    Some(name.clone()),
                                    label,
                                );
                            }
                        });
                    if ui
                        .small_button("↻")
                        .on_hover_text("Re-scan audio input devices")
                        .clicked()
                    {
                        self.refresh_device_lists();
                    }
                });
                ui.end_row();

                ui.label("Audio output");
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt("audio_output_device")
                        .selected_text(
                            self.config
                                .audio
                                .output_device
                                .as_deref()
                                .unwrap_or("System default"),
                        )
                        .width((ui.available_width() - 34.0).max(180.0))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.config.audio.output_device,
                                None,
                                "System default",
                            );
                            for name in &output_devices {
                                let label =
                                    Self::device_choice_label(name, &output_users, "available");
                                ui.selectable_value(
                                    &mut self.config.audio.output_device,
                                    Some(name.clone()),
                                    label,
                                );
                            }
                        });
                    if ui
                        .small_button("↻")
                        .on_hover_text("Re-scan audio output devices")
                        .clicked()
                    {
                        self.refresh_device_lists();
                    }
                });
                ui.end_row();

                if include_monitor {
                    ui.label("RX monitor output");
                    ui.horizontal(|ui| {
                        egui::ComboBox::from_id_salt("settings_monitor_output_device")
                            .selected_text(
                                self.config
                                    .audio
                                    .monitor_output_device
                                    .as_deref()
                                    .or(self.config.audio.output_device.as_deref())
                                    .unwrap_or("Audio output device"),
                            )
                            .width((ui.available_width() - 34.0).max(180.0))
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut self.config.audio.monitor_output_device,
                                    None,
                                    "Use audio output device",
                                );
                                for name in &output_devices {
                                    let label = Self::device_choice_label(
                                        name,
                                        &monitor_users,
                                        "available",
                                    );
                                    ui.selectable_value(
                                        &mut self.config.audio.monitor_output_device,
                                        Some(name.clone()),
                                        label,
                                    );
                                }
                            });
                        if ui
                            .small_button("↻")
                            .on_hover_text("Re-scan audio output devices")
                            .clicked()
                        {
                            self.refresh_device_lists();
                        }
                    });
                    ui.end_row();
                }

                ui.label("Radio / USB serial");
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt("radio_serial_port")
                        .selected_text(match self.config.radio.serial_port.as_deref() {
                            Some(port) => self
                                .radio_serial_port_labels
                                .get(port)
                                .map(String::as_str)
                                .unwrap_or(port),
                            None => "Auto detect",
                        })
                        .width((ui.available_width() - 34.0).max(180.0))
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.config.radio.serial_port,
                                None,
                                "Auto detect",
                            );
                            for name in &serial_ports {
                                let port_label = self
                                    .radio_serial_port_labels
                                    .get(name)
                                    .map(String::as_str)
                                    .unwrap_or(name.as_str());
                                let label = match serial_users.get(name) {
                                    Some(profiles) if !profiles.is_empty() => {
                                        format!("{port_label} · used by {}", profiles.join(", "))
                                    }
                                    _ => format!("{port_label} · available"),
                                };
                                ui.selectable_value(
                                    &mut self.config.radio.serial_port,
                                    Some(name.clone()),
                                    label,
                                );
                            }
                        });
                    if ui
                        .small_button("↻")
                        .on_hover_text("Re-scan radio and USB serial devices")
                        .clicked()
                    {
                        self.refresh_device_lists();
                    }
                });
                ui.end_row();

                if self.config.radio.backend.eq_ignore_ascii_case("native") {
                    let baud_rates = radio_baud_rates(&self.config.radio.model);
                    if baud_rates.is_empty() {
                        ui.label("CAT baud rate");
                        ui.label(
                            RichText::new("No baud rates available for this radio model")
                                .small()
                                .color(theme_warning(ui)),
                        );
                        ui.end_row();
                    } else {
                        ui.label("CAT baud rate");
                        if !baud_rates.contains(&self.config.radio.baud_rate) {
                            self.config.radio.baud_rate = find_model(&self.config.radio.model)
                                .map(|profile| profile.preferred_baud_rate())
                                .filter(|baud| baud_rates.contains(baud))
                                .unwrap_or(baud_rates[0]);
                        }
                        egui::ComboBox::from_id_salt("radio_baud_rate")
                            .selected_text(self.config.radio.baud_rate.to_string())
                            .width(ui.available_width().max(180.0))
                            .show_ui(ui, |ui| {
                                for baud_rate in baud_rates {
                                    ui.selectable_value(
                                        &mut self.config.radio.baud_rate,
                                        *baud_rate,
                                        baud_rate.to_string(),
                                    );
                                }
                            });
                        ui.end_row();
                    }
                }
            });

        if matches!(
            self.config.radio.backend.to_ascii_lowercase().as_str(),
            "null" | "mock"
        ) {
            // NullRadio is a complete virtual radio, not a native model with
            // a missing serial device. Normalize the radio-side hardware
            // fields, but deliberately leave the audio fields untouched so
            // this tab can use real input/output devices for decoding.
            self.config.radio.model = "NullRadio".to_string();
            self.config.radio.serial_port = None;
            self.config.radio.endpoint.clear();
        }

        if self.config.radio.backend.eq_ignore_ascii_case("native") {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let testing = self.cat_test_rx.is_some();
                if ui
                    .add_enabled(!testing, egui::Button::new(if testing {
                        "Testing CAT…"
                    } else {
                        "Test CAT connection"
                    }))
                    .clicked()
                {
                    self.test_cat_connection();
                }
                ui.label(
                    RichText::new("Sends a read-only identification/query probe using the selected port and baud rate.")
                        .small()
                        .color(theme_muted(ui)),
                );
            });
            if let Some(result) = &self.cat_test_status {
                let (message, color) = match result {
                    Ok(message) => (message.as_str(), theme_success(ui)),
                    Err(error) => (error.as_str(), theme_warning(ui)),
                };
                ui.label(RichText::new(message).small().color(color));
            }
        }

        ui.add_space(6.0);
        /* Radio capability details are intentionally kept out of Settings. */
        /*
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
        */
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

        let audio_changed = old_audio_enabled != self.config.audio.enabled
            || old_input != self.config.audio.input_device
            || old_output != self.config.audio.output_device
            || old_monitor != self.config.audio.monitor_enabled
            || old_monitor_device != self.config.audio.monitor_output_device;
        let radio_changed = old_port != self.config.radio.serial_port
            || old_enabled != self.config.radio.enabled
            || old_backend != self.config.radio.backend
            || old_endpoint != self.config.radio.endpoint
            || old_model != self.config.radio.model
            || old_baud != self.config.radio.baud_rate;
        let radio_changed = radio_changed
            || old_hostbridge_key != self.config.radio.hostbridge_access_key
            || old_hostbridge_password != self.config.radio.hostbridge_password
            || old_hostbridge_radio_id != self.config.radio.hostbridge_radio_id
            || old_hostbridge_audio_source_id != self.config.radio.hostbridge_audio_source_id
            || old_hostbridge_audio_output_id != self.config.radio.hostbridge_audio_output_id;
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

        let audio_runtime_running = self.audio_worker_handle.is_some();
        ui.horizontal(|ui| {
            if audio_runtime_running {
                if ui.button("Stop audio worker").clicked() {
                    self.config.audio.enabled = false;
                    self.profile_dirty = true;
                    self.persist_profile("Audio worker stopped for");
                    self.restart_audio();
                }
            } else if ui
                .button(if self.config.audio.enabled {
                    "Retry audio worker"
                } else {
                    "Start audio worker"
                })
                .clicked()
            {
                if !self.config.audio.enabled {
                    self.config.audio.enabled = true;
                    self.profile_dirty = true;
                    self.persist_profile("Audio worker started for");
                }
                self.restart_audio();
            }
            ui.label(
                RichText::new("Stops/releases the selected audio input and monitor output.")
                    .small()
                    .color(theme_muted(ui)),
            );
        });

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

        let radio_runtime_running = self.command_tx.is_some()
            || (!self.radio_init_attempted && self.radio_init_rx.is_some());
        ui.horizontal(|ui| {
            if radio_runtime_running {
                if ui.button("Stop radio worker").clicked() {
                    self.config.radio.enabled = false;
                    self.profile_dirty = true;
                    self.persist_profile("Radio worker stopped for");
                    self.reconnect_radio();
                }
            } else if ui
                .button(if self.config.radio.enabled {
                    "Retry profile runtime"
                } else {
                    "Start radio worker"
                })
                .clicked()
            {
                if !self.config.radio.enabled {
                    self.config.radio.enabled = true;
                    self.profile_dirty = true;
                    self.persist_profile("Radio worker started for");
                }
                self.reconnect_radio();
            }
            ui.label(
                RichText::new("Releases or claims this profile’s radio connection.")
                    .small()
                    .color(theme_muted(ui)),
            );
        });
    }
}

const RADIO_BACKENDS: &[(&str, &str)] = &[
    ("native", "Native Rigwright"),
    ("hostbridge", "Remote HostBridge"),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::panels::radio_contract::{
        apply_radio_reconnect, mark_radio_reconnect_disabled, spawn_cat_connection_test,
        stop_radio_worker_for_reconnect,
    };

    #[test]
    fn native_radio_labels_and_controls_follow_rigwright_profiles() {
        assert_eq!(
            selected_radio_label("FTDX10"),
            "Yaesu FTDX10 — hardware validated"
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
        }
    }

    #[test]
    fn backend_labels_are_ui_owned_connection_labels() {
        assert_eq!(radio_backend_label("native"), "Native Rigwright");
        assert_eq!(radio_backend_label("RIGCTLD"), "Hamlib rigctld");
        assert_eq!(radio_backend_label("custom"), "custom");
    }

    #[test]
    fn unknown_or_virtual_models_have_no_unsafe_baud_rate_fallback() {
        assert!(radio_baud_rates("NullRadio").is_empty());
        assert!(radio_baud_rates("not-a-model").is_empty());
    }

    #[test]
    fn cat_probe_rejects_non_native_and_missing_port_before_opening_hardware() {
        let non_native = QsonautGuiApp::cat_connection_result(
            "rigctld",
            "IC-7300",
            "/dev/ttyUSB0",
            115_200,
            0xE0,
            0x94,
        )
        .expect_err("external backends are not probed here");
        assert!(non_native.contains("Native Rigwright backend"));

        let missing_port =
            QsonautGuiApp::cat_connection_result("native", "IC-7300", "", 115_200, 0xE0, 0x94)
                .expect_err("a CAT probe requires a selected port");
        assert!(missing_port.contains("specific serial port"));
    }

    #[test]
    fn cat_probe_reports_model_and_serial_open_failures() {
        let unknown_model = QsonautGuiApp::cat_connection_result(
            "native",
            "not-a-real-model",
            "/dev/null",
            115_200,
            0xE0,
            0x94,
        )
        .expect_err("unknown model");
        assert!(unknown_model.contains("Could not open CAT port"));
        assert!(unknown_model.contains("not-a-real-model"));

        let missing_device = QsonautGuiApp::cat_connection_result(
            "native",
            "IC-7300",
            "/definitely-not-a-real-serial-device",
            115_200,
            0xE0,
            0x94,
        )
        .expect_err("missing serial device");
        assert!(missing_device.contains("CAT probe failed"));
        assert!(missing_device.contains("IC-7300"));
    }

    #[test]
    fn cat_probe_projects_each_configured_vendor_result() {
        let null = QsonautGuiApp::cat_probe_result(
            "offline",
            115_200,
            ConfiguredRadio::Null(qsonaut_radio::NullRadio::new()),
        )
        .expect_err("offline profile has no CAT verification");
        assert!(null.contains("not implemented"));

        let yaesu = QsonautGuiApp::cat_probe_result(
            "FT-818",
            38_400,
            ConfiguredRadio::Yaesu(qsonaut_radio::yaesu::YaesuCatRadio::new_generic(
                "/definitely-not-a-real-serial-device",
                38_400,
            )),
        )
        .expect_err("unavailable Yaesu transport");
        assert!(yaesu.contains("CAT probe failed for FT-818"));

        let kenwood = QsonautGuiApp::cat_probe_result(
            "TS-890S",
            115_200,
            ConfiguredRadio::Kenwood(qsonaut_radio::kenwood::KenwoodCatRadio::new_generic(
                "/definitely-not-a-real-serial-device",
                115_200,
            )),
        )
        .expect_err("unavailable Kenwood transport");
        assert!(kenwood.contains("CAT probe failed for TS-890S"));
    }

    #[test]
    fn cat_connection_worker_propagates_validation_and_transport_errors() {
        let invalid_backend = spawn_cat_connection_test(
            "rigctld".to_string(),
            "IC-7300".to_string(),
            "/dev/null".to_string(),
            115_200,
            0xE0,
            0x94,
        )
        .recv()
        .expect("CAT worker result");
        assert!(invalid_backend
            .expect_err("non-native backend must be rejected")
            .contains("Native Rigwright backend"));

        let missing_port = spawn_cat_connection_test(
            "native".to_string(),
            "IC-7300".to_string(),
            String::new(),
            115_200,
            0xE0,
            0x94,
        )
        .recv()
        .expect("CAT worker result");
        assert!(missing_port
            .expect_err("missing port must be rejected")
            .contains("specific serial port"));
    }

    #[test]
    fn radio_session_stop_clears_command_and_replaces_stop_token() {
        let (tx, rx) = mpsc::channel();
        let old_stop = Arc::new(AtomicBool::new(false));
        let old_stop_identity = Arc::as_ptr(&old_stop);
        let mut stop = old_stop;
        let mut command_tx = Some(tx);
        let mut worker_handle = Some(thread::spawn(|| {}));

        stop_radio_worker_for_reconnect(&mut command_tx, &mut stop, &mut worker_handle);

        assert!(command_tx.is_none());
        assert!(worker_handle.is_none());
        assert!(!stop.load(Ordering::Relaxed));
        assert_ne!(Arc::as_ptr(&stop), old_stop_identity);
        assert!(matches!(rx.try_recv(), Ok(GuiCommand::Quit)));
    }

    #[test]
    fn disabled_radio_reconnect_clears_init_and_reports_unavailable() {
        let state = Arc::new(Mutex::new(GuiState::default()));
        let (_tx, rx) = mpsc::channel();
        let mut init_rx = Some(rx);
        let mut attempted = false;

        mark_radio_reconnect_disabled(&state, &mut init_rx, &mut attempted);

        assert!(init_rx.is_none());
        assert!(attempted);
        assert_eq!(
            state.lock().expect("state lock").radio_waterfall_status,
            "UNAVAILABLE (radio disabled)"
        );
    }

    #[test]
    fn enabled_radio_reconnect_queues_init_and_restarts_audio_when_requested() {
        let state = Arc::new(Mutex::new(GuiState::default()));
        let (_tx, rx) = mpsc::channel();
        let mut init_rx = None;
        let mut attempted = true;
        let mut restart_count = 0;
        let mut device_restart_required = true;

        apply_radio_reconnect(
            &mut init_rx,
            &mut attempted,
            &mut device_restart_required,
            &state,
            true,
            || rx,
            || restart_count += 1,
        );

        assert!(init_rx.is_some());
        assert!(!attempted);
        assert!(!device_restart_required);
        assert_eq!(restart_count, 1);
        assert_eq!(
            state.lock().expect("state lock").radio_waterfall_status,
            "CONNECTING…"
        );
    }

    #[test]
    fn selected_radio_label_keeps_unknown_profiles_explicit() {
        assert_eq!(
            selected_radio_label("not-a-real-model"),
            "Unknown profile: not-a-real-model"
        );
        assert_eq!(radio_backend_label(" NATIVE "), " NATIVE ");
    }
}
