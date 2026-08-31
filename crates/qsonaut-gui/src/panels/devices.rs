use super::super::*;

impl QsonautGuiApp {
    fn test_cat_connection(&mut self) {
        let backend = self.config.radio.backend.clone();
        let model = self.config.radio.model.clone();
        let port = self.config.radio.serial_port.clone().unwrap_or_default();
        let baud_rate = self.config.radio.baud_rate;
        let controller_civ_address = self.config.radio.controller_civ_address;
        let radio_civ_address = self.config.radio.civ_address;
        let (tx, rx) = mpsc::channel();

        self.cat_test_status = None;
        self.cat_test_rx = Some(rx);
        info!(
            backend = %backend,
            model = %model,
            port = %if port.is_empty() { "auto" } else { &port },
            baud = baud_rate,
            "CAT connection test requested"
        );

        thread::spawn(move || {
            let result = if !backend.eq_ignore_ascii_case("native") {
                Err(format!(
                    "CAT test requires the Native Rigwright backend; selected backend is '{}'.",
                    backend
                ))
            } else if port.is_empty() {
                Err("CAT test requires a specific serial port; select a radio USB/serial port first.".into())
            } else {
                match open_model_with_radio_address(
                    &model,
                    &port,
                    baud_rate,
                    controller_civ_address,
                    Some(radio_civ_address),
                ) {
                    Ok(radio) => match radio {
                        ConfiguredRadio::Yaesu(yaesu) => match yaesu.verify_model() {
                            Ok(()) => Ok(format!(
                                "CAT OK: {} answered and matched the selected profile at {} baud.",
                                model, baud_rate
                            )),
                            Err(error) => Err(format!(
                                "CAT probe failed for {} at {} baud: {error}",
                                model, baud_rate
                            )),
                        },
                        ConfiguredRadio::Kenwood(kenwood) => match kenwood.verify_model() {
                            Ok(()) => Ok(format!(
                                "CAT OK: {} answered and matched the selected profile at {} baud.",
                                model, baud_rate
                            )),
                            Err(error) => Err(format!(
                                "CAT probe failed for {} at {} baud: {error}",
                                model, baud_rate
                            )),
                        },
                        ConfiguredRadio::Icom(icom) => {
                            match tokio::runtime::Builder::new_current_thread()
                                .enable_all()
                                .build()
                            {
                                Ok(runtime) => {
                                    match runtime.block_on(Radio::get_frequency_hz(&icom)) {
                                        Ok(frequency_hz) => Ok(format!(
                                            "CAT OK: {} answered at {} baud (frequency {} Hz).",
                                            model, baud_rate, frequency_hz
                                        )),
                                        Err(error) => Err(format!(
                                            "CAT probe failed for {} at {} baud: {error}",
                                            model, baud_rate
                                        )),
                                    }
                                }
                                Err(error) => Err(format!("CAT test runtime failed: {error}")),
                            }
                        }
                        _ => Err(format!(
                            "CAT testing is not implemented for the selected native profile '{}'.",
                            model
                        )),
                    },
                    Err(error) => Err(format!(
                        "Could not open CAT port '{}' at {} baud for {}: {error}",
                        port, baud_rate, model
                    )),
                }
            };

            match &result {
                Ok(message) => info!(
                    model = %model,
                    port = %port,
                    baud = baud_rate,
                    result = %message,
                    "CAT connection test succeeded"
                ),
                Err(error) => warn!(
                    model = %model,
                    port = %port,
                    baud = baud_rate,
                    error = %error,
                    "CAT connection test failed"
                ),
            }
            let _ = tx.send(result);
        });
    }

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

    pub(in super::super) fn reconnect_radio(&mut self) {
        info!(backend = %self.config.radio.backend, model = %self.config.radio.model, "Radio reconnect requested");
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
            info!("Radio reconnect skipped: radio is disabled");
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
        if self.config.audio.enabled && self.audio_worker_handle.is_none() {
            self.restart_audio();
        }
        info!(port = %self.config.radio.serial_port.as_deref().unwrap_or("auto"), "Radio reconnect initialization queued");
        let mut state = self.state.lock().expect("ui state lock poisoned");
        state.radio_waterfall_status = "CONNECTING…".to_string();
        state.last_error = None;
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
        let old_monitor = self.config.audio.monitor_enabled;
        let old_monitor_device = self.config.audio.monitor_output_device.clone();
        let null_radio = matches!(
            self.config.radio.backend.to_ascii_lowercase().as_str(),
            "null" | "mock"
        );
        let input_users = self.profile_device_users(
            |profile| profile.audio_input_device.as_ref(),
            |profile| profile.audio_enabled,
        );
        let output_users = self.profile_device_users(
            |profile| profile.audio_output_device.as_ref(),
            |profile| profile.audio_enabled,
        );
        let monitor_users = self.profile_device_users(
            |profile| profile.audio_monitor_output_device.as_ref(),
            |profile| profile.audio_enabled && profile.audio_monitor_enabled,
        );
        let serial_users = self.profile_device_users(
            |profile| profile.radio_serial_port.as_ref(),
            |profile| profile.radio_enabled,
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
                if null_radio {
                    ui.label("Audio devices");
                    ui.label(
                        RichText::new("QSONaut Null Sound Card · virtual input/output")
                            .small()
                            .color(theme_success(ui)),
                    );
                    ui.end_row();
                } else {
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

                ui.label("CAT baud rate");
                let baud_rates = radio_baud_rates(&self.config.radio.model);
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
            });

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
}
