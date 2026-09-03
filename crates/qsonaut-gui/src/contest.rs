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

fn has_logged_contact(contacts: &[QsoRecord], target_call: &str, mode: &str, band: &str) -> bool {
    contacts.iter().any(|contact| {
        callsign_eq(&contact.callsign, target_call)
            && contact.mode.eq_ignore_ascii_case(mode)
            && contact.band.eq_ignore_ascii_case(band)
    })
}

fn contest_effective_tx_tone(
    enabled: bool,
    split_policy: SplitPolicy,
    rx_tone_hz: u32,
    fake_split_offset_hz: u32,
    tx_tone_hz: u32,
) -> u32 {
    if enabled && split_policy == SplitPolicy::Fake {
        rx_tone_hz
            .saturating_add(fake_split_offset_hz)
            .clamp(100, AUDIO_MAX_FREQ_HZ)
    } else {
        tx_tone_hz
    }
}

fn contest_exchange_preview(
    template: &str,
    serial_current: u32,
    target_call: &str,
    my_call: &str,
    my_grid: &str,
) -> String {
    let serial = serial_current.max(1);
    let template = if template.trim().is_empty() {
        "5NN ${serial}".to_string()
    } else {
        template.trim().to_string()
    };
    template
        .replace("${serial}", &format!("{serial:03}"))
        .replace("${call}", target_call)
        .replace("${target}", target_call)
        .replace("${my_call}", my_call)
        .replace("${grid}", my_grid)
}

fn advance_contest_serial(current: u32, start: u32, step: u32) -> u32 {
    current.max(start.max(1)).saturating_add(step.max(1))
}

fn contest_guidance_text(split_policy: SplitPolicy, role: FoxHoundRole) -> String {
    let split_hint = match split_policy {
        SplitPolicy::Off => "split off",
        SplitPolicy::Fake => "fake split uses a software TX offset",
        SplitPolicy::Rig => "rig split requested (no direct split command yet)",
    };
    let role_hint = match role {
        FoxHoundRole::Disabled => "role disabled",
        FoxHoundRole::Fox => "Fox: call CQ, keep the pileup flowing",
        FoxHoundRole::Hound => "Hound: answer quickly and stay on the caller",
    };
    format!("{} · {}", split_hint, role_hint)
}

impl QsonautGuiApp {
    pub(super) fn has_logged_contact_with(
        &self,
        target_call: &str,
        mode: &str,
        band: &str,
    ) -> bool {
        has_logged_contact(&self.qso_log.contacts, target_call, mode, band)
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
                "🛡 Dupe avoided",
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
        contest_effective_tx_tone(
            self.contest_enabled,
            self.contest_split_policy,
            self.rx_tone_hz,
            self.contest_fake_split_offset_hz,
            self.tx_tone_hz,
        )
    }

    pub(super) fn contest_exchange_preview(&self, target_call: &str) -> String {
        contest_exchange_preview(
            &self.contest_exchange_template,
            self.contest_serial_current,
            target_call,
            self.station_callsign_or_default(),
            self.station_grid_or_default(),
        )
    }

    pub(super) fn advance_contest_serial(&mut self) {
        self.contest_serial_current = advance_contest_serial(
            self.contest_serial_current,
            self.contest_serial_start,
            self.contest_serial_step,
        );
    }

    pub(super) fn contest_guidance_text(&self) -> String {
        contest_guidance_text(self.contest_split_policy, self.contest_fox_hound_role)
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

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use eframe::egui;

    use super::{
        advance_contest_serial, contest_effective_tx_tone, contest_exchange_preview,
        contest_guidance_text, contest_operating_mode_label, fox_hound_role_label,
        has_logged_contact, split_policy_label,
    };
    use crate::{ContestOperatingMode, FoxHoundRole, QsoRecord, SplitPolicy};

    #[test]
    fn labels_all_contest_operating_modes() {
        assert_eq!(
            contest_operating_mode_label(ContestOperatingMode::Run),
            "run"
        );
        assert_eq!(
            contest_operating_mode_label(ContestOperatingMode::SearchAndPounce),
            "search_and_pounce"
        );
    }

    #[test]
    fn labels_all_split_policies() {
        assert_eq!(split_policy_label(SplitPolicy::Off), "off");
        assert_eq!(split_policy_label(SplitPolicy::Fake), "fake");
        assert_eq!(split_policy_label(SplitPolicy::Rig), "rig");
    }

    #[test]
    fn labels_all_fox_and_hound_roles() {
        assert_eq!(fox_hound_role_label(FoxHoundRole::Disabled), "disabled");
        assert_eq!(fox_hound_role_label(FoxHoundRole::Fox), "fox");
        assert_eq!(fox_hound_role_label(FoxHoundRole::Hound), "hound");
    }

    #[test]
    fn contest_helpers_cover_dupes_tones_exchange_serials_and_guidance() {
        let mut contact = QsoRecord::new("K1ABC", "ft8", "20M", 14_074_000, 0, 1);
        assert!(!has_logged_contact(&[], "K1ABC", "FT8", "20M"));
        assert!(!has_logged_contact(
            &[contact.clone()],
            "K1ABC",
            "CW",
            "20M"
        ));
        assert!(has_logged_contact(
            &[contact.clone()],
            "k1abc",
            "FT8",
            "20m"
        ));
        contact.callsign = " ".to_string();
        assert!(!has_logged_contact(&[contact], "K1ABC", "FT8", "20M"));

        assert_eq!(
            contest_effective_tx_tone(false, SplitPolicy::Fake, 1_000, 500, 700),
            700
        );
        assert_eq!(
            contest_effective_tx_tone(true, SplitPolicy::Off, 1_000, 500, 700),
            700
        );
        assert_eq!(
            contest_effective_tx_tone(true, SplitPolicy::Fake, 1_000, 500, 700),
            1_500
        );
        assert_eq!(
            contest_effective_tx_tone(true, SplitPolicy::Fake, 3_500, u32::MAX, 700),
            4_000
        );
        assert_eq!(
            contest_effective_tx_tone(true, SplitPolicy::Fake, 0, 0, 700),
            100
        );

        assert_eq!(
            contest_exchange_preview("", 0, "K1ABC", "N0CALL", "AA00"),
            "5NN 001"
        );
        assert_eq!(
            contest_exchange_preview(
                "${call} ${target} ${my_call} ${grid} ${serial}",
                7,
                "K1ABC",
                "N0CALL",
                "AA00"
            ),
            "K1ABC K1ABC N0CALL AA00 007"
        );
        assert_eq!(advance_contest_serial(0, 5, 0), 6);
        assert_eq!(advance_contest_serial(u32::MAX, 1, 1), u32::MAX);

        assert_eq!(
            contest_guidance_text(SplitPolicy::Off, FoxHoundRole::Disabled),
            "split off · role disabled"
        );
        assert!(
            contest_guidance_text(SplitPolicy::Fake, FoxHoundRole::Fox).contains("keep the pileup")
        );
        assert!(contest_guidance_text(SplitPolicy::Rig, FoxHoundRole::Hound)
            .contains("stay on the caller"));
    }

    #[test]
    fn duplicate_guard_covers_safe_noop_and_blocking_paths() {
        let icon = eframe::icon_data::from_png_bytes(crate::QSONAUT_ICON_PNG).expect("test icon");
        let context = egui::Context::default();
        let mut app = crate::QsonautGuiApp::new_with_context(
            crate::AppConfig::default(),
            false,
            false,
            &context,
            &icon,
            eframe::Renderer::Wgpu,
            None,
            crate::GraphicsPreferences::from_environment(),
            None,
            Vec::new(),
            Arc::new(Mutex::new(None)),
        );

        assert!(!app.block_duplicate_tx_if_needed(crate::WorkspaceMode::Cw, "K1ABC N0CALL 599"));
        app.contest_dupe_check = true;
        assert!(!app.block_duplicate_tx_if_needed(crate::WorkspaceMode::Cw, "K1ABC N0CALL 599"));
        app.state
            .lock()
            .expect("ui state lock poisoned")
            .frequency_hz = Some(14_050_000);
        assert!(!app.block_duplicate_tx_if_needed(crate::WorkspaceMode::Cw, "CQ CQ DE N0CALL"));
        assert!(!app.block_duplicate_tx_if_needed(crate::WorkspaceMode::Cw, "N0CALL N0CALL 599"));

        app.qso_log
            .contacts
            .push(QsoRecord::new("K1ABC", "CW", "20m", 14_050_000, 0, 1));
        assert!(app.block_duplicate_tx_if_needed(crate::WorkspaceMode::Cw, "K1ABC N0CALL 599"));
        assert_eq!(app.hunter_dupe_blocks, 1);
        assert!(!app.block_duplicate_tx_if_needed(crate::WorkspaceMode::Cw, "K2ABC N0CALL 599"));
    }
}
