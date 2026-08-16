use super::*;

impl QsonautGuiApp {
    pub(super) fn reconnect_server(&mut self) {
        let enabled = self.config.server.enabled;
        let url = self.config.server.url.trim();
        let token = self.config.server.device_token.trim();
        if enabled && (url.is_empty() || token.is_empty()) {
            self.profile_io_status =
                "Server needs both an endpoint and device token before connecting".to_string();
            return;
        }

        let next_client = enabled.then(|| {
            ServerClient::spawn(ServerConnectionConfig {
                server_url: url.to_string(),
                device_token: token.to_string(),
                client_version: env!("CARGO_PKG_VERSION").to_string(),
            })
        });
        self.server_client = next_client;
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
            "event_id": null,
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
        let radio_model = details.then(|| self.config.radio.model.clone());
        let radio_manufacturer = radio_model.as_deref().and_then(|model| {
            if model.starts_with("IC-") {
                Some("Icom".to_string())
            } else if model.starts_with("FT-") || model.starts_with("FTDX") {
                Some("Yaesu".to_string())
            } else if model.starts_with("TS-") {
                Some("Kenwood".to_string())
            } else {
                None
            }
        });
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
        let snapshot = self.state.lock().expect("ui state lock poisoned").clone();
        client.publish_diagnostic(serde_json::json!({
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
            }
        }));
        self.profile_io_status = "Diagnostic snapshot queued for QSONaut Server".to_string();
    }
}
