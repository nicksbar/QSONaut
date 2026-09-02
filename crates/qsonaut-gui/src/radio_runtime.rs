use super::*;

/// Initialize a configured radio off the UI thread and verify native radios
/// with a real CI-V transaction before reporting success.
#[allow(dead_code)]
pub(super) fn spawn_radio_init(
    backend: String,
    model: String,
    port: String,
    endpoint: String,
    baud_rate: u32,
    controller_civ_address: u8,
    radio_civ_address: u8,
) -> mpsc::Receiver<Option<RadioHandle>> {
    spawn_radio_init_with_hostbridge(
        backend,
        model,
        port,
        endpoint,
        baud_rate,
        controller_civ_address,
        radio_civ_address,
        String::new(),
        String::new(),
        None,
        None,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn spawn_radio_init_with_hostbridge(
    backend: String,
    model: String,
    port: String,
    endpoint: String,
    baud_rate: u32,
    controller_civ_address: u8,
    radio_civ_address: u8,
    hostbridge_access_key: String,
    hostbridge_password: String,
    hostbridge_radio_id: Option<String>,
    hostbridge_audio_source_id: Option<String>,
    hostbridge_audio_output_id: Option<String>,
) -> mpsc::Receiver<Option<RadioHandle>> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let start = std::time::Instant::now();
        let _timeout = std::time::Duration::from_secs(5);

        let radio = match backend.trim().to_ascii_lowercase().as_str() {
            "none" => None,
            "null" | "mock" => Some(open_null().into()),
            "rigctld" | "rigctl" => Some(open_rigctld(endpoint).into()),
            "dxlab" | "dxlab-commander" | "commander" => Some(open_dxlab(endpoint).into()),
            "hostbridge" => {
                let config = HostBridgeConfig {
                    endpoint,
                    client_name: "QSONaut desktop".into(),
                    access_key: hostbridge_access_key,
                    password: hostbridge_password,
                    radio_device_id: hostbridge_radio_id,
                    audio_source_id: hostbridge_audio_source_id,
                    audio_output_id: hostbridge_audio_output_id,
                    ..Default::default()
                };
                match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime
                        .block_on(HostBridgeRadio::connect(config))
                        .map(|radio| RadioHandle::Remote(Box::new(radio)))
                        .ok(),
                    Err(error) => {
                        error!(%error, "HostBridge startup runtime failed");
                        None
                    }
                }
            }
            "native" => match open_model_with_radio_address(
                &model,
                &port,
                baud_rate,
                controller_civ_address,
                Some(radio_civ_address),
            ) {
                Ok(radio) => match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => match runtime.block_on(radio.get_frequency_hz()) {
                        Ok(frequency_hz) => {
                            info!(
                                model = %model,
                                port = %if port.is_empty() { "auto" } else { &port },
                                frequency_hz,
                                elapsed = ?start.elapsed(),
                                "Radio CI-V startup probe succeeded"
                            );
                            Some(radio.into())
                        }
                        Err(err) => {
                            error!(
                                backend = %backend,
                                model = %model,
                                endpoint = %endpoint,
                                port = %if port.is_empty() { "auto" } else { &port },
                                baud = baud_rate,
                                error = %err,
                                elapsed = ?start.elapsed(),
                                "Radio startup probe failed"
                            );
                            None
                        }
                    },
                    Err(err) => {
                        error!(error = %err, "Radio startup probe runtime failed");
                        None
                    }
                },
                Err(err) => {
                    error!(
                        backend = %backend,
                        model = %model,
                        endpoint = %endpoint,
                        port = %port,
                        baud = baud_rate,
                        error = %err,
                        elapsed = ?start.elapsed(),
                        "Radio initialization failed"
                    );
                    None
                }
            },
            unsupported => {
                error!(backend = unsupported, "Unsupported radio backend");
                None
            }
        };

        let _ = tx.send(radio);
    });
    rx
}

pub(super) fn request_radio_session_stop(session: &RadioSession) {
    request_radio_session_stop_handles(
        session.command_tx.as_ref(),
        &session.worker_stop,
        &session.audio_worker_stop,
        &session.swr_sweep_abort,
    );
}

pub(super) fn request_radio_session_stop_handles(
    command_tx: Option<&mpsc::Sender<GuiCommand>>,
    worker_stop: &Arc<AtomicBool>,
    audio_worker_stop: &Arc<AtomicBool>,
    swr_sweep_abort: &Arc<AtomicBool>,
) {
    worker_stop.store(true, Ordering::Relaxed);
    audio_worker_stop.store(true, Ordering::Relaxed);
    swr_sweep_abort.store(true, Ordering::Relaxed);
    if let Some(tx) = command_tx {
        let _ = tx.send(GuiCommand::Quit);
    }
}

pub(super) fn join_radio_session(mut session: RadioSession) {
    if let Some(handle) = session.worker_handle.take() {
        join_handle_for_shutdown(handle, "radio session");
    }
    if let Some(handle) = session.audio_worker_handle.take() {
        join_handle_for_shutdown(handle, "audio session");
    }
}

pub(super) fn join_handle_for_shutdown(handle: std::thread::JoinHandle<()>, worker: &str) {
    const SHUTDOWN_JOIN_BUDGET: Duration = Duration::from_millis(250);
    let deadline = Instant::now() + SHUTDOWN_JOIN_BUDGET;
    while !handle.is_finished() && Instant::now() < deadline {
        std::thread::yield_now();
    }
    if handle.is_finished() {
        let _ = handle.join();
    } else {
        warn!(
            worker,
            "Worker did not stop within shutdown budget; detaching"
        );
        // Dropping a JoinHandle detaches the thread. This is preferable to
        // preventing the GUI process from exiting when a driver call is
        // blocked in a device or OS API that cannot be interrupted.
    }
}

pub(super) fn stop_radio_session(session: RadioSession) {
    request_radio_session_stop(&session);
    join_radio_session(session);
}

pub(super) fn radio_config_from_operator_profile(profile: &OperatorProfile) -> RadioConfig {
    RadioConfig {
        enabled: profile.radio.enabled,
        backend: profile.radio.backend.clone(),
        endpoint: profile.radio.endpoint.clone(),
        model: profile.radio.model.clone(),
        serial_port: profile.radio.serial_port.clone(),
        baud_rate: profile.radio.baud_rate,
        civ_address: profile.radio.civ_address,
        controller_civ_address: profile.radio.controller_civ_address,
        hostbridge_access_key: profile.radio.hostbridge_access_key.clone(),
        hostbridge_password: profile.radio.hostbridge_password.clone(),
        hostbridge_radio_id: profile.radio.hostbridge_radio_id.clone(),
        hostbridge_audio_source_id: profile.radio.hostbridge_audio_source_id.clone(),
        hostbridge_audio_output_id: profile.radio.hostbridge_audio_output_id.clone(),
    }
}

impl QsonautGuiApp {
    pub(super) fn set_tab_workers_running(&mut self, name: &str, running: bool) {
        if name == self.selected_profile_name {
            self.config.radio.enabled = running;
            self.config.audio.enabled = running;
            self.ptt_allowed.store(running, Ordering::Release);
            self.profile_dirty = true;
            self.persist_profile(if running {
                "Radio and audio workers started for"
            } else {
                "Radio and audio workers stopped for"
            });
            if running {
                self.reconnect_radio();
                self.restart_audio();
            } else {
                self.radio_worker_stop.store(true, Ordering::Relaxed);
                self.audio_worker_stop.store(true, Ordering::Relaxed);
                if let Some(tx) = &self.command_tx {
                    let _ = tx.send(GuiCommand::Quit);
                }
                if let Some(handle) = self.radio_worker_handle.take() {
                    join_handle_for_shutdown(handle, "active radio");
                }
                if let Some(handle) = self.audio_worker_handle.take() {
                    join_handle_for_shutdown(handle, "active audio");
                }
                self.command_tx = None;
                if let Ok(mut state) = self.state.lock() {
                    state.radio_waterfall_status = "STOPPED (by operator)".to_string();
                    state.audio_spectrum_status = "STOPPED (by operator)".to_string();
                }
            }
            return;
        }
        let Some(session) = self.parked_radio_sessions.get_mut(name) else {
            return;
        };
        session.config.enabled = running;
        session.audio_config.enabled = running;
        session.profile.radio.enabled = running;
        session.profile.audio.enabled = running;
        if running {
            session.worker_stop = Arc::new(AtomicBool::new(false));
            session.audio_worker_stop = Arc::new(AtomicBool::new(false));
            let port = session.config.serial_port.clone().unwrap_or_default();
            session.init_rx = Some(spawn_radio_init_with_hostbridge(
                session.config.backend.clone(),
                session.config.model.clone(),
                port,
                session.config.endpoint.clone(),
                session.config.baud_rate,
                session.config.controller_civ_address,
                session.config.civ_address,
                session.config.hostbridge_access_key.clone(),
                session.config.hostbridge_password.clone(),
                session.config.hostbridge_radio_id.clone(),
                session.config.hostbridge_audio_source_id.clone(),
                session.config.hostbridge_audio_output_id.clone(),
            ));
            session.init_attempted = false;
            if session.audio_worker_handle.is_none() {
                session.audio_worker_handle = Some(spawn_audio_spectrum_worker(
                    session.state.clone(),
                    session.audio_worker_stop.clone(),
                    session.ft8_tx_active.clone(),
                    session.digital_tx_active.clone(),
                    true,
                    false,
                    session.audio_config.sample_rate_hz,
                    session.audio_config.channels,
                    effective_audio_input_device(
                        &session.config.backend,
                        session.audio_config.input_device.clone(),
                    ),
                    session.audio_config.monitor_enabled,
                    effective_audio_output_device(
                        &session.config.backend,
                        session
                            .audio_config
                            .monitor_output_device
                            .clone()
                            .or_else(|| session.audio_config.output_device.clone()),
                    ),
                    session.monitor_volume.clone(),
                    self.repaint_ctx.clone(),
                    session.display_tuning.clone(),
                ));
            }
        } else {
            session.worker_stop.store(true, Ordering::Relaxed);
            session.audio_worker_stop.store(true, Ordering::Relaxed);
            if let Some(tx) = &session.command_tx {
                let _ = tx.send(GuiCommand::Quit);
            }
            if let Some(handle) = session.worker_handle.take() {
                join_handle_for_shutdown(handle, "parked radio");
            }
            if let Some(handle) = session.audio_worker_handle.take() {
                join_handle_for_shutdown(handle, "parked audio");
            }
            session.command_tx = None;
            session.init_rx = None;
        }
        let _ = save_operator_profile_named(name, &session.profile);
    }

    pub(super) fn switch_radio_tab(&mut self, name: &str) {
        self.switch_radio_tab_with_save(name, true);
    }

    pub(super) fn switch_radio_tab_with_save(&mut self, name: &str, save_previous: bool) {
        if name == self.selected_profile_name {
            return;
        }
        let Some(profile) = load_operator_profile_named(name) else {
            self.profile_io_status = format!("Profile ‘{name}’ was not found");
            return;
        };

        self.disarm_all_tx_with_persistence("Radio tab switch: all TX disarmed", false);
        self.park_active_radio_session();
        if save_previous {
            if let Some(session) = self.parked_radio_sessions.get(&self.selected_profile_name) {
                let profile = session.profile.clone();
                let previous_name = self.selected_profile_name.clone();
                self.persist_profile_snapshot(&previous_name, &profile, "Saved");
            }
        }
        self.selected_profile_name = name.to_string();
        self.new_profile_name = name.to_string();
        self.config.radio = self.radio_config_for_profile(&profile);

        if let Some(session) = self.parked_radio_sessions.remove(name) {
            let session_profile = session.profile;
            let session_view = session.view_state;
            self.config.radio = session.config;
            self.config.audio = session.audio_config;
            self.state = session.state;
            self.command_tx = session.command_tx;
            self.radio_worker_stop = session.worker_stop;
            self.audio_worker_stop = session.audio_worker_stop;
            self.swr_sweep_abort = session.swr_sweep_abort;
            self.display_tuning = session.display_tuning;
            self.monitor_volume = session.monitor_volume;
            self.ft8_tx_active = session.ft8_tx_active;
            self.ptt_allowed = session.ptt_allowed;
            self.digital_tx_active = session.digital_tx_active;
            self.radio_init_rx = session.init_rx;
            self.radio_init_attempted = session.init_attempted;
            self.radio_worker_handle = session.worker_handle;
            self.audio_worker_handle = session.audio_worker_handle;
            self.ptt_allowed.store(true, Ordering::Release);
            self.apply_tab_preferences(&session_profile);
            self.restore_tab_view_state(session_view);
            if self.radio_worker_handle.is_none() && self.radio_init_rx.is_none() {
                self.start_active_radio_session();
            }
            if self.audio_worker_handle.is_none() {
                self.audio_worker_handle = Some(spawn_audio_spectrum_worker(
                    self.state.clone(),
                    self.audio_worker_stop.clone(),
                    self.ft8_tx_active.clone(),
                    self.digital_tx_active.clone(),
                    self.config.audio.enabled,
                    true,
                    self.config.audio.sample_rate_hz,
                    self.config.audio.channels,
                    effective_audio_input_device(
                        &self.config.radio.backend,
                        self.config.audio.input_device.clone(),
                    ),
                    self.config.audio.monitor_enabled,
                    effective_audio_output_device(
                        &self.config.radio.backend,
                        self.config
                            .audio
                            .monitor_output_device
                            .clone()
                            .or_else(|| self.config.audio.output_device.clone()),
                    ),
                    self.monitor_volume.clone(),
                    self.repaint_ctx.clone(),
                    self.display_tuning.clone(),
                ));
            }
            info!(
                profile = name,
                radio_running = self.command_tx.is_some(),
                audio_running = self.audio_worker_handle.is_some(),
                "Profile tab activated without reconnecting workers"
            );
        } else {
            self.config.audio = audio_config_from_operator_profile(&profile, &self.config.audio);
            self.state = Arc::new(Mutex::new(GuiState::default()));
            self.command_tx = None;
            self.radio_worker_handle = None;
            self.radio_worker_stop = Arc::new(AtomicBool::new(false));
            self.audio_worker_stop = Arc::new(AtomicBool::new(false));
            self.swr_sweep_abort = Arc::new(AtomicBool::new(false));
            self.display_tuning = Arc::new(Mutex::new(DisplayTuning::default()));
            self.monitor_volume =
                Arc::new(AtomicU32::new(self.config.audio.monitor_volume.to_bits()));
            self.ft8_tx_active = Arc::new(AtomicBool::new(false));
            self.ptt_allowed = Arc::new(AtomicBool::new(true));
            self.digital_tx_active = Arc::new(AtomicBool::new(false));
            self.start_active_radio_session();
            self.audio_worker_handle = Some(spawn_audio_spectrum_worker(
                self.state.clone(),
                self.audio_worker_stop.clone(),
                self.ft8_tx_active.clone(),
                self.digital_tx_active.clone(),
                self.config.audio.enabled,
                true,
                self.config.audio.sample_rate_hz,
                self.config.audio.channels,
                effective_audio_input_device(
                    &self.config.radio.backend,
                    self.config.audio.input_device.clone(),
                ),
                self.config.audio.monitor_enabled,
                effective_audio_output_device(
                    &self.config.radio.backend,
                    self.config
                        .audio
                        .monitor_output_device
                        .clone()
                        .or_else(|| self.config.audio.output_device.clone()),
                ),
                self.monitor_volume.clone(),
                self.repaint_ctx.clone(),
                self.display_tuning.clone(),
            ));
            self.apply_tab_preferences(&profile);
            self.restore_tab_view_state(TabViewState::default());
            info!(
                profile = name,
                "Radio tab created and initialization queued"
            );
        }
        if let Err(error) = select_operator_profile(name) {
            warn!(profile = name, %error, "Failed to persist active tab selection");
        }
        self.radio_waterfall_texture = None;
        self.audio_waterfall_texture = None;
        self.sstv_texture = None;
        self.profile_io_status = format!("Radio tab ‘{name}’ active");
    }

    pub(super) fn park_active_radio_session(&mut self) {
        let name = self.selected_profile_name.clone();
        let profile = self.current_operator_profile();
        let view_state = self.take_tab_view_state();
        let session = RadioSession {
            profile,
            view_state,
            config: self.config.radio.clone(),
            audio_config: self.config.audio.clone(),
            state: std::mem::replace(&mut self.state, Arc::new(Mutex::new(GuiState::default()))),
            command_tx: self.command_tx.take(),
            worker_stop: self.radio_worker_stop.clone(),
            audio_worker_stop: self.audio_worker_stop.clone(),
            swr_sweep_abort: self.swr_sweep_abort.clone(),
            display_tuning: self.display_tuning.clone(),
            monitor_volume: self.monitor_volume.clone(),
            ft8_tx_active: self.ft8_tx_active.clone(),
            digital_tx_active: self.digital_tx_active.clone(),
            ptt_allowed: self.ptt_allowed.clone(),
            init_rx: self.radio_init_rx.take(),
            init_attempted: self.radio_init_attempted,
            worker_handle: self.radio_worker_handle.take(),
            audio_worker_handle: self.audio_worker_handle.take(),
        };
        session.ptt_allowed.store(false, Ordering::Release);
        if let Some(previous) = self.parked_radio_sessions.insert(name, session) {
            stop_radio_session(previous);
        }
        info!(profile = %self.selected_profile_name, "Profile runtime moved to background with PTT disabled");
    }

    pub(super) fn start_active_radio_session(&mut self) {
        self.radio_worker_stop = Arc::new(AtomicBool::new(false));
        if !self.config.radio.enabled {
            self.radio_init_rx = None;
            self.radio_init_attempted = true;
            if let Ok(mut state) = self.state.lock() {
                state.radio_waterfall_status = "UNAVAILABLE (radio disabled)".to_string();
            }
            return;
        }
        let port = self.config.radio.serial_port.clone().unwrap_or_default();
        self.radio_init_rx = Some(spawn_radio_init_with_hostbridge(
            self.config.radio.backend.clone(),
            self.config.radio.model.clone(),
            port,
            self.config.radio.endpoint.clone(),
            self.config.radio.baud_rate,
            self.config.radio.controller_civ_address,
            self.config.radio.civ_address,
            self.config.radio.hostbridge_access_key.clone(),
            self.config.radio.hostbridge_password.clone(),
            self.config.radio.hostbridge_radio_id.clone(),
            self.config.radio.hostbridge_audio_source_id.clone(),
            self.config.radio.hostbridge_audio_output_id.clone(),
        ));
        self.radio_init_attempted = false;
        if let Ok(mut state) = self.state.lock() {
            state.radio_waterfall_status = "CONNECTING…".to_string();
            state.last_error = None;
        }
    }

    pub(super) fn pump_parked_radio_sessions(&mut self) {
        let repaint_ctx = self.repaint_ctx.clone();
        for (name, session) in &mut self.parked_radio_sessions {
            if session
                .audio_worker_handle
                .as_ref()
                .is_some_and(std::thread::JoinHandle::is_finished)
            {
                if let Some(handle) = session.audio_worker_handle.take() {
                    let _ = handle.join();
                }
                if let Ok(mut state) = session.state.lock() {
                    state.audio_spectrum_status = "STOPPED (audio worker failed)".to_string();
                }
                warn!(profile = name, "Profile audio worker stopped");
            }
            if session
                .worker_handle
                .as_ref()
                .is_some_and(std::thread::JoinHandle::is_finished)
            {
                if let Some(handle) = session.worker_handle.take() {
                    let _ = handle.join();
                }
                session.command_tx = None;
                if let Ok(mut state) = session.state.lock() {
                    state.radio_waterfall_status = "STOPPED (radio worker failed)".to_string();
                }
                warn!(profile = name, "Profile radio worker stopped");
            }
            if session.init_attempted || !session.config.enabled {
                continue;
            }
            let Some(rx) = &session.init_rx else { continue };
            match rx.try_recv() {
                Ok(Some(radio)) => {
                    session.init_attempted = true;
                    let (tx, command_rx) = mpsc::channel::<GuiCommand>();
                    session.worker_handle = Some(workers::radio::spawn_radio_worker(
                        radio,
                        session.state.clone(),
                        session.worker_stop.clone(),
                        session.swr_sweep_abort.clone(),
                        session.display_tuning.clone(),
                        command_rx,
                        repaint_ctx.clone(),
                        session.ptt_allowed.clone(),
                    ));
                    session.command_tx = Some(tx);
                    info!(
                        profile = name,
                        "Started inactive radio worker with PTT disabled"
                    );
                }
                Ok(None) | Err(mpsc::TryRecvError::Disconnected) => {
                    session.init_attempted = true;
                    session.init_rx = None;
                    warn!(
                        profile = name,
                        "Inactive profile radio initialization failed"
                    );
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
    }
}
