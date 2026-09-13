use super::*;
use qsonaut_server_client::{
    poll_browser_link, request_browser_link, BrowserLinkPoll, ServerIdentity,
};
use serde_json::Value;

const DIAGNOSTIC_LOG_BYTES: usize = 24 * 1024;

fn canonical_server_exchange_fields(fields: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    fields
        .iter()
        .map(|(key, value)| (key.trim().to_ascii_lowercase(), value.clone()))
        .collect()
}

fn resolve_server_identity_id(
    identities: &[ServerIdentity],
    record: &QsoRecord,
    occurred_at: &str,
) -> Option<Uuid> {
    let requested_id = if record.managed_callsign_id.trim().is_empty() {
        None
    } else {
        Some(Uuid::parse_str(record.managed_callsign_id.trim()).ok()?)
    };
    let event_id = if record.server_event_id.trim().is_empty() {
        None
    } else {
        Some(Uuid::parse_str(record.server_event_id.trim()).ok()?)
    };
    let occurred_at =
        time::OffsetDateTime::parse(occurred_at, &time::format_description::well_known::Rfc3339)
            .ok()?;
    identities
        .iter()
        .filter(|identity| {
            let identity_event_matches = match (identity.event_id.as_deref(), event_id) {
                (None, _) => true,
                (Some(identity_event), Some(event_id)) => {
                    Uuid::parse_str(identity_event).ok() == Some(event_id)
                }
                (Some(_), None) => false,
            };
            let effective_at = identity.effective_from.as_deref().is_none_or(|value| {
                time::OffsetDateTime::parse(value, &time::format_description::well_known::Rfc3339)
                    .is_ok_and(|effective| occurred_at >= effective)
            });
            let before_expiry = identity.expires_at.as_deref().is_none_or(|value| {
                time::OffsetDateTime::parse(value, &time::format_description::well_known::Rfc3339)
                    .is_ok_and(|expires| occurred_at < expires)
            });
            identity
                .callsign
                .eq_ignore_ascii_case(record.station_callsign.trim())
                && identity.status == "active"
                && identity.verification_status == "verified"
                && identity_event_matches
                && effective_at
                && before_expiry
        })
        .find(|identity| {
            requested_id.is_some_and(|id| Uuid::parse_str(&identity.id).ok() == Some(id))
                || (requested_id.is_none() && identity.identity_type == "personal")
        })
        .and_then(|identity| Uuid::parse_str(&identity.id).ok())
}

fn server_qso_payload(record: &QsoRecord, callsign_id: Uuid, occurred_at: &str) -> Value {
    serde_json::json!({
        "event_id": Uuid::parse_str(record.server_event_id.trim()).ok(),
        "operating_callsign": record.station_callsign.trim(),
        "callsign_id": callsign_id,
        "idempotency_key": log_idempotency_key(record.id),
        "callsign": record.callsign.trim(),
        "band": record.band.trim(),
        "mode": record.mode.trim(),
        "frequency_hz": i64::try_from(record.frequency_hz).ok(),
        "occurred_at": occurred_at,
        "rst_sent": (!record.report_sent.trim().is_empty()).then_some(record.report_sent.trim()),
        "rst_received": (!record.report_received.trim().is_empty()).then_some(record.report_received.trim()),
        "exchange": {
            "sent": record.contest_exchange_sent,
            "received": record.contest_exchange_received,
            "fields_sent": canonical_server_exchange_fields(&record.contest_fields_sent),
            "fields_received": canonical_server_exchange_fields(&record.contest_fields_received),
            "serial_sent": record.contest_serial_sent,
            "serial_received": record.contest_serial_received,
            "grid": record.grid,
            "operator_callsign": record.operator_callsign,
            "station_callsign": record.station_callsign,
            "contest_template_id": record.contest_template_id,
            "club_id": record.club_id,
        },
        "points": 0,
        "source": "qsonaut",
    })
}

fn redact_log_value(text: &mut String, value: &str, replacement: &str) {
    let value = value.trim();
    if !value.is_empty() {
        *text = text.replace(value, replacement);
    }
}

fn redacted_diagnostic_log(raw: String, config: &AppConfig) -> String {
    let mut text = raw;
    redact_log_value(
        &mut text,
        &config.server.device_token,
        "[REDACTED SERVER TOKEN]",
    );
    for device in [
        config.radio.serial_port.as_deref(),
        config.audio.input_device.as_deref(),
        config.audio.output_device.as_deref(),
        config.audio.monitor_output_device.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        redact_log_value(&mut text, device, "[REDACTED DEVICE]");
    }
    if let Some(home) = std::env::var_os("HOME").and_then(|value| value.into_string().ok()) {
        redact_log_value(&mut text, &home, "[HOME]");
    }
    text
}

impl QsonautGuiApp {
    pub(super) fn poll_server_browser_link(&mut self) {
        let Some(rx) = self.server_browser_link_rx.take() else {
            return;
        };
        let mut keep_receiver = true;
        loop {
            match rx.try_recv() {
                Ok(BrowserLinkProgress::ApprovalReady { url, user_code }) => {
                    self.server_browser_link_url = Some(url);
                    self.server_browser_link_code = Some(user_code);
                    self.server_browser_link_status =
                        "Browser approval is open; waiting for authorization".to_owned();
                }
                Ok(BrowserLinkProgress::Authorized(token)) => {
                    self.config.server.device_token = token;
                    self.config.server.enabled = true;
                    self.server_browser_link_status =
                        "Browser approval complete; connecting to QSONaut Server".to_owned();
                    self.profile_dirty = true;
                    keep_receiver = false;
                    self.server_browser_link_stop = None;
                    self.reconnect_server();
                    break;
                }
                Ok(BrowserLinkProgress::Failed(message)) => {
                    self.server_browser_link_status = message;
                    keep_receiver = false;
                    self.server_browser_link_stop = None;
                    break;
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.server_browser_link_status =
                        "Browser-link worker stopped unexpectedly".to_owned();
                    keep_receiver = false;
                    self.server_browser_link_stop = None;
                    break;
                }
            }
        }
        if keep_receiver {
            self.server_browser_link_rx = Some(rx);
        }
    }

    pub(super) fn start_server_browser_link(&mut self) {
        if self.server_browser_link_rx.is_some() {
            self.server_browser_link_status =
                "A browser link is already waiting for approval".to_owned();
            return;
        }
        let server_url = if self.config.server.url.trim().is_empty() {
            DEFAULT_SERVER_URL.to_owned()
        } else {
            self.config.server.url.trim().to_owned()
        };
        self.config.server.url = server_url.clone();
        self.profile_dirty = true;
        self.server_browser_link_url = None;
        self.server_browser_link_code = None;
        self.server_browser_link_status = "Requesting a browser link…".to_owned();
        let (tx, rx) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let client_version = env!("CARGO_PKG_VERSION").to_owned();
        let device_name = "QSONaut desktop".to_owned();
        thread::spawn(move || {
            let link = match request_browser_link(&server_url, &device_name, &client_version) {
                Ok(link) => link,
                Err(error) => {
                    let _ = tx.send(BrowserLinkProgress::Failed(error.to_string()));
                    return;
                }
            };
            let approval_url = match link.approval_url() {
                Ok(url) => url,
                Err(error) => {
                    let _ = tx.send(BrowserLinkProgress::Failed(error.to_string()));
                    return;
                }
            };
            if tx
                .send(BrowserLinkProgress::ApprovalReady {
                    url: approval_url.clone(),
                    user_code: link.user_code.clone(),
                })
                .is_err()
            {
                return;
            }
            let _ = webbrowser::open(&approval_url);
            let deadline = Instant::now() + Duration::from_secs(link.expires_in.max(1));
            let mut retry_after_secs = link.interval.clamp(1, 60);
            loop {
                if worker_stop.load(Ordering::Acquire) {
                    return;
                }
                if Instant::now() >= deadline {
                    let _ = tx.send(BrowserLinkProgress::Failed(
                        "Browser authorization expired; start the link again".to_owned(),
                    ));
                    return;
                }
                thread::sleep(Duration::from_secs(retry_after_secs));
                match poll_browser_link(
                    &server_url,
                    &link.device_code,
                    &client_version,
                    retry_after_secs,
                ) {
                    Ok(BrowserLinkPoll::Pending {
                        retry_after_secs: next,
                    }) => {
                        retry_after_secs = next.clamp(1, 60);
                    }
                    Ok(BrowserLinkPoll::Authorized { device_token }) => {
                        let _ = tx.send(BrowserLinkProgress::Authorized(device_token));
                        return;
                    }
                    Ok(BrowserLinkPoll::Denied { message })
                    | Ok(BrowserLinkPoll::Expired { message }) => {
                        let _ = tx.send(BrowserLinkProgress::Failed(message));
                        return;
                    }
                    Err(error) => {
                        let _ = tx.send(BrowserLinkProgress::Failed(error.to_string()));
                        return;
                    }
                }
            }
        });
        self.server_browser_link_rx = Some(rx);
        self.server_browser_link_stop = Some(stop);
    }

    pub(super) fn cancel_server_browser_link(&mut self) {
        if let Some(stop) = self.server_browser_link_stop.take() {
            stop.store(true, Ordering::Release);
        }
        self.server_browser_link_rx = None;
        self.server_browser_link_status = "Browser link canceled".to_owned();
    }

    pub(super) fn reconcile_server_activity_context(&mut self) {
        let Some(client) = &self.server_client else {
            return;
        };
        let status = client.status();
        if let Some((event_id, _)) = &self.server_active_event {
            let now = time::OffsetDateTime::now_utc();
            if status.state != ServerConnectionState::Connected
                || !status.active_events.iter().any(|event| {
                    event.id == *event_id
                        && time::OffsetDateTime::parse(
                            &event.starts_at,
                            &time::format_description::well_known::Rfc3339,
                        )
                        .is_ok_and(|starts| now >= starts)
                        && time::OffsetDateTime::parse(
                            &event.ends_at,
                            &time::format_description::well_known::Rfc3339,
                        )
                        .is_ok_and(|ends| now < ends)
                })
            {
                self.disarm_all_tx_with_persistence("Server event authorization expired", false);
                self.server_active_event = None;
                self.server_active_club = None;
                self.server_active_identity = None;
                return;
            }
        }
        let Some((identity_id, callsign)) = &self.server_active_identity else {
            return;
        };
        let identity_valid = status.identities.iter().any(|identity| {
            identity.id == *identity_id
                && identity.callsign.eq_ignore_ascii_case(callsign)
                && identity.status == "active"
                && identity.verification_status == "verified"
                && identity.event_id.as_deref().is_none_or(|identity_event| {
                    self.server_active_event
                        .as_ref()
                        .is_some_and(|(event_id, _)| event_id == identity_event)
                })
                && identity.effective_from.as_deref().is_none_or(|value| {
                    time::OffsetDateTime::parse(
                        value,
                        &time::format_description::well_known::Rfc3339,
                    )
                    .is_ok_and(|effective| time::OffsetDateTime::now_utc() >= effective)
                })
                && identity.expires_at.as_deref().is_none_or(|value| {
                    time::OffsetDateTime::parse(
                        value,
                        &time::format_description::well_known::Rfc3339,
                    )
                    .is_ok_and(|expires| time::OffsetDateTime::now_utc() < expires)
                })
        });
        let assignment_valid = self
            .server_active_event
            .as_ref()
            .is_none_or(|(event_id, _)| {
                status.participants.iter().any(|participant| {
                    participant.event_id == *event_id
                        && participant.callsign_id == *identity_id
                        && participant.status == "active"
                        && matches!(
                            participant.role.as_str(),
                            "operator" | "coordinator" | "logger"
                        )
                        && status.operator_callsign.as_deref().is_some_and(|operator| {
                            operator.eq_ignore_ascii_case(&participant.operator_callsign)
                        })
                        && participant.starts_at.as_deref().is_none_or(|value| {
                            time::OffsetDateTime::parse(
                                value,
                                &time::format_description::well_known::Rfc3339,
                            )
                            .is_ok_and(|starts| time::OffsetDateTime::now_utc() >= starts)
                        })
                        && participant.ends_at.as_deref().is_none_or(|value| {
                            time::OffsetDateTime::parse(
                                value,
                                &time::format_description::well_known::Rfc3339,
                            )
                            .is_ok_and(|ends| time::OffsetDateTime::now_utc() < ends)
                        })
                })
            });
        if !identity_valid || !assignment_valid {
            self.disarm_all_tx_with_persistence("Operating identity authorization expired", false);
            self.server_active_identity = None;
        }
    }

    pub(super) fn poll_radio_validation(&mut self) {
        let Some(rx) = self.radio_validation_rx.take() else {
            return;
        };
        match rx.try_recv() {
            Ok(Ok(report)) => {
                self.radio_validation_active = false;
                self.radio_validation_status =
                    "Validation complete; submitting report…".to_string();
                self.publish_radio_validation_report(report);
            }
            Ok(Err(error)) => {
                self.radio_validation_active = false;
                self.radio_validation_status = format!("Validation failed: {error}");
            }
            Err(mpsc::TryRecvError::Empty) => {
                self.radio_validation_rx = Some(rx);
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.radio_validation_active = false;
                self.radio_validation_status = "Validation worker stopped unexpectedly".to_string();
            }
        }
    }

    pub(super) fn start_radio_validation(&mut self) {
        if !self.config.server.enabled || !self.config.server.share_diagnostics {
            self.radio_validation_status =
                "Enable manual diagnostic snapshots before submitting validation reports"
                    .to_string();
            return;
        }
        let Some(client) = &self.server_client else {
            self.radio_validation_status =
                "Connect to QSONaut Server before submitting validation reports".to_string();
            return;
        };
        if client.status().state != ServerConnectionState::Connected {
            self.radio_validation_status = "Wait for QSONaut Server to show CONNECTED".to_string();
            return;
        }
        if self.radio_validation_low_power && !self.radio_validation_confirm_low_power {
            self.radio_validation_status =
                "Confirm the low-power PTT safety check before starting".to_string();
            return;
        }
        let Some(command_tx) = &self.command_tx else {
            self.radio_validation_status = "Radio worker is not running".to_string();
            return;
        };
        let (ack_tx, ack_rx) = mpsc::channel();
        if command_tx
            .send(GuiCommand::RunRadioValidation {
                include_ptt: self.radio_validation_low_power,
                rf_power_level: self.radio_validation_power_level,
                ack_tx,
            })
            .is_err()
        {
            self.radio_validation_status = "Radio worker is unavailable".to_string();
            return;
        }
        self.radio_validation_rx = Some(ack_rx);
        self.radio_validation_active = true;
        self.radio_validation_status = "Running full hardware validation…".to_string();
    }

    fn publish_radio_validation_report(&mut self, report: serde_json::Value) {
        let Some(client) = &self.server_client else {
            self.radio_validation_status =
                "Validation complete, but Server is disconnected".to_string();
            return;
        };
        let diagnostic = serde_json::json!({
            "instance_id": self.server_instance_id,
            "category": "radio_validation",
            "summary": format!("{} radio validation report", self.config.radio.model),
            "payload": {
                "qsonaut": {
                    "version": env!("CARGO_PKG_VERSION"),
                    "platform": format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
                },
                "radio_config": {
                    "backend": self.config.radio.backend,
                    "model": self.config.radio.model,
                    "baud_rate": self.config.radio.baud_rate,
                    "civ_address": self.config.radio.civ_address,
                    "controller_civ_address": self.config.radio.controller_civ_address,
                    "serial_port_configured": self.config.radio.serial_port.is_some(),
                },
                "validation": report,
            },
        });
        self.radio_validation_status = match client.publish_diagnostic(diagnostic) {
            Ok(()) => "Validation report submitted; waiting for server acceptance".to_string(),
            Err(error) => format!("Validation report could not be submitted: {error}"),
        };
    }

    pub(super) fn reconnect_server(&mut self) {
        self.disarm_all_tx_with_persistence("Server connection changed", false);
        self.server_active_event = None;
        self.server_active_club = None;
        self.server_active_identity = None;
        let enabled = self.config.server.enabled;
        let url = self.config.server.url.trim();
        let token = self.config.server.device_token.trim();
        if enabled && (url.is_empty() || token.is_empty()) {
            self.server_client = None;
            warn!("Server connection requires both endpoint and device token");
            self.profile_io_status =
                "Server needs both an endpoint and device token before connecting".to_string();
            return;
        }

        let next_client = enabled.then(|| {
            ServerClient::spawn(ServerConnectionConfig {
                server_url: url.to_string(),
                device_token: token.to_string(),
                client_version: env!("CARGO_PKG_VERSION").to_string(),
                queue_path: app_config_dir().join("server-log-queue.json"),
                share_logs: self.config.server.share_logs,
            })
        });
        self.server_client = next_client;
        info!(enabled, endpoint = %url, "Server connection configuration changed");
        self.profile_dirty = true;
        self.persist_profile(if enabled {
            "Server settings saved to"
        } else {
            "Server disabled in"
        });
        self.server_last_presence = Instant::now() - Duration::from_secs(60);
    }

    pub(super) fn publish_qso_to_server(&self, record: &QsoRecord) {
        if !self.config.server.enabled || !self.config.server.share_logs {
            return;
        }
        let Some(client) = &self.server_client else {
            return;
        };
        let Some(occurred_at) = qso_timestamp(record) else {
            return;
        };
        if record.callsign.trim().is_empty()
            || record.band.trim().is_empty()
            || record.mode.trim().is_empty()
        {
            warn!("QSO not queued for server: callsign, band, and mode are required");
            return;
        }
        let Some(callsign_id) =
            resolve_server_identity_id(&client.status().identities, record, &occurred_at)
        else {
            warn!(callsign = %record.station_callsign, "QSO not queued for server: no active verified managed operating identity");
            return;
        };
        let operating_callsign = record.station_callsign.trim();
        if operating_callsign.is_empty() {
            warn!("QSO not queued for server: operating callsign is empty");
            return;
        }
        client.publish_log(server_qso_payload(record, callsign_id, &occurred_at));
        info!(callsign = %record.callsign, band = %record.band, mode = %record.mode, "QSO queued for server log publishing");
    }

    pub(super) fn publish_server_presence(&mut self, snapshot: &GuiState) {
        if !self.config.server.share_presence
            || self.server_last_presence.elapsed() < Duration::from_secs(15)
        {
            return;
        }
        self.server_last_presence = Instant::now();
        let Some(client) = &self.server_client else {
            return;
        };
        let details = self.config.server.share_radio_details;
        let frequency_hz = details
            .then_some(snapshot.frequency_hz)
            .flatten()
            .and_then(|value| i64::try_from(value).ok());
        let band = frequency_hz
            .and_then(|value| u64::try_from(value).ok())
            .map(band_for_frequency)
            .filter(|band| !band.is_empty())
            .map(str::to_owned);
        let radio_profile = details
            .then(|| native_radio_profile(&self.config.radio.backend, &self.config.radio.model))
            .flatten();
        let radio_model = radio_profile.map(|profile| profile.model.to_string());
        let radio_manufacturer =
            radio_profile.map(|profile| profile.manufacturer.label().to_string());
        client.publish_presence(ServerPresence {
            instance_id: self.server_instance_id.clone(),
            station_label: format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
            radio_manufacturer,
            radio_model,
            frequency_hz,
            band,
            mode: details.then(|| self.workspace_mode.label().to_string()),
            qsonaut_version: env!("CARGO_PKG_VERSION").to_string(),
            platform: format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
            status: "online".to_string(),
            metadata: if details {
                serde_json::json!({
                    "grid": self.station_grid_or_default(),
                    "contest_enabled": self.contest_enabled,
                    "radio_backend": self.config.radio.backend,
                    "radio_baud_rate": self.config.radio.baud_rate,
                    "civ_address": self.config.radio.civ_address,
                    "controller_civ_address": self.config.radio.controller_civ_address,
                    "data_mode": snapshot.data_mode,
                    "filter": snapshot.filter,
                    "af_gain": snapshot.af_gain,
                    "rf_gain": snapshot.rf_gain,
                    "rf_power": snapshot.rf_power,
                    "scope_enabled": snapshot.radio_spectrum_enabled,
                    "scope_status": snapshot.radio_waterfall_status,
                    "audio_status": snapshot.audio_spectrum_status,
                    "audio_level_dbfs": snapshot.audio_level_dbfs,
                    "audio_clip_percent": snapshot.audio_clip_percent,
                    "compute_backend": format!("{:?}", snapshot.compute_backend),
                })
            } else {
                serde_json::json!({})
            },
        });
    }

    pub(super) fn publish_diagnostic_snapshot(&mut self) {
        if !self.config.server.enabled || !self.config.server.share_diagnostics {
            self.profile_io_status =
                "Enable manual diagnostic snapshots before sending".to_string();
            return;
        }
        let Some(client) = &self.server_client else {
            self.profile_io_status = "Connect to QSONaut Server before sending".to_string();
            return;
        };
        if client.status().state != ServerConnectionState::Connected {
            self.profile_io_status =
                "Wait for QSONaut Server to show CONNECTED before sending".to_string();
            return;
        }
        let snapshot = self.state.lock().expect("ui state lock poisoned").clone();
        let recent_app_log = self.config.server.share_debug_logs.then(|| {
            read_log_tail(DIAGNOSTIC_LOG_BYTES)
                .map(|text| redacted_diagnostic_log(text, &self.config))
                .unwrap_or_else(|_| "Application log unavailable".to_string())
        });
        let diagnostic = serde_json::json!({
            "instance_id": self.server_instance_id,
            "category": "radio_snapshot",
            "summary": format!("{} radio and runtime snapshot", self.config.radio.model),
            "payload": {
                "qsonaut": { "version": env!("CARGO_PKG_VERSION"), "platform": format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH) },
                "radio_config": {
                    "enabled": self.config.radio.enabled,
                    "backend": self.config.radio.backend,
                    "model": self.config.radio.model,
                    "baud_rate": self.config.radio.baud_rate,
                    "civ_address": self.config.radio.civ_address,
                    "controller_civ_address": self.config.radio.controller_civ_address,
                    "serial_port_configured": self.config.radio.serial_port.is_some(),
                },
                "radio_state": {
                    "frequency_hz": snapshot.frequency_hz,
                    "mode": snapshot.mode,
                    "data_mode": snapshot.data_mode,
                    "filter": snapshot.filter,
                    "af_gain": snapshot.af_gain,
                    "rf_gain": snapshot.rf_gain,
                    "rf_power": snapshot.rf_power,
                    "ptt_on": snapshot.ptt_on,
                    "scope_enabled": snapshot.radio_spectrum_enabled,
                    "scope_status": snapshot.radio_waterfall_status,
                    "radio_power_on": snapshot.radio_power_on,
                    "radio_power_settling": snapshot.radio_power_settling,
                    "supported_controls": snapshot
                        .supported_controls
                        .iter()
                        .map(|id| format!("{id:?}"))
                        .collect::<Vec<_>>(),
                    "supported_meters": snapshot
                        .supported_meters
                        .iter()
                        .map(|id| format!("{id:?}"))
                        .collect::<Vec<_>>(),
                },
                "audio": {
                    "enabled": self.config.audio.enabled,
                    "sample_rate_hz": self.config.audio.sample_rate_hz,
                    "channels": self.config.audio.channels,
                    "canonical_sample_rate_hz": qsonaut_audio::CANONICAL_SAMPLE_RATE_HZ,
                    "canonical_channels": qsonaut_audio::CANONICAL_CHANNELS,
                    "device_sample_rate_hz": snapshot.audio_device_sample_rate_hz,
                    "device_channels": snapshot.audio_device_channels,
                    "device_sample_format": snapshot.audio_device_sample_format,
                    "input_fallback_attempts": snapshot.audio_input_fallback_attempts,
                    "monitor_adjustment_ppm": snapshot.audio_monitor_adjustment_ppm,
                    "monitor_buffered_ms": snapshot.audio_monitor_buffered_ms,
                    "monitor_underruns": snapshot.audio_monitor_underruns,
                    "input_configured": self.config.audio.input_device.is_some(),
                    "output_configured": self.config.audio.output_device.is_some(),
                    "status": snapshot.audio_spectrum_status,
                    "level_dbfs": snapshot.audio_level_dbfs,
                    "clip_percent": snapshot.audio_clip_percent,
                },
                "decoder": {
                    "workspace": self.workspace_mode.label(),
                    "ft8_status": snapshot.ft8_decode_status,
                    "digital_status": snapshot.digital_decode_status,
                    "compute_backend": format!("{:?}", snapshot.compute_backend),
                },
                "last_error": snapshot.last_error,
                "recent_app_log": recent_app_log,
            }
        });
        self.profile_io_status = match client.publish_diagnostic(diagnostic) {
            Ok(()) => {
                info!("Server diagnostic snapshot queued");
                "Diagnostic snapshot sent; waiting for server acceptance".to_string()
            }
            Err(error) => {
                warn!(error = %error, "Server diagnostic snapshot could not be queued");
                format!("Diagnostic snapshot could not be queued: {error}")
            }
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headless_app() -> QsonautGuiApp {
        let icon = eframe::icon_data::from_png_bytes(crate::QSONAUT_ICON_PNG).expect("test icon");
        QsonautGuiApp::new_with_context(
            AppConfig::default(),
            false,
            false,
            &egui::Context::default(),
            &icon,
            eframe::Renderer::Wgpu,
            None,
            GraphicsPreferences::from_environment(),
            None,
            Vec::new(),
            Arc::new(Mutex::new(None)),
        )
    }

    #[test]
    fn diagnostic_log_redacts_tokens_and_configured_devices() {
        let mut config = AppConfig::default();
        config.server.device_token = "secret-device-token".to_string();
        config.radio.serial_port = Some("/dev/ttyUSB9".to_string());
        config.audio.input_device = Some("Private microphone".to_string());
        let redacted = redacted_diagnostic_log(
            "token=secret-device-token port=/dev/ttyUSB9 input=Private microphone".to_string(),
            &config,
        );
        assert!(!redacted.contains("secret-device-token"));
        assert!(!redacted.contains("/dev/ttyUSB9"));
        assert!(!redacted.contains("Private microphone"));
        assert!(redacted.contains("[REDACTED SERVER TOKEN]"));
        assert!(redacted.contains("[REDACTED DEVICE]"));
    }

    #[test]
    fn diagnostic_redaction_handles_optional_devices_and_empty_secrets() {
        let mut config = AppConfig::default();
        config.server.device_token = "  ".to_string();
        config.audio.output_device = Some("Private speakers".to_string());
        config.audio.monitor_output_device = Some("Monitor output".to_string());
        let mut raw = "empty= token= speakers=Private speakers monitor=Monitor output".to_string();
        redact_log_value(&mut raw, "  ", "[REDACTED]");
        let redacted = redacted_diagnostic_log(raw, &config);
        assert!(!redacted.contains("Private speakers"));
        assert!(!redacted.contains("Monitor output"));
        assert!(redacted.contains("empty= token="));
    }

    #[test]
    fn server_operations_guard_disabled_and_incomplete_configurations() {
        let mut app = headless_app();
        app.config.server.enabled = true;
        app.config.server.url.clear();
        app.config.server.device_token.clear();
        app.reconnect_server();
        assert_eq!(
            app.profile_io_status,
            "Server needs both an endpoint and device token before connecting"
        );
        assert!(app.server_client.is_none());

        let record = QsoRecord::new("W1AW", "FT8", "20m", 14_074_000, 1, 2);
        app.publish_qso_to_server(&record);
        assert!(app.server_client.is_none());

        app.publish_server_presence(&GuiState::default());
        assert!(app.server_client.is_none());

        app.config.server.enabled = false;
        app.publish_diagnostic_snapshot();
        assert_eq!(
            app.profile_io_status,
            "Enable manual diagnostic snapshots before sending"
        );
    }

    #[test]
    fn server_contest_exchange_fields_match_server_catalog_keys() {
        let fields = BTreeMap::from([
            ("CLASS".to_owned(), "1A".to_owned()),
            ("SECTION".to_owned(), "WMA".to_owned()),
        ]);
        let canonical = canonical_server_exchange_fields(&fields);
        assert_eq!(canonical.get("class").map(String::as_str), Some("1A"));
        assert_eq!(canonical.get("section").map(String::as_str), Some("WMA"));
        assert!(!canonical.contains_key("CLASS"));
    }

    #[test]
    fn ordinary_qsos_resolve_their_personal_server_identity() {
        let identity_id = Uuid::new_v4();
        let identities = vec![ServerIdentity {
            id: identity_id.to_string(),
            callsign: "N7UF".to_owned(),
            identity_type: "personal".to_owned(),
            club_id: None,
            event_id: None,
            status: "active".to_owned(),
            verification_status: "verified".to_owned(),
            effective_from: None,
            expires_at: None,
        }];
        let mut record = QsoRecord::new("W1AW", "FT8", "20m", 14_074_000, 1, 2);
        record.station_callsign = "N7UF".to_owned();
        let occurred_at = qso_timestamp(&record).unwrap();
        assert_eq!(
            resolve_server_identity_id(&identities, &record, &occurred_at),
            Some(identity_id)
        );
    }

    #[test]
    fn server_identity_resolution_rejects_invalid_explicit_and_event_context() {
        let identity_id = Uuid::new_v4();
        let event_id = Uuid::new_v4();
        let identities = vec![ServerIdentity {
            id: identity_id.to_string(),
            callsign: "N7UF".to_owned(),
            identity_type: "event".to_owned(),
            club_id: None,
            event_id: Some(event_id.to_string()),
            status: "active".to_owned(),
            verification_status: "verified".to_owned(),
            effective_from: Some("2026-09-11T00:00:00Z".to_owned()),
            expires_at: Some("2026-09-12T00:00:00Z".to_owned()),
        }];
        let mut record = QsoRecord::new("W1AW", "FT8", "20m", 14_074_000, 1, 2);
        record.station_callsign = "N7UF".to_owned();
        record.server_event_id = event_id.to_string();
        record.managed_callsign_id = identity_id.to_string();
        assert_eq!(
            resolve_server_identity_id(&identities, &record, "2026-09-11T12:00:00Z"),
            Some(identity_id)
        );

        record.server_event_id = Uuid::new_v4().to_string();
        assert!(resolve_server_identity_id(&identities, &record, "2026-09-11T12:00:00Z").is_none());

        record.server_event_id = event_id.to_string();
        record.managed_callsign_id = "not-a-uuid".to_owned();
        assert!(resolve_server_identity_id(&identities, &record, "2026-09-11T12:00:00Z").is_none());
        assert!(resolve_server_identity_id(&identities, &record, "2026-09-12T00:00:00Z").is_none());
    }

    #[test]
    fn server_qso_payload_matches_the_server_log_contract() {
        let identity_id = Uuid::new_v4();
        let event_id = Uuid::new_v4();
        let mut record = QsoRecord::new(" w1aw ", " ft8 ", " 20m ", 14_074_000, 1, 2);
        record.station_callsign = "N7UF".to_owned();
        record.operator_callsign = "K1OP".to_owned();
        record.server_event_id = event_id.to_string();
        record.report_sent = "-10".to_owned();
        record.report_received = "-12".to_owned();
        record.contest_exchange_sent = "1A WMA".to_owned();
        record
            .contest_fields_sent
            .insert("CLASS".to_owned(), "1A".to_owned());
        record
            .contest_fields_received
            .insert("SECTION".to_owned(), "WMA".to_owned());
        record.contest_serial_sent = Some(7);
        let payload = server_qso_payload(&record, identity_id, "2026-09-11T12:00:00Z");
        assert_eq!(payload["event_id"], event_id.to_string());
        assert_eq!(payload["callsign_id"], identity_id.to_string());
        assert_eq!(payload["operating_callsign"], "N7UF");
        assert_eq!(payload["callsign"], "W1AW");
        assert_eq!(payload["band"], "20m");
        assert_eq!(payload["mode"], "FT8");
        assert_eq!(payload["exchange"]["fields_sent"]["class"], "1A");
        assert_eq!(payload["exchange"]["fields_received"]["section"], "WMA");
        assert_eq!(payload["exchange"]["serial_sent"], 7);
        assert_eq!(payload["source"], "qsonaut");
    }
}
