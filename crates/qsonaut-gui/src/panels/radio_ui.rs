use super::super::*;
use super::radio_contract::{
    apply_radio_reconnect, mark_radio_reconnect_disabled, spawn_cat_connection_test,
    stop_radio_worker_for_reconnect,
};

impl QsonautGuiApp {
    pub(in super::super) fn reconnect_radio(&mut self) {
        info!(backend = %self.config.radio.backend, model = %self.config.radio.model, "Radio reconnect requested");
        stop_radio_worker_for_reconnect(
            &mut self.command_tx,
            &mut self.radio_worker_stop,
            &mut self.radio_worker_handle,
        );

        if !self.config.radio.enabled {
            info!("Radio reconnect skipped: radio is disabled");
            mark_radio_reconnect_disabled(
                &self.state,
                &mut self.radio_init_rx,
                &mut self.radio_init_attempted,
            );
            return;
        }

        let port = self.config.radio.serial_port.clone().unwrap_or_default();
        let backend = self.config.radio.backend.clone();
        let model = self.config.radio.model.clone();
        let endpoint = self.config.radio.endpoint.clone();
        let baud_rate = self.config.radio.baud_rate;
        let controller_civ_address = self.config.radio.controller_civ_address;
        let radio_civ_address = self.config.radio.civ_address;
        let hostbridge_access_key = self.config.radio.hostbridge_access_key.clone();
        let hostbridge_password = self.config.radio.hostbridge_password.clone();
        let hostbridge_radio_id = self.config.radio.hostbridge_radio_id.clone();
        let hostbridge_audio_source_id = self.config.radio.hostbridge_audio_source_id.clone();
        let hostbridge_audio_output_id = self.config.radio.hostbridge_audio_output_id.clone();
        let should_restart_audio = self.config.audio.enabled && self.audio_worker_handle.is_none();
        apply_radio_reconnect(
            &mut self.radio_init_rx,
            &mut self.radio_init_attempted,
            &mut self.device_restart_required,
            &self.state,
            should_restart_audio,
            || {
                spawn_radio_init_with_hostbridge(
                    backend,
                    model,
                    port,
                    endpoint,
                    baud_rate,
                    controller_civ_address,
                    radio_civ_address,
                    hostbridge_access_key,
                    hostbridge_password,
                    hostbridge_radio_id,
                    hostbridge_audio_source_id,
                    hostbridge_audio_output_id,
                )
            },
            || {},
        );
        if should_restart_audio {
            self.restart_audio();
        }
        info!(port = %self.config.radio.serial_port.as_deref().unwrap_or("auto"), "Radio reconnect initialization queued");
    }

    pub(in super::super) fn test_cat_connection(&mut self) {
        let backend = self.config.radio.backend.clone();
        let model = self.config.radio.model.clone();
        let port = self.config.radio.serial_port.clone().unwrap_or_default();
        let baud_rate = self.config.radio.baud_rate;
        let controller_civ_address = self.config.radio.controller_civ_address;
        let radio_civ_address = self.config.radio.civ_address;
        self.cat_test_status = None;

        // The CAT probe opens the serial port, which is exclusively owned by
        // the running radio worker on Windows. Stop the worker first so the
        // probe can open the port; the worker is restarted once the probe
        // completes (see the cat_test_rx handling in update()).
        let had_radio_worker = self.radio_worker_handle.is_some();
        if had_radio_worker {
            stop_radio_worker_for_reconnect(
                &mut self.command_tx,
                &mut self.radio_worker_stop,
                &mut self.radio_worker_handle,
            );
        }
        self.cat_test_restart_radio = had_radio_worker && self.config.radio.enabled;

        info!(
            backend = %backend,
            model = %model,
            port = %if port.is_empty() { "auto" } else { &port },
            baud = baud_rate,
            paused_worker = had_radio_worker,
            "CAT connection test requested"
        );
        self.cat_test_rx = Some(spawn_cat_connection_test(
            backend,
            model,
            port,
            baud_rate,
            controller_civ_address,
            radio_civ_address,
        ));
    }
}
