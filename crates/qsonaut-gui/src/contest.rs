use super::*;

fn contest_operating_mode_label(mode: ContestOperatingMode) -> &'static str {
    match mode {
        ContestOperatingMode::Run => "run",
        ContestOperatingMode::SearchAndPounce => "search_and_pounce",
    }
}

fn split_policy_label(policy: SplitPolicy) -> &'static str {
    match policy {
        SplitPolicy::Off => "off",
        SplitPolicy::Fake => "fake",
        SplitPolicy::Rig => "rig",
    }
}

fn fox_hound_role_label(role: FoxHoundRole) -> &'static str {
    match role {
        FoxHoundRole::Disabled => "disabled",
        FoxHoundRole::Fox => "fox",
        FoxHoundRole::Hound => "hound",
    }
}

impl QsonautGuiApp {
    pub(super) fn has_logged_contact_with(
        &self,
        target_call: &str,
        mode: &str,
        band: &str,
    ) -> bool {
        self.qso_log.contacts.iter().any(|contact| {
            callsign_eq(&contact.callsign, target_call)
                && contact.mode.eq_ignore_ascii_case(mode)
                && contact.band.eq_ignore_ascii_case(band)
        })
    }

    pub(super) fn block_duplicate_tx_if_needed(
        &mut self,
        mode: WorkspaceMode,
        compose: &str,
    ) -> bool {
        if !self.contest_dupe_check {
            return false;
        }

        let Some(frequency_hz) = self
            .state
            .lock()
            .expect("ui state lock poisoned")
            .frequency_hz
        else {
            return false;
        };
        let band = band_for_frequency(frequency_hz);

        let operator_call = self.station_callsign_or_default().to_string();
        let Some(target_call) = parse_tx_target_from_compose(compose, &operator_call) else {
            return false;
        };
        if callsign_eq(&target_call, &operator_call) {
            return false;
        }

        if self.has_logged_contact_with(&target_call, mode.label(), band) {
            tracing::warn!(
                target = %target_call,
                mode = %mode.label(),
                band = %band,
                "duplicate-contact guard rejected TX"
            );
            let status = format!(
                "Dupe check blocked TX: {target_call} already worked on {band} {}",
                mode.label()
            );
            self.ft8_seq_status = status.clone();
            self.digital_tx_status = status;
            self.hunter_dupe_blocks = self.hunter_dupe_blocks.saturating_add(1);
            self.push_hunter_alert(
                "🛡️ Dupe avoided",
                format!("{target_call} on {band} {}", mode.label()),
                Color32::from_rgb(255, 170, 75),
            );
            if self.hunter_dupe_blocks >= 10 {
                self.unlock_achievement(
                    AchievementKind::DupeShield,
                    "Dupe Shield",
                    "Prevented 10 duplicate TX attempts",
                );
            }
            return true;
        }
        false
    }

    pub(super) fn contest_effective_tx_tone_hz(&self) -> u32 {
        if self.contest_enabled && self.contest_split_policy == SplitPolicy::Fake {
            self.rx_tone_hz
                .saturating_add(self.contest_fake_split_offset_hz)
                .clamp(100, AUDIO_MAX_FREQ_HZ)
        } else {
            self.tx_tone_hz
        }
    }

    pub(super) fn contest_exchange_preview(&self, target_call: &str) -> String {
        let serial = self.contest_serial_current.max(1);
        let my_call = self.station_callsign_or_default();
        let my_grid = self.station_grid_or_default();
        let template = if self.contest_exchange_template.trim().is_empty() {
            "5NN ${serial}".to_string()
        } else {
            self.contest_exchange_template.trim().to_string()
        };
        template
            .replace("${serial}", &format!("{serial:03}"))
            .replace("${call}", target_call)
            .replace("${target}", target_call)
            .replace("${my_call}", my_call)
            .replace("${grid}", my_grid)
    }

    pub(super) fn advance_contest_serial(&mut self) {
        let next = self
            .contest_serial_current
            .max(self.contest_serial_start.max(1))
            .saturating_add(self.contest_serial_step.max(1));
        self.contest_serial_current = next;
    }

    pub(super) fn contest_guidance_text(&self) -> String {
        let split_hint = match self.contest_split_policy {
            SplitPolicy::Off => "split off",
            SplitPolicy::Fake => "fake split uses a software TX offset",
            SplitPolicy::Rig => "rig split requested (no direct split command yet)",
        };
        let role_hint = match self.contest_fox_hound_role {
            FoxHoundRole::Disabled => "role disabled",
            FoxHoundRole::Fox => "Fox: call CQ, keep the pileup flowing",
            FoxHoundRole::Hound => "Hound: answer quickly and stay on the caller",
        };
        format!("{} · {}", split_hint, role_hint)
    }

    pub(super) fn emit_contest_profile_hooks(&self) {
        info!(
            enabled = self.contest_enabled,
            mode = contest_operating_mode_label(self.contest_operating_mode),
            split = split_policy_label(self.contest_split_policy),
            role = fox_hound_role_label(self.contest_fox_hound_role),
            serial = self.contest_serial_current,
            "Contest operating profile changed"
        );
        self.app_events.publish(AppEvent::ContestProfileChanged {
            enabled: self.contest_enabled,
            operating_mode: contest_operating_mode_label(self.contest_operating_mode).to_string(),
            split_policy: split_policy_label(self.contest_split_policy).to_string(),
            fox_hound_role: fox_hound_role_label(self.contest_fox_hound_role).to_string(),
        });

        self.app_events.publish(AppEvent::AutomationHook {
            kind: "contest_state".to_string(),
            source: "gui.contest_profile".to_string(),
            detail: format!(
                "enabled={} mode={} split={} role={} serial_start={} serial_step={} serial_current={} fake_split_offset_hz={} dupe_check={}",
                self.contest_enabled,
                contest_operating_mode_label(self.contest_operating_mode),
                split_policy_label(self.contest_split_policy),
                fox_hound_role_label(self.contest_fox_hound_role),
                self.contest_serial_start,
                self.contest_serial_step,
                self.contest_serial_current,
                self.contest_fake_split_offset_hz,
                self.contest_dupe_check
            ),
        });
    }
}
