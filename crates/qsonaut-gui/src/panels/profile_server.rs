use super::super::*;

fn edit_optional_u8(ui: &mut egui::Ui, value: &mut Option<u8>, min: u8, max: u8) {
    let mut current = i32::from(value.unwrap_or(min));
    if ui
        .add(egui::DragValue::new(&mut current).range(i32::from(min)..=i32::from(max)))
        .changed()
    {
        *value = Some(current.clamp(i32::from(min), i32::from(max)) as u8);
    }
}

impl QsonautGuiApp {
    pub(in super::super) fn draw_profile_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Operator Profile");
        ui.separator();

        ui.horizontal(|ui| {
            ui.label(RichText::new("Call").strong());
            let changed = ui
                .add(
                    egui::TextEdit::singleline(&mut self.station_callsign)
                        .desired_width(110.0)
                        .hint_text("N0CALL")
                        .font(egui::TextStyle::Monospace),
                )
                .changed();
            if changed {
                self.station_callsign = self.station_callsign.trim().to_ascii_uppercase();
                let val = self.station_callsign.trim();
                self.config.station.callsign = if val.is_empty() {
                    None
                } else {
                    Some(val.to_string())
                };
                self.restart_psk_reporter();
                self.profile_dirty = true;
                self.persist_profile("Auto-saved");
                self.emit_operator_profile_hook(format!(
                    "callsign_changed={}",
                    self.station_callsign_or_default()
                ));
            }

            ui.label(RichText::new("Grid").strong());
            let changed = ui
                .add(
                    egui::TextEdit::singleline(&mut self.station_grid)
                        .desired_width(90.0)
                        .hint_text("AA00")
                        .font(egui::TextStyle::Monospace),
                )
                .changed();
            if changed {
                self.station_grid = self.station_grid.trim().to_ascii_uppercase();
                let val = self.station_grid.trim();
                self.config.station.grid = if val.is_empty() {
                    None
                } else {
                    Some(val.to_string())
                };
                self.restart_psk_reporter();
                self.profile_dirty = true;
                self.persist_profile("Auto-saved");
                self.emit_operator_profile_hook(format!(
                    "grid_changed={}",
                    self.station_grid_or_default()
                ));
            }
        });

        if ui
            .button("Load license profile from HamDB")
            .on_hover_text(
                "Look up this callsign and fill the station profile from its license record",
            )
            .clicked()
        {
            self.load_profile_from_hamdb();
        }

        ui.horizontal(|ui| {
            ui.label(RichText::new("QTH").strong());
            let qth_changed = ui
                .add(
                    egui::TextEdit::singleline(&mut self.station_qth)
                        .desired_width(ui.available_width())
                        .hint_text("City / locator notes")
                        .font(egui::TextStyle::Monospace),
                )
                .changed();
            if qth_changed {
                self.profile_dirty = true;
                self.persist_profile("Auto-saved");
                self.emit_operator_profile_hook("qth_changed");
            }
        });

        ui.add_space(8.0);
        ui.label(RichText::new("Saved profiles").strong());
        ui.horizontal_wrapped(|ui| {
            let previous_profile = self.selected_profile_name.clone();
            egui::ComboBox::from_id_salt("operator_profile_select")
                .selected_text(&self.selected_profile_name)
                .width(150.0)
                .show_ui(ui, |ui| {
                    for profile in &self.available_profiles {
                        ui.selectable_value(
                            &mut self.selected_profile_name,
                            profile.clone(),
                            profile,
                        );
                    }
                });
            if self.selected_profile_name != previous_profile {
                match load_operator_profile_named(&self.selected_profile_name) {
                    Some(profile) => {
                        self.apply_operator_profile(profile);
                        if let Err(error) = select_operator_profile(&self.selected_profile_name) {
                            self.profile_io_status = format!("Profile selection failed: {error}");
                        } else {
                            self.profile_io_status =
                                format!("Loaded profile ‘{}’", self.selected_profile_name);
                        }
                    }
                    None => {
                        self.selected_profile_name = previous_profile;
                        self.profile_io_status = "Selected profile could not be loaded".to_string();
                    }
                }
            }
            if ui.small_button("Save").clicked() {
                self.persist_profile("Saved");
            }
            if ui.small_button("Reload").clicked() {
                match load_operator_profile_named(&self.selected_profile_name) {
                    Some(profile) => {
                        self.apply_operator_profile(profile);
                        self.profile_io_status =
                            format!("Reloaded profile ‘{}’", self.selected_profile_name);
                    }
                    None => {
                        self.profile_io_status =
                            format!("Profile ‘{}’ was not found", self.selected_profile_name);
                    }
                }
            }
        });

        ui.horizontal_wrapped(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.new_profile_name)
                    .desired_width(150.0)
                    .hint_text("New profile name"),
            );
            if ui.button("Create new profile").clicked() {
                let name = self.new_profile_name.trim().to_string();
                let duplicate = self
                    .available_profiles
                    .iter()
                    .any(|profile| profile.eq_ignore_ascii_case(&name));
                if duplicate {
                    self.profile_io_status = format!("Profile ‘{name}’ already exists");
                } else {
                    match save_operator_profile_named(&name, &self.current_operator_profile()) {
                        Ok(()) => {
                            self.selected_profile_name = name.clone();
                            self.available_profiles = list_operator_profiles();
                            self.new_profile_name.clear();
                            self.profile_io_status = format!("Created profile ‘{name}’");
                            self.profile_dirty = false;
                        }
                        Err(error) => {
                            self.profile_io_status = format!("Profile creation failed: {error}");
                        }
                    }
                }
            }
        });
        ui.add_space(4.0);
        ui.label(
            RichText::new(&self.profile_io_status)
                .small()
                .color(if self.profile_dirty {
                    Color32::YELLOW
                } else {
                    Color32::GRAY
                }),
        );
    }

    pub(in super::super) fn draw_contest_panel(&mut self, ui: &mut egui::Ui) {
        ui.group(|ui| {
            ui.heading("🏁 Contest profile");
            if ui
                .checkbox(&mut self.contest_enabled, "Enable contest workflow profile")
                .changed()
            {
                self.config.contest.enabled = self.contest_enabled;
                self.profile_dirty = true;
                self.persist_profile("Auto-saved");
                self.emit_contest_profile_hooks();
            }

            ui.horizontal_wrapped(|ui| {
                ui.label("Operating mode");
                egui::ComboBox::from_id_salt("contest_operating_mode")
                    .selected_text(match self.contest_operating_mode {
                        ContestOperatingMode::Run => "Run",
                        ContestOperatingMode::SearchAndPounce => "Search & Pounce",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.contest_operating_mode,
                            ContestOperatingMode::Run,
                            "Run",
                        );
                        ui.selectable_value(
                            &mut self.contest_operating_mode,
                            ContestOperatingMode::SearchAndPounce,
                            "Search & Pounce",
                        );
                    });

                ui.label("Split policy");
                egui::ComboBox::from_id_salt("contest_split_policy")
                    .selected_text(match self.contest_split_policy {
                        SplitPolicy::Off => "Off",
                        SplitPolicy::Fake => "Fake split",
                        SplitPolicy::Rig => "Rig split",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.contest_split_policy,
                            SplitPolicy::Off,
                            "Off",
                        );
                        ui.selectable_value(
                            &mut self.contest_split_policy,
                            SplitPolicy::Fake,
                            "Fake split",
                        );
                        ui.selectable_value(
                            &mut self.contest_split_policy,
                            SplitPolicy::Rig,
                            "Rig split",
                        );
                    });
            });

            ui.horizontal_wrapped(|ui| {
                ui.label("Fox/Hound");
                egui::ComboBox::from_id_salt("contest_fox_hound")
                    .selected_text(match self.contest_fox_hound_role {
                        FoxHoundRole::Disabled => "Disabled",
                        FoxHoundRole::Fox => "Fox",
                        FoxHoundRole::Hound => "Hound",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            &mut self.contest_fox_hound_role,
                            FoxHoundRole::Disabled,
                            "Disabled",
                        );
                        ui.selectable_value(
                            &mut self.contest_fox_hound_role,
                            FoxHoundRole::Fox,
                            "Fox",
                        );
                        ui.selectable_value(
                            &mut self.contest_fox_hound_role,
                            FoxHoundRole::Hound,
                            "Hound",
                        );
                    });
            });

            ui.horizontal(|ui| {
                ui.label("Exchange template");
                ui.add(
                    egui::TextEdit::singleline(&mut self.contest_exchange_template)
                        .desired_width(260.0)
                        .hint_text("e.g. 5NN ${serial}"),
                );
            });

            ui.horizontal_wrapped(|ui| {
                ui.label("Serial start");
                ui.add(egui::DragValue::new(&mut self.contest_serial_start).range(1..=999_999));
                ui.label("Step");
                ui.add(egui::DragValue::new(&mut self.contest_serial_step).range(1..=100));
                ui.checkbox(&mut self.contest_dupe_check, "Dupe check");
            });

            ui.horizontal_wrapped(|ui| {
                ui.label("Serial current");
                ui.label(
                    RichText::new(format!("{:03}", self.contest_serial_current.max(1)))
                        .monospace()
                        .strong(),
                );
                if ui.small_button("Reset").clicked() {
                    self.contest_serial_current = self.contest_serial_start.max(1);
                    self.profile_dirty = true;
                    self.persist_profile("Auto-saved");
                }
                if ui.small_button("-Step").clicked() {
                    self.contest_serial_current = self
                        .contest_serial_current
                        .saturating_sub(self.contest_serial_step.max(1))
                        .max(self.contest_serial_start.max(1));
                    self.profile_dirty = true;
                    self.persist_profile("Auto-saved");
                }
                if ui.small_button("+Step").clicked() {
                    self.advance_contest_serial();
                    self.profile_dirty = true;
                    self.persist_profile("Auto-saved");
                }
            });

            ui.horizontal_wrapped(|ui| {
                ui.label("Fake split offset");
                ui.add(
                    egui::DragValue::new(&mut self.contest_fake_split_offset_hz)
                        .range(0..=2_000)
                        .suffix(" Hz"),
                );
                if ui.small_button("Use RX+offset").clicked() {
                    self.contest_split_policy = SplitPolicy::Fake;
                    self.profile_dirty = true;
                    self.persist_profile("Auto-saved");
                    self.emit_contest_profile_hooks();
                }
            });

            ui.label(
                RichText::new(self.contest_guidance_text())
                    .small()
                    .color(Color32::from_rgb(132, 228, 255)),
            );
            if self.contest_enabled && self.contest_split_policy == SplitPolicy::Fake {
                ui.label(
                    RichText::new(format!(
                        "Fake split active · TX offset {} Hz · software-only guardrail",
                        self.contest_fake_split_offset_hz
                    ))
                    .small()
                    .color(Color32::from_rgb(255, 201, 92)),
                );
            }

            self.contest_serial_start = self.contest_serial_start.max(1);
            self.contest_serial_step = self.contest_serial_step.max(1);
            self.contest_serial_current = self
                .contest_serial_current
                .max(self.contest_serial_start.max(1));

            if self.config.contest.enabled != self.contest_enabled
                || self.config.contest.operating_mode != self.contest_operating_mode
                || self.config.contest.split_policy != self.contest_split_policy
                || self.config.contest.fox_hound_role != self.contest_fox_hound_role
                || self
                    .config
                    .contest
                    .exchange_template
                    .as_deref()
                    .unwrap_or_default()
                    != self.contest_exchange_template.trim()
                || self.config.contest.serial_start != self.contest_serial_start
                || self.config.contest.serial_step != self.contest_serial_step
                || self.config.contest.dupe_check != self.contest_dupe_check
            {
                self.config.contest = ContestProfile {
                    enabled: self.contest_enabled,
                    operating_mode: self.contest_operating_mode,
                    split_policy: self.contest_split_policy,
                    fox_hound_role: self.contest_fox_hound_role,
                    exchange_template: if self.contest_exchange_template.trim().is_empty() {
                        None
                    } else {
                        Some(self.contest_exchange_template.trim().to_string())
                    },
                    serial_start: self.contest_serial_start,
                    serial_step: self.contest_serial_step,
                    dupe_check: self.contest_dupe_check,
                };
                self.profile_dirty = true;
                self.persist_profile("Auto-saved");
                self.emit_contest_profile_hooks();
            }

            ui.label(
                RichText::new("Automation hook targets: contest_state + operator_profile events")
                    .small()
                    .color(Color32::GRAY),
            );
        });
    }

    pub(in super::super) fn draw_reporting_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("PSK reporting");
        ui.separator();
        let changed = ui.checkbox(&mut self.psk_reporter_enabled, "📡 Report decoded stations to PSK Reporter").on_hover_text("Opt-in: batches reception reports to report.pskreporter.info over UDP about every five minutes").changed();
        if changed {
            self.restart_psk_reporter();
            self.profile_dirty = true;
            self.persist_profile("PSK Reporter preference saved to");
            self.emit_operator_profile_hook(format!(
                "psk_reporter_enabled={}",
                self.psk_reporter_enabled
            ));
        }
        if self.psk_reporter_enabled {
            if let Some(reporter) = &self.psk_reporter {
                let status = reporter.status();
                let detail = status
                    .last_error
                    .map(|error| format!("network error: {error}"))
                    .unwrap_or_else(|| {
                        format!(
                            "{} queued · {} sent · five-minute batching",
                            status.queued, status.sent
                        )
                    });
                ui.label(RichText::new(detail).small().color(Color32::LIGHT_GREEN));
            } else {
                ui.label(
                    RichText::new("Set a real callsign and grid before reporting")
                        .small()
                        .color(Color32::YELLOW),
                );
            }
        } else {
            ui.label(
                RichText::new("Private by default · no reception data leaves QSONaut")
                    .small()
                    .color(Color32::GRAY),
            );
        }

        ui.add_space(8.0);
        ui.label(RichText::new("Submission rules").strong());
        ui.label(
            RichText::new(
                "These follow PSK Reporter's IPFIX/UDP guidance. The service asks clients to \
                 batch reports and to avoid flooding it with repeats of the same station.",
            )
            .small()
            .color(Color32::GRAY),
        );
        let mut tuning_changed = false;
        ui.horizontal(|ui| {
            ui.label("Batch every");
            let previous = self.psk_batch_interval_secs;
            ui.add(
                egui::DragValue::new(&mut self.psk_batch_interval_secs)
                    .range(60..=3_600)
                    .suffix(" s"),
            )
            .on_hover_text(
                "How often queued reports are sent. The actual interval is randomized up to \
                 +30 s so bursts from many clients don't collide. WSJT-X uses 300 s.",
            );
            tuning_changed |= previous != self.psk_batch_interval_secs;
        });
        ui.horizontal(|ui| {
            ui.label("Re-report same call after");
            let previous = self.psk_repeat_cache_secs;
            ui.add(
                egui::DragValue::new(&mut self.psk_repeat_cache_secs)
                    .range(60..=3_600)
                    .suffix(" s"),
            )
            .on_hover_text(
                "Minimum time before the same callsign is reported again. PSK Reporter asks \
                 clients to avoid repeating a station too often. WSJT-X uses 300 s.",
            );
            tuning_changed |= previous != self.psk_repeat_cache_secs;
        });
        ui.horizontal(|ui| {
            ui.label("Max pending");
            let previous = self.psk_max_pending;
            ui.add(
                egui::DragValue::new(&mut self.psk_max_pending)
                    .range(1..=2_048)
                    .suffix(" spots"),
            )
            .on_hover_text(
                "Largest number of reports held before a batch is forced out early. WSJT-X \
                 uses 2048; QSONaut's default is 80.",
            );
            tuning_changed |= previous != self.psk_max_pending;
        });
        if tuning_changed {
            self.restart_psk_reporter();
            self.profile_dirty = true;
            self.persist_profile("PSK Reporter tuning saved to");
        }
    }

    pub(in super::super) fn draw_settings_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Settings");
        ui.separator();
        let previous_scale = self.gui_scale;
        egui::ComboBox::from_id_salt("gui_scale")
            .selected_text(format!("UI {:.0}%", gui_scale_percent(self.gui_scale)))
            .width(90.0)
            .show_ui(ui, |ui| {
                for percent in [75_u32, 85, 100, 110, 125] {
                    let scale = gui_scale_from_percent(percent);
                    ui.selectable_value(&mut self.gui_scale, scale, format!("{percent}%"));
                }
            });
        if (previous_scale - self.gui_scale).abs() > 0.001 {
            self.profile_dirty = true;
            self.persist_profile("Auto-saved");
        }
        ui.add_space(8.0);
        ui.label(RichText::new("Compute").strong());
        let previous_compute = self.compute_preference;
        egui::ComboBox::from_id_salt("compute_preference")
            .selected_text(self.compute_preference.label())
            .show_ui(ui, |ui| {
                for preference in ComputePreference::ALL {
                    ui.selectable_value(
                        &mut self.compute_preference,
                        preference,
                        preference.label(),
                    );
                }
            });
        if self.compute_preference != previous_compute {
            self.refresh_acceleration_report();
            self.profile_dirty = true;
            self.persist_profile("Compute policy saved to");
        }
        ui.label(
            RichText::new(format!(
                "{} · {}",
                self.acceleration_report.summary(),
                self.acceleration_report.hardware_detail()
            ))
            .small()
            .color(Color32::GRAY),
        );
        ui.add_space(8.0);
        ui.label(RichText::new("Digital TX timing").strong());
        ui.label(
            RichText::new("Profiles apply only to supported controls and are safe to reapply.")
                .small()
                .color(Color32::GRAY),
        );
        let profile_names = self
            .radio_profiles
            .iter()
            .map(|profile| profile.name.clone())
            .collect::<Vec<_>>();
        for mode in ["FT8", "FT4", "CW", "Other"] {
            ui.horizontal(|ui| {
                ui.label(format!("{mode} default"));
                let selected = self
                    .mode_radio_profile
                    .get(mode)
                    .cloned()
                    .unwrap_or_else(|| "None".to_string());
                egui::ComboBox::from_id_salt(format!("radio_profile_default_{mode}"))
                    .selected_text(selected)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(
                            self.mode_radio_profile.entry(mode.to_string()).or_default(),
                            String::new(),
                            "None",
                        );
                        for name in &profile_names {
                            ui.selectable_value(
                                self.mode_radio_profile.entry(mode.to_string()).or_default(),
                                name.clone(),
                                name,
                            );
                        }
                    });
            });
        }
        if ui.button("Save current radio state as profile").clicked() {
            self.radio_profiles.push(RadioProfile {
                name: format!("Profile {}", self.radio_profiles.len() + 1),
                mode: Some(self.workspace_mode.label().to_string()),
                data_mode: None,
                filter: None,
                af_gain: None,
                rf_gain: None,
                rf_power: None,
                preamp: None,
                attenuator: None,
                noise_blank: None,
                noise_reduction: None,
                agc: None,
            });
            self.profile_dirty = true;
            self.persist_profile("Radio profile saved to");
        }
        ui.label(
            RichText::new("Applied before and after FT8/FT4 audio transmission.")
                .small()
                .color(Color32::GRAY),
        );
        let previous_ptt_lead = self.ptt_lead_ms;
        ui.horizontal(|ui| {
            ui.label("PTT lead");
            ui.add(
                egui::DragValue::new(&mut self.ptt_lead_ms)
                    .range(0..=500)
                    .suffix(" ms"),
            );
        });
        let previous_ptt_tail = self.ptt_tail_ms;
        ui.horizontal(|ui| {
            ui.label("PTT tail");
            ui.add(
                egui::DragValue::new(&mut self.ptt_tail_ms)
                    .range(0..=500)
                    .suffix(" ms"),
            );
        });
        if self.ptt_lead_ms != previous_ptt_lead || self.ptt_tail_ms != previous_ptt_tail {
            self.profile_dirty = true;
            self.persist_profile("PTT timing saved to");
        }
        ui.add_space(8.0);
        self.draw_device_settings(ui);
    }

    pub(in super::super) fn draw_radio_tuning_panel(
        &mut self,
        ui: &mut egui::Ui,
        snapshot: &GuiState,
    ) {
        ui.heading("Radio Tuning Profiles");
        ui.separator();
        ui.label(
            RichText::new("Save reusable IC-7300 settings, edit every stored option, apply them to the radio, and assign defaults per QSONaut mode.")
                .small()
                .color(Color32::GRAY),
        );

        let mut selected_name = self
            .radio_profiles
            .first()
            .map(|profile| profile.name.clone())
            .unwrap_or_default();
        if self.radio_profile_name_input.is_empty() {
            self.radio_profile_name_input = selected_name.clone();
        }
        ui.horizontal(|ui| {
            ui.label("Profile");
            egui::ComboBox::from_id_salt("radio_tuning_profile")
                .selected_text(if selected_name.is_empty() {
                    "New profile"
                } else {
                    &selected_name
                })
                .show_ui(ui, |ui| {
                    for profile in &self.radio_profiles {
                        ui.selectable_value(
                            &mut selected_name,
                            profile.name.clone(),
                            &profile.name,
                        );
                    }
                });
            if ui.button("Apply").clicked() {
                if let Some(profile) = self
                    .radio_profiles
                    .iter()
                    .find(|profile| profile.name == selected_name)
                {
                    self.apply_radio_profile(profile.clone());
                }
            }
            if ui.button("Delete").clicked() && !selected_name.is_empty() {
                self.radio_profiles
                    .retain(|profile| profile.name != selected_name);
                self.mode_radio_profile
                    .retain(|_, name| name != &selected_name);
                self.profile_dirty = true;
                self.persist_profile("Radio profile deleted from");
            }
        });

        ui.horizontal(|ui| {
            ui.label("Name");
            ui.text_edit_singleline(&mut self.radio_profile_name_input);
            if ui.button("Save current as").clicked()
                && !self.radio_profile_name_input.trim().is_empty()
            {
                let profile =
                    self.read_radio_profile(self.radio_profile_name_input.trim(), snapshot);
                self.radio_profiles
                    .retain(|existing| existing.name != profile.name);
                self.radio_profiles.push(profile);
                selected_name = self.radio_profile_name_input.trim().to_string();
                self.radio_profile_name_input.clear();
                self.profile_dirty = true;
                self.persist_profile("Radio profile saved to");
            }
        });

        let profile = self
            .radio_profiles
            .iter_mut()
            .find(|profile| profile.name == selected_name);
        if let Some(profile) = profile {
            ui.separator();
            ui.label(RichText::new("Stored options").strong());
            ui.horizontal_wrapped(|ui| {
                ui.label("Mode");
                ui.text_edit_singleline(profile.mode.get_or_insert_with(String::new));
                ui.checkbox(profile.data_mode.get_or_insert(false), "Data mode");
                ui.label("Filter");
                edit_optional_u8(ui, &mut profile.filter, 1, 3);
            });
            ui.horizontal_wrapped(|ui| {
                for (label, value) in [
                    ("AF", &mut profile.af_gain),
                    ("RF", &mut profile.rf_gain),
                    ("Power", &mut profile.rf_power),
                    ("AGC", &mut profile.agc),
                ] {
                    ui.label(label);
                    edit_optional_u8(ui, value, 0, 255);
                }
            });
            ui.horizontal_wrapped(|ui| {
                ui.checkbox(profile.preamp.get_or_insert(false), "Preamp");
                ui.checkbox(profile.attenuator.get_or_insert(false), "Attenuator");
                ui.checkbox(profile.noise_blank.get_or_insert(false), "Noise blanker");
                ui.checkbox(
                    profile.noise_reduction.get_or_insert(false),
                    "Noise reduction",
                );
            });
            if ui.button("Save edits").clicked() {
                self.profile_dirty = true;
                self.persist_profile("Radio profile edits saved to");
            }
        }

        ui.separator();
        ui.label(RichText::new("Mode defaults").strong());
        for mode in ["FT8", "FT4", "CW", "Other"] {
            ui.horizontal(|ui| {
                ui.label(mode);
                let value = self.mode_radio_profile.entry(mode.to_string()).or_default();
                egui::ComboBox::from_id_salt(format!("radio_tuning_default_{mode}"))
                    .selected_text(if value.is_empty() {
                        "None"
                    } else {
                        value.as_str()
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(value, String::new(), "None");
                        for profile in &self.radio_profiles {
                            ui.selectable_value(value, profile.name.clone(), &profile.name);
                        }
                    });
            });
        }
        if self.radio_profiles.is_empty() {
            ui.label(
                RichText::new("No profiles yet. Enter a name and save the current radio state.")
                    .color(Color32::GRAY),
            );
        }
    }

    pub(in super::super) fn draw_waterfall_panel(
        &mut self,
        ui: &mut egui::Ui,
        snapshot: &GuiState,
    ) {
        ui.heading("Waterfall");
        ui.separator();
        let supports_radio_scope = find_model(&self.config.radio.model)
            .is_some_and(|profile| profile.capabilities.spectrum);
        if ui
            .add_enabled(
                supports_radio_scope,
                egui::Checkbox::new(&mut self.civ_spectrum_on, "Show radio scope waterfall"),
            )
            .changed()
        {
            self.profile_dirty = true;
            self.persist_profile("Auto-saved");
        }
        ui.label(
            RichText::new(&snapshot.radio_waterfall_status)
                .small()
                .color(if snapshot.radio_spectrum_enabled {
                    Color32::LIGHT_GREEN
                } else if self.civ_spectrum_on {
                    Color32::YELLOW
                } else {
                    Color32::GRAY
                }),
        );

        ui.add_space(6.0);
        ui.label(RichText::new("Shared display").strong());
        ui.label(
            RichText::new("Color theme and sweep speed apply to both radio and audio waterfalls.")
                .small()
                .color(Color32::GRAY),
        );
        let previous_theme = self.waterfall_theme;
        egui::ComboBox::from_id_salt("waterfall_theme_settings")
            .selected_text(self.waterfall_theme.label())
            .show_ui(ui, |ui| {
                for theme in [
                    WaterfallTheme::RadioBlue,
                    WaterfallTheme::Inferno,
                    WaterfallTheme::Phosphor,
                    WaterfallTheme::Monochrome,
                ] {
                    ui.selectable_value(&mut self.waterfall_theme, theme, theme.label());
                }
            });
        if self.waterfall_theme != previous_theme {
            self.profile_dirty = true;
            self.persist_profile("Auto-saved");
        }

        ui.add_space(6.0);
        ui.label(RichText::new("Sweep speed").strong());
        ui.horizontal_wrapped(|ui| {
            let mut tuning = self.display_tuning.lock().expect("tuning lock poisoned");
            if ui.selectable_label(tuning.auto_visual, "Auto").clicked() {
                tuning.auto_visual = true;
            }
            for (speed, label) in [
                (WaterfallSpeed::Fast, "Fast"),
                (WaterfallSpeed::Mid, "Mid"),
                (WaterfallSpeed::Slow, "Slow"),
            ] {
                if ui
                    .selectable_label(
                        !tuning.auto_visual && tuning.waterfall_speed == speed,
                        label,
                    )
                    .clicked()
                {
                    tuning.auto_visual = false;
                    tuning.waterfall_speed = speed;
                }
            }
        });

        ui.add_space(6.0);
        ui.label(RichText::new("Radio scope only").strong());
        ui.horizontal_wrapped(|ui| {
            ui.selectable_value(
                &mut self.radio_scope_view,
                RadioScopeView::Narrow,
                "Narrow passband",
            );
            ui.selectable_value(
                &mut self.radio_scope_view,
                RadioScopeView::Overview,
                "Active band overview",
            );
        });
        ui.add(
            egui::Slider::new(&mut self.radio_scope_contrast, 0.7..=3.0)
                .text("Intensity")
                .clamping(egui::SliderClamping::Always),
        );
        ui.checkbox(
            &mut self.radio_scope_lock_if_to_filter,
            "Match span to selected FIL",
        );
        ui.checkbox(&mut self.radio_scope_vbw_wide, "Wide video bandwidth");
        if self.radio_scope_lock_if_to_filter {
            self.radio_scope_span_code = scope_span_for_filter(&snapshot.mode, snapshot.filter);
            ui.small(format!(
                "Automatic span: {}",
                scope_span_label(self.radio_scope_span_code)
            ));
        } else {
            egui::ComboBox::from_id_salt("radio_scope_span_settings")
                .selected_text(scope_span_label(self.radio_scope_span_code))
                .show_ui(ui, |ui| {
                    for (code, label) in [
                        (0_u8, "±2.5 kHz"),
                        (1_u8, "±5 kHz"),
                        (2_u8, "±10 kHz"),
                        (3_u8, "±25 kHz"),
                        (4_u8, "±50 kHz"),
                        (5_u8, "±100 kHz"),
                        (6_u8, "±250 kHz"),
                        (7_u8, "±500 kHz"),
                    ] {
                        ui.selectable_value(&mut self.radio_scope_span_code, code, label);
                    }
                });
        }
        ui.checkbox(&mut self.radio_scope_hold, "Hold radio scope");
        ui.add(
            egui::Slider::new(&mut self.radio_scope_reference_tenths_db, -200..=200)
                .step_by(5.0)
                .custom_formatter(|value, _| format!("{:.1} dB", value / 10.0))
                .text("Reference"),
        );
        ui.add_space(6.0);
        ui.label(RichText::new("Audio waterfall only").strong());
        ui.label(
            RichText::new(
                "Bandwidth follows the selected radio filter. Left-click sets RX; right-click sets TX.",
            )
            .small()
            .color(Color32::GRAY),
        );
    }

    pub(in super::super) fn draw_server_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("🌐 QSONaut Server");
        ui.separator();
        ui.label(RichText::new("Use http://localhost:8080 for local development, a LAN address when the server is on another machine, or the hosted HTTPS address. QSONaut selects WS/WSS automatically; reverse proxies require no specialty port.").small().color(Color32::GRAY));
        let server_settings_before = self.config.server.clone();
        ui.checkbox(
            &mut self.config.server.enabled,
            "Connect this QSONaut instance",
        );
        ui.horizontal(|ui| {
            ui.label("Endpoint");
            ui.add(
                egui::TextEdit::singleline(&mut self.config.server.url)
                    .desired_width(ui.available_width())
                    .hint_text("http://localhost:8080 or https://qsonaut.example.org"),
            );
        });
        ui.horizontal(|ui| {
            ui.label("Device token");
            ui.add(
                egui::TextEdit::singleline(&mut self.config.server.device_token)
                    .password(true)
                    .desired_width(ui.available_width())
                    .hint_text("Paste the token issued by QSONaut Server"),
            );
        });
        ui.label(RichText::new("The token is stored locally in profile.toml with owner-only permissions on Unix systems.").small().color(Color32::GRAY));
        ui.add_space(5.0);
        ui.label(RichText::new("Privacy controls").strong());
        ui.checkbox(
            &mut self.config.server.share_presence,
            "Share online presence and operating mode",
        );
        ui.add_enabled_ui(self.config.server.share_presence, |ui| {
            ui.checkbox(
                &mut self.config.server.share_radio_details,
                "Share radio, frequency, and operating metadata",
            );
        });
        ui.checkbox(
            &mut self.config.server.share_logs,
            "Share contact/QSO logs with the server",
        );
        ui.checkbox(
            &mut self.config.server.share_diagnostics,
            "Allow manual radio/debug snapshots",
        );
        if self.config.server != server_settings_before {
            self.profile_dirty = true;
        }
        ui.horizontal_wrapped(|ui| {
            if ui.button("Save & reconnect").clicked() {
                self.reconnect_server();
            }
            if ui.button("Disconnect").clicked() {
                self.config.server.enabled = false;
                self.reconnect_server();
            }
            if ui.add_enabled(self.config.server.share_diagnostics, egui::Button::new("Send diagnostic snapshot now")).on_hover_text("Sends radio configuration, live state, audio/decoder health, and the latest error; never sends the server token or audio samples").clicked() {
                self.publish_diagnostic_snapshot();
            }
            ui.label(RichText::new("Nothing is shared unless its control is enabled.").small().color(Color32::GRAY));
        });
        if self.profile_io_status.contains("Diagnostic snapshot")
            || self.profile_io_status.contains("QSONaut Server rejected")
        {
            let color = if self.profile_io_status.contains("accepted") {
                Color32::LIGHT_GREEN
            } else if self.profile_io_status.contains("rejected")
                || self.profile_io_status.contains("could not")
            {
                Color32::YELLOW
            } else {
                Color32::LIGHT_BLUE
            };
            ui.label(
                RichText::new(&self.profile_io_status)
                    .small()
                    .strong()
                    .color(color),
            );
        }

        ui.add_space(8.0);
        ui.group(|ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("🌐 QSONaut Server").strong());
                if let Some(client) = &self.server_client {
                    let status = client.status();
                    let (label, color) = match status.state {
                        ServerConnectionState::Connected => ("CONNECTED", Color32::LIGHT_GREEN),
                        ServerConnectionState::Connecting => ("CONNECTING", Color32::YELLOW),
                        ServerConnectionState::Reconnecting => ("RECONNECTING", Color32::YELLOW),
                        ServerConnectionState::Disabled | ServerConnectionState::Stopped => {
                            ("OFFLINE", Color32::GRAY)
                        }
                    };
                    ui.label(RichText::new(label).monospace().strong().color(color));
                    if ui.small_button("Refresh events").clicked() {
                        client.request_sync();
                    }
                    ui.label(
                        RichText::new(format!(
                            "{} active · {} contest models",
                            status.active_event_count, status.catalog_size
                        ))
                        .small()
                        .color(Color32::GRAY),
                    );
                    if let Some(error) = status.last_error {
                        ui.label(RichText::new(error).small().color(Color32::YELLOW));
                    }
                } else {
                    ui.label(RichText::new("DISABLED").monospace().color(Color32::GRAY));
                }
            });
            ui.label(
                RichText::new(format!(
                    "Presence: {} · radio details: {} · QSO logs: {} · diagnostics: {}",
                    if self.config.server.share_presence {
                        "shared"
                    } else {
                        "private"
                    },
                    if self.config.server.share_presence && self.config.server.share_radio_details {
                        "shared"
                    } else {
                        "private"
                    },
                    if self.config.server.share_logs {
                        "shared"
                    } else {
                        "private"
                    },
                    if self.config.server.share_diagnostics {
                        "manual"
                    } else {
                        "private"
                    },
                ))
                .small()
                .color(Color32::GRAY),
            );
        });

        ui.add_space(8.0);
        ui.group(|ui| {
            ui.heading("💬 Automation ingress test bench");
            ui.label(RichText::new("Publish an external_message event locally to validate rules before wiring live adapters.").small().color(Color32::GRAY));
            ui.horizontal_wrapped(|ui| {
                ui.label("Source");
                ui.add(egui::TextEdit::singleline(&mut self.external_ingress_source).desired_width(130.0).hint_text("discord:shack"));
                ui.label("Author");
                ui.add(egui::TextEdit::singleline(&mut self.external_ingress_author).desired_width(120.0).hint_text("K1ABC"));
                ui.label("Channel");
                ui.add(egui::TextEdit::singleline(&mut self.external_ingress_channel).desired_width(120.0).hint_text("#qsonaut"));
            });
            ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.external_ingress_message)
                            .desired_width((ui.available_width() - 150.0).max(120.0))
                            .hint_text("!rig"),
                    );
                    if ui.button("Inject event").clicked() {
                        self.publish_external_ingress_message();
                    }
                });

                ui.add_space(6.0);
                let transport_summary = if self.automation_external_transports.is_empty() {
                    "none".to_string()
                } else {
                    let mut transports: Vec<_> =
                        self.automation_external_transports.iter().cloned().collect();
                    transports.sort();
                    transports.join(", ")
                };
                ui.label(
                    RichText::new(format!(
                        "Configured external transports: {transport_summary}"
                    ))
                    .small()
                    .color(Color32::GRAY),
                );
                if let Some(last) = self.automation_external_outbox.back() {
                    ui.label(
                        RichText::new(format!(
                            "Last queued send {} · {} → {} · {}",
                            last.utc, last.source, last.target, last.message
                        ))
                        .small()
                        .color(Color32::from_rgb(158, 217, 255)),
                    );
                    ui.label(
                        RichText::new(format!(
                            "Outbox depth: {} (adapter polling still pending)",
                            self.automation_external_outbox.len()
                        ))
                        .small()
                        .color(Color32::GRAY),
                    );
                } else {
                    ui.label(
                        RichText::new("Outbox is empty")
                            .small()
                            .color(Color32::GRAY),
                    );
                }
            });

        ui.add_space(4.0);
        ui.label(
            RichText::new(&self.automation_status)
                .small()
                .color(Color32::from_rgb(158, 217, 255)),
        );
        ui.add_space(2.0);
        ui.label(
            RichText::new(&self.profile_io_status)
                .small()
                .color(Color32::GRAY),
        );
    }
}
