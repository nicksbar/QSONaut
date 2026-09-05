use super::super::*;

/// WASAPI and serial enumeration each take hundreds of milliseconds, which is
/// long enough to delay the first paint and leave a ghost window on Windows.
pub(crate) fn spawn_device_scan() -> mpsc::Receiver<DeviceInventory> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        info!("Device inventory scan started");
        let (serial_ports, serial_port_labels, detected_models) =
            radio_port_inventory(enumerate_serial_port_descriptors().unwrap_or_default());
        let inventory = DeviceInventory {
            audio_inputs: std::iter::once(NULL_INPUT_DEVICE.to_string())
                .chain(AudioService::input_devices().unwrap_or_default())
                .collect(),
            audio_outputs: std::iter::once(NULL_OUTPUT_DEVICE.to_string())
                .chain(AudioService::output_devices().unwrap_or_default())
                .collect(),
            serial_ports,
            serial_port_labels,
            detected_models,
        };
        info!(
            audio_inputs = inventory.audio_inputs.len(),
            audio_outputs = inventory.audio_outputs.len(),
            serial_ports = inventory.serial_ports.len(),
            detected_models = inventory.detected_models.len(),
            "Device inventory scan completed"
        );
        let _ = tx.send(inventory);
    });
    rx
}

pub(crate) fn audio_config_from_operator_profile(
    profile: &OperatorProfile,
    fallback: &AudioConfig,
) -> AudioConfig {
    if profile.profile_version < 3 {
        return fallback.clone();
    }
    AudioConfig {
        enabled: profile.audio.enabled,
        input_device: profile.audio.input_device.clone(),
        output_device: profile.audio.output_device.clone(),
        monitor_enabled: if profile.profile_version >= AUDIO_MONITOR_PROFILE_VERSION {
            profile.audio.monitor_enabled
        } else {
            fallback.monitor_enabled
        },
        monitor_output_device: if profile.profile_version >= AUDIO_MONITOR_PROFILE_VERSION {
            profile.audio.monitor_output_device.clone()
        } else {
            fallback.monitor_output_device.clone()
        },
        monitor_volume: if profile.profile_version >= AUDIO_MONITOR_PROFILE_VERSION {
            profile.audio.monitor_volume.clamp(0.0, 2.0)
        } else {
            fallback.monitor_volume
        },
        sample_rate_hz: profile.audio.sample_rate_hz,
        channels: profile.audio.channels,
    }
}

pub(crate) fn spawn_acceleration_probe(
    preference: ComputePreference,
) -> mpsc::Receiver<AccelerationReport> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let _ = tx.send(AccelerationReport::probe(preference));
    });
    rx
}

pub(crate) fn configure_unix_gui_environment() {
    #[cfg(target_os = "linux")]
    {
        if std::env::var_os("NO_AT_BRIDGE").is_none() {
            std::env::set_var("NO_AT_BRIDGE", "1");
        }
    }
}

impl QsonautGuiApp {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        config: AppConfig,
        cc: &eframe::CreationContext<'_>,
        app_icon: &egui::IconData,
        selected_renderer: eframe::Renderer,
        stored_geometry: Option<WindowGeometry>,
        graphics_preferences: GraphicsPreferences,
        active_graphics_adapter: Option<GraphicsAdapterInfo>,
        available_graphics_adapters: Vec<GraphicsAdapterInfo>,
        graphics_restart_request: Arc<Mutex<Option<GraphicsPreferences>>>,
    ) -> Self {
        Self::new_with_context(
            config,
            true,
            true,
            &cc.egui_ctx,
            app_icon,
            selected_renderer,
            stored_geometry,
            graphics_preferences,
            active_graphics_adapter,
            available_graphics_adapters,
            graphics_restart_request,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new_with_context(
        mut config: AppConfig,
        start_workers: bool,
        apply_saved_profile: bool,
        ctx: &egui::Context,
        app_icon: &egui::IconData,
        selected_renderer: eframe::Renderer,
        stored_geometry: Option<WindowGeometry>,
        graphics_preferences: GraphicsPreferences,
        active_graphics_adapter: Option<GraphicsAdapterInfo>,
        available_graphics_adapters: Vec<GraphicsAdapterInfo>,
        graphics_restart_request: Arc<Mutex<Option<GraphicsPreferences>>>,
    ) -> Self {
        // Keep egui's bundled font fallback chain active. It includes the
        // monochrome Noto Emoji and emoji icon fonts, making emoji rendering
        // independent of the host OS font installation.
        ctx.set_fonts(egui::FontDefinitions::default());
        let brand_image = ColorImage::from_rgba_unmultiplied(
            [app_icon.width as usize, app_icon.height as usize],
            &app_icon.rgba,
        );
        let brand_icon =
            ctx.load_texture("qsonaut-brand-icon", brand_image, TextureOptions::LINEAR);
        let available_profiles = list_operator_profiles();
        let active_profile_name = active_operator_profile_name();
        let selected_profile_name = available_profiles
            .iter()
            .find(|name| name.eq_ignore_ascii_case(&active_profile_name))
            .cloned()
            .unwrap_or_else(|| "Default".to_string());

        // Resolve the selected profile once, then apply that exact file. The
        // old path loaded the active profile before resolving the active name,
        // which made startup vulnerable to a stale/fallback profile source and
        // could boot a remote tab with another tab's native radio config.
        if apply_saved_profile {
            if let Some(profile) = load_operator_profile_named(&selected_profile_name) {
                if profile.profile_version >= 3 {
                    config.audio.input_device = profile.audio.input_device.clone();
                    config.audio.enabled = profile.audio.enabled;
                    config.audio.output_device = profile.audio.output_device.clone();
                    config.audio.sample_rate_hz = profile.audio.sample_rate_hz;
                    config.audio.channels = profile.audio.channels;
                    if profile.profile_version >= AUDIO_MONITOR_PROFILE_VERSION {
                        config.audio.monitor_enabled = profile.audio.monitor_enabled;
                        config.audio.monitor_output_device =
                            profile.audio.monitor_output_device.clone();
                        config.audio.monitor_volume = profile.audio.monitor_volume.clamp(0.0, 2.0);
                    }
                    config.radio.enabled = profile.radio.enabled;
                    config.radio.serial_port = profile.radio.serial_port.clone();
                    config.radio.backend = profile.radio.backend.clone();
                    config.radio.endpoint = profile.radio.endpoint.clone();
                    config.radio.hostbridge_access_key =
                        profile.radio.hostbridge_access_key.clone();
                    config.radio.hostbridge_password = profile.radio.hostbridge_password.clone();
                    config.radio.hostbridge_radio_id = profile.radio.hostbridge_radio_id.clone();
                    config.radio.hostbridge_audio_source_id =
                        profile.radio.hostbridge_audio_source_id.clone();
                    config.radio.hostbridge_audio_output_id =
                        profile.radio.hostbridge_audio_output_id.clone();
                    if config.radio.backend.trim().eq_ignore_ascii_case("none") {
                        config.radio.backend = "native".to_string();
                    }
                    if profile.profile_version >= 8 {
                        config.radio.model = profile.radio.model.clone();
                        config.radio.baud_rate = profile.radio.baud_rate;
                    }
                    config.radio.civ_address = profile.radio.civ_address;
                    config.radio.controller_civ_address = profile.radio.controller_civ_address;
                }
            }
        }

        info!(
            profile = %selected_profile_name,
            backend = %config.radio.backend,
            model = %config.radio.model,
            serial_port = ?config.radio.serial_port,
            audio_input = ?config.audio.input_device,
            audio_output = ?config.audio.output_device,
            "Loaded active operator profile for startup"
        );

        let state = Arc::new(Mutex::new(GuiState::default()));
        if let Some(profile) = load_operator_profile_named(&selected_profile_name) {
            if let Ok(mut state) = state.lock() {
                state.recording_enabled = profile.recording_enabled;
                state.recording_modes = profile
                    .recording_modes
                    .iter()
                    .filter(|(_, enabled)| **enabled)
                    .filter_map(|(mode, _)| parse_workspace_mode_token(mode))
                    .collect();
                state.recording_full_width = profile.recording_full_width;
                state.recording_stream = profile.recording_stream;
            }
        }
        let app_events = AppEventBus::new(256);
        let automation_event_rx = app_events.subscribe();
        let (automation_host, automation_status, automation_external_transports) =
            bootstrap_automation_host();
        let radio_worker_stop = Arc::new(AtomicBool::new(false));
        let swr_sweep_abort = Arc::new(AtomicBool::new(false));
        let audio_worker_stop = Arc::new(AtomicBool::new(false));
        let display_tuning = Arc::new(Mutex::new(DisplayTuning::default()));

        let repaint_ctx: Arc<OnceLock<egui::Context>> = Arc::new(OnceLock::new());

        // Spawn radio initialization on a background thread to avoid blocking UI appearance
        let (radio_init_rx, radio_waterfall_status_init) = if config.radio.enabled {
            let port = config.radio.serial_port.clone().unwrap_or_default();
            let rx = spawn_radio_init_with_hostbridge(
                config.radio.backend.clone(),
                config.radio.model.clone(),
                port,
                config.radio.endpoint.clone(),
                config.radio.baud_rate,
                config.radio.controller_civ_address,
                config.radio.civ_address,
                config.radio.hostbridge_access_key.clone(),
                config.radio.hostbridge_password.clone(),
                config.radio.hostbridge_radio_id.clone(),
                config.radio.hostbridge_audio_source_id.clone(),
                config.radio.hostbridge_audio_output_id.clone(),
            );
            (Some(rx), "CONNECTING…".to_string())
        } else {
            (None, "UNAVAILABLE (radio disabled)".to_string())
        };

        // Set initial radio status
        {
            let mut s = state.lock().expect("ui state lock poisoned");
            s.radio_waterfall_status = radio_waterfall_status_init;
            if radio_init_rx.is_none() {
                s.last_error = Some(
                    "Radio is disabled in config; UI running in monitor-only mode".to_string(),
                );
            }
        }

        let (command_tx, radio_worker_handle) = (None, None);

        // Every saved profile owns a live runtime. Inactive tabs continue
        // receiving and decoding, but their PTT path is disabled.
        let mut parked_radio_sessions = HashMap::new();
        if start_workers {
            for profile_name in &available_profiles {
                if profile_name == &selected_profile_name {
                    continue;
                }
                let Some(profile) = load_operator_profile_named(profile_name) else {
                    continue;
                };
                let session_config = radio_config_from_operator_profile(&profile);
                let session_audio_config =
                    audio_config_from_operator_profile(&profile, &config.audio);
                let session_state = Arc::new(Mutex::new(GuiState::default()));
                // Parked tabs are deliberately inert at startup. Opening
                // their CI-V device and audio stream here creates hidden
                // hardware I/O, FFT work, and decoder load before the user
                // has selected those tabs.
                let init_rx = None;
                let status = "PARKED (select tab to start)";
                if let Ok(mut state) = session_state.lock() {
                    state.radio_waterfall_status = status.to_string();
                    state.workspace_mode = parse_workspace_mode_token(&profile.workspace_mode)
                        .unwrap_or(WorkspaceMode::Ft8);
                    state.ft8_deep_decode = profile.deep_decode;
                    state.ft4_deep_decode = profile.ft4_deep_decode;
                    state.selected_audio_hz = profile.rx_tone_hz;
                    state.cw_wpm = profile.cw_wpm.clamp(5, 40);
                    state.recording_enabled = profile.recording_enabled;
                    state.recording_modes = profile
                        .recording_modes
                        .iter()
                        .filter(|(_, enabled)| **enabled)
                        .filter_map(|(mode, _)| parse_workspace_mode_token(mode))
                        .collect();
                    state.recording_full_width = profile.recording_full_width;
                    state.recording_stream = profile.recording_stream;
                    state.radio_spectrum_desired = profile.civ_spectrum_on;
                    state.radio_scope_vbw_wide = profile.radio_scope_vbw_wide;
                    state.radio_scope_view = profile.radio_scope_view;
                }
                let session_audio_worker_stop = Arc::new(AtomicBool::new(false));
                let session_swr_sweep_abort = Arc::new(AtomicBool::new(false));
                let session_display_tuning = Arc::new(Mutex::new(DisplayTuning::default()));
                let session_monitor_volume = Arc::new(AtomicU32::new(
                    session_audio_config.monitor_volume.to_bits(),
                ));
                let session_ft8_tx_active = Arc::new(AtomicBool::new(false));
                let session_digital_tx_active = Arc::new(AtomicBool::new(false));
                let session_ptt_allowed = Arc::new(AtomicBool::new(false));
                let session_audio_worker_handle = None;
                info!(
                    profile = profile_name,
                    radio_enabled = profile.radio.enabled,
                    audio_enabled = profile.audio.enabled,
                    "Profile runtime initialization queued"
                );
                parked_radio_sessions.insert(
                    profile_name.clone(),
                    RadioSession {
                        profile,
                        view_state: TabViewState::default(),
                        config: session_config,
                        audio_config: session_audio_config,
                        state: session_state,
                        command_tx: None,
                        worker_stop: Arc::new(AtomicBool::new(false)),
                        audio_worker_stop: session_audio_worker_stop,
                        swr_sweep_abort: session_swr_sweep_abort,
                        display_tuning: session_display_tuning,
                        monitor_volume: session_monitor_volume,
                        ft8_tx_active: session_ft8_tx_active,
                        digital_tx_active: session_digital_tx_active,
                        ptt_allowed: session_ptt_allowed,
                        init_rx,
                        init_attempted: false,
                        worker_handle: None,
                        audio_worker_handle: session_audio_worker_handle,
                    },
                );
            }
        }

        let ft8_tx_active = Arc::new(AtomicBool::new(false));
        let ptt_allowed = Arc::new(AtomicBool::new(true));
        let digital_tx_active = Arc::new(AtomicBool::new(false));
        let monitor_volume = Arc::new(AtomicU32::new(config.audio.monitor_volume.to_bits()));
        let audio_worker_handle = start_workers.then(|| {
            spawn_audio_spectrum_worker(
                state.clone(),
                audio_worker_stop.clone(),
                ft8_tx_active.clone(),
                digital_tx_active.clone(),
                config.audio.enabled,
                true,
                config.audio.sample_rate_hz,
                config.audio.channels,
                effective_audio_input_device(
                    &config.radio.backend,
                    config.audio.input_device.clone(),
                ),
                config.audio.monitor_enabled,
                effective_audio_output_device(
                    &config.radio.backend,
                    config
                        .audio
                        .monitor_output_device
                        .clone()
                        .or_else(|| config.audio.output_device.clone()),
                ),
                monitor_volume.clone(),
                repaint_ctx.clone(),
                display_tuning.clone(),
            )
        });

        let global_settings = load_global_settings();
        let station_callsign = global_settings.callsign.clone();
        let station_grid = global_settings.grid.clone();

        let station_qth = global_settings.qth.clone();
        let mut station_rig = global_settings.station_rig.clone();
        let mut station_antenna = global_settings.station_antenna.clone();
        let station_notes = global_settings.station_notes.clone();
        let llm_prompt_context = global_settings.llm_prompt_context.clone();
        let sstv_image_requirements = global_settings.sstv_image_requirements.clone();
        let llm_model_notes = global_settings.llm_model_notes.clone();
        let mut ft8_follow_log = true;
        let mut ft8_max_log_entries = 300usize;
        let mut ft8_deep_decode = false;
        let mut ft4_deep_decode = false;
        let mut ft4_autoseq = false;
        let mut ft4_auto_reply_policy = AutoReplyPolicy::default();
        let mut ft4_cq_only_view = false;
        let mut ft4_follow_log = true;
        let mut ft4_max_log_entries = 300usize;
        let mut ft4_max_attempts = default_ft8_max_attempts();
        let mut ft8_autoseq = false;
        let mut ft8_auto_reply_policy = AutoReplyPolicy::default();
        let mut ft8_auto_answer_cq = false;
        let mut automation_unlocked = false;
        let mut ft8_cq_only_view = false;
        let mut civ_spectrum_on = false;
        let mut radio_scope_vbw_wide = false;
        let mut radio_scope_view = RadioScopeView::Narrow;
        let mut waterfall_theme = WaterfallTheme::default();
        let mut radio_waterfall_theme = WaterfallTheme::default();
        let mut waterfall_deck_height = default_waterfall_deck_height();
        let ft8_stop_policy = AutoTxStopPolicy::Continuous;
        let mut ft8_max_attempts = default_ft8_max_attempts();
        let mut ft8_hold_tx_freq = false;
        let mut rx_tone_hz = default_rx_tone_hz();
        let mut tx_tone_hz = default_tx_tone_hz();
        let mut ptt_lead_ms = default_ptt_lead_ms();
        let mut ptt_tail_ms = default_ptt_tail_ms();
        let mut cw_wpm = default_cw_wpm();
        let mut cw_tone_hz = default_cw_tone_hz();
        let mut recording_enabled = false;
        let mut recording_modes = std::collections::BTreeMap::new();
        let mut recording_full_width = true;
        let mut recording_stream = true;
        let gui_scale = global_settings.gui_scale;
        let compute_preference = global_settings.compute_preference;
        let mut psk_reporter_enabled = false;
        let mut pota_enabled = true;
        let mut psk_batch_interval_secs = default_psk_batch_interval_secs();
        let mut psk_repeat_cache_secs = default_psk_repeat_cache_secs();
        let mut psk_max_pending = default_psk_max_pending();
        let mut server_instance_id = new_instance_id();
        let mut contest_enabled = config.contest.enabled;
        let mut contest_operating_mode = config.contest.operating_mode;
        let mut contest_split_policy = config.contest.split_policy;
        let mut contest_fox_hound_role = config.contest.fox_hound_role;
        let mut contest_exchange_template =
            config.contest.exchange_template.clone().unwrap_or_default();
        let mut contest_serial_start = config.contest.serial_start.max(1);
        let mut contest_serial_step = config.contest.serial_step.max(1);
        let mut contest_dupe_check = config.contest.dupe_check;
        let mut contest_serial_current = contest_serial_start;
        let mut contest_fake_split_offset_hz = default_contest_fake_split_offset_hz();
        let mut hunter_unlocked = HashSet::new();
        let mut hunter_acknowledged = HashSet::new();
        let mut hunter_alerts_enabled = true;
        let mut hunter_custom_rules = Vec::new();
        let mut radio_profiles = load_radio_profile_library();
        let mut mode_radio_profile = std::collections::BTreeMap::new();
        let mut workspace_mode = WorkspaceMode::Ft8;
        let profile_io_status: String;

        if let Some(p) = load_operator_profile() {
            // Older profiles kept station data in global settings. Prefer the
            // profile values once present, while allowing a one-time migration
            // for existing installations.
            if p.station_rig.is_empty() && !global_settings.station_rig.is_empty() {
                station_rig = global_settings.station_rig.clone();
            } else {
                station_rig = p.station_rig.clone();
            }
            if p.station_antenna.is_empty() && !global_settings.station_antenna.is_empty() {
                station_antenna = global_settings.station_antenna.clone();
            } else {
                station_antenna = p.station_antenna.clone();
            }
            ft8_follow_log = p.follow_log;
            ft8_max_log_entries = p.max_log_entries.clamp(80, 1000);
            ft8_deep_decode = p.deep_decode;
            ft4_deep_decode = p.ft4_deep_decode;
            // Transmit automation is never restored as armed at startup.
            ft4_autoseq = false;
            ft4_auto_reply_policy = p.ft4_auto_reply_policy;
            ft4_cq_only_view = p.ft4_cq_only_view;
            ft4_follow_log = p.ft4_follow_log;
            ft4_max_log_entries = p.ft4_max_log_entries.clamp(80, 300);
            ft4_max_attempts = p.ft4_max_attempts.clamp(1, 20);
            // Transmit automation is never restored as armed at startup.
            ft8_autoseq = false;
            ft8_auto_reply_policy = p.auto_reply_policy;
            // Unattended CQ answering must be explicitly re-enabled each run.
            ft8_auto_answer_cq = false;
            automation_unlocked = p.automation_unlocked;
            ft8_cq_only_view = p.cq_only_view;
            civ_spectrum_on = p.civ_spectrum_on;
            radio_scope_vbw_wide = p.radio_scope_vbw_wide;
            radio_scope_view = p.radio_scope_view;
            waterfall_theme = p.waterfall_theme;
            radio_waterfall_theme = p.radio_waterfall_theme;
            waterfall_deck_height = p.waterfall_deck_height.clamp(170.0, 560.0);
            ft8_max_attempts = p.ft8_max_attempts.clamp(1, 20);
            ft8_hold_tx_freq = if p.profile_version >= 3 {
                p.hold_tx_freq
            } else {
                false
            };
            rx_tone_hz = p.rx_tone_hz;
            tx_tone_hz = p.tx_tone_hz;
            if !ft8_hold_tx_freq {
                tx_tone_hz = rx_tone_hz;
            }
            ptt_lead_ms = p.ptt_lead_ms.clamp(0, 500);
            ptt_tail_ms = p.ptt_tail_ms.clamp(0, 500);
            cw_wpm = p.cw_wpm.clamp(5, 40);
            cw_tone_hz = p.cw_tone_hz.clamp(200, 3_000);
            recording_enabled = p.recording_enabled;
            recording_modes = p.recording_modes;
            recording_full_width = p.recording_full_width;
            recording_stream = p.recording_stream;
            psk_reporter_enabled = p.psk_reporter_enabled;
            pota_enabled = p.pota_enabled;
            psk_batch_interval_secs = p.psk_batch_interval_secs.clamp(60, 3_600);
            psk_repeat_cache_secs = p.psk_repeat_cache_secs.clamp(60, 3_600);
            psk_max_pending = p.psk_max_pending.clamp(1, 2_048);
            server_instance_id = p.server_instance_id;
            if let Some(server) = p.server {
                config.server = server;
            }
            contest_enabled = p.contest_enabled;
            contest_operating_mode = p.contest_operating_mode;
            contest_split_policy = p.contest_split_policy;
            contest_fox_hound_role = p.contest_fox_hound_role;
            contest_exchange_template = p.contest_exchange_template;
            contest_serial_start = p.contest_serial_start.max(1);
            contest_serial_step = p.contest_serial_step.max(1);
            contest_dupe_check = p.contest_dupe_check;
            contest_serial_current = p.contest_serial_current.max(contest_serial_start).max(1);
            contest_fake_split_offset_hz = p.contest_fake_split_offset_hz.clamp(0, 2_000);
            hunter_unlocked = p.hunter_unlocked.into_iter().collect();
            hunter_acknowledged = p.hunter_acknowledged.into_iter().collect();
            hunter_alerts_enabled = p.hunter_alerts_enabled;
            hunter_custom_rules = p.hunter_custom_rules;
            if radio_profiles.is_empty() {
                radio_profiles = p.radio_profiles.clone();
                for name in list_operator_profiles() {
                    if let Some(legacy_profile) = load_operator_profile_named(&name) {
                        for candidate in legacy_profile.radio_profiles {
                            if !radio_profiles
                                .iter()
                                .any(|profile| profile.name == candidate.name)
                            {
                                radio_profiles.push(candidate);
                            }
                        }
                    }
                }
                if !radio_profiles.is_empty() {
                    if let Err(error) = save_radio_profile_library(&radio_profiles) {
                        warn!(%error, "Failed to migrate radio profiles to global library");
                    } else {
                        info!(
                            count = radio_profiles.len(),
                            "Migrated radio profiles to global library"
                        );
                    }
                }
            }
            mode_radio_profile = p.mode_radio_profile;
            workspace_mode =
                parse_workspace_mode_token(&p.workspace_mode).unwrap_or(WorkspaceMode::Ft8);
            config.station.callsign = Some(station_callsign.clone());
            config.station.grid = Some(station_grid.clone());
            config.contest = ContestProfile {
                enabled: contest_enabled,
                operating_mode: contest_operating_mode,
                split_policy: contest_split_policy,
                fox_hound_role: contest_fox_hound_role,
                exchange_template: if contest_exchange_template.trim().is_empty() {
                    None
                } else {
                    Some(contest_exchange_template.trim().to_string())
                },
                serial_start: contest_serial_start,
                serial_step: contest_serial_step,
                dupe_check: contest_dupe_check,
            };
            profile_io_status = format!("Loaded {}", OPERATOR_PROFILE_FILE);
        } else {
            let bootstrap = OperatorProfile {
                profile_version: OPERATOR_PROFILE_VERSION,
                callsign: station_callsign.clone(),
                grid: station_grid.clone(),
                qth: station_qth.clone(),
                station_rig: station_rig.clone(),
                station_antenna: station_antenna.clone(),
                station_notes: station_notes.clone(),
                llm_prompt_context: llm_prompt_context.clone(),
                sstv_image_requirements: sstv_image_requirements.clone(),
                llm_model_notes: llm_model_notes.clone(),
                follow_log: ft8_follow_log,
                max_log_entries: ft8_max_log_entries,
                deep_decode: ft8_deep_decode,
                ft4_deep_decode,
                ft4_autoseq,
                ft4_auto_reply_policy,
                ft4_cq_only_view,
                ft4_follow_log,
                ft4_max_log_entries,
                ft4_max_attempts,
                autoseq: ft8_autoseq,
                auto_reply_policy: ft8_auto_reply_policy,
                auto_answer_cq: ft8_auto_answer_cq,
                automation_unlocked,
                cq_only_view: ft8_cq_only_view,
                civ_spectrum_on,
                radio_scope_vbw_wide,
                radio_scope_view,
                waterfall_theme,
                radio_waterfall_theme,
                waterfall_auto_visual: true,
                waterfall_speed: WaterfallSpeed::Mid,
                waterfall_deck_height,
                halt_after_tx: false,
                ft8_max_attempts,
                hold_tx_freq: ft8_hold_tx_freq,
                rx_tone_hz,
                tx_tone_hz,
                ptt_lead_ms,
                ptt_tail_ms,
                cw_wpm,
                cw_tone_hz,
                cw_auto_target_timeout_s: 3,
                recording_enabled,
                recording_modes: recording_modes.clone(),
                recording_full_width,
                recording_stream,
                audio: profile::AudioProfileSettings {
                    input_device: config.audio.input_device.clone(),
                    enabled: config.audio.enabled,
                    output_device: config.audio.output_device.clone(),
                    monitor_enabled: config.audio.monitor_enabled,
                    monitor_output_device: config.audio.monitor_output_device.clone(),
                    monitor_volume: config.audio.monitor_volume.clamp(0.0, 2.0),
                    sample_rate_hz: config.audio.sample_rate_hz,
                    channels: config.audio.channels,
                },
                radio: profile::RadioProfileSettings {
                    enabled: config.radio.enabled,
                    serial_port: config.radio.serial_port.clone(),
                    backend: config.radio.backend.clone(),
                    endpoint: config.radio.endpoint.clone(),
                    model: config.radio.model.clone(),
                    baud_rate: config.radio.baud_rate,
                    civ_address: config.radio.civ_address,
                    controller_civ_address: config.radio.controller_civ_address,
                    hostbridge_access_key: config.radio.hostbridge_access_key.clone(),
                    hostbridge_password: config.radio.hostbridge_password.clone(),
                    hostbridge_radio_id: config.radio.hostbridge_radio_id.clone(),
                    hostbridge_audio_source_id: config.radio.hostbridge_audio_source_id.clone(),
                    hostbridge_audio_output_id: config.radio.hostbridge_audio_output_id.clone(),
                },
                gui_scale,
                compute_preference,
                psk_reporter_enabled,
                pota_enabled,
                psk_batch_interval_secs,
                psk_repeat_cache_secs,
                psk_max_pending,
                server_instance_id: server_instance_id.clone(),
                server: Some(config.server.clone()),
                contest_enabled,
                contest_operating_mode,
                contest_split_policy,
                contest_fox_hound_role,
                contest_exchange_template: contest_exchange_template.trim().to_string(),
                contest_serial_start,
                contest_serial_step,
                contest_dupe_check,
                contest_serial_current,
                contest_fake_split_offset_hz,
                hunter_unlocked: Vec::new(),
                hunter_acknowledged: Vec::new(),
                hunter_alerts_enabled: true,
                hunter_custom_rules: Vec::new(),
                radio_profiles: Vec::new(),
                mode_radio_profile: std::collections::BTreeMap::new(),
                workspace_mode: workspace_mode.label().to_string(),
            };
            match save_operator_profile(&bootstrap) {
                Ok(_) => {
                    profile_io_status = format!("Created {}", OPERATOR_PROFILE_FILE);
                }
                Err(err) => {
                    profile_io_status = format!("Profile init failed: {err}");
                }
            }
        }

        if server_instance_id.is_empty() {
            server_instance_id = new_instance_id();
        }

        let server_client = (config.server.enabled
            && !config.server.url.trim().is_empty()
            && !config.server.device_token.trim().is_empty())
        .then(|| {
            ServerClient::spawn(ServerConnectionConfig {
                server_url: config.server.url.trim().to_string(),
                device_token: config.server.device_token.trim().to_string(),
                client_version: env!("CARGO_PKG_VERSION").to_string(),
                queue_path: app_config_dir().join("server-log-queue.json"),
                share_logs: config.server.share_logs,
            })
        });

        if !native_radio_profile(&config.radio.backend, &config.radio.model)
            .is_some_and(|profile| profile.capabilities.spectrum)
        {
            civ_spectrum_on = false;
        }

        let (ft8_tx_event_tx, ft8_tx_event_rx) = mpsc::channel();
        let (digital_tx_event_tx, digital_tx_event_rx) = mpsc::channel();
        let (local_image_event_tx, local_image_event_rx) = mpsc::channel();
        let (qso_log, qso_log_status) = match QsoLog::load(&qso_log_path()) {
            Ok(log) => {
                let count = log.contacts.len();
                (log, format!("Loaded {count} contacts"))
            }
            Err(error) => (QsoLog::default(), format!("Log load failed: {error}")),
        };

        let acceleration_report = AccelerationReport::pending(compute_preference);
        let acceleration_probe = Some(spawn_acceleration_probe(compute_preference));
        let (os_dpi_adjustment, os_name, os_dpi_env) = os_dpi_adjustment();
        info!(
            os = os_name,
            adjustment = os_dpi_adjustment,
            env = os_dpi_env,
            "Using OS DPI adjustment"
        );
        // Applied before the first paint so the window is never laid out at one
        // scale and immediately re-laid out at another.
        ctx.set_zoom_factor(gui_scale * os_dpi_adjustment);
        let psk_reporter = start_psk_reporter(
            psk_reporter_enabled,
            &config.radio.backend,
            &station_callsign,
            &station_grid,
            ReporterTuning {
                batch_interval_secs: psk_batch_interval_secs,
                repeat_cache_secs: psk_repeat_cache_secs,
                max_pending: psk_max_pending,
            },
            &state,
        );
        let psk_sender = psk_reporter.as_ref().map(Reporter::sender);
        for session in parked_radio_sessions.values() {
            if let Ok(mut session_state) = session.state.lock() {
                session_state.psk_report_sender = (!matches!(
                    session.config.backend.trim().to_ascii_lowercase().as_str(),
                    "null" | "mock" | "none"
                ))
                .then(|| psk_sender.clone())
                .flatten();
            }
        }

        Self {
            config,
            app_events,
            automation_event_rx,
            automation_host,
            automation_status,
            last_radio_state_signature: None,
            automation_external_transports,
            automation_external_outbox: VecDeque::new(),
            hunter_unlocked,
            hunter_acknowledged,
            hunter_show_acknowledged: false,
            hunter_alerts_enabled,
            hunter_feed: VecDeque::new(),
            hunter_unique_heard: HashSet::new(),
            hunter_directed_hits: 0,
            hunter_dupe_blocks: 0,
            hunter_decode_bursts: 0,
            hunter_custom_rules,
            radio_profiles,
            mode_radio_profile,
            radio_profile_name_input: String::new(),
            hunter_custom_title_input: String::new(),
            hunter_custom_detail_input: String::new(),
            hunter_custom_metric_input: HunterMetric::UniqueHeard,
            hunter_custom_threshold_input: 1,
            hunter_custom_enabled_input: true,
            external_ingress_source: "discord:shack".to_string(),
            external_ingress_author: "operator".to_string(),
            external_ingress_channel: "#qsonaut".to_string(),
            external_ingress_message: "!rig".to_string(),
            state,
            driver_metadata: DriverMetadata::default(),
            command_tx,
            radio_worker_stop,
            swr_sweep_abort,
            audio_worker_stop,
            radio_init_rx,
            cat_test_rx: None,
            cat_test_status: None,
            cat_test_restart_radio: false,
            hamdb_lookup_rx: None,
            hamdb_profile_lookup_rx: None,
            pota_spots: Vec::new(),
            pota_lookup_rx: None,
            pota_last_lookup: Instant::now() - Duration::from_secs(60),
            pota_history: VecDeque::new(),
            pota_last_updated: None,
            pota_last_error: None,
            logo_clicks: VecDeque::new(),
            logo_spin_until: None,
            radio_init_attempted: false,
            radio_worker_handle,
            parked_radio_sessions,
            audio_worker_handle,
            radio_waterfall_texture: None,
            radio_waterfall_texture_revision: 0,
            radio_waterfall_texture_bins: 0,
            radio_waterfall_texture_view: RadioScopeView::Narrow,
            radio_waterfall_texture_theme: WaterfallTheme::RadioBlue,
            audio_waterfall_texture: None,
            audio_waterfall_texture_revision: 0,
            audio_waterfall_texture_bins: 0,
            audio_waterfall_texture_theme: WaterfallTheme::RadioBlue,
            audio_waterfall_cached_rows: VecDeque::new(),
            audio_waterfall_cached_source_revision: 0,
            sstv_texture: None,
            sstv_texture_revision: 0,
            sstv_tx_armed: false,
            sstv_tuning_offset_hz: 0,
            sstv_auto_target: true,
            sstv_rx_width_percent: 43,
            sstv_tx_rgb: Vec::new(),
            sstv_tx_base_rgb: Vec::new(),
            sstv_tx_width: qsonaut_sstv::WIDTH,
            sstv_tx_height: qsonaut_sstv::HEIGHT,
            sstv_tx_revision: 0,
            sstv_tx_texture: None,
            sstv_tx_texture_revision: 0,
            sstv_tx_mode: qsonaut_sstv::SstvMode::MartinM1,
            sstv_overlay_callsign: true,
            sstv_overlay_grid: true,
            sstv_overlay_frequency: false,
            sstv_overlay_mode: true,
            sstv_overlay_corner: SstvOverlayCorner::BottomLeft,
            sstv_overlay_background: Color32::BLACK,
            sstv_overlay_background_opacity: 0.65,
            sstv_background_zoom: 1.0,
            sstv_background_pan_x: 0.0,
            sstv_background_pan_y: 0.0,
            sstv_overlay_revision: 0,
            sstv_file_dialog: egui_file_dialog::FileDialog::new(),
            sstv_image_path: String::new(),
            sstv_ai_prompt: String::new(),
            local_image_settings: LocalImageSettings::load(),
            local_image_models: Vec::new(),
            local_image_status: "Local image server not checked".to_string(),
            local_image_refresh_started: false,
            local_image_event_tx,
            local_image_event_rx,
            sstv_ai_pipeline_mode: SstvAiPipelineMode::StationQsl,
            sstv_active_received_id: None,
            sstv_show_prior_received: false,
            sstv_received_textures: HashMap::new(),
            sstv_received_texture_revision: 0,
            sstv_reinterpret_prompt: String::new(),
            workspace_mode,
            activity: OperatingActivity::General,
            fst4_submode: modes::fst4::Submode::default(),
            cw_auto_target_timeout_s: 3,
            q65_submode: qsonaut_third_party::wsjt::Q65Submode::A30,
            display_tuning,
            repaint_ctx,
            ft8_log: Vec::new(),
            ft8_tx_chat: VecDeque::new(),
            ft8_seen_decode_period: None,
            qso_log,
            qso_selected: None,
            qso_log_status,
            qso_log_dirty: false,
            ft8_compose: String::new(),
            ft8_selected: None,
            ft8_autoseq,
            ft8_auto_reply_policy,
            ft8_auto_answer_cq,
            automation_unlocked,
            ft8_session: None,
            ft8_seq_state: Ft8SeqState::Idle,
            ft8_seq_target: None,
            ft8_seq_status: "🌙 RX deck ready · listening for signals".to_string(),
            ft8_last_click: None,
            ft8_tx_queued_period: None,
            ft8_tx_pcm: None,
            ft8_queued_tx_message: None,
            ft8_last_tx_message: None,
            ft8_tx_started_period: None,
            ft8_last_tx_period: None,
            ft8_suppress_canceled_tx_events: false,
            ft8_pending_manual_reply: None,
            ft8_tx_abort: Arc::new(AtomicBool::new(false)),
            ft8_tx_active,
            ptt_allowed,
            ft8_tx_event_tx,
            ft8_tx_event_rx,
            ft8_last_tx_was_cq: false,
            digital_compose: String::new(),
            digital_selected: None,
            digital_last_click: None,
            digital_seq_target: None,
            ft4_session: None,
            ft4_seen_decodes: HashSet::new(),
            digital_tx_chat: VecDeque::new(),
            digital_queued_tx_message: None,
            digital_last_tx_message: None,
            digital_tx_status: "🌊 RX deck ready · listening for signals".to_string(),
            digital_tx_started: None,
            ft4_last_tx_period: None,
            ft4_seen_decode_period: None,
            digital_tx_abort: Arc::new(AtomicBool::new(false)),
            digital_tx_active,
            digital_suppress_canceled_tx_events: false,
            digital_tx_event_tx,
            digital_tx_event_rx,
            monitor_volume,
            native_autoseq_mode: None,
            native_auto_reply_policy: AutoReplyPolicy::default(),
            native_stop_policy: AutoTxStopPolicy::Continuous,
            native_sessions: HashMap::new(),
            native_seen_decodes: HashMap::new(),
            native_last_tx_periods: HashMap::new(),
            native_attempts: HashMap::new(),
            ft8_stop_policy,
            ft8_max_attempts,
            ft4_stop_policy: AutoTxStopPolicy::Continuous,
            ft4_max_attempts,
            ft8_hold_tx_freq,
            ft8_deep_decode,
            ft4_deep_decode,
            ft4_autoseq,
            ft4_auto_reply_policy,
            ft4_cq_only_view,
            ft4_follow_log,
            ft4_max_log_entries,
            ft8_cq_only_view,
            ft8_follow_log,
            ft8_max_log_entries,
            station_callsign,
            station_grid,
            station_qth,
            station_rig,
            station_antenna,
            station_notes,
            llm_prompt_context,
            sstv_image_requirements,
            llm_model_notes,
            voice_callsign: String::new(),
            voice_grid: String::new(),
            voice_state: String::new(),
            voice_rst_sent: "59".to_string(),
            voice_rst_received: "59".to_string(),
            voice_contest_serial_sent: String::new(),
            voice_contest_serial_received: String::new(),
            voice_notes: String::new(),
            voice_contest_fields: Vec::new(),
            voice_qso_started_at: None,
            voice_lookup_requested: String::new(),
            voice_lookup_status: String::new(),
            voice_hamdb: None,
            contest_enabled,
            contest_operating_mode,
            contest_split_policy,
            contest_fox_hound_role,
            contest_exchange_template,
            contest_serial_start,
            contest_serial_step,
            contest_dupe_check,
            contest_serial_current,
            contest_fake_split_offset_hz,
            civ_spectrum_on,
            rx_tone_hz,
            tx_tone_hz,
            ptt_lead_ms,
            ptt_tail_ms,
            cw_wpm,
            cw_tone_hz,
            cw_qso_callsign: String::new(),
            cw_qso_rst_sent: "599".to_string(),
            cw_qso_rst_received: "599".to_string(),
            cw_qso_exchange_received: String::new(),
            cw_qso_notes: String::new(),
            cw_qso_started_at: None,
            recording_enabled,
            recording_modes,
            recording_full_width,
            recording_stream,
            selected_profile_name,
            new_profile_name: String::new(),
            new_profile_tab_editing: false,
            pending_profile_delete: None,
            available_profiles,
            profile_io_status,
            profile_dirty: false,
            app_log_text: String::new(),
            app_log_status: String::new(),
            app_log_filter: String::new(),
            app_log_level_filter: AppLogLevelFilter::All,
            app_log_follow: true,
            app_log_last_refresh: Instant::now() - Duration::from_secs(1),
            audio_input_devices: Vec::new(),
            audio_output_devices: Vec::new(),
            radio_serial_ports: Vec::new(),
            radio_serial_port_labels: HashMap::new(),
            radio_detected_models: Vec::new(),
            device_scan: start_workers.then(spawn_device_scan),
            hostbridge_catalog: None,
            hostbridge_scan: None,
            hostbridge_scan_status: String::new(),
            radio_scope_contrast: 1.2,
            radio_scope_span_code: 0,
            radio_scope_vbw_wide,
            radio_scope_hold: false,
            radio_scope_reference_tenths_db: 0,
            radio_scope_view,
            radio_scope_lock_if_to_filter: true,
            waterfall_theme,
            radio_waterfall_theme,
            waterfall_deck_height,
            show_signal_panel: true,
            show_meter_panel: false,
            meter_panel_was_tx: false,
            meter_panel_close_deadline: None,
            show_profile_drawer: false,
            profile_drawer_anchor: None,
            radio_faq_window_open: false,
            radio_guide_window_open: false,
            radio_faq_document: RadioHelpDocument::Model,
            radio_guide_document: RadioHelpDocument::Model,
            radio_help_window_model: String::new(),
            profile_drawer_tab: ProfileDrawerTab::Profile,
            signal_panel_tab: SignalPanelTab::Achievements,
            device_restart_required: false,
            audio_restart_required: false,
            gui_scale,
            os_dpi_adjustment,
            graphics_active: graphics_preferences.clone(),
            graphics_pending: graphics_preferences,
            active_graphics_adapter,
            available_graphics_adapters,
            graphics_restart_request,
            compute_preference,
            acceleration_report,
            acceleration_probe,
            psk_reporter_enabled,
            pota_enabled,
            psk_batch_interval_secs,
            psk_repeat_cache_secs,
            psk_max_pending,
            psk_reporter,
            server_client,
            server_active_club: None,
            server_active_event: None,
            server_instance_id,
            server_last_presence: Instant::now() - Duration::from_secs(60),
            brand_icon,
            selected_renderer,
            first_frame_logged: false,
            last_viewport_log: None,
            window_geometry: stored_geometry,
        }
    }
}
