use super::super::*;

pub(crate) const BAND_PLAN: &[(&str, u64)] = &[
    ("160m", 1_840_000),
    ("80m", 3_575_000),
    ("60m", 5_357_000),
    ("40m", 7_076_000),
    ("30m", 10_136_000),
    ("20m", 14_076_000),
    ("17m", 18_104_000),
    ("15m", 21_076_000),
    ("12m", 24_924_000),
    ("10m", 28_076_000),
    ("6m", 50_318_000),
    ("2m", 144_174_000),
    ("70cm", 432_076_000),
];

impl QsonautGuiApp {
    fn draw_ft4_conversation(&self, ui: &mut egui::Ui, snapshot: &GuiState, height: f32) {
        let operator_call = self.station_callsign_or_default().to_string();
        let target = self.digital_seq_target.clone().or_else(|| {
            self.digital_selected
                .as_ref()
                .and_then(|entry| parse_message(&entry.message))
                .map(|message| message.from)
        });
        let mut lines = Vec::new();
        for entry in snapshot
            .digital_decodes
            .iter()
            .filter(|entry| entry.mode == WorkspaceMode::Ft4)
        {
            let belongs = target.as_deref().map_or_else(
                || false,
                |target| {
                    parse_message(&entry.message)
                        .is_some_and(|message| super::exchange::callsign_eq(&message.from, target))
                },
            );
            if belongs {
                lines.push(Ft8ChatLine {
                    period: entry.period,
                    utc: entry.utc.clone(),
                    message: entry.message.clone(),
                    detail: format!(
                        "RX {:+.1} dB · {:+.2}s · {} Hz",
                        entry.snr_db, entry.dt_s, entry.freq_hz
                    ),
                    direction: Ft8ChatDirection::Rx,
                });
            }
        }
        for entry in self
            .digital_tx_chat
            .iter()
            .filter(|entry| entry.mode == WorkspaceMode::Ft4)
        {
            let belongs = target.as_deref().is_some_and(|target| {
                parse_message(&entry.message).is_some_and(|message| {
                    message
                        .to
                        .as_deref()
                        .is_some_and(|to| super::exchange::callsign_eq(to, target))
                })
            });
            if belongs {
                lines.push(Ft8ChatLine {
                    period: entry.period,
                    utc: entry.utc.clone(),
                    message: entry.message.clone(),
                    detail: format!("TX · {} Hz", self.tx_tone_hz),
                    direction: Ft8ChatDirection::Tx,
                });
            }
        }
        lines.sort_by_key(|line| (line.period, line.direction == Ft8ChatDirection::Tx));
        if lines.len() > 30 {
            lines.drain(..lines.len() - 30);
        }
        let rx_level = audio_cursor_level(&snapshot.audio_waterfall_rows, self.rx_tone_hz);
        let tx_level = audio_cursor_level(&snapshot.audio_waterfall_rows, self.tx_tone_hz);

        egui::Frame::group(ui.style())
            .fill(Color32::from_rgb(20, 23, 28))
            .show(ui, |ui| {
                ui.set_min_height(height);
                ui.set_max_height(height);
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new(
                            target
                                .as_deref()
                                .map(|call| format!("💬 FT4 CONTACT VIEW · {call}"))
                                .unwrap_or_else(|| {
                                    "💬 FT4 CONTACT VIEW · SELECT A CALLSIGN".to_string()
                                }),
                        )
                        .strong()
                        .color(Color32::LIGHT_BLUE),
                    );
                    if let Some(session) = &self.ft4_session {
                        ui.label(
                            RichText::new(qso_stage_label(session.stage))
                                .small()
                                .color(Color32::from_rgb(220, 180, 90)),
                        );
                    }
                    ui.separator();
                    ui.label(
                        RichText::new(format!("RX CURSOR {} Hz", self.rx_tone_hz))
                            .monospace()
                            .color(Color32::from_rgb(120, 220, 120)),
                    );
                    ui.add(egui::ProgressBar::new(rx_level as f32 / 255.0).desired_width(70.0));
                    ui.label(
                        RichText::new(format!("TX CURSOR {} Hz", self.tx_tone_hz))
                            .monospace()
                            .color(Color32::from_rgb(220, 160, 80)),
                    );
                    ui.add(egui::ProgressBar::new(tx_level as f32 / 255.0).desired_width(70.0));
                });
                ui.separator();
                egui::ScrollArea::vertical()
                    .id_salt("ft4_conversation")
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        if lines.is_empty() {
                            ui.centered_and_justified(|ui| {
                                ui.label(
                                    RichText::new(
                                        "✨ Select an FT4 decode to track that callsign here.",
                                    )
                                    .color(Color32::GRAY),
                                );
                            });
                        }
                        for line in lines {
                            let is_tx = line.direction == Ft8ChatDirection::Tx;
                            let call_hit = (!is_tx)
                                .then(|| operator_call_hit(&line.message, &operator_call))
                                .flatten();
                            let layout = if is_tx {
                                egui::Layout::right_to_left(egui::Align::Min)
                            } else {
                                egui::Layout::left_to_right(egui::Align::Min)
                            };
                            ui.with_layout(layout, |ui| {
                                let (fill, stroke) = call_hit.map_or_else(
                                    || {
                                        (
                                            if is_tx {
                                                Color32::from_rgb(53, 43, 25)
                                            } else {
                                                Color32::from_rgb(25, 49, 38)
                                            },
                                            egui::Stroke::NONE,
                                        )
                                    },
                                    |hit| {
                                        let (_, accent, fill) = call_hit_badge(hit);
                                        (fill, egui::Stroke::new(2.0_f32, accent))
                                    },
                                );
                                egui::Frame::group(ui.style())
                                    .fill(fill)
                                    .stroke(stroke)
                                    .show(ui, |ui| {
                                        if let Some(hit) = call_hit {
                                            let (badge, accent, _) = call_hit_badge(hit);
                                            ui.label(RichText::new(badge).strong().color(accent));
                                        }
                                        ui.label(RichText::new(&line.message).monospace().strong());
                                        ui.label(
                                            RichText::new(format!(
                                                "{} · {}",
                                                line.utc, line.detail
                                            ))
                                            .small()
                                            .color(Color32::GRAY),
                                        );
                                    });
                            });
                            ui.add_space(2.0);
                        }
                    });
            });
    }

    pub(crate) fn draw_ft4_workspace(&mut self, ui: &mut egui::Ui, snapshot: &GuiState) {
        let now_s = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs_f64())
            .unwrap_or_default();
        let progress = (now_s % FT4_SLOT_SECONDS) / FT4_SLOT_SECONDS;
        let tx_active = self.digital_tx_active.load(Ordering::Acquire);
        ui.horizontal_wrapped(|ui| {
            ui.heading("FT4");
            ui.separator();
            let phase_label = if snapshot.ptt_on {
                "TX NOW"
            } else if tx_active {
                "TX QUEUED"
            } else if self.ft4_autoseq {
                "AUTO TX ARMED"
            } else {
                "RX · DISARMED"
            };
            ui.label(
                RichText::new(phase_label)
                    .strong()
                    .color(if snapshot.ptt_on || tx_active {
                        Color32::from_rgb(210, 90, 60)
                    } else if self.ft4_autoseq {
                        Color32::from_rgb(255, 170, 75)
                    } else {
                        Color32::from_rgb(105, 190, 225)
                    }),
            );
            ui.add(
                egui::ProgressBar::new(progress as f32)
                    .desired_width(150.0)
                    .text(format!("{:.1}s", FT4_SLOT_SECONDS * (1.0 - progress))),
            );
            if let Some(hz) = snapshot.frequency_hz {
                ui.label(
                    RichText::new(format!("{:.3} MHz", hz as f64 / 1_000_000.0))
                        .monospace()
                        .strong(),
                );
            }
            ui.label(RichText::new(&snapshot.mode).monospace());
            ui.separator();
            ui.label(
                RichText::new(format!("RX {} Hz", self.rx_tone_hz))
                    .monospace()
                    .color(Color32::from_rgb(120, 220, 120)),
            );
            ui.label(
                RichText::new(format!("TX {} Hz", self.tx_tone_hz))
                    .monospace()
                    .color(Color32::from_rgb(220, 160, 80)),
            );
            ui.separator();
            let seq_label = self
                .ft4_session
                .as_ref()
                .map(|session| qso_stage_label(session.stage))
                .unwrap_or(if self.ft4_autoseq { "ARMED" } else { "IDLE" });
            ui.label(
                RichText::new(format!("SEQ {seq_label}"))
                    .monospace()
                    .color(Color32::LIGHT_BLUE),
            );

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let previous_stop_policy = self.ft4_stop_policy;
                egui::ComboBox::from_id_salt("ft4_stop_policy")
                    .selected_text(self.ft4_stop_policy.label())
                    .show_ui(ui, |ui| {
                        for policy in AutoTxStopPolicy::ALL {
                            ui.selectable_value(&mut self.ft4_stop_policy, policy, policy.label());
                        }
                    });
                if self.ft4_stop_policy != previous_stop_policy {
                    self.digital_tx_status = match self.ft4_stop_policy {
                        AutoTxStopPolicy::Continuous => {
                            "FT4 watchdog set to keep running".to_string()
                        }
                        AutoTxStopPolicy::AfterNextTx => {
                            "FT4 will stop after the next transmission".to_string()
                        }
                        AutoTxStopPolicy::AfterCurrentQso => {
                            "FT4 will stop when the current QSO completes".to_string()
                        }
                    };
                }

                let hold_label = if self.ft8_hold_tx_freq {
                    "HOLD TX FREQ"
                } else {
                    "TX TRACKS RX"
                };
                if ui
                    .button(RichText::new(hold_label).color(if self.ft8_hold_tx_freq {
                        Color32::from_rgb(120, 200, 220)
                    } else {
                        Color32::from_rgb(120, 220, 120)
                    }))
                    .clicked()
                {
                    self.ft8_hold_tx_freq = !self.ft8_hold_tx_freq;
                    if !self.ft8_hold_tx_freq {
                        self.tx_tone_hz = self.rx_tone_hz;
                    }
                    self.profile_dirty = true;
                    self.persist_profile("Auto-saved");
                }

                let deep_label = if self.ft4_deep_decode {
                    "DECODE: DEEP"
                } else {
                    "DECODE: FAST"
                };
                if ui
                    .button(RichText::new(deep_label).color(if self.ft4_deep_decode {
                        Color32::YELLOW
                    } else {
                        Color32::LIGHT_GREEN
                    }))
                    .clicked()
                {
                    self.ft4_deep_decode = !self.ft4_deep_decode;
                    self.profile_dirty = true;
                    self.persist_profile("Auto-saved");
                }
            });
        });
        ui.horizontal_wrapped(|ui| {
            let (auto_label, auto_fill, auto_stroke) = if self.ft4_autoseq {
                (
                    "🔥 FT4 TX ARMED · CLICK TO DISARM ALL",
                    Color32::from_rgb(92, 43, 25),
                    Color32::from_rgb(255, 151, 72),
                )
            } else {
                (
                    "🔒 FT4 AUTO DISARMED · CLICK TO ARM",
                    Color32::from_rgb(28, 52, 70),
                    Color32::from_rgb(92, 174, 220),
                )
            };
            if ui
                .add(
                    egui::Button::new(RichText::new(auto_label).strong().color(Color32::WHITE))
                        .fill(auto_fill)
                        .stroke(egui::Stroke::new(1.5_f32, auto_stroke)),
                )
                .clicked()
            {
                if self.ft4_autoseq {
                    self.disarm_all_tx("All TX stopped and disarmed by operator");
                } else {
                    self.ft4_autoseq = true;
                    self.digital_tx_status =
                        "FT4 automatic operation armed; waiting for an exchange".to_string();
                    self.profile_dirty = true;
                    self.persist_profile("FT4 TX armed");
                }
            }
            ui.label("Select caller:");
            let previous_policy = self.ft4_auto_reply_policy;
            egui::ComboBox::from_id_salt("ft4_auto_reply_policy")
                .selected_text(self.ft4_auto_reply_policy.label())
                .show_ui(ui, |ui| {
                    for policy in AutoReplyPolicy::ALL {
                        ui.selectable_value(
                            &mut self.ft4_auto_reply_policy,
                            policy,
                            policy.label(),
                        );
                    }
                });
            if self.ft4_auto_reply_policy != previous_policy {
                self.profile_dirty = true;
                self.persist_profile("Auto-saved");
            }
            if ui
                .checkbox(&mut self.ft8_auto_answer_cq, "Answer unattended CQs")
                .on_hover_text(
                    "When armed, automatically reply to stations calling CQ even if you are not \
                     actively watching. QSONaut picks the strongest/nearest caller and starts the \
                     exchange on its own. Leave OFF if you want to choose every caller manually. \
                     Shared FT8/FT4 policy; only active while this mode is armed.",
                )
                .changed()
            {
                self.profile_dirty = true;
                self.persist_profile("Auto-saved");
            }
            ui.separator();
            ui.label("Max unanswered");
            if ui
                .add(egui::DragValue::new(&mut self.ft4_max_attempts).range(1..=20))
                .changed()
            {
                self.profile_dirty = true;
                self.persist_profile("Auto-saved");
            }
            ui.separator();
            ui.label(
                RichText::new(&snapshot.digital_decode_status)
                    .small()
                    .color(Color32::GRAY),
            );
            if let Some(offset) = snapshot.ft4_clock_offset_s {
                ui.label(
                    RichText::new(format!("Adaptive clock dT {offset:+.2}s"))
                        .small()
                        .color(if offset.abs() > 0.5 {
                            Color32::YELLOW
                        } else {
                            Color32::LIGHT_GREEN
                        }),
                );
            }
            if let Some(level) = snapshot.audio_level_dbfs {
                ui.label(
                    RichText::new(format!(
                        "Input {level:.0} dBFS / clip {:.1}%",
                        snapshot.audio_clip_percent
                    ))
                    .small()
                    .color(Color32::GRAY),
                );
            }
        });
        ui.separator();

        let stats = digital_activity_stats(&snapshot.digital_decodes, WorkspaceMode::Ft4);
        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new("📊 BAND PULSE")
                    .strong()
                    .color(Color32::LIGHT_BLUE),
            );
            ft8_stat_chip(
                ui,
                "This cycle",
                stats.latest_cycle.to_string(),
                format!("{} CQ", stats.cq_this_cycle),
            );
            ft8_stat_chip(
                ui,
                "Average",
                format!("{:.1}/cycle", stats.average_per_cycle),
                "rolling log".to_string(),
            );
            ft8_stat_chip(
                ui,
                "Stations",
                stats.unique_stations.to_string(),
                "unique heard".to_string(),
            );
            let (heard, detail) = stats
                .most_heard
                .map(|(call, count)| (call, format!("{count} decodes")))
                .unwrap_or_else(|| ("—".to_string(), "waiting".to_string()));
            ft8_stat_chip(ui, "Most heard", heard, detail);
            ft8_stat_chip(
                ui,
                "Median SNR",
                stats
                    .median_snr
                    .map(|snr| format!("{snr:+} dB"))
                    .unwrap_or_else(|| "—".to_string()),
                "rolling log".to_string(),
            );
        });
        ui.add_space(4.0);

        let mut entries: Vec<DigitalDecodeEntry> = snapshot
            .digital_decodes
            .iter()
            .filter(|entry| {
                entry.mode == WorkspaceMode::Ft4
                    && (!self.ft4_cq_only_view || entry.message.starts_with("CQ "))
            })
            .cloned()
            .collect();
        if entries.len() > self.ft4_max_log_entries {
            entries.drain(..entries.len() - self.ft4_max_log_entries);
        }
        let operator_call = self.station_callsign_or_default().to_string();
        let active_band = snapshot.frequency_hz.map(band_for_frequency).unwrap_or("");
        egui::TopBottomPanel::top("ft4_decode_deck")
            .resizable(true)
            .show_separator_line(true)
            .default_height(360.0)
            .height_range(220.0..=700.0)
            .show_inside(ui, |ui| {
                let deck_rect = ui.available_rect_before_wrap();
                ui.allocate_rect(deck_rect, egui::Sense::hover());
                let mut deck_ui = ui.new_child(
                    egui::UiBuilder::new()
                        .id_salt("ft4_decode_deck_contents")
                        .max_rect(deck_rect)
                        .layout(egui::Layout::top_down(egui::Align::Min)),
                );
                deck_ui.set_clip_rect(deck_rect);
                let panel_h = deck_ui.available_height();
                let decode_h = panel_h;
                let conversation_h = panel_h;
                deck_ui.columns(2, |columns| {
                    let left = &mut columns[0];
                    left.set_min_width(0.0);
                    egui::Frame::dark_canvas(left.style()).show(left, |ui| {
                        ui.set_min_height(decode_h);
                        ui.set_max_height(decode_h);
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("⚡ FT4 LIVE DECODES")
                                    .strong()
                                    .color(Color32::LIGHT_BLUE),
                            );
                            ui.separator();
                            if ui.checkbox(&mut self.ft4_cq_only_view, "CQ only").changed() {
                                self.profile_dirty = true;
                                self.persist_profile("Auto-saved");
                            }
                            if ui.checkbox(&mut self.ft4_follow_log, "Follow").changed() {
                                self.profile_dirty = true;
                                self.persist_profile("Auto-saved");
                            }
                            ui.label("Keep");
                            if ui
                                .add(
                                    egui::DragValue::new(&mut self.ft4_max_log_entries)
                                        .range(80..=300)
                                        .speed(5),
                                )
                                .changed()
                            {
                                self.profile_dirty = true;
                                self.persist_profile("Auto-saved");
                            }
                            ui.label("rows");
                            ui.label(format!("{} msgs", entries.len()));
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui.small_button("Clear").clicked() {
                                        self.state
                                            .lock()
                                            .expect("ui state lock poisoned")
                                            .digital_decodes
                                            .retain(|entry| entry.mode != WorkspaceMode::Ft4);
                                        self.digital_selected = None;
                                        self.digital_seq_target = None;
                                        self.ft4_session = None;
                                        self.digital_tx_chat
                                            .retain(|entry| entry.mode != WorkspaceMode::Ft4);
                                    }
                                },
                            );
                        });
                        ui.separator();
                        ui.label(
                            RichText::new("UTC          SNR     dT      Hz  Message")
                                .monospace()
                                .strong(),
                        );
                        ui.separator();
                        egui::ScrollArea::vertical()
                            .id_salt("ft4_global_decodes")
                            .stick_to_bottom(self.ft4_follow_log)
                            .show(ui, |ui| {
                                if entries.is_empty() {
                                    ui.label(
                                        RichText::new(
                                            "🌊 Listening hard… collecting the next FT4 waveform.",
                                        )
                                        .color(Color32::GRAY),
                                    );
                                }
                                for entry in &entries {
                                    let selected =
                                        self.digital_selected.as_ref().is_some_and(|selected| {
                                            selected.period == entry.period
                                                && selected.freq_hz == entry.freq_hz
                                                && selected.message == entry.message
                                        });
                                    let call_hit =
                                        operator_call_hit(&entry.message, &operator_call);
                                    let worked_call =
                                        parse_message(&entry.message).map(|parsed| parsed.from);
                                    let is_worked = worked_call.as_ref().is_some_and(|call| {
                                        !call.is_empty()
                                            && !active_band.is_empty()
                                            && self.has_logged_contact_with(
                                                call,
                                                "FT4",
                                                active_band,
                                            )
                                    });
                                    let row = RichText::new(format!(
                                        "{:12}  {:+5.1}  {:+6.2}  {:>5}  {}",
                                        entry.utc,
                                        entry.snr_db,
                                        entry.dt_s,
                                        entry.freq_hz,
                                        entry.message
                                    ))
                                    .monospace()
                                    .color(
                                        if let Some(hit) = call_hit {
                                            call_hit_badge(hit).1
                                        } else if entry.message.starts_with("CQ ") {
                                            Color32::LIGHT_GREEN
                                        } else {
                                            Color32::LIGHT_GRAY
                                        },
                                    );
                                    let response = if let Some(hit) = call_hit {
                                        let (badge, accent, fill) = call_hit_badge(hit);
                                        egui::Frame::group(ui.style())
                                            .fill(fill)
                                            .stroke(egui::Stroke::new(1.5_f32, accent))
                                            .show(ui, |ui| {
                                                ui.horizontal(|ui| {
                                                    ui.label(
                                                        RichText::new(badge).strong().color(accent),
                                                    );
                                                    ui.selectable_label(selected, row)
                                                })
                                                .inner
                                            })
                                            .inner
                                    } else if is_worked {
                                        egui::Frame::group(ui.style())
                                            .fill(Color32::from_rgb(20, 58, 30))
                                            .stroke(egui::Stroke::new(
                                                1.2_f32,
                                                Color32::from_rgb(120, 220, 145),
                                            ))
                                            .show(ui, |ui| {
                                                ui.horizontal(|ui| {
                                                    ui.label(
                                                        RichText::new("✅ WORKED").strong().color(
                                                            Color32::from_rgb(120, 220, 145),
                                                        ),
                                                    );
                                                    ui.selectable_label(selected, row)
                                                })
                                                .inner
                                            })
                                            .inner
                                    } else {
                                        ui.selectable_label(selected, row)
                                    };
                                    let now = Instant::now();
                                    let synthetic_double = self
                                        .digital_last_click
                                        .as_ref()
                                        .is_some_and(|(period, freq_hz, message, at)| {
                                            *period == entry.period
                                                && *freq_hz == entry.freq_hz
                                                && message == &entry.message
                                                && now.duration_since(*at)
                                                    <= Duration::from_millis(500)
                                        });
                                    if response.clicked() {
                                        self.digital_last_click = Some((
                                            entry.period,
                                            entry.freq_hz,
                                            entry.message.clone(),
                                            now,
                                        ));
                                        self.digital_selected = Some(entry.clone());
                                        self.digital_seq_target = parse_message(&entry.message)
                                            .map(|message| message.from);
                                        self.rx_tone_hz = entry.freq_hz;
                                        if !self.ft8_hold_tx_freq {
                                            self.tx_tone_hz = entry.freq_hz;
                                        }
                                    }
                                    if synthetic_double || response.double_clicked() {
                                        if let Some(message) = parse_message(&entry.message) {
                                            let my_call =
                                                self.station_callsign_or_default().to_string();
                                            let my_grid =
                                                self.station_grid_or_default().to_string();
                                            let mut session = QsoSession::start(
                                                message.from.clone(),
                                                entry.period,
                                            );
                                            if let Some(reply) = session.response_to(
                                                &message,
                                                &my_call,
                                                &my_grid,
                                                entry.snr_db.round() as i8,
                                                entry.period,
                                            ) {
                                                self.digital_compose = reply;
                                                self.digital_seq_target = Some(message.from);
                                                self.ft4_session = Some(session);
                                                self.ft4_autoseq = true;
                                                self.queue_native_digital_tx(WorkspaceMode::Ft4);
                                                self.profile_dirty = true;
                                                self.persist_profile("Auto-saved");
                                            }
                                        }
                                    }
                                }
                            });
                    });
                    let right = &mut columns[1];
                    right.set_min_width(0.0);
                    self.draw_ft4_conversation(right, snapshot, conversation_h);
                });
            });
        ui.add_space(4.0);

        let tx_h = (ui.available_height() * 0.22).clamp(88.0, 180.0);

        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_min_height(tx_h);
            ui.horizontal(|ui| {
                ui.label(RichText::new("📣 FT4 TX DECK").strong());
                ui.add_enabled(
                    !tx_active,
                    egui::TextEdit::singleline(&mut self.digital_compose)
                        .desired_width((ui.available_width() - 260.0).max(180.0))
                        .hint_text("CQ W1AW FN20")
                        .font(egui::TextStyle::Monospace),
                );
                if ui.button("CALL CQ").clicked() {
                    self.digital_compose = format!(
                        "CQ {} {}",
                        self.station_callsign_or_default(),
                        self.station_grid_or_default()
                    );
                    self.digital_seq_target = None;
                    self.ft4_session = None;
                    self.ft4_autoseq = true;
                    self.queue_native_digital_tx(WorkspaceMode::Ft4);
                }
                if ui
                    .add_enabled(
                        !tx_active && !self.digital_compose.trim().is_empty(),
                        egui::Button::new("SEND NEXT SLOT"),
                    )
                    .clicked()
                {
                    self.queue_native_digital_tx(WorkspaceMode::Ft4);
                }
                if ui
                    .add(
                        egui::Button::new(
                            RichText::new("⛔ STOP + DISARM ALL")
                                .strong()
                                .color(Color32::WHITE),
                        )
                        .fill(Color32::from_rgb(112, 30, 38))
                        .stroke(egui::Stroke::new(1.5_f32, Color32::from_rgb(255, 100, 110))),
                    )
                    .on_hover_text(
                        "Drop PTT and permanently cancel FT8/FT4 automatic TX until re-armed",
                    )
                    .clicked()
                {
                    self.disarm_all_tx("All TX stopped and disarmed by operator");
                }
            });
            ui.horizontal_wrapped(|ui| {
                if let Some(target) = self.digital_seq_target.clone() {
                    let my = self.station_callsign_or_default().to_string();
                    let grid = self.station_grid_or_default().to_string();
                    for (label, exchange) in
                        [("Grid", grid.as_str()), ("RR73", "RR73"), ("73", "73")]
                    {
                        if ui.small_button(label).clicked() {
                            self.digital_compose = format!("{target} {my} {exchange}");
                        }
                    }
                }
                if let Some(target_call) = parse_tx_target_from_compose(
                    &self.digital_compose,
                    self.station_callsign_or_default(),
                ) {
                    if let Some(frequency_hz) = snapshot.frequency_hz {
                        let band = band_for_frequency(frequency_hz);
                        if !band.is_empty()
                            && self.has_logged_contact_with(&target_call, "FT4", band)
                        {
                            ui.label(
                                RichText::new(format!(
                                    "⚠ Dupe risk: {target_call} already worked on {band} FT4"
                                ))
                                .small()
                                .color(Color32::from_rgb(255, 180, 82)),
                            );
                        }
                    }
                }
                ui.label(
                    RichText::new(&self.digital_tx_status)
                        .small()
                        .color(Color32::GRAY),
                );
            });
            ui.horizontal(|ui| {
                ui.label("PTT lead");
                if ui
                    .add(
                        egui::DragValue::new(&mut self.ptt_lead_ms)
                            .range(0..=500)
                            .suffix(" ms"),
                    )
                    .changed()
                {
                    self.profile_dirty = true;
                    self.persist_profile("Auto-saved");
                }
                ui.label("tail");
                if ui
                    .add(
                        egui::DragValue::new(&mut self.ptt_tail_ms)
                            .range(0..=500)
                            .suffix(" ms"),
                    )
                    .changed()
                {
                    self.profile_dirty = true;
                    self.persist_profile("Auto-saved");
                }
            });
        });
    }
}
