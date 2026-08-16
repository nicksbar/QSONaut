use super::super::*;

impl QsonautGuiApp {
    pub(in super::super) fn draw_profile_or_server_panel(&mut self, ui: &mut egui::Ui) {
        if self.signal_panel_tab != SignalPanelTab::Server {
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
                    RichText::new(
                        "Automation hook targets: contest_state + operator_profile events",
                    )
                    .small()
                    .color(Color32::GRAY),
                );
            });

            ui.add_space(6.0);
            let changed = ui
            .checkbox(
                &mut self.psk_reporter_enabled,
                "📡 Report decoded stations to PSK Reporter",
            )
            .on_hover_text(
                "Opt-in: batches reception reports to report.pskreporter.info over UDP about every five minutes",
            )
            .changed();
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

            ui.add_space(4.0);
            ui.label(
                RichText::new(&self.profile_io_status)
                    .small()
                    .color(Color32::GRAY),
            );
            return;
        }

        ui.heading("🌐 QSONaut Server");
        ui.separator();
        ui.label(
            RichText::new("Use http://localhost:8080 for local development, a LAN address when the server is on another machine, or the hosted HTTPS address. QSONaut selects WS/WSS automatically; reverse proxies require no specialty port.")
                .small()
                .color(Color32::GRAY),
        );
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
        ui.label(
            RichText::new("The token is stored locally in profile.toml with owner-only permissions on Unix systems.")
                .small()
                .color(Color32::GRAY),
        );
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
            if ui
                .add_enabled(
                    self.config.server.share_diagnostics,
                    egui::Button::new("Send diagnostic snapshot now"),
                )
                .on_hover_text("Sends radio configuration, live state, audio/decoder health, and the latest error; never sends the server token or audio samples")
                .clicked()
            {
                self.publish_diagnostic_snapshot();
            }
            ui.label(
                RichText::new("Nothing is shared unless its control is enabled.")
                    .small()
                    .color(Color32::GRAY),
            );
        });

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
                ui.label(
                    RichText::new(
                        "Publish an external_message event locally to validate rules before wiring live adapters.",
                    )
                    .small()
                    .color(Color32::GRAY),
                );
                ui.horizontal_wrapped(|ui| {
                    ui.label("Source");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.external_ingress_source)
                            .desired_width(130.0)
                            .hint_text("discord:shack"),
                    );
                    ui.label("Author");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.external_ingress_author)
                            .desired_width(120.0)
                            .hint_text("K1ABC"),
                    );
                    ui.label("Channel");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.external_ingress_channel)
                            .desired_width(120.0)
                            .hint_text("#qsonaut"),
                    );
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
