use super::*;

const DIAGNOSTIC_LOG_BYTES: usize = 24 * 1024;

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
    pub(super) fn reconnect_server(&mut self) {
        let enabled = self.config.server.enabled;
        let url = self.config.server.url.trim();
        let token = self.config.server.device_token.trim();
        if enabled && (url.is_empty() || token.is_empty()) {
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
        client.publish_log(serde_json::json!({
            "event_id": self.server_active_event.as_ref().and_then(|(id, _)| Uuid::parse_str(id).ok()),
            "idempotency_key": log_idempotency_key(record.id),
            "callsign": record.callsign,
            "band": record.band,
            "mode": record.mode,
            "frequency_hz": i64::try_from(record.frequency_hz).ok(),
            "occurred_at": occurred_at,
            "rst_sent": (!record.report_sent.is_empty()).then_some(&record.report_sent),
            "rst_received": (!record.report_received.is_empty()).then_some(&record.report_received),
            "exchange": {
                "sent": record.contest_exchange_sent,
                "received": record.contest_exchange_received,
                "serial_sent": record.contest_serial_sent,
                "serial_received": record.contest_serial_received,
                "grid": record.grid,
            },
            "points": 0,
            "source": "qsonaut",
        }));
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
}
