use anyhow::Result;

use super::profile::{
    list_operator_profiles, remove_operator_profile_named, save_global_settings,
    save_operator_profile_named, save_radio_profile_library, select_operator_profile,
    GlobalSettings, OperatorProfile, RadioProfile,
};
use super::*;

/// Coordinates persistence of profile-owned data without exposing filesystem
/// mechanics to the GUI coordinator.
pub(super) struct ProfileManager;

impl ProfileManager {
    pub(super) fn save_global_and_radio_library(
        global: &GlobalSettings,
        radio_profiles: &[RadioProfile],
    ) -> Result<()> {
        save_global_settings(global)?;
        if !radio_profiles.is_empty() {
            save_radio_profile_library(radio_profiles)?;
        }
        Ok(())
    }

    pub(super) fn save_operator_profile_snapshot(
        name: &str,
        profile: &OperatorProfile,
    ) -> Result<Vec<String>> {
        save_operator_profile_named(name, profile)?;
        Ok(list_operator_profiles())
    }

    pub(super) fn create_profile(name: &str, profile: &OperatorProfile) -> Result<Vec<String>> {
        save_operator_profile_named(name, profile)?;
        Ok(list_operator_profiles())
    }

    pub(super) fn rename_profile(
        old_name: &str,
        new_name: &str,
        profile: &OperatorProfile,
    ) -> Result<Vec<String>> {
        save_operator_profile_named(new_name, profile)?;
        remove_operator_profile_named(old_name)?;
        select_operator_profile(new_name)?;
        Ok(list_operator_profiles())
    }

    pub(super) fn delete_profile(name: &str) -> Result<Vec<String>> {
        remove_operator_profile_named(name)?;
        Ok(list_operator_profiles())
    }
}

impl QsonautGuiApp {
    pub(super) fn current_operator_profile(&self) -> OperatorProfile {
        let display_tuning = self
            .display_tuning
            .lock()
            .expect("display tuning lock poisoned");
        OperatorProfile {
            profile_version: OPERATOR_PROFILE_VERSION,
            callsign: self.station_callsign_or_default().to_string(),
            grid: self.station_grid_or_default().to_string(),
            qth: self.station_qth.trim().to_string(),
            station_rig: self.station_rig.trim().to_string(),
            station_antenna: self.station_antenna.trim().to_string(),
            station_notes: self.station_notes.trim().to_string(),
            llm_prompt_context: self.llm_prompt_context.trim().to_string(),
            sstv_image_requirements: self.sstv_image_requirements.trim().to_string(),
            llm_model_notes: self.llm_model_notes.trim().to_string(),
            follow_log: self.ft8_follow_log,
            max_log_entries: self.ft8_max_log_entries.clamp(80, 1000),
            deep_decode: self.ft8_deep_decode,
            ft4_deep_decode: self.ft4_deep_decode,
            ft4_autoseq: self.ft4_autoseq,
            ft4_auto_reply_policy: self.ft4_auto_reply_policy,
            ft4_cq_only_view: self.ft4_cq_only_view,
            ft4_follow_log: self.ft4_follow_log,
            ft4_max_log_entries: self.ft4_max_log_entries.clamp(80, 300),
            ft4_max_attempts: self.ft4_max_attempts.clamp(1, 20),
            autoseq: self.ft8_autoseq,
            auto_reply_policy: self.ft8_auto_reply_policy,
            auto_answer_cq: self.ft8_auto_answer_cq,
            automation_unlocked: self.automation_unlocked,
            cq_only_view: self.ft8_cq_only_view,
            civ_spectrum_on: self.civ_spectrum_on,
            radio_scope_vbw_wide: self.radio_scope_vbw_wide,
            radio_scope_view: self.radio_scope_view,
            waterfall_theme: self.waterfall_theme,
            radio_waterfall_theme: self.radio_waterfall_theme,
            waterfall_auto_visual: display_tuning.audio_auto_visual,
            waterfall_speed: display_tuning.audio_waterfall_speed,
            waterfall_deck_height: self.waterfall_deck_height,
            halt_after_tx: false,
            ft8_max_attempts: self.ft8_max_attempts.clamp(1, 20),
            hold_tx_freq: self.ft8_hold_tx_freq,
            rx_tone_hz: self.rx_tone_hz,
            tx_tone_hz: self.tx_tone_hz,
            ptt_lead_ms: self.ptt_lead_ms.clamp(0, 500),
            ptt_tail_ms: self.ptt_tail_ms.clamp(0, 500),
            cw_wpm: self.cw_wpm.clamp(5, 40),
            cw_tone_hz: self.cw_tone_hz.clamp(200, 3_000),
            recording_enabled: self.recording_enabled,
            recording_modes: self.recording_modes.clone(),
            recording_full_width: self.recording_full_width,
            recording_stream: self.recording_stream,
            audio: profile::AudioProfileSettings {
                input_device: self.config.audio.input_device.clone(),
                enabled: self.config.audio.enabled,
                output_device: self.config.audio.output_device.clone(),
                monitor_enabled: self.config.audio.monitor_enabled,
                monitor_output_device: self.config.audio.monitor_output_device.clone(),
                monitor_volume: self.config.audio.monitor_volume.clamp(0.0, 2.0),
                sample_rate_hz: self.config.audio.sample_rate_hz,
                channels: self.config.audio.channels,
            },
            radio: profile::RadioProfileSettings {
                enabled: self.config.radio.enabled,
                serial_port: self.config.radio.serial_port.clone(),
                backend: self.config.radio.backend.clone(),
                endpoint: self.config.radio.endpoint.clone(),
                model: self.config.radio.model.clone(),
                baud_rate: self.config.radio.baud_rate,
                civ_address: self.config.radio.civ_address,
                controller_civ_address: self.config.radio.controller_civ_address,
            },
            gui_scale: self.gui_scale.clamp(GUI_SCALE_MIN, GUI_SCALE_MAX),
            compute_preference: self.compute_preference,
            psk_reporter_enabled: self.psk_reporter_enabled,
            pota_enabled: self.pota_enabled,
            psk_batch_interval_secs: self.psk_batch_interval_secs.clamp(60, 3_600),
            psk_repeat_cache_secs: self.psk_repeat_cache_secs.clamp(60, 3_600),
            psk_max_pending: self.psk_max_pending.clamp(1, 2_048),
            server_instance_id: self.server_instance_id.clone(),
            server: Some(self.config.server.clone()),
            contest_enabled: self.contest_enabled,
            contest_operating_mode: self.contest_operating_mode,
            contest_split_policy: self.contest_split_policy,
            contest_fox_hound_role: self.contest_fox_hound_role,
            contest_exchange_template: self.contest_exchange_template.trim().to_string(),
            contest_serial_start: self.contest_serial_start.max(1),
            contest_serial_step: self.contest_serial_step.max(1),
            contest_dupe_check: self.contest_dupe_check,
            contest_serial_current: self
                .contest_serial_current
                .max(self.contest_serial_start.max(1)),
            contest_fake_split_offset_hz: self.contest_fake_split_offset_hz,
            hunter_unlocked: self.hunter_unlocked.iter().copied().collect(),
            hunter_acknowledged: self.hunter_acknowledged.iter().copied().collect(),
            hunter_alerts_enabled: self.hunter_alerts_enabled,
            hunter_custom_rules: self.hunter_custom_rules.clone(),
            radio_profiles: Vec::new(),
            mode_radio_profile: self.mode_radio_profile.clone(),
            workspace_mode: self.workspace_mode.label().to_string(),
        }
    }

    pub(super) fn apply_tab_preferences(&mut self, profile: &OperatorProfile) {
        self.workspace_mode =
            parse_workspace_mode_token(&profile.workspace_mode).unwrap_or(WorkspaceMode::Ft8);
        self.ft8_follow_log = profile.follow_log;
        self.ft8_max_log_entries = profile.max_log_entries.clamp(80, 1000);
        self.ft8_deep_decode = profile.deep_decode;
        self.ft8_auto_reply_policy = profile.auto_reply_policy;
        self.ft8_cq_only_view = profile.cq_only_view;
        self.ft8_max_attempts = profile.ft8_max_attempts.clamp(1, 20);
        self.ft8_hold_tx_freq = profile.profile_version >= 3 && profile.hold_tx_freq;
        self.ft4_deep_decode = profile.ft4_deep_decode;
        self.ft4_auto_reply_policy = profile.ft4_auto_reply_policy;
        self.ft4_cq_only_view = profile.ft4_cq_only_view;
        self.ft4_follow_log = profile.ft4_follow_log;
        self.ft4_max_log_entries = profile.ft4_max_log_entries.clamp(80, 300);
        self.ft4_max_attempts = profile.ft4_max_attempts.clamp(1, 20);
        self.automation_unlocked = profile.automation_unlocked;
        self.civ_spectrum_on = profile.civ_spectrum_on;
        self.radio_scope_vbw_wide = profile.radio_scope_vbw_wide;
        self.radio_scope_view = profile.radio_scope_view;
        self.waterfall_theme = profile.waterfall_theme;
        self.radio_waterfall_theme = profile.radio_waterfall_theme;
        // Waterfall geometry is application-wide. Do not reload the profile's
        // historical value when switching radio tabs; doing so makes a tab
        // switch snap the shared deck back to that profile's default.
        if let Ok(mut tuning) = self.display_tuning.lock() {
            tuning.audio_auto_visual = profile.waterfall_auto_visual;
            tuning.audio_waterfall_speed = profile.waterfall_speed;
        }
        self.rx_tone_hz = profile.rx_tone_hz;
        self.tx_tone_hz = if self.ft8_hold_tx_freq {
            profile.tx_tone_hz
        } else {
            profile.rx_tone_hz
        };
        self.ptt_lead_ms = profile.ptt_lead_ms.clamp(0, 500);
        self.ptt_tail_ms = profile.ptt_tail_ms.clamp(0, 500);
        self.cw_wpm = profile.cw_wpm.clamp(5, 40);
        self.cw_tone_hz = profile.cw_tone_hz.clamp(200, 3_000);
        self.recording_enabled = profile.recording_enabled;
        self.recording_modes = profile.recording_modes.clone();
        self.recording_full_width = profile.recording_full_width;
        self.recording_stream = profile.recording_stream;
        if let Ok(mut state) = self.state.lock() {
            state.recording_enabled = self.recording_enabled;
            state.recording_modes = self
                .recording_modes
                .iter()
                .filter(|(_, enabled)| **enabled)
                .filter_map(|(mode, _)| parse_workspace_mode_token(mode))
                .collect();
            state.recording_full_width = self.recording_full_width;
            state.recording_stream = self.recording_stream;
        }
        self.contest_enabled = profile.contest_enabled;
        self.contest_operating_mode = profile.contest_operating_mode;
        self.contest_split_policy = profile.contest_split_policy;
        self.contest_fox_hound_role = profile.contest_fox_hound_role;
        self.contest_exchange_template = profile.contest_exchange_template.clone();
        self.contest_serial_start = profile.contest_serial_start.max(1);
        self.contest_serial_step = profile.contest_serial_step.max(1);
        self.contest_dupe_check = profile.contest_dupe_check;
        self.contest_serial_current = profile
            .contest_serial_current
            .max(self.contest_serial_start)
            .max(1);
        self.contest_fake_split_offset_hz = profile.contest_fake_split_offset_hz.clamp(0, 2_000);
        self.mode_radio_profile = profile.mode_radio_profile.clone();

        // Switching tabs never restores armed or in-flight transmit state.
        self.ft8_autoseq = false;
        self.ft4_autoseq = false;
        self.sstv_tx_armed = false;
        self.ft8_stop_policy = AutoTxStopPolicy::Continuous;
        self.ft4_stop_policy = AutoTxStopPolicy::Continuous;
    }

    pub(crate) fn persist_profile(&mut self, status_prefix: &str) {
        let profile_name = self.selected_profile_name.clone();
        let profile = self.current_operator_profile();
        self.persist_profile_snapshot(&profile_name, &profile, status_prefix);
    }

    pub(crate) fn persist_profile_snapshot(
        &mut self,
        profile_name: &str,
        profile: &OperatorProfile,
        status_prefix: &str,
    ) {
        if let Err(error) = ProfileManager::save_global_and_radio_library(
            &self.current_global_settings(),
            &self.radio_profiles,
        ) {
            warn!(%error, "Global or radio profile library save failed");
        }
        match ProfileManager::save_operator_profile_snapshot(profile_name, profile) {
            Ok(available_profiles) => {
                info!(
                    profile = %profile_name,
                    radio_backend = %profile.radio.backend,
                    radio_model = %profile.radio.model,
                    radio_enabled = profile.radio.enabled,
                    status = %status_prefix,
                    "Operator profile saved"
                );
                self.profile_io_status = format!("{status_prefix} profile ‘{profile_name}’");
                self.available_profiles = available_profiles;
                self.profile_dirty = false;
            }
            Err(err) => {
                warn!(profile = %profile_name, error = %err, "Operator profile save failed");
                self.profile_io_status = format!("Save failed: {err}");
            }
        }
    }

    pub(crate) fn current_global_settings(&self) -> GlobalSettings {
        GlobalSettings {
            callsign: self.station_callsign_or_default().to_string(),
            grid: self.station_grid_or_default().to_string(),
            qth: self.station_qth.trim().to_string(),
            station_rig: String::new(),
            station_antenna: String::new(),
            station_notes: self.station_notes.trim().to_string(),
            llm_prompt_context: self.llm_prompt_context.trim().to_string(),
            sstv_image_requirements: self.sstv_image_requirements.trim().to_string(),
            llm_model_notes: self.llm_model_notes.trim().to_string(),
            gui_scale: self.gui_scale.clamp(GUI_SCALE_MIN, GUI_SCALE_MAX),
            compute_preference: self.compute_preference,
        }
    }

    pub(crate) fn create_profile_from_tab_name(&mut self) {
        let name = self.new_profile_name.trim().to_string();
        if name.is_empty() {
            self.profile_io_status = "Profile name cannot be empty".to_string();
            return;
        }
        if self
            .available_profiles
            .iter()
            .any(|profile| profile.eq_ignore_ascii_case(&name))
        {
            self.profile_io_status = format!("Profile ‘{name}’ already exists");
            return;
        }
        match ProfileManager::create_profile(&name, &self.current_operator_profile()) {
            Ok(available_profiles) => {
                self.available_profiles = available_profiles;
                self.new_profile_name.clear();
                self.new_profile_tab_editing = false;
                self.switch_radio_tab(&name);
                self.profile_io_status = format!("Created profile ‘{name}’");
                self.profile_dirty = false;
            }
            Err(error) => {
                self.profile_io_status = format!("Profile creation failed: {error}");
            }
        }
    }

    pub(crate) fn rename_selected_profile(&mut self) {
        let old_name = self.selected_profile_name.clone();
        let new_name = self.new_profile_name.trim().to_string();
        if new_name.is_empty() {
            self.profile_io_status = "Profile name cannot be empty".to_string();
            return;
        }
        if new_name.eq_ignore_ascii_case(&old_name) {
            self.new_profile_name = old_name;
            return;
        }
        if self
            .available_profiles
            .iter()
            .any(|profile| profile.eq_ignore_ascii_case(&new_name))
        {
            self.profile_io_status = format!("Profile ‘{new_name}’ already exists");
            return;
        }
        let profile = self.current_operator_profile();
        match ProfileManager::rename_profile(&old_name, &new_name, &profile) {
            Ok(available_profiles) => {
                self.selected_profile_name = new_name.clone();
                self.available_profiles = available_profiles;
                self.new_profile_name = new_name.clone();
                self.profile_dirty = false;
                self.profile_io_status = format!("Renamed profile to ‘{new_name}’");
            }
            Err(error) => {
                self.profile_io_status = format!("Profile rename failed: {error}");
            }
        }
    }

    pub(crate) fn delete_operator_profile(&mut self, name: &str) {
        if self.available_profiles.len() <= 1 {
            self.profile_io_status = "The last profile cannot be deleted".to_string();
            return;
        }
        let Some(replacement) = self
            .available_profiles
            .iter()
            .find(|candidate| !candidate.eq_ignore_ascii_case(name))
            .cloned()
        else {
            self.profile_io_status = "No replacement profile is available".to_string();
            return;
        };

        let available_profiles = match ProfileManager::delete_profile(name) {
            Ok(available_profiles) => available_profiles,
            Err(error) => {
                warn!(profile = name, %error, "Operator profile deletion failed");
                self.profile_io_status = format!("Profile deletion failed: {error}");
                return;
            }
        };

        let was_active = name == self.selected_profile_name;
        if was_active {
            self.switch_radio_tab_with_save(&replacement, false);
            if let Some(session) = self.parked_radio_sessions.remove(name) {
                stop_radio_session(session);
            }
        } else if let Some(session) = self.parked_radio_sessions.remove(name) {
            stop_radio_session(session);
        }
        self.available_profiles = available_profiles;
        if self.selected_profile_name.eq_ignore_ascii_case(name) {
            self.selected_profile_name = replacement.clone();
            let _ = select_operator_profile(&replacement);
        }
        info!(
            profile = name,
            "Operator profile deleted and radio tab stopped"
        );
        self.profile_io_status = format!("Deleted profile ‘{name}’ and stopped its tab");
        self.profile_dirty = false;
    }
}
