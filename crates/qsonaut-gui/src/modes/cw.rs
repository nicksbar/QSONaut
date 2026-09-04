use super::super::*;

pub(crate) const BAND_PLAN: &[(&str, u64)] = &[
    ("160m", 1_836_000),
    ("80m", 3_570_000),
    ("60m", 5_351_500),
    ("40m", 7_030_000),
    ("30m", 10_120_000),
    ("20m", 14_050_000),
    ("17m", 18_086_000),
    ("15m", 21_050_000),
    ("12m", 24_930_000),
    ("10m", 28_050_000),
    ("6m", 50_100_000),
    ("2m", 146_520_000),
    ("70cm", 432_100_000),
];

pub(crate) fn workspace_description() -> &'static str {
    "CW · Software Audio"
}

fn next_auto_target_state(scanning: bool, retarget: bool) -> (bool, bool) {
    if !scanning && !retarget {
        (true, false)
    } else if scanning && !retarget {
        (false, true)
    } else {
        (false, false)
    }
}

fn detected_callsigns(text: &str, own_call: &str) -> Vec<String> {
    let mut calls: Vec<String> = Vec::new();
    for token in text.split_whitespace() {
        let call = token.trim_matches(|character: char| !character.is_ascii_alphanumeric());
        if call.is_empty()
            || call.eq_ignore_ascii_case(own_call)
            || !is_probable_callsign(call)
            || calls.iter().any(|seen| seen.eq_ignore_ascii_case(call))
        {
            continue;
        }
        calls.push(call.to_ascii_uppercase());
        if calls.len() == 8 {
            break;
        }
    }
    calls
}

impl QsonautGuiApp {
    fn build_cw_qso_record(&self, snapshot: &GuiState, now: u64) -> Option<QsoRecord> {
        let callsign = self.cw_qso_callsign.trim().to_ascii_uppercase();
        if !is_probable_callsign(&callsign) {
            return None;
        }
        let frequency_hz = snapshot.frequency_hz.unwrap_or_default();
        let mut record = QsoRecord::new(
            &callsign,
            "CW",
            band_for_frequency(frequency_hz),
            frequency_hz,
            self.cw_qso_started_at.unwrap_or(now),
            now,
        );
        record.operation_mode = if self.contest_enabled {
            "Contest".to_string()
        } else {
            self.activity.label().to_string()
        };
        record.report_sent = self.cw_qso_rst_sent.trim().to_ascii_uppercase();
        record.report_received = self.cw_qso_rst_received.trim().to_ascii_uppercase();
        if self.contest_enabled {
            record.contest_serial_sent = Some(self.contest_serial_current.max(1));
            record.contest_exchange_sent = self.contest_exchange_preview(&callsign);
            record.contest_exchange_received = self.cw_qso_exchange_received.trim().to_string();
        }
        record.notes = self.cw_qso_notes.trim().to_string();
        Some(record)
    }

    fn log_cw_qso(&mut self, snapshot: &GuiState) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or_default();
        let Some(record) = self.build_cw_qso_record(snapshot, now) else {
            self.digital_tx_status = "CW QSO needs a valid callsign".to_string();
            return;
        };
        if self.contest_enabled {
            self.advance_contest_serial();
            self.profile_dirty = true;
            self.persist_profile("Auto-saved");
        }
        self.append_qso(record, "CW QSO saved");
        self.cw_qso_callsign.clear();
        self.cw_qso_rst_sent = "599".to_string();
        self.cw_qso_rst_received = "599".to_string();
        self.cw_qso_exchange_received.clear();
        self.cw_qso_notes.clear();
        self.cw_qso_started_at = None;
    }

    pub(crate) fn draw_cw_workspace(&mut self, ui: &mut egui::Ui, snapshot: &GuiState) {
        if let Some(tone_hz) = snapshot.cw_auto_target_tone_hz {
            self.cw_tone_hz = tone_hz.clamp(200, 3_000) as u16;
        }
        ui.horizontal(|ui| {
            ui.heading(workspace_description());
            ui.separator();
            ui.hyperlink_to("cw-dit", "https://github.com/swilcox/cw-dit");
            ui.label("Software audio");
            ui.separator();
            let target_label = if snapshot.cw_auto_target {
                if snapshot.cw_auto_retarget {
                    "🎯 RETARGETING".to_string()
                } else {
                    "🎯 AUTO TARGET".to_string()
                }
            } else if snapshot.cw_auto_retarget {
                "🎯 LOCKED".to_string()
            } else {
                "🎯 AUTO TARGET".to_string()
            };
            let auto_target = ui
                .add_sized(
                    [112.0, 24.0],
                    egui::Button::new(&target_label)
                        .selected(snapshot.cw_auto_target || snapshot.cw_auto_retarget),
                )
                .on_hover_text(
                    "Click cycles: off → scan and lock → keep locked, then scan again after the configured timeout without a signal",
                );
            if auto_target.clicked() {
                let mut state = self.state.lock().expect("ui state lock poisoned");
                (state.cw_auto_target, state.cw_auto_retarget) =
                    next_auto_target_state(state.cw_auto_target, state.cw_auto_retarget);
                state.cw_auto_target_tone_hz = None;
                info!(
                    scanning = state.cw_auto_target,
                    retarget = state.cw_auto_retarget,
                    "CW auto target changed"
                );
            }
            ui.label("Retarget");
            if ui
                .add(
                    egui::Slider::new(&mut self.cw_auto_target_timeout_s, 1..=10)
                        .suffix(" s")
                )
                .changed()
            {
                let mut state = self.state.lock().expect("ui state lock poisoned");
                state.cw_auto_target_timeout_s = self.cw_auto_target_timeout_s;
            }
            if let Some(tone_hz) = snapshot.cw_auto_target_tone_hz {
                ui.label(
                    RichText::new(format!("LOCKED {tone_hz} Hz"))
                        .small()
                        .color(theme_success(ui)),
                );
            }
            ui.separator();
            ui.add_sized(
                [145.0, 22.0],
                egui::Label::new(
                    RichText::new(format!(
                        "{} chars · {:.1} env/s",
                        snapshot.cw_character_count, snapshot.cw_envelope_rate
                    ))
                        .monospace()
                        .color(theme_muted(ui)),
                )
                .truncate(),
            );
            if snapshot.recording_enabled {
                ui.add_sized(
                    [220.0, 22.0],
                    egui::Label::new(
                        RichText::new(&snapshot.cw_recording_status)
                            .small()
                            .color(theme_muted(ui)),
                    )
                    .truncate(),
                );
            }
        });
        ui.separator();

        egui::Frame::group(ui.style())
            .fill(Color32::from_rgb(20, 34, 28))
            .stroke(egui::Stroke::new(1.0_f32, Color32::from_rgb(90, 210, 130)))
            .show(ui, |ui| {
                ui.label(
                    RichText::new(&self.digital_tx_status)
                        .small()
                        .color(theme_muted(ui)),
                );
                ui.label(
                    RichText::new("LIVE DECODE")
                        .small()
                        .strong()
                        .color(theme_success(ui)),
                );
                if ui.small_button("Clear").clicked() {
                    self.state
                        .lock()
                        .expect("ui state lock poisoned")
                        .cw_live_text
                        .clear();
                }
                ui.label(
                    RichText::new(if snapshot.cw_live_text.is_empty() {
                        "Listening…"
                    } else {
                        &snapshot.cw_live_text
                    })
                    .monospace()
                    .strong()
                    .size(18.0),
                );
            });
        ui.separator();

        let own_call = self.station_callsign_or_default().to_string();
        let calls = detected_callsigns(&snapshot.cw_live_text, &own_call);
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_min_height(190.0);
            ui.columns(2, |columns| {
            let left = &mut columns[0];
            left.heading("QSO ASSIST");
            left.label("Detected callsigns");
            if calls.is_empty() {
                left.label(
                    RichText::new("A callsign will appear here as CW is decoded.")
                        .small()
                        .color(theme_muted(left)),
                );
            } else {
                for call in &calls {
                    if left
                        .add(egui::Button::new(format!("{call} · use")))
                        .clicked()
                    {
                        self.cw_qso_callsign.clone_from(call);
                        self.cw_qso_started_at.get_or_insert_with(|| {
                            SystemTime::now()
                                .duration_since(UNIX_EPOCH)
                                .map(|duration| duration.as_secs())
                                .unwrap_or_default()
                        });
                        self.digital_compose = format!("{call} {own_call} 599");
                    }
                }
            }
            left.horizontal(|ui| {
                ui.label("Call");
                ui.add(
                    egui::TextEdit::singleline(&mut self.cw_qso_callsign)
                        .desired_width(120.0)
                        .hint_text("W1AW"),
                );
                if ui.button("Start").clicked() {
                    self.cw_qso_started_at = Some(
                        SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .map(|duration| duration.as_secs())
                            .unwrap_or_default(),
                    );
                }
            });
            left.horizontal(|ui| {
                ui.label("RST out");
                ui.add(
                    egui::TextEdit::singleline(&mut self.cw_qso_rst_sent)
                        .desired_width(55.0)
                        .hint_text("599"),
                );
                ui.label("RST in");
                ui.add(
                    egui::TextEdit::singleline(&mut self.cw_qso_rst_received)
                        .desired_width(55.0)
                        .hint_text("599"),
                );
            });
            left.horizontal(|ui| {
                ui.label("Exchange in");
                ui.add(
                    egui::TextEdit::singleline(&mut self.cw_qso_exchange_received)
                        .desired_width(150.0)
                        .hint_text("optional contest exchange"),
                );
                ui.label("Notes");
                ui.add(
                    egui::TextEdit::singleline(&mut self.cw_qso_notes)
                        .desired_width(150.0)
                        .hint_text("optional"),
                );
            });
            let can_log = is_probable_callsign(self.cw_qso_callsign.trim());
            if left
                .add_enabled(can_log, egui::Button::new("LOG CW QSO").min_size(egui::vec2(140.0, 30.0)))
                .clicked()
            {
                self.log_cw_qso(snapshot);
            }

            let right = &mut columns[1];
            right.heading("CW OPERATING DESK");
            right.horizontal_wrapped(|ui| {
                for (label, message) in [
                    ("CQ", format!("CQ CQ DE {own_call}")),
                    ("599", "599".to_string()),
                    ("TU", "TU 73".to_string()),
                ] {
                    if ui.small_button(label).clicked() {
                        self.digital_compose = message;
                    }
                }
                if self.contest_enabled
                    && ui.small_button("EXCHANGE").on_hover_text("Insert the configured contest exchange").clicked()
                {
                    self.digital_compose = self.contest_exchange_preview(
                        self.cw_qso_callsign.trim(),
                    );
                }
            });
            if right.checkbox(&mut self.contest_enabled, "Contest mode").changed() {
                self.emit_contest_profile_hooks();
                self.profile_dirty = true;
                self.persist_profile("CW contest settings saved to");
            }
            if self.contest_enabled {
                right.horizontal(|ui| {
                    ui.label(format!("Next serial {:03}", self.contest_serial_current.max(1)));
                    egui::ComboBox::from_id_salt("cw_contest_operator_mode")
                        .selected_text(match self.contest_operating_mode {
                            ContestOperatingMode::Run => "Run",
                            ContestOperatingMode::SearchAndPounce => "Search & Pounce",
                        })
                        .show_ui(ui, |ui| {
                            if ui
                                .selectable_value(
                                    &mut self.contest_operating_mode,
                                    ContestOperatingMode::Run,
                                    "Run",
                                )
                                .changed()
                            {
                                self.emit_contest_profile_hooks();
                                self.profile_dirty = true;
                                self.persist_profile("CW contest settings saved to");
                            }
                            if ui
                                .selectable_value(
                                    &mut self.contest_operating_mode,
                                    ContestOperatingMode::SearchAndPounce,
                                    "Search & Pounce",
                                )
                                .changed()
                            {
                                self.emit_contest_profile_hooks();
                                self.profile_dirty = true;
                                self.persist_profile("CW contest settings saved to");
                            }
                        });
                    ui.label(self.contest_guidance_text());
                });
                right.horizontal(|ui| {
                    ui.label("Template");
                    if ui
                        .add(
                            egui::TextEdit::singleline(&mut self.contest_exchange_template)
                                .desired_width(180.0)
                                .hint_text("5NN ${serial}"),
                        )
                        .changed()
                    {
                        self.emit_contest_profile_hooks();
                        self.profile_dirty = true;
                        self.persist_profile("CW contest settings saved to");
                    }
                    if ui.checkbox(&mut self.contest_dupe_check, "Dupe check").changed() {
                        self.emit_contest_profile_hooks();
                        self.profile_dirty = true;
                        self.persist_profile("CW contest settings saved to");
                    }
                });
                right.label(
                    RichText::new(format!(
                        "Exchange: {}",
                        self.contest_exchange_preview(self.cw_qso_callsign.trim())
                    ))
                    .small()
                    .color(theme_accent(right)),
                );
            }
            right.label(
                RichText::new("Select a detected callsign to prepare the next transmission and start a log entry.")
                    .small()
                    .color(theme_muted(right)),
            );
            if !self.cw_qso_callsign.trim().is_empty() {
                right.label(
                    RichText::new(format!("ACTIVE QSO · {}", self.cw_qso_callsign.trim()))
                        .strong()
                        .color(theme_success(right)),
                );
            }
            });
        });
        ui.separator();
        ui.horizontal_wrapped(|ui| {
            ui.label("TX speed");
            if ui
                .add(egui::Slider::new(&mut self.cw_wpm, 5..=40).suffix(" WPM"))
                .changed()
            {
                info!(wpm = self.cw_wpm, "CW transmit speed changed");
                self.profile_dirty = true;
                self.persist_profile("CW speed saved to");
            }
            ui.separator();
            ui.label("RX/TX CW tone");
            if ui
                .add(egui::Slider::new(&mut self.cw_tone_hz, 200..=3_000).suffix(" Hz"))
                .changed()
            {
                info!(tone_hz = self.cw_tone_hz, "CW operating tone changed");
                self.state
                    .lock()
                    .expect("ui state lock poisoned")
                    .cw_auto_target_tone_hz = None;
                self.profile_dirty = true;
                self.persist_profile("CW tone saved to");
            }
        });

        ui.horizontal(|ui| {
            ui.label(RichText::new("TX").strong());
            ui.add(
                egui::TextEdit::singleline(&mut self.digital_compose)
                    .desired_width((ui.available_width() - 180.0).max(180.0))
                    .hint_text("CQ CQ DE W1AW (A-Z, 0-9, spaces)")
                    .font(egui::TextStyle::Monospace),
            );
            if ui
                .add_enabled(
                    !self.digital_compose.trim().is_empty()
                        && !self.digital_tx_active.load(Ordering::Acquire),
                    egui::Button::new("SEND CW"),
                )
                .clicked()
            {
                self.queue_native_digital_tx(WorkspaceMode::Cw);
            }
            if ui
                .add_enabled(
                    self.digital_tx_active.load(Ordering::Acquire),
                    egui::Button::new("STOP TX"),
                )
                .clicked()
            {
                self.stop_native_digital_tx();
            }
        });

        let cw_tx: Vec<_> = self
            .digital_tx_chat
            .iter()
            .rev()
            .filter(|entry| entry.mode == WorkspaceMode::Cw)
            .take(8)
            .collect();
        if !cw_tx.is_empty() {
            ui.separator();
            ui.label(RichText::new("Recent CW TX").strong());
            for entry in cw_tx.into_iter().rev() {
                ui.label(RichText::new(format!("{}  {}", entry.utc, entry.message)).monospace());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use crate::{AppConfig, GraphicsPreferences, GuiState, QsonautGuiApp, QSONAUT_ICON_PNG};
    use eframe::egui;

    use super::{detected_callsigns, next_auto_target_state, workspace_description, BAND_PLAN};

    #[test]
    fn exposes_the_complete_cw_band_plan() {
        assert_eq!(BAND_PLAN.len(), 13);
        assert_eq!(BAND_PLAN.first(), Some(&("160m", 1_836_000)));
        assert_eq!(BAND_PLAN.last(), Some(&("70cm", 432_100_000)));
        assert!(BAND_PLAN.windows(2).all(|bands| bands[0].1 < bands[1].1));
    }

    #[test]
    fn identifies_the_software_cw_workspace() {
        assert_eq!(workspace_description(), "CW · Software Audio");
    }

    #[test]
    fn auto_target_cycles_off_scan_retarget_off() {
        assert_eq!(next_auto_target_state(false, false), (true, false));
        assert_eq!(next_auto_target_state(true, false), (false, true));
        assert_eq!(next_auto_target_state(false, true), (false, false));
    }

    #[test]
    fn detects_clickable_callsigns_without_echoing_the_operator() {
        assert_eq!(
            detected_callsigns("CQ K1ABC K1ABC N0CALL 599", "N0CALL"),
            vec!["K1ABC"]
        );
    }

    #[test]
    fn renders_cw_operator_desk_for_idle_live_and_contest_states() {
        let icon = eframe::icon_data::from_png_bytes(QSONAUT_ICON_PNG).expect("test icon");
        let context = egui::Context::default();
        let mut app = QsonautGuiApp::new_with_context(
            AppConfig::default(),
            false,
            false,
            &context,
            &icon,
            eframe::Renderer::Wgpu,
            None,
            GraphicsPreferences::from_environment(),
            None,
            Vec::new(),
            Arc::new(Mutex::new(None)),
        );
        let mut snapshot = GuiState {
            frequency_hz: Some(14_050_000),
            cw_live_text: "CQ K1ABC 599".to_string(),
            cw_character_count: 12,
            cw_envelope_rate: 4.5,
            cw_auto_target_tone_hz: Some(700),
            recording_enabled: true,
            cw_recording_status: "Recording".to_string(),
            ..GuiState::default()
        };

        app.cw_qso_callsign = " k1abc ".to_string();
        app.cw_qso_rst_sent = " 599 ".to_string();
        app.cw_qso_rst_received = " 579 ".to_string();
        app.cw_qso_notes = "good copy".to_string();
        let normal = app
            .build_cw_qso_record(&snapshot, 100)
            .expect("valid CW record");
        assert_eq!(normal.callsign, "K1ABC");
        assert_eq!(normal.mode, "CW");
        assert_eq!(normal.band, "20m");
        assert_eq!(normal.report_sent, "599");
        assert_eq!(normal.report_received, "579");
        assert_eq!(normal.notes, "good copy");

        app.contest_enabled = true;
        app.contest_serial_current = 7;
        app.cw_qso_exchange_received = "MA".to_string();
        let contest = app
            .build_cw_qso_record(&snapshot, 100)
            .expect("valid contest CW record");
        assert_eq!(contest.operation_mode, "Contest");
        assert_eq!(contest.contest_serial_sent, Some(7));
        assert_eq!(contest.contest_exchange_received, "MA");

        app.cw_qso_callsign = "not-a-call".to_string();
        assert!(app.build_cw_qso_record(&snapshot, 100).is_none());
        app.log_cw_qso(&snapshot);
        assert_eq!(app.digital_tx_status, "CW QSO needs a valid callsign");
        app.cw_qso_callsign = "K1ABC".to_string();

        app.digital_tx_chat.push_back(crate::DigitalTxChatEntry {
            mode: crate::WorkspaceMode::Cw,
            period: 0,
            utc: "00:00:00".to_string(),
            message: "CQ CQ DE N0CALL".to_string(),
        });
        app.digital_tx_chat.push_back(crate::DigitalTxChatEntry {
            mode: crate::WorkspaceMode::Ft8,
            period: 0,
            utc: "00:00:01".to_string(),
            message: "not CW".to_string(),
        });

        for (scanning, retarget, contest_enabled) in [
            (false, false, false),
            (true, false, false),
            (false, true, true),
        ] {
            app.contest_enabled = contest_enabled;
            app.cw_qso_callsign = "K1ABC".to_string();
            snapshot.cw_auto_target = scanning;
            snapshot.cw_auto_retarget = retarget;
            let mut state = app.state.lock().expect("ui state lock poisoned");
            state.cw_auto_target = scanning;
            state.cw_auto_retarget = retarget;
            drop(state);
            let _ = context.run(Default::default(), |context| {
                egui::CentralPanel::default().show(context, |ui| {
                    app.draw_cw_workspace(ui, &snapshot);
                });
            });
            snapshot.recording_enabled = false;
            snapshot.cw_auto_target_tone_hz = None;
        }

        snapshot.cw_live_text.clear();
        app.cw_qso_callsign.clear();
        let _ = context.run(Default::default(), |context| {
            egui::CentralPanel::default().show(context, |ui| {
                app.draw_cw_workspace(ui, &snapshot);
            });
        });
    }
}
