use super::super::*;

fn migrate_hostbridge_radio_id(config: &mut RadioConfig, hello: &HostHello) -> bool {
    let Some(saved_id) = config.hostbridge_radio_id.clone() else {
        return false;
    };
    if hello
        .capabilities
        .radio_devices
        .iter()
        .any(|device| device.id == saved_id)
    {
        return false;
    }
    let Some((physical_id, _legacy_driver)) = saved_id.rsplit_once(':') else {
        return false;
    };
    if hello
        .capabilities
        .radio_devices
        .iter()
        .any(|device| device.id == physical_id)
    {
        info!(old_id = %saved_id, new_id = %physical_id, "Migrated legacy HostBridge radio selection to physical device ID");
        config.hostbridge_radio_id = Some(physical_id.to_string());
        true
    } else {
        false
    }
}

impl QsonautGuiApp {
    pub(crate) fn update_impl(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.pump_hamdb_lookup();
        // The HostBridge catalog is independent of exclusive radio leasing.
        // Load it at startup so remote audio devices remain selectable even
        // when a radio session is temporarily busy or offline.
        if self.config.radio.backend.eq_ignore_ascii_case("hostbridge")
            && self.hostbridge_catalog.is_none()
            && self.hostbridge_scan.is_none()
            && self.hostbridge_scan_status.is_empty()
        {
            self.enumerate_hostbridge();
        }
        let mut cat_test_finished = false;
        if let Some(rx) = &self.cat_test_rx {
            match rx.try_recv() {
                Ok(result) => {
                    self.cat_test_status = Some(result);
                    self.cat_test_rx = None;
                    cat_test_finished = true;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.cat_test_status = Some(Err("CAT test worker stopped unexpectedly".into()));
                    self.cat_test_rx = None;
                    cat_test_finished = true;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        // The CAT test paused the radio worker to release the exclusive
        // serial port; restore it now that the probe has finished.
        if cat_test_finished && self.cat_test_restart_radio {
            self.cat_test_restart_radio = false;
            self.reconnect_radio();
        }
        if let Some(geometry) = WindowGeometry::read(ctx, self.window_geometry) {
            self.window_geometry = Some(geometry);
        }
        let viewport_state = ctx.input(|input| {
            let viewport = input.viewport();
            format!(
                "outer={:?}; inner={:?}; maximized={:?}; minimized={:?}; focused={:?}; close_requested={:?}",
                viewport.outer_rect,
                viewport.inner_rect,
                viewport.maximized,
                viewport.minimized,
                viewport.focused,
                viewport.close_requested(),
            )
        });
        if self.last_viewport_log.as_deref() != Some(viewport_state.as_str()) {
            info!(state = %viewport_state, "Native viewport state changed");
            self.last_viewport_log = Some(viewport_state);
        }
        if !self.first_frame_logged {
            self.first_frame_logged = true;
            info!(
                renderer = %self.selected_renderer,
                os = std::env::consts::OS,
                os_dpi_adjustment = self.os_dpi_adjustment,
                zoom_factor = ctx.zoom_factor(),
                pixels_per_point = ctx.pixels_per_point(),
                effective_pixels_per_point = ctx.pixels_per_point(),
                "QSONaut first GUI frame reached"
            );
        }
        if !self.local_image_refresh_started {
            self.local_image_refresh_started = true;
            self.refresh_local_image_models();
        }
        // Zoom is layered on top of the OS DPI scale, so text, controls,
        // spacing, hit targets, and custom drawings stay in proportion.
        let target_zoom = self.gui_scale * self.os_dpi_adjustment;
        if (ctx.zoom_factor() - target_zoom).abs() > 0.001 {
            ctx.set_zoom_factor(target_zoom);
        }
        // Give background workers a handle so they can trigger repaints directly.
        let _ = self.repaint_ctx.get_or_init(|| ctx.clone());
        // Safety-net repaint in case no worker data arrives for a long time.
        ctx.request_repaint_after(Duration::from_secs(1));

        if let Some(rx) = &self.acceleration_probe {
            match rx.try_recv() {
                Ok(report) => {
                    self.acceleration_report = report;
                    self.acceleration_probe = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => self.acceleration_probe = None,
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }

        if let Some(rx) = &self.device_scan {
            match rx.try_recv() {
                Ok(inventory) => {
                    self.device_scan = None;
                    self.apply_device_inventory(inventory);
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    warn!("Device inventory scan worker disconnected before delivering results");
                    self.device_scan = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }

        if let Some(rx) = &self.hostbridge_scan {
            match rx.try_recv() {
                Ok(Ok(hello)) => {
                    if self.config.radio.backend.eq_ignore_ascii_case("hostbridge") {
                        self.profile_dirty |=
                            migrate_hostbridge_radio_id(&mut self.config.radio, &hello);
                    }
                    self.hostbridge_catalog = Some(hello);
                    self.hostbridge_scan = None;
                    self.hostbridge_scan_status = "Connected · options loaded".to_string();
                }
                Ok(Err(error)) => {
                    self.hostbridge_scan = None;
                    self.hostbridge_scan_status = error;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.hostbridge_scan = None;
                    self.hostbridge_scan_status = "Enumeration stopped unexpectedly".to_string();
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }

        // Keep inactive tabs receiving/decoding; only their PTT path is gated.
        self.pump_parked_radio_sessions();

        let active_audio_finished = self
            .audio_worker_handle
            .as_ref()
            .is_some_and(std::thread::JoinHandle::is_finished);
        if active_audio_finished {
            if let Some(handle) = self.audio_worker_handle.take() {
                let _ = handle.join();
            }
            warn!(profile = %self.selected_profile_name, "Active profile audio worker stopped");
            if let Ok(mut state) = self.state.lock() {
                state.audio_spectrum_status = "STOPPED (audio worker failed)".to_string();
            }
        }
        let active_radio_finished = self
            .radio_worker_handle
            .as_ref()
            .is_some_and(std::thread::JoinHandle::is_finished);
        if active_radio_finished {
            if let Some(handle) = self.radio_worker_handle.take() {
                let _ = handle.join();
            }
            self.command_tx = None;
            warn!(profile = %self.selected_profile_name, "Active profile radio worker stopped");
            if let Ok(mut state) = self.state.lock() {
                state.radio_waterfall_status = "STOPPED (radio worker failed)".to_string();
            }
        }

        // Poll for the selected radio initialization result from background thread
        if !self.radio_init_attempted {
            if let Some(rx) = &self.radio_init_rx {
                match rx.try_recv() {
                    Ok(Some(radio)) => {
                        // Radio initialization succeeded; start the worker
                        self.radio_init_attempted = true;
                        if let Some(catalog) = radio.hostbridge_catalog() {
                            if self.config.radio.backend.eq_ignore_ascii_case("hostbridge") {
                                self.profile_dirty |=
                                    migrate_hostbridge_radio_id(&mut self.config.radio, &catalog);
                            }
                            self.hostbridge_catalog = Some(catalog);
                            self.hostbridge_scan_status = "Connected · options loaded".to_string();
                        }
                        let (tx, rx) = mpsc::channel::<GuiCommand>();
                        let display_port = self
                            .config
                            .radio
                            .serial_port
                            .clone()
                            .unwrap_or_else(|| "auto".to_string());
                        info!(
                            backend = %self.config.radio.backend,
                            model = %self.config.radio.model,
                            endpoint = %self.config.radio.endpoint,
                            port = %display_port,
                            baud = self.config.radio.baud_rate,
                            "Starting GUI radio worker (deferred initialization)"
                        );
                        let handle = workers::radio::spawn_radio_worker(
                            radio,
                            self.state.clone(),
                            self.radio_worker_stop.clone(),
                            self.swr_sweep_abort.clone(),
                            self.display_tuning.clone(),
                            rx,
                            self.repaint_ctx.clone(),
                            self.ptt_allowed.clone(),
                        );
                        self.command_tx = Some(tx);
                        self.radio_worker_handle = Some(handle);
                        // Settings changed while the hardware was unavailable
                        // remain dirty in memory. Persist them only now that
                        // the configured radio worker has actually started.
                        if self.profile_dirty {
                            self.persist_profile("Saved after radio worker started");
                        }
                    }
                    Ok(None) => {
                        // Radio initialization failed
                        self.radio_init_attempted = true;
                        self.radio_init_rx = None;
                        // Radio initialization failure is isolated to this
                        // profile. Its audio worker remains independent.
                        {
                            let mut s = self.state.lock().expect("ui state lock poisoned");
                            s.radio_waterfall_status =
                                "UNAVAILABLE (connection failed)".to_string();
                            s.last_error = Some(format!(
                                "Failed to initialize radio backend '{}' (model '{}', endpoint '{}', serial port '{}')",
                                self.config.radio.backend,
                                self.config.radio.model,
                                self.config.radio.endpoint,
                                self.config.radio.serial_port.as_deref().unwrap_or("auto"),
                            ));
                        }
                        warn!(profile = %self.selected_profile_name, "Radio initialization failed; profile runtime stopped");
                    }
                    Err(mpsc::TryRecvError::Disconnected) => {
                        // Thread panicked or dropped
                        self.radio_init_attempted = true;
                        self.radio_init_rx = None;
                        // Radio initialization failure is isolated to this
                        // profile. Its audio worker remains independent.
                        {
                            let mut s = self.state.lock().expect("ui state lock poisoned");
                            s.radio_waterfall_status =
                                "UNAVAILABLE (init thread crashed)".to_string();
                        }
                        warn!(profile = %self.selected_profile_name, "Radio initialization thread failed; profile runtime stopped");
                    }
                    Err(mpsc::TryRecvError::Empty) => {
                        // Still initializing...
                    }
                }
            }
        }

        // Drain FT8 decodes from the shared pending queue into app-local log.
        let (new_decodes, latest_decode_period) = {
            let mut s = self.state.lock().expect("ui state lock poisoned");
            s.workspace_mode = self.workspace_mode;
            s.fst4_submode = self.fst4_submode;
            s.ft8_deep_decode = self.ft8_deep_decode;
            s.ft4_deep_decode = self.ft4_deep_decode;
            s.selected_audio_hz = if self.workspace_mode == WorkspaceMode::Cw {
                u32::from(self.cw_tone_hz)
            } else {
                self.rx_tone_hz
            };
            s.sstv_tuning_offset_hz = self.sstv_tuning_offset_hz;
            s.sstv_auto_target = self.sstv_auto_target;
            s.cw_wpm = self.cw_wpm;
            s.compute_backend = self.acceleration_report.active;
            s.radio_spectrum_desired = self.civ_spectrum_on;
            s.radio_scope_contrast = self.radio_scope_contrast;
            s.radio_scope_span_code = self.radio_scope_span_code;
            s.radio_scope_vbw_wide = self.radio_scope_vbw_wide;
            s.radio_scope_hold = self.radio_scope_hold;
            s.radio_scope_reference_tenths_db = self.radio_scope_reference_tenths_db;
            s.radio_scope_view = self.radio_scope_view;
            (std::mem::take(&mut s.ft8_pending), s.ft8_last_decode_period)
        };
        let completed_decode_period =
            latest_decode_period.filter(|period| self.ft8_seen_decode_period != Some(*period));
        if completed_decode_period.is_some() {
            self.ft8_seen_decode_period = completed_decode_period;
        }
        self.process_ft8_tx_pipeline();
        self.process_native_digital_tx_pipeline();
        self.pump_server_automation_events();
        self.pump_automation_events();
        self.pump_hamdb_profile_lookup();
        self.pump_pota_spots();
        let (ft4_decodes, latest_ft4_period) = {
            let shared = self.state.lock().expect("ui state lock poisoned");
            (
                shared
                    .digital_decodes
                    .iter()
                    .filter(|entry| entry.mode == WorkspaceMode::Ft4)
                    .cloned()
                    .collect::<Vec<_>>(),
                shared.ft4_last_decode_period,
            )
        };
        let completed_ft4_period =
            latest_ft4_period.filter(|period| self.ft4_seen_decode_period != Some(*period));
        if completed_ft4_period.is_some() {
            self.ft4_seen_decode_period = completed_ft4_period;
        }
        self.handle_ft4_decodes(&ft4_decodes, completed_ft4_period);
        for mode in [
            WorkspaceMode::Fst4,
            WorkspaceMode::Jt9,
            WorkspaceMode::Jt65,
            WorkspaceMode::Q65,
            WorkspaceMode::Msk144,
        ] {
            let native_decodes = {
                let shared = self.state.lock().expect("ui state lock poisoned");
                shared
                    .digital_decodes
                    .iter()
                    .filter(|entry| entry.mode == mode)
                    .cloned()
                    .collect::<Vec<_>>()
            };
            self.handle_native_sequence(mode, &native_decodes, None);
        }
        let max_entries = self.ft8_max_log_entries.max(80);
        self.handle_ft8_decodes(&new_decodes, completed_decode_period);
        let removed = append_ft8_log_entries(&mut self.ft8_log, &new_decodes, max_entries);
        if removed > 0 {
            self.ft8_log.drain(..removed);
            if let Some(sel) = self.ft8_selected {
                self.ft8_selected = sel.checked_sub(removed);
            }
        }
        if !new_decodes.is_empty() {
            info!(
                received = new_decodes.len(),
                visible_log_entries = self.ft8_log.len(),
                "FT8 decodes transferred to GUI log"
            );
        }
        if removed > 0 {
            debug!(removed, max_entries, "FT8 GUI log bounded");
        }

        let snapshot = self.state.lock().expect("ui state lock poisoned").clone();
        self.emit_radio_state_hook_if_changed(&snapshot);
        self.publish_server_presence(&snapshot);

        // Keep the detailed meter panel visible throughout TX and briefly
        // afterward so the operator can see the final transmit readings.
        let now = Instant::now();
        if snapshot.ptt_on {
            self.show_meter_panel = true;
            self.meter_panel_close_deadline = None;
        } else if self.meter_panel_was_tx {
            self.meter_panel_close_deadline = Some(now + Duration::from_secs(2));
        }
        self.meter_panel_was_tx = snapshot.ptt_on;
        if self
            .meter_panel_close_deadline
            .is_some_and(|deadline| now >= deadline)
        {
            self.show_meter_panel = false;
            self.meter_panel_close_deadline = None;
        }

        // Use a compact, stable rail so the responsive control rows do not
        // inherit stale panel height or consume the waterfall's workspace.
        egui::TopBottomPanel::top("header_control_deck")
            .resizable(false)
            .show(ctx, |ui| {
                ui.scope(|ui| {
                    ui.spacing_mut().item_spacing = egui::vec2(7.0, 2.0);
                    ui.spacing_mut().button_padding = egui::vec2(7.0, 3.0);
                    ui.style_mut().override_font_id = Some(egui::FontId::proportional(13.0));
                    let visuals = ui.visuals_mut();
                    visuals.widgets.inactive.corner_radius = egui::CornerRadius::same(8);
                    visuals.widgets.hovered.corner_radius = egui::CornerRadius::same(8);
                    visuals.widgets.active.corner_radius = egui::CornerRadius::same(8);
                    let radio_tabs = self.available_profiles.clone();
                    let mut activate_tab = None;
                    let mut worker_action = None;
                    let mut open_config = None;
                    let mut commit_new_profile = false;
                    let mut section_divider_x = None;
                    let header_row = ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 3.0;
                        ui.allocate_ui_with_layout(
                            egui::vec2(360.0, 116.0),
                            egui::Layout::top_down(egui::Align::Min),
                            |ui| {
                                ui.horizontal(|ui| self.draw_header_branding(ui));
                                self.draw_header_identity_and_activity(ui);
                            },
                        );
                        let (divider_marker, _) =
                            ui.allocate_exact_size(egui::vec2(1.0, 1.0), egui::Sense::hover());
                        section_divider_x = Some(divider_marker.center().x);
                        ui.vertical(|ui| {
                            if !radio_tabs.is_empty() {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 3.0;
                            for name in radio_tabs {
                                let active = name == self.selected_profile_name;
                                let (indicator, indicator_color, identity, status) =
                                    self.radio_tab_status(&name, &snapshot);
                                let tab_fill = if active {
                                    Color32::from_rgb(24, 92, 116)
                                } else {
                                    Color32::from_rgb(48, 48, 52)
                                };
                                let tab_stroke = if active {
                                    Color32::from_rgb(88, 205, 235)
                                } else {
                                    indicator_color.linear_multiply(0.65)
                                };
                                egui::Frame::new()
                                    .fill(tab_fill)
                                    .stroke(egui::Stroke::new(1.0_f32, tab_stroke))
                                    .corner_radius(egui::CornerRadius::same(8))
                                    .inner_margin(egui::Margin::symmetric(5, 3))
                                    .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                    let text = RichText::new(format!("{indicator} {name} · {identity}"))
                                        .small()
                                        .strong()
                                        .color(if active {
                                            Color32::from_rgb(110, 220, 255)
                                        } else {
                                            indicator_color
                                        });
                                    if ui
                                        .add(egui::Button::selectable(active, text).small())
                                        .on_hover_text(format!("{name}: {status}"))
                                        .clicked()
                                    {
                                        activate_tab = Some(name.clone());
                                    }
                                    let radio_running = if active {
                                        self.command_tx.is_some()
                                            || (!self.radio_init_attempted
                                                && self.radio_init_rx.is_some())
                                    } else {
                                        self.parked_radio_sessions
                                            .get(&name)
                                            .is_some_and(|session| {
                                                session.command_tx.is_some()
                                                    || (!session.init_attempted
                                                        && session.init_rx.is_some())
                                            })
                                    };
                                    let audio_running = if active {
                                        self.audio_worker_handle.is_some()
                                    } else {
                                        self.parked_radio_sessions
                                            .get(&name)
                                            .is_some_and(|session| {
                                                session.audio_worker_handle.is_some()
                                            })
                                    };
                                    let workers_running = radio_running && audio_running;
                                    let worker_label = if workers_running { "■" } else { "▶" };
                                    let worker_hint = if workers_running {
                                        "Stop this profile's radio and audio workers"
                                    } else {
                                        "Start this profile's radio and audio workers"
                                    };
                                    let worker_button = egui::Button::new(worker_label)
                                    .small()
                                    .fill(if workers_running {
                                        Color32::from_rgb(126, 25, 39)
                                    } else {
                                        Color32::from_rgb(30, 105, 74)
                                    });
                                    if ui
                                        .add(worker_button)
                                        .on_hover_text(worker_hint)
                                        .clicked()
                                    {
                                        worker_action = Some((name.clone(), !workers_running));
                                    }
                                    let settings_response = ui
                                        .small_button("⚙")
                                        .on_hover_text("Open this radio tab's configuration");
                                    if settings_response.clicked() {
                                        open_config = Some((
                                            name.clone(),
                                            settings_response.rect.left_bottom()
                                                + egui::vec2(0.0, 4.0),
                                        ));
                                    }
                                    });
                                });
                            }
                            if self.new_profile_tab_editing {
                                let response = ui.add(
                                    egui::TextEdit::singleline(&mut self.new_profile_name)
                                        .desired_width(150.0)
                                        .hint_text("New profile name"),
                                );
                                commit_new_profile = response.lost_focus()
                                    || ui.input(|input| input.key_pressed(egui::Key::Enter));
                            } else if ui
                                .small_button("+")
                                .on_hover_text("Create a new radio profile")
                                .clicked()
                            {
                                self.new_profile_name.clear();
                                self.new_profile_tab_editing = true;
                            }
                        });
                        if let Some((name, running)) = worker_action {
                            self.set_tab_workers_running(&name, running);
                        } else if let Some(name) = activate_tab {
                            self.switch_radio_tab(&name);
                        } else if let Some((name, anchor)) = open_config {
                            if name != self.selected_profile_name {
                                self.switch_radio_tab(&name);
                            }
                            self.profile_drawer_anchor = Some(anchor);
                            self.show_profile_drawer = true;
                        }
                    } else {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("RADIOS")
                                    .small()
                                    .strong()
                                    .color(Color32::GRAY),
                            );
                        });
                    }
                    let row_divider_y = ui.cursor().top();
                    let row_divider_left = ui.max_rect().left();
                    ui.painter().line_segment(
                        [
                            egui::pos2(row_divider_left, row_divider_y),
                            egui::pos2(ui.max_rect().right(), row_divider_y),
                        ],
                        ui.visuals().widgets.noninteractive.bg_stroke,
                    );
                    ui.add_space(1.0);
                    ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 3.0;
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                    let frequency = snapshot
                        .frequency_hz
                        .map(|hz| format!("{:.6} MHz", hz as f64 / 1_000_000.0))
                        .unwrap_or_else(|| "RADIO OFFLINE".to_string());
                    ui.label(
                        RichText::new(frequency)
                            .monospace()
                            .strong()
                            .size(25.0)
                            .color(if snapshot.frequency_hz.is_some() {
                                Color32::from_rgb(120, 225, 255)
                            } else {
                                theme_warning(ui)
                            }),
                    );
                    let supports_vfo = snapshot
                        .supported_controls
                        .contains(&ControlId::Vfo);
                    let vfo = snapshot.active_vfo.min(1);
                    let vfo_label = if vfo == 0 { "VFO A" } else { "VFO B" };
                    if ui
                        .add_enabled(
                            supports_vfo && snapshot.radio_power_on == Some(true),
                            egui::Button::new(
                                RichText::new(vfo_label)
                                    .monospace()
                                    .strong()
                                    .color(if vfo == 0 {
                                        Color32::from_rgb(130, 220, 255)
                                    } else {
                                        Color32::from_rgb(255, 190, 105)
                                    }),
                            ),
                        )
                        .on_hover_text("Toggle the active radio VFO")
                        .clicked()
                    {
                        self.send_command(GuiCommand::SetControl(
                            ControlId::Vfo,
                            // Rigwright's CI-V VFO selector is a write-only
                            // command and its current HAL contract is U8.
                            ControlValue::U8(1 - vfo),
                        ));
                    }
                    let radio_profile = self
                        .config
                        .radio
                        .enabled
                        .then(|| {
                            native_radio_profile(
                                &self.config.radio.backend,
                                &self.config.radio.model,
                            )
                        })
                        .flatten();
                    let current_band = snapshot
                        .frequency_hz
                        .map(band_for_frequency)
                        .filter(|band| !band.is_empty())
                        .unwrap_or("—");
                    let band_menu = ui.menu_button(
                        RichText::new(current_band)
                            .strong()
                            .color(Color32::from_rgb(220, 190, 100)),
                        |ui| {
                            ui.label(RichText::new("AVAILABLE BANDS").strong());
                            ui.separator();
                            let current_hz = snapshot.frequency_hz.unwrap_or(0);
                            let activity_bands = self.activity.profile().bands.labels();
                            let mut visible_bands = 0;
                            ui.horizontal_wrapped(|ui| {
                                for (label, frequency_hz) in band_picker_plan(self.workspace_mode) {
                                    if !radio_supports_band(radio_profile, label) {
                                        continue;
                                    }
                                    visible_bands += 1;
                                    let selected = current_hz.abs_diff(frequency_hz) < 200_000;
                                    // Radio capability controls visibility; the
                                    // operating activity controls availability.
                                    // A mode's focused calling-frequency plan is
                                    // not a band restriction. Never hide or
                                    // disable a radio-supported band merely
                                    // because the current mode has no preset.
                                    let available_for_activity = activity_bands.contains(&label);
                                    if styled_selection_button(
                                        ui,
                                        label,
                                        selected,
                                        Color32::from_rgb(220, 190, 100),
                                        available_for_activity,
                                    )
                                    .on_hover_text(if available_for_activity {
                                        format!("{:.6} MHz", frequency_hz as f64 / 1_000_000.0)
                                    } else {
                                        format!(
                                            "{:.6} MHz · unavailable for {}",
                                            frequency_hz as f64 / 1_000_000.0,
                                            self.activity.label()
                                        )
                                    })
                                    .clicked()
                                    {
                                        self.send_command(GuiCommand::ApplyWorkspace {
                                            mode: self.workspace_mode,
                                            frequency_hz,
                                        });
                                        ui.close();
                                    }
                                }
                            });
                            if visible_bands == 0 {
                                ui.label(RichText::new("No bands available for this mode").weak());
                            }
                        },
                    );
                    band_menu.response.on_hover_text("Select the operating band");
                    self.draw_rf_power_control(ui, &snapshot);
                    ui.separator();
                    let current_radio_mode = radio_mode_label(&snapshot.mode, snapshot.data_mode);
                    let mode_menu = ui.menu_button(
                        // Keep the control width stable when the radio reports
                        // USB versus USB-D so neighboring controls do not
                        // jump during a mode-status refresh.
                        RichText::new(format!("{current_radio_mode: <5}"))
                            .monospace()
                            .strong()
                            .color(Color32::WHITE),
                        |ui| {
                            ui.label(RichText::new("RADIO MODE").strong());
                            ui.separator();
                            for (mode, label) in [
                                (Mode::Usb, "USB"),
                                (Mode::Lsb, "LSB"),
                                (Mode::Cw, "CW"),
                                (Mode::Data, "USB-D"),
                                (Mode::Am, "AM"),
                                (Mode::Fm, "FM"),
                                (Mode::Rtty, "RTTY"),
                                (Mode::CwReverse, "CW-R"),
                                (Mode::RttyReverse, "RTTY-R"),
                            ] {
                                if styled_selection_button(
                                    ui,
                                    label,
                                    current_radio_mode == label,
                                    Color32::from_rgb(190, 215, 235),
                                    true,
                                )
                                .clicked()
                                {
                                    self.send_command(GuiCommand::SetRadioMode(mode));
                                    ui.close();
                                }
                            }
                        },
                    );
                    mode_menu.response.on_hover_text("Select the radio operating mode");
                    let supports_filter = snapshot.supported_controls.contains(&ControlId::Filter);
                    let filter_label = snapshot
                        .filter
                        .map(|filter| format!("FIL{filter}"))
                        .unwrap_or_else(|| "FIL?".to_string());
                    let filter_menu = ui.menu_button(
                        RichText::new(filter_label).monospace().color(Color32::GRAY),
                        |ui| {
                            ui.label(RichText::new("FILTER").strong());
                            ui.separator();
                            for filter in 1_u8..=3 {
                                if styled_selection_button(
                                    ui,
                                    &format!("FIL{filter}"),
                                    snapshot.filter == Some(filter),
                                    Color32::from_rgb(160, 205, 230),
                                    supports_filter,
                                )
                                .clicked()
                                {
                                    self.send_command(GuiCommand::SetFilter(filter));
                                    ui.close();
                                }
                            }
                        },
                    );
                    filter_menu.response.on_hover_text("Select the radio IF filter");
                    self.draw_banner_radio_controls(ui, &snapshot);
                    ui.horizontal(|ui| {
                    let radio_ready = snapshot.radio_power_on == Some(true)
                        && !snapshot.radio_power_command_pending;
                    let supports_control = |id| snapshot.supported_controls.contains(&id);
                        ui.scope(|ui| {
                        ui.spacing_mut().button_padding.y = 4.0;
                        ui.horizontal(|ui| {
                            ui.separator();
                            ui.horizontal(|ui| {
                        let speaker_color = if radio_ready {
                            Color32::LIGHT_BLUE
                        } else {
                            Color32::GRAY
                        };
                        let (speaker_rect, speaker_response) = ui.allocate_exact_size(
                            egui::vec2(22.0, 22.0),
                            egui::Sense::hover(),
                        );
                        draw_speaker_icon(&ui.painter_at(speaker_rect), speaker_rect, speaker_color);
                        speaker_response.on_hover_text("RX/TX volume controls");
                            });
                            for (label, id, value, tooltip) in [
                        ("AF", ControlId::AfGain, snapshot.af_gain, "Audio receive gain"),
                        ("RF", ControlId::RfGain, snapshot.rf_gain, "RF receive gain"),
                        ("SQ", ControlId::Squelch, snapshot.squelch, "Squelch threshold"),
                        ("TX", ControlId::RfPower, snapshot.rf_power, "RF transmit power"),
                            ] {
                        let color = if supports_control(id) && radio_ready {
                            Color32::LIGHT_BLUE
                        } else {
                            Color32::GRAY
                        };
                        ui.menu_button(
                            RichText::new(label).size(12.0).monospace().color(color),
                            |ui| {
                                let mut percent = value
                                    .map(|raw| f32::from(raw) * 100.0 / 255.0)
                                    .unwrap_or_default();
                                let response = ui.add_enabled(
                                    supports_control(id) && radio_ready,
                                    egui::Slider::new(&mut percent, 0.0..=100.0)
                                        .vertical()
                                        .show_value(false),
                                );
                                ui.label(format!("{percent:.0}%"));
                                if response.changed() && response.drag_stopped() {
                                    let normalized = (percent.clamp(0.0, 100.0) * 255.0 / 100.0)
                                        .round()
                                        as u8;
                                    self.send_command(GuiCommand::SetControl(
                                        id,
                                        ControlValue::U8(normalized),
                                    ));
                                }
                                response.on_hover_text(if !supports_control(id) {
                                    "This control is not supported by the loaded radio profile"
                                } else if !radio_ready {
                                    "Unavailable while the radio is offline or waking"
                                } else {
                                    tooltip
                                });
                            },
                        );
                            }
                        });
                        });
                    if supports_control(ControlId::Tuner) {
                        let tuner_color = if snapshot.tuner_status.is_some_and(|status| status.tuning) {
                            Color32::YELLOW
                        } else if snapshot.tuner_status.is_some_and(|status| status.enabled) {
                            Color32::LIGHT_GREEN
                        } else {
                            Color32::GRAY
                        };
                        ui.menu_button(RichText::new("TUNE").color(tuner_color), |ui| {
                            ui.label(if snapshot.tuner_status.is_some_and(|status| status.tuning) {
                                "Tuning in progress"
                            } else if snapshot.tuner_status.is_some_and(|status| status.enabled) {
                                "Tuner enabled"
                            } else {
                                "Tuner disabled"
                            });
                            ui.horizontal(|ui| {
                                let enabled = snapshot.tuner_status.is_some_and(|status| status.enabled);
                                if ui
                                    .add_enabled(
                                        radio_ready && !snapshot.swr_sweep_active,
                                        egui::Button::new(if enabled { "Disable" } else { "Enable" }),
                                    )
                                    .on_hover_text(if enabled {
                                        "Disable the radio's antenna tuner"
                                    } else {
                                        "Enable the radio's antenna tuner"
                                    })
                                    .clicked()
                                {
                                    self.send_command(GuiCommand::SetControl(
                                        ControlId::Tuner,
                                        ControlValue::Bool(!enabled),
                                    ));
                                    ui.close();
                                }
                                if ui
                                    .add_enabled(radio_ready && !snapshot.swr_sweep_active, egui::Button::new("Tune"))
                                    .on_hover_text("Start the radio's antenna-tuner cycle; this may transmit")
                                    .clicked()
                                {
                                    self.send_command(GuiCommand::StartTuner);
                                    ui.close();
                                }
                            });
                        }).response.on_hover_text("Enable the antenna tuner or start tuning");
                    }
                    if snapshot.supported_meters.contains(&MeterId::Swr) {
                        if let Some((band_start, band_stop, band_name)) =
                            band_edges_for_frequency(snapshot.frequency_hz)
                        {
                            let mut state = self.state.lock().expect("ui state lock poisoned");
                            if state.swr_sweep_band.as_deref() != Some(band_name) {
                                let width = band_stop.saturating_sub(band_start);
                                state.swr_sweep_start_hz = band_start;
                                state.swr_sweep_stop_hz = band_stop;
                                state.swr_sweep_step_hz = (width / 100).max(1_000);
                                state.swr_sweep_band = Some(band_name.to_string());
                            }
                        }
                        let swr_button = ui
                            .button(RichText::new("SWR").color(Color32::LIGHT_BLUE))
                            .on_hover_text("Read the SWR meter or scan the active band");
                        egui::Popup::menu(&swr_button)
                            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                            .show(|ui| {
                            ui.label("SWR meter and sweep");
                            if let Some((band_start, band_stop, band_name)) =
                                band_edges_for_frequency(snapshot.frequency_hz)
                            {
                                ui.label(format!(
                                    "Active band: {band_name} ({band_start}–{band_stop} Hz)"
                                ));
                            } else {
                                ui.label("Active band unavailable; enter a manual range");
                            }
                            ui.separator();
                            ui.label(format!(
                                "SWR: {}",
                                format_swr_display(&self.config.radio.model, snapshot.swr)
                            ));
                            ui.colored_label(
                                Color32::YELLOW,
                                "SWR sweep: RTTY carrier at approximately 30 W; TX is restored afterward.",
                            );
                            ui.horizontal(|ui| {
                                ui.label("Start");
                                ui.add(egui::DragValue::new(&mut self.state.lock().expect("ui state lock poisoned").swr_sweep_start_hz).speed(100.0));
                            });
                            ui.horizontal(|ui| {
                                ui.label("Stop");
                                ui.add(egui::DragValue::new(&mut self.state.lock().expect("ui state lock poisoned").swr_sweep_stop_hz).speed(100.0));
                            });
                            ui.horizontal(|ui| {
                                ui.label("Step");
                                ui.add(egui::DragValue::new(&mut self.state.lock().expect("ui state lock poisoned").swr_sweep_step_hz).speed(100.0));
                            });
                            ui.horizontal(|ui| {
                                ui.label("Interval ms");
                                ui.add(egui::DragValue::new(&mut self.state.lock().expect("ui state lock poisoned").swr_sweep_interval_ms).range(100..=10_000));
                            });
                            let sweep_enabled = radio_ready && !snapshot.swr_sweep_active;
                            if ui.add_enabled(sweep_enabled, egui::Button::new("Start TX SWR sweep")).clicked() {
                                self.swr_sweep_abort.store(false, Ordering::Relaxed);
                                let s = self.state.lock().expect("ui state lock poisoned");
                                self.send_command(GuiCommand::StartSwrSweep {
                                    start_hz: s.swr_sweep_start_hz,
                                    stop_hz: s.swr_sweep_stop_hz,
                                    step_hz: s.swr_sweep_step_hz,
                                    interval_ms: s.swr_sweep_interval_ms,
                                });
                            }
                            if snapshot.swr_sweep_active {
                                ui.spinner();
                                if ui.button("Stop sweep").clicked() {
                                    self.swr_sweep_abort.store(true, Ordering::Relaxed);
                                }
                            }
                            ui.label(&snapshot.swr_sweep_status);
                            let (rect, _) = ui.allocate_exact_size(egui::vec2(420.0, 180.0), egui::Sense::hover());
                            let painter = ui.painter_at(rect);
                            painter.rect_filled(rect, 2.0, Color32::from_gray(24));
                            let points = &snapshot.swr_sweep_points;
                            let chart_left = rect.left() + 42.0;
                            let chart_right = rect.right() - 8.0;
                            let chart_top = rect.top() + 10.0;
                            let chart_bottom = rect.bottom() - 22.0;
                            let chart_rect = egui::Rect::from_min_max(
                                egui::pos2(chart_left, chart_top),
                                egui::pos2(chart_right, chart_bottom),
                            );
                            let icom_swr_chart = native_radio_profile(
                                "native",
                                &self.config.radio.model,
                            )
                            .and_then(|profile| {
                                profile.calibrated_meter_value(MeterId::Swr, 0)
                            })
                            .is_some();
                            let chart_axes = if icom_swr_chart {
                                [(1.0_f32, "1.0:1"), (1.5, "1.5:1"), (2.0, "2.0:1"), (2.5, "2.5:1"), (3.0, "3.0:1")]
                            } else {
                                [(0.0_f32, "0%"), (25.0, "25%"), (50.0, "50%"), (75.0, "75%"), (100.0, "100%")]
                            };
                            let chart_min = chart_axes[0].0;
                            let chart_max = chart_axes[4].0;
                            for (value, label) in chart_axes {
                                let y = chart_bottom
                                    - ((value - chart_min) / (chart_max - chart_min)) * chart_rect.height();
                                painter.line_segment(
                                    [egui::pos2(chart_left, y), egui::pos2(chart_right, y)],
                                    egui::Stroke::new(1.0_f32, Color32::from_gray(65)),
                                );
                                painter.text(
                                    egui::pos2(rect.left() + 4.0, y - 7.0),
                                    egui::Align2::LEFT_TOP,
                                    label,
                                    egui::FontId::monospace(10.0),
                                    Color32::GRAY,
                                );
                            }
                            if points.len() > 1 {
                                let min_hz = points.first().map(|point| point.0).unwrap_or(0) as f32;
                                let max_hz = points.last().map(|point| point.0).unwrap_or(1).max(points.first().map(|point| point.0).unwrap_or(0) + 1) as f32;
                                let polyline: Vec<_> = points.iter().map(|(hz, raw)| {
                                    let x = chart_left + ((*hz as f32 - min_hz) / (max_hz - min_hz)) * chart_rect.width();
                                    let value = swr_chart_value(&self.config.radio.model, *raw)
                                        .clamp(chart_min, chart_max);
                                    let y = chart_bottom
                                        - ((value - chart_min) / (chart_max - chart_min)) * chart_rect.height();
                                    egui::pos2(x, y)
                                }).collect();
                                painter.add(egui::Shape::line(polyline.clone(), egui::Stroke::new(2.0_f32, Color32::LIGHT_GREEN)));
                                for point in polyline {
                                    painter.circle_filled(point, 2.5, Color32::WHITE);
                                }
                                painter.text(
                                    egui::pos2(chart_left, chart_bottom + 4.0),
                                    egui::Align2::LEFT_TOP,
                                    format!("{}", points.first().map(|point| point.0).unwrap_or(0)),
                                    egui::FontId::monospace(10.0),
                                    Color32::GRAY,
                                );
                                painter.text(
                                    egui::pos2(chart_right, chart_bottom + 4.0),
                                    egui::Align2::RIGHT_TOP,
                                    format!("{}", points.last().map(|point| point.0).unwrap_or(0)),
                                    egui::FontId::monospace(10.0),
                                    Color32::GRAY,
                                );
                            } else if points.len() == 1 {
                                painter.circle_filled(chart_rect.center(), 3.0, Color32::WHITE);
                            }
                            });
                    }
                    if supports_control(ControlId::NoiseBlanker) {
                        let color = match snapshot.noise_blank {
                            Some(true) => Color32::LIGHT_GREEN,
                            Some(false) => Color32::GRAY,
                            None => Color32::DARK_GRAY,
                        };
                        if ui
                            .add_enabled(
                                radio_ready,
                                egui::Button::new(RichText::new("NB").color(Color32::WHITE))
                                    .small()
                                    .fill(color),
                            )
                            .on_hover_text("Toggle noise blanker")
                            .clicked()
                        {
                            self.send_command(GuiCommand::SetControl(
                                ControlId::NoiseBlanker,
                                ControlValue::Bool(snapshot.noise_blank != Some(true)),
                            ));
                        }
                    }
                    if supports_control(ControlId::NoiseReduction) {
                        let color = match snapshot.noise_reduction {
                            Some(true) => Color32::LIGHT_GREEN,
                            Some(false) => Color32::GRAY,
                            None => Color32::DARK_GRAY,
                        };
                        if ui
                            .add_enabled(
                                radio_ready,
                                egui::Button::new(RichText::new("NR").color(Color32::WHITE))
                                    .small()
                                    .fill(color),
                            )
                            .on_hover_text("Toggle noise reduction")
                            .clicked()
                        {
                            self.send_command(GuiCommand::SetControl(
                                ControlId::NoiseReduction,
                                ControlValue::Bool(snapshot.noise_reduction != Some(true)),
                            ));
                        }
                    }
                    if supports_control(ControlId::NoiseReductionLevel) {
                        let max_level = radio_control_max(
                            &self.config.radio.model,
                            ControlId::NoiseReductionLevel,
                            15,
                        );
                        ui.menu_button(
                            RichText::new("NRL").color(if snapshot.noise_reduction_level.is_some() {
                                Color32::LIGHT_BLUE
                            } else {
                                Color32::DARK_GRAY
                            }),
                            |ui| {
                                ui.label("Noise reduction level");
                                let mut level = snapshot.noise_reduction_level.unwrap_or(8) as f32;
                                let response = ui.add(
                                    egui::Slider::new(&mut level, 1.0..=f32::from(max_level))
                                        .step_by(1.0)
                                        .show_value(true),
                                );
                                if response.changed() && response.drag_stopped() {
                                    self.send_command(GuiCommand::SetControl(
                                        ControlId::NoiseReductionLevel,
                                        ControlValue::U8(level.round() as u8),
                                    ));
                                }
                            },
                        )
                        .response
                        .on_hover_text("Set the Yaesu noise-reduction level");
                    }
                    if supports_control(ControlId::IpPlus) {
                        let color = match snapshot.ip_plus {
                            Some(true) => Color32::LIGHT_GREEN,
                            Some(false) => Color32::GRAY,
                            None => Color32::DARK_GRAY,
                        };
                        if ui
                            .add_enabled(
                                radio_ready,
                                egui::Button::new(RichText::new("IP+").color(Color32::WHITE))
                                    .small()
                                    .fill(color),
                            )
                            .on_hover_text("Toggle Icom IP Plus receiver optimization")
                            .clicked()
                        {
                            self.send_command(GuiCommand::SetControl(
                                ControlId::IpPlus,
                                ControlValue::Bool(snapshot.ip_plus != Some(true)),
                            ));
                        }
                    }
                    if supports_control(ControlId::Notch) {
                        let notch_label = if snapshot.notch_manual == Some(true) {
                            "MN"
                        } else if snapshot.notch_auto == Some(true) {
                            "AN"
                        } else {
                            "NT"
                        };
                        let notch_color = if snapshot.notch_auto == Some(true)
                            || snapshot.notch_manual == Some(true)
                        {
                            Color32::LIGHT_GREEN
                        } else {
                            Color32::GRAY
                        };
                        ui.menu_button(
                            RichText::new(notch_label).color(notch_color),
                            |ui| {
                                for (label, auto, manual) in [
                                    ("Off", false, false),
                                    ("Auto notch", true, false),
                                    ("Manual notch", false, true),
                                ] {
                                    if ui
                                        .selectable_label(
                                            snapshot.notch_auto == Some(auto)
                                                && snapshot.notch_manual == Some(manual),
                                            label,
                                        )
                                        .clicked()
                                    {
                                        self.send_command(GuiCommand::SetControl(
                                            ControlId::Notch,
                                            ControlValue::Bool(auto),
                                        ));
                                        self.send_command(GuiCommand::SetControl(
                                            ControlId::ManualNotch,
                                            ControlValue::Bool(manual),
                                        ));
                                        ui.close();
                                    }
                                }
                            },
                        )
                        .response
                        .on_hover_text("Select off, auto notch, or manual notch");
                    }
                    if supports_control(ControlId::Agc) {
                        let max_agc = radio_control_max(&self.config.radio.model, ControlId::Agc, 4);
                        let color = if snapshot.agc.is_some() {
                            Color32::LIGHT_BLUE
                        } else {
                            Color32::DARK_GRAY
                        };
                        ui.menu_button(RichText::new("AGC").color(color), |ui| {
                            for value in 0_u8..=max_agc {
                                if ui
                                    .selectable_label(snapshot.agc == Some(value), format!("AGC {value}"))
                                    .clicked()
                                {
                                    self.send_command(GuiCommand::SetControl(
                                        ControlId::Agc,
                                        ControlValue::U8(value),
                                    ));
                                    ui.close();
                                }
                            }
                        })
                        .response
                        .on_hover_text("Select the automatic gain-control level");
                    }
                    if !snapshot.supported_meters.is_empty() {
                        ui.menu_button(RichText::new("MTR").color(Color32::LIGHT_BLUE), |ui| {
                            ui.label("Normalized meter levels");
                            for (label, id, value) in [
                                ("SIG", MeterId::Signal, snapshot.signal_meter),
                                ("PWR", MeterId::Power, snapshot.power_meter),
                                ("SWR", MeterId::Swr, snapshot.swr),
                                ("ALC", MeterId::Alc, snapshot.alc_meter),
                                ("COMP", MeterId::Compression, snapshot.compression_meter),
                                ("I", MeterId::Current, snapshot.current_meter),
                                ("V", MeterId::Voltage, snapshot.voltage_meter),
                                ("TEMP", MeterId::Temperature, snapshot.temperature_meter),
                            ] {
                                if snapshot.supported_meters.contains(&id) {
                                    ui.horizontal(|ui| {
                                        ui.label(label);
                                        let fraction = value.map_or(0.0, |raw| f32::from(raw) / 255.0);
                                        ui.add(
                                            egui::ProgressBar::new(fraction)
                                                .desired_width(120.0),
                                        );
                                    });
                                }
                            }
                        })
                        .response
                        .on_hover_text("Normalized vendor meter levels; physical units and SWR ratios remain vendor-specific");
                    }
                    if supports_control(ControlId::Preamp) {
                        let color = match snapshot.preamp {
                            Some(value) if value > 0 => Color32::LIGHT_GREEN,
                            Some(_) => Color32::GRAY,
                            None => Color32::DARK_GRAY,
                        };
                        let max_preamp = native_radio_profile("native", &self.config.radio.model)
                            .and_then(|profile| profile.control_max(ControlId::Preamp))
                            .unwrap_or(0);
                        ui.menu_button(RichText::new("PRE").color(color), |ui| {
                            for value in 0_u8..=max_preamp {
                                if ui
                                    .selectable_label(
                                        snapshot.preamp == Some(value),
                                        format!("PRE {value}"),
                                    )
                                    .clicked()
                                {
                                    self.send_command(GuiCommand::SetControl(
                                        ControlId::Preamp,
                                        ControlValue::U8(value),
                                    ));
                                    ui.close();
                                }
                            }
                        })
                        .response
                        .on_hover_text("Select the radio preamplifier level");
                    }
                    if supports_control(ControlId::Attenuator) {
                        let attenuator_values = native_radio_profile(
                            "native",
                            &self.config.radio.model,
                        )
                        .and_then(|profile| {
                            profile.supported_control_values(ControlId::Attenuator)
                        })
                        .unwrap_or(&[]);
                        let color = match snapshot.attenuator {
                            Some(value) if value > 0 => Color32::from_rgb(255, 190, 105),
                            Some(_) => Color32::GRAY,
                            None => Color32::DARK_GRAY,
                        };
                        ui.menu_button(RichText::new("ATT").color(color), |ui| {
                            for &value in attenuator_values {
                                if ui
                                    .selectable_label(
                                        snapshot.attenuator == Some(value),
                                        format!("ATT {value}"),
                                    )
                                    .clicked()
                                {
                                    self.send_command(GuiCommand::SetControl(
                                        ControlId::Attenuator,
                                        ControlValue::U8(value),
                                    ));
                                    ui.close();
                                }
                            }
                        })
                        .response
                        .on_hover_text("Select the radio input attenuator level");
                    }
                    ui.separator();
                    self.draw_waterfall_buttons(ui, &snapshot);
                    let monitor_label = "MON";
                    let monitor_color = if self.config.audio.monitor_enabled {
                        Color32::LIGHT_GREEN
                    } else {
                        Color32::GRAY
                    };
                    if ui
                        .selectable_label(
                            self.config.audio.monitor_enabled,
                            RichText::new(monitor_label).color(monitor_color),
                        )
                        .on_hover_text("Toggle captured RX audio to the selected monitor output")
                        .clicked()
                    {
                        self.config.audio.monitor_enabled = !self.config.audio.monitor_enabled;
                        self.profile_dirty = true;
                        self.persist_profile("Audio monitor saved to");
                        self.restart_audio();
                    }
                    let (speaker_rect, speaker_button) = ui.allocate_exact_size(
                        egui::vec2(28.0, 28.0),
                        egui::Sense::click(),
                    );
                    let speaker_color = if self.config.audio.monitor_enabled {
                        Color32::LIGHT_GREEN
                    } else {
                        Color32::GRAY
                    };
                    let speaker_painter = ui.painter_at(speaker_rect);
                    let speaker_center = speaker_rect.center();
                    speaker_painter.rect_filled(
                        egui::Rect::from_center_size(
                            egui::pos2(speaker_center.x - 5.0, speaker_center.y),
                            egui::vec2(4.0, 9.0),
                        ),
                        1.0,
                        speaker_color,
                    );
                    speaker_painter.add(egui::Shape::convex_polygon(
                        vec![
                            egui::pos2(speaker_center.x - 3.0, speaker_center.y - 5.0),
                            egui::pos2(speaker_center.x + 3.0, speaker_center.y - 9.0),
                            egui::pos2(speaker_center.x + 3.0, speaker_center.y + 9.0),
                            egui::pos2(speaker_center.x - 3.0, speaker_center.y + 5.0),
                        ],
                        speaker_color,
                        egui::Stroke::NONE,
                    ));
                    speaker_painter.line_segment(
                        [
                            egui::pos2(speaker_center.x + 6.0, speaker_center.y - 5.0),
                            egui::pos2(speaker_center.x + 9.0, speaker_center.y - 2.0),
                        ],
                        egui::Stroke::new(1.5_f32, speaker_color),
                    );
                    speaker_painter.line_segment(
                        [
                            egui::pos2(speaker_center.x + 6.0, speaker_center.y + 5.0),
                            egui::pos2(speaker_center.x + 9.0, speaker_center.y + 2.0),
                        ],
                        egui::Stroke::new(1.5_f32, speaker_color),
                    );
                    let speaker_button = speaker_button.on_hover_text("RX monitor volume");
                    egui::Popup::menu(&speaker_button).show(|ui| {
                        ui.horizontal(|ui| {
                            let mut volume = self.config.audio.monitor_volume.clamp(0.0, 2.0);
                            let response = ui.add(
                                egui::Slider::new(&mut volume, 0.0..=2.0)
                                    .vertical()
                                    .show_value(false),
                            );
                            ui.vertical(|ui| {
                                ui.label("RX");
                                ui.label(format!("{:.0}%", volume * 100.0));
                            });
                            if response.changed() {
                                self.config.audio.monitor_volume = volume;
                                self.monitor_volume
                                    .store(volume.to_bits(), Ordering::Relaxed);
                                self.profile_dirty = true;
                                self.persist_profile("RX monitor volume saved to");
                            }
                        });
                    });
                    });
                    });
                    ui.horizontal(|ui| {
                        self.draw_meter_display(ui, &snapshot);
                        self.draw_banner_op_modes(ui, &snapshot);
                    });
                    });
                    });
                    });
                    });
                    if let Some(divider_x) = section_divider_x {
                        ui.painter().line_segment(
                            [
                                egui::pos2(divider_x, header_row.response.rect.top()),
                                egui::pos2(divider_x, header_row.response.rect.bottom()),
                            ],
                            ui.visuals().widgets.noninteractive.bg_stroke,
                        );
                    }
                    if commit_new_profile {
                        self.create_profile_from_tab_name();
                    }
                });
            });

        egui::Area::new(egui::Id::new("radio_profile_power_top_right"))
            .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-8.0, 6.0))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let active_profile = self.active_radio_profile_name().unwrap_or("None");
                    ui.label(
                        RichText::new(format!("RADIO · {active_profile}"))
                            .small()
                            .color(if active_profile == "None" {
                                Color32::GRAY
                            } else {
                                Color32::from_rgb(255, 201, 92)
                            }),
                    )
                    .on_hover_text(format!(
                        "Active radio profile\n{active_profile}\nThis profile owns the radio connection and its per-radio settings."
                    ));
                    self.draw_power_button(ui, &snapshot);
                });
            });

        let supports_radio_scope =
            native_radio_profile(&self.config.radio.backend, &self.config.radio.model)
                .is_some_and(|profile| profile.capabilities.spectrum);
        let radio_scope_visible = self.civ_spectrum_on
            && supports_radio_scope
            && !snapshot.radio_waterfall_status.starts_with("UNAVAILABLE");
        // Bottom panels are stacked in declaration order: the first one owns
        // the outermost bottom strip. The monitor is rendered in the remaining
        // central region below, so its height follows the window naturally.
        egui::TopBottomPanel::bottom("connection_status")
            .resizable(false)
            .exact_height(30.0)
            .show(ctx, |ui| self.draw_connection_status(ui, &snapshot));

        egui::TopBottomPanel::bottom("global_contact_log")
            .resizable(true)
            .show_separator_line(true)
            .default_height(260.0)
            .height_range(150.0..=420.0)
            .show(ctx, |ui| {
                // Keep the log contents inside the panel's exact rectangle. This
                // mirrors the waterfall deck and prevents the editor controls
                // from expanding the panel and leaving a black overflow area.
                let log_rect = ui.available_rect_before_wrap();
                ui.allocate_rect(log_rect, egui::Sense::hover());
                let mut log_ui = ui.new_child(
                    egui::UiBuilder::new()
                        .id_salt("global_contact_log_contents")
                        .max_rect(log_rect)
                        .layout(egui::Layout::top_down(egui::Align::Min)),
                );
                log_ui.set_clip_rect(log_rect);
                self.draw_contact_log(&mut log_ui, &snapshot);
            });

        if self.show_signal_panel {
            egui::SidePanel::left("signals")
                .resizable(true)
                .default_width(430.0)
                .min_width(300.0)
                .max_width(ctx.content_rect().width() * 0.72)
                .show(ctx, |ui| {
                    self.draw_tx_safety_card(ui, &snapshot);
                    ui.add_space(6.0);
                    ui.horizontal_wrapped(|ui| {
                        for (tab, icon, label, icon_color) in [
                            (
                                SignalPanelTab::Achievements,
                                "🏆",
                                "ACHIEVEMENTS",
                                Color32::from_rgb(255, 201, 92),
                            ),
                            (
                                SignalPanelTab::Station,
                                "📡",
                                "STATION",
                                Color32::from_rgb(120, 225, 255),
                            ),
                            (
                                SignalPanelTab::Contest,
                                "🏁",
                                "CONTEST",
                                Color32::from_rgb(255, 151, 72),
                            ),
                            (
                                SignalPanelTab::Reporting,
                                "📡",
                                "REPORTING",
                                Color32::from_rgb(132, 228, 255),
                            ),
                            (
                                SignalPanelTab::Settings,
                                "⚙",
                                "SETTINGS",
                                Color32::from_rgb(190, 190, 205),
                            ),
                            (
                                SignalPanelTab::Ai,
                                "",
                                "AI",
                                Color32::from_rgb(205, 150, 255),
                            ),
                            (
                                SignalPanelTab::Server,
                                "🌐",
                                "SERVER",
                                Color32::from_rgb(110, 220, 255),
                            ),
                            (
                                SignalPanelTab::RadioTuning,
                                "📻",
                                "RADIO TUNING",
                                Color32::from_rgb(255, 190, 105),
                            ),
                            (
                                SignalPanelTab::AppLog,
                                "📋",
                                "APP LOG",
                                Color32::from_rgb(180, 190, 205),
                            ),
                        ] {
                            let selected = self.signal_panel_tab == tab;
                            let compact_ai_spacing = tab == SignalPanelTab::Ai;
                            let previous_item_spacing = ui.spacing().item_spacing.x;
                            if compact_ai_spacing {
                                ui.spacing_mut().item_spacing.x = 2.0;
                            }
                            let icon_response = if tab == SignalPanelTab::Ai {
                                let (icon_rect, response) = ui.allocate_exact_size(
                                    egui::vec2(13.0, 18.0),
                                    egui::Sense::click(),
                                );
                                draw_ai_icon(ui.painter(), icon_rect, icon_color);
                                Some(response)
                            } else {
                                None
                            };
                            let tab_text = if icon.is_empty() {
                                label.to_string()
                            } else {
                                format!("{icon} {label}")
                            };
                            let text = if selected {
                                RichText::new(tab_text)
                                    .strong()
                                    .color(Color32::from_rgb(120, 225, 255))
                            } else {
                                RichText::new(tab_text).color(icon_color)
                            };
                            let text_clicked = ui.selectable_label(selected, text).clicked();
                            if compact_ai_spacing {
                                ui.spacing_mut().item_spacing.x = previous_item_spacing;
                            }
                            if text_clicked
                                || icon_response.is_some_and(|response| response.clicked())
                            {
                                self.signal_panel_tab = tab;
                            }
                        }
                    });
                    ui.separator();
                    if self.signal_panel_tab == SignalPanelTab::AppLog {
                        self.draw_app_log_panel(ui);
                    } else {
                        egui::ScrollArea::vertical()
                            .id_salt("signals_scroll")
                            .auto_shrink([false, false])
                            .show(ui, |ui| match self.signal_panel_tab {
                                SignalPanelTab::Achievements => {
                                    self.draw_hunter_panel(ui, &snapshot)
                                }
                                SignalPanelTab::Station => self.draw_station_panel(ui),
                                SignalPanelTab::Contest => self.draw_contest_panel(ui),
                                SignalPanelTab::Reporting => self.draw_reporting_panel(ui),
                                SignalPanelTab::Settings => {
                                    self.draw_application_settings_panel(ui)
                                }
                                SignalPanelTab::Ai => self.draw_ai_panel(ui),
                                SignalPanelTab::Server => self.draw_server_panel(ui),
                                SignalPanelTab::RadioTuning => {
                                    self.draw_radio_tuning_panel(ui, &snapshot)
                                }
                                SignalPanelTab::AppLog => {
                                    unreachable!("app log has its own scroll area")
                                }
                            });
                    }
                });
        }

        if self.show_profile_drawer {
            let mut drawer_open = true;
            let drawer_anchor = self.profile_drawer_anchor.take();
            let mut drawer = egui::Window::new("⚙ Profile management")
                .open(&mut drawer_open)
                .default_width(460.0)
                .default_height(560.0)
                .min_width(360.0)
                .min_height(260.0)
                .max_width(560.0)
                .max_height(760.0)
                .resizable(true)
                .movable(true);
            if let Some(anchor) = drawer_anchor {
                drawer = drawer.current_pos(anchor);
            }
            drawer.show(ctx, |ui| {
                ui.label(
                    RichText::new(format!(
                        "{} · {}",
                        self.selected_profile_name,
                        if self.profile_dirty {
                            "unsaved changes"
                        } else {
                            "saved"
                        }
                    ))
                    .small()
                    .color(if self.profile_dirty {
                        theme_warning(ui)
                    } else {
                        Color32::GRAY
                    }),
                );
                ui.separator();
                ui.horizontal(|ui| {
                    for (tab, label) in [
                        (ProfileDrawerTab::Profile, "PROFILE"),
                        (ProfileDrawerTab::Radio, "RADIO"),
                        (ProfileDrawerTab::Tuning, "TUNING"),
                        (ProfileDrawerTab::DigitalTiming, "DIGITAL TIMING"),
                        (ProfileDrawerTab::Monitoring, "MONITORING"),
                    ] {
                        if ui
                            .selectable_label(self.profile_drawer_tab == tab, label)
                            .clicked()
                        {
                            self.profile_drawer_tab = tab;
                        }
                    }
                });
                ui.separator();
                egui::ScrollArea::vertical()
                    .id_salt("profile_management_drawer")
                    .auto_shrink([false, false])
                    .show(ui, |ui| match self.profile_drawer_tab {
                        ProfileDrawerTab::Profile => self.draw_profile_panel(ui),
                        ProfileDrawerTab::Radio => self.draw_radio_profile_settings(ui),
                        ProfileDrawerTab::Tuning => self.draw_radio_profile_assignments(ui),
                        ProfileDrawerTab::DigitalTiming => self.draw_digital_timing_settings(ui),
                        ProfileDrawerTab::Monitoring => self.draw_monitoring_settings(ui),
                    });
            });
            if !drawer_open {
                self.show_profile_drawer = false;
            }
        }

        if self.radio_faq_window_open || self.radio_guide_window_open {
            let help = help_for_model(&self.radio_help_window_model);
            let mut faq_open = self.radio_faq_window_open;
            let (faq_title, faq_document) = match self.radio_faq_document {
                RadioHelpDocument::Audio => ("Audio FAQ", AUDIO_FAQ),
                RadioHelpDocument::Manufacturer => ("Manufacturer FAQ", help.manufacturer_faq),
                RadioHelpDocument::Model => ("Model FAQ", help.model_faq),
            };
            egui::Window::new(format!("{} — {}", help.title, faq_title))
                .open(&mut faq_open)
                .default_width(620.0)
                .default_height(560.0)
                .min_width(360.0)
                .min_height(240.0)
                .resizable(true)
                .movable(true)
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            render_document(ui, faq_document);
                        });
                });
            self.radio_faq_window_open = faq_open;

            let mut guide_open = self.radio_guide_window_open;
            let (guide_title, guide_document) = match self.radio_guide_document {
                RadioHelpDocument::Audio => ("Audio FAQ", AUDIO_FAQ),
                RadioHelpDocument::Manufacturer => ("Manufacturer Guide", help.manufacturer_guide),
                RadioHelpDocument::Model => ("Model Guide", help.model_guide),
            };
            egui::Window::new(format!("{} — {}", help.title, guide_title))
                .open(&mut guide_open)
                .default_width(620.0)
                .default_height(560.0)
                .min_width(360.0)
                .min_height(240.0)
                .resizable(true)
                .movable(true)
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            render_document(ui, guide_document);
                        });
                });
            self.radio_guide_window_open = guide_open;
        }

        let monitor_has_data = !snapshot.audio_waterfall_rows.is_empty()
            || (radio_scope_visible && !snapshot.radio_waterfall_rows.is_empty());
        if monitor_has_data {
            // The waterfall deck is a real user-resizable panel. Its height
            // must not track incoming rows or silently change the workspace
            // layout while a radio is running.
            let resize_id = egui::Id::new("waterfall_deck").with("__resize");
            let resize_in_progress = ctx
                .read_response(resize_id)
                .is_some_and(|response| response.dragged());
            let mut waterfall_panel = egui::TopBottomPanel::top("waterfall_deck")
                .resizable(true)
                .default_height(self.waterfall_deck_height)
                .height_range(170.0..=ctx.content_rect().height().max(240.0) * 0.75)
                .show_separator_line(true);
            // egui keeps its own panel size between frames. Reassert the
            // application's last chosen height whenever the resize handle is
            // idle, otherwise a stale/clamped panel state can feed a smaller
            // rectangle back to the waterfall after an empty-input transition.
            if !resize_in_progress {
                waterfall_panel = waterfall_panel.min_height(self.waterfall_deck_height);
            }
            let waterfall_panel = waterfall_panel.show(ctx, |ui| {
                if radio_scope_visible {
                    let total_width = ui.available_width();
                    let radio_default_width = total_width * 0.5;
                    let radio_max_width = (total_width - 260.0).max(280.0);
                    egui::SidePanel::left("radio_waterfall_split")
                        .resizable(true)
                        .default_width(radio_default_width)
                        .width_range(280.0..=radio_max_width)
                        .show_inside(ui, |ui| {
                            self.draw_radio_waterfall(ui, ctx, &snapshot);
                        });
                    self.draw_audio_waterfall(ui, ctx, &snapshot);
                } else {
                    self.draw_audio_waterfall(ui, ctx, &snapshot);
                }
            });
            // Only accept a new height while the panel resize handle is being
            // dragged. The panel response also reflects layout constraints, so
            // copying its height every frame lets changing/empty waterfall
            // content overwrite the user's chosen height and creates a
            // shrink-to-minimum feedback loop.
            if resize_in_progress {
                let actual_height = waterfall_panel.response.rect.height();
                if actual_height.is_finite()
                    && (actual_height - self.waterfall_deck_height).abs() > 0.5
                {
                    self.waterfall_deck_height = actual_height.clamp(170.0, 560.0);
                    self.profile_dirty = true;
                }
            }
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            self.draw_bounded_workspace(ui, ctx, &snapshot);
        });
    }
}

pub(crate) fn update(app: &mut QsonautGuiApp, ctx: &egui::Context, frame: &mut eframe::Frame) {
    app.update_impl(ctx, frame);
}
