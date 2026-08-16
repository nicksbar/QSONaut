use super::super::*;

pub(crate) const BAND_PLAN: &[(&str, u64)] = &[
    ("160m", 1_840_000), ("80m", 3_573_000), ("60m", 5_357_000),
    ("40m", 7_074_000), ("30m", 10_136_000), ("20m", 14_074_000),
    ("17m", 18_100_000), ("15m", 21_074_000), ("12m", 24_924_000),
    ("10m", 28_074_000), ("6m", 50_313_000), ("2m", 144_174_000),
    ("70cm", 432_074_000),
];

impl QsonautGuiApp {
    fn ft8_conversation_target(&self) -> Option<String> {
        self.ft8_seq_target
            .clone()
            .or_else(|| {
                self.ft8_session
                    .as_ref()
                    .map(|session| session.target.clone())
            })
            .or_else(|| {
                self.ft8_selected
                    .and_then(|index| self.ft8_log.get(index))
                    .and_then(|entry| parse_message(&entry.message))
                    .map(|message| message.from)
            })
    }

    fn draw_ft8_activity_stats(&self, ui: &mut egui::Ui) {
        let stats = ft8_activity_stats(&self.ft8_log);
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
            let (most_heard, detail) = stats
                .most_heard
                .map(|(call, count)| (call, format!("{count} decodes")))
                .unwrap_or_else(|| ("—".to_string(), "waiting".to_string()));
            ft8_stat_chip(ui, "Most heard", most_heard, detail);
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
    }

    fn draw_ft8_conversation(&self, ui: &mut egui::Ui, snapshot: &GuiState, height: f32) {
        let target = self.ft8_conversation_target();
        let operator_call = self.station_callsign_or_default().to_string();
        let rx_level = audio_cursor_level(&snapshot.audio_waterfall_rows, self.rx_tone_hz);
        let tx_level = audio_cursor_level(&snapshot.audio_waterfall_rows, self.tx_tone_hz);
        let mut lines = Vec::new();

        for entry in &self.ft8_log {
            let belongs = if let Some(target) = target.as_deref() {
                parse_message(&entry.message).is_some_and(|message| {
                    // RX chat contains transmissions from the station we are
                    // working, not unrelated callers transmitting to them.
                    super::exchange::callsign_eq(&message.from, target)
                })
            } else {
                false
            };
            if belongs {
                lines.push(Ft8ChatLine {
                    period: entry.period,
                    utc: entry.utc.clone(),
                    message: entry.message.clone(),
                    detail: format!(
                        "RX {:+} dB · {:.1}s · {} Hz",
                        entry.snr_db, entry.dt_s, entry.freq_hz
                    ),
                    direction: Ft8ChatDirection::Rx,
                });
            }
        }
        for entry in &self.ft8_tx_chat {
            let belongs = if let Some(target) = target.as_deref() {
                parse_message(&entry.message).is_some_and(|message| {
                    super::exchange::callsign_eq(&message.from, target)
                        || message
                            .to
                            .as_deref()
                            .is_some_and(|to| super::exchange::callsign_eq(to, target))
                })
            } else {
                false
            };
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

        egui::Frame::group(ui.style())
            .fill(Color32::from_rgb(20, 23, 28))
            .show(ui, |ui| {
                ui.set_min_height(height);
                ui.set_max_height(height);
                ui.horizontal_wrapped(|ui| {
                    let title = target
                        .as_deref()
                        .map(|call| format!("💬 CONTACT VIEW · {call}"))
                        .unwrap_or_else(|| "💬 CONTACT VIEW · SELECT A CALLSIGN".to_string());
                    ui.label(RichText::new(title).strong().color(Color32::LIGHT_BLUE));
                    if let Some(session) = &self.ft8_session {
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
                    .id_salt("ft8_conversation")
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        if lines.is_empty() {
                            ui.centered_and_justified(|ui| {
                                ui.label(
                                    RichText::new(if target.is_some() {
                                        "📡 Listening for this station’s next move…"
                                    } else {
                                        "✨ Select a decode to track that callsign here."
                                    })
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

    pub(crate) fn draw_ft8_workspace(
        &mut self,
        ui: &mut egui::Ui,
        _ctx: &egui::Context,
        snapshot: &GuiState,
    ) {
        // ── Header row ──────────────────────────────────────────────────────
        ui.horizontal(|ui| {
            ui.heading("FT8");
            ui.separator();

            // 15-second period progress
            let progress = ft8_period_progress();
            let tx_active = snapshot.ptt_on || self.ft8_tx_active.load(Ordering::Acquire);
            let phase_label = if snapshot.ptt_on {
                "TX NOW"
            } else if self.ft8_tx_queued_period.is_some() {
                "TX QUEUED"
            } else if self.ft8_autoseq && self.ft8_session.is_some() {
                "AUTO TX ARMED"
            } else {
                "RX"
            };
            let period_color = if tx_active || self.ft8_tx_queued_period.is_some() {
                Color32::from_rgb(190, 70, 35)
            } else if self.ft8_autoseq && self.ft8_session.is_some() {
                Color32::from_rgb(220, 160, 80)
            } else {
                Color32::from_rgb(30, 130, 30)
            };
            ui.label(RichText::new(phase_label).strong().color(period_color));
            let bar_w = 140.0;
            let bar_h = 14.0;
            let (rect, _) = ui.allocate_exact_size(egui::vec2(bar_w, bar_h), egui::Sense::hover());
            ui.painter().rect_filled(rect, 2.0, Color32::from_gray(30));
            let fill = egui::Rect::from_min_size(rect.min, egui::vec2(bar_w * progress, bar_h));
            ui.painter().rect_filled(fill, 2.0, period_color);
            let remaining = 15.0 * (1.0 - progress);
            ui.label(format!("{remaining:.1}s"));

            ui.separator();
            // Freq display
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
            ui.label(
                RichText::new(format!("SEQ {}", self.ft8_seq_state.label()))
                    .monospace()
                    .color(Color32::LIGHT_BLUE),
            );

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let previous_stop_policy = self.ft8_stop_policy;
                egui::ComboBox::from_id_salt("ft8_stop_policy")
                    .selected_text(self.ft8_stop_policy.label())
                    .show_ui(ui, |ui| {
                        for policy in AutoTxStopPolicy::ALL {
                            ui.selectable_value(&mut self.ft8_stop_policy, policy, policy.label());
                        }
                    });
                if self.ft8_stop_policy != previous_stop_policy {
                    self.ft8_seq_status = match self.ft8_stop_policy {
                        AutoTxStopPolicy::Continuous => {
                            "Automatic TX watchdog set to keep running".to_string()
                        }
                        AutoTxStopPolicy::AfterNextTx => {
                            "Automatic TX will stop after the next transmission".to_string()
                        }
                        AutoTxStopPolicy::AfterCurrentQso => {
                            "Automatic TX will stop when the current QSO completes".to_string()
                        }
                    };
                }

                let hold_label = if self.ft8_hold_tx_freq {
                    "HOLD TX FREQ"
                } else {
                    "TX TRACKS RX"
                };
                let hold_color = if self.ft8_hold_tx_freq {
                    Color32::from_rgb(120, 200, 220)
                } else {
                    Color32::from_rgb(120, 220, 120)
                };
                if ui
                    .button(RichText::new(hold_label).color(hold_color))
                    .clicked()
                {
                    self.ft8_hold_tx_freq = !self.ft8_hold_tx_freq;
                    if !self.ft8_hold_tx_freq {
                        self.tx_tone_hz = self.rx_tone_hz;
                    }
                    self.profile_dirty = true;
                    self.persist_profile("Auto-saved");
                }

                let deep_label = if self.ft8_deep_decode {
                    "DECODE: DEEP"
                } else {
                    "DECODE: FAST"
                };
                let deep_color = if self.ft8_deep_decode {
                    Color32::YELLOW
                } else {
                    Color32::LIGHT_GREEN
                };
                if ui
                    .button(RichText::new(deep_label).color(deep_color))
                    .clicked()
                {
                    self.ft8_deep_decode = !self.ft8_deep_decode;
                    self.profile_dirty = true;
                    self.persist_profile("Auto-saved");
                }
            });
        });

        ui.horizontal_wrapped(|ui| {
            let (auto_label, auto_fill, auto_stroke) = if self.ft8_autoseq {
                (
                    "🔥 FT8 TX ARMED · CLICK TO DISARM ALL",
                    Color32::from_rgb(92, 43, 25),
                    Color32::from_rgb(255, 151, 72),
                )
            } else {
                (
                    "🔒 FT8 AUTO DISARMED · CLICK TO ARM",
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
                .on_hover_text(if self.ft8_autoseq {
                    "Cancel queued/active TX, drop PTT, and disarm FT8 and FT4"
                } else {
                    "Arm FT8 automatic sequencing; no transmission is sent until an exchange is started"
                })
                .clicked()
            {
                if self.ft8_autoseq {
                    self.disarm_all_tx("All TX stopped and disarmed by operator");
                } else {
                    self.ft8_autoseq = true;
                    if self.ft8_session.is_some() {
                        self.ft8_seq_state = Ft8SeqState::ReplyArmed;
                        self.ft8_seq_status = "Automatic TX resumed; waiting for reply".to_string();
                    } else {
                        self.ft8_seq_status =
                            "FT8 automatic operation armed; waiting for an exchange".to_string();
                    }
                    self.profile_dirty = true;
                    self.persist_profile("FT8 TX armed");
                }
            }
            ui.label("Select caller:");
            let previous_policy = self.ft8_auto_reply_policy;
            egui::ComboBox::from_id_salt("ft8_auto_reply_policy")
                .selected_text(self.ft8_auto_reply_policy.label())
                .show_ui(ui, |ui| {
                    for policy in AutoReplyPolicy::ALL {
                        ui.selectable_value(
                            &mut self.ft8_auto_reply_policy,
                            policy,
                            policy.label(),
                        );
                    }
                });
            if self.ft8_auto_reply_policy != previous_policy {
                self.profile_dirty = true;
                self.persist_profile("Auto-saved");
            }
            if ui
                .checkbox(&mut self.ft8_auto_answer_cq, "Answer unattended CQs")
                .changed()
            {
                self.profile_dirty = true;
                self.persist_profile("Auto-saved");
            }
            ui.separator();
            ui.label("Max unanswered");
            if ui
                .add(egui::DragValue::new(&mut self.ft8_max_attempts).range(1..=20))
                .changed()
            {
                self.profile_dirty = true;
                self.persist_profile("Auto-saved");
            }
            ui.separator();
            ui.label(
                RichText::new(&snapshot.ft8_decode_status)
                    .small()
                    .color(Color32::GRAY),
            );
            if let Some(level) = snapshot.audio_level_dbfs {
                let color = if snapshot.audio_clip_percent > 0.1 || level < -45.0 {
                    Color32::YELLOW
                } else {
                    Color32::LIGHT_GREEN
                };
                ui.label(
                    RichText::new(format!(
                        "Input {level:.0} dBFS / clip {:.1}%",
                        snapshot.audio_clip_percent
                    ))
                    .small()
                    .color(color),
                );
            }
            if let Some(offset) = snapshot.ft8_clock_offset_s {
                let color = if offset.abs() > 1.0 {
                    Color32::YELLOW
                } else {
                    Color32::LIGHT_GREEN
                };
                ui.label(
                    RichText::new(format!("Clock dT {offset:+.2}s"))
                        .small()
                        .color(color),
                );
            }
        });

        ui.separator();
        self.draw_ft8_activity_stats(ui);
        ui.add_space(4.0);

        let panel_h = ui.available_height();
        let conversation_h = (panel_h * 0.28).clamp(150.0, 260.0);
        let decode_h = (panel_h * 0.38).max(170.0);
        let tx_h = (panel_h * 0.20).max(120.0);
        let operator_call = self.station_callsign_or_default().to_string();
        let active_band = snapshot.frequency_hz.map(band_for_frequency).unwrap_or("");

        if let Some((entry, hit)) = self.ft8_log.iter().rev().find_map(|entry| {
            operator_call_hit(&entry.message, &operator_call).map(|hit| (entry, hit))
        }) {
            draw_operator_call_banner(ui, "FT8", &operator_call, &entry.message, hit);
            ui.add_space(4.0);
        }

        // ── Decode log ───────────────────────────────────────────────────────
        egui::Frame::dark_canvas(ui.style()).show(ui, |ui| {
            ui.set_min_height(decode_h);
            ui.set_max_height(decode_h);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("📡 LIVE DECODES")
                        .strong()
                        .color(Color32::LIGHT_BLUE),
                );
                ui.separator();
                if ui.checkbox(&mut self.ft8_cq_only_view, "CQ only").changed() {
                    self.profile_dirty = true;
                    self.persist_profile("Auto-saved");
                }
                if ui.checkbox(&mut self.ft8_follow_log, "Follow").changed() {
                    self.profile_dirty = true;
                    self.persist_profile("Auto-saved");
                }
                ui.label("Keep");
                if ui
                    .add(
                        egui::DragValue::new(&mut self.ft8_max_log_entries)
                            .range(80..=1000)
                            .speed(5),
                    )
                    .changed()
                {
                    self.profile_dirty = true;
                    self.persist_profile("Auto-saved");
                }
                ui.label("rows");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("Clear").clicked() {
                        self.ft8_log.clear();
                        self.ft8_tx_chat.clear();
                        self.ft8_selected = None;
                    }
                    ui.label(format!("{} msgs", self.ft8_log.len()));
                });
            });
            ui.separator();

            ui.horizontal(|ui| {
                ui.label(RichText::new("UTC").monospace().strong());
                ui.add_space(8.0);
                ui.label(RichText::new("SNR").monospace().strong());
                ui.add_space(8.0);
                ui.label(RichText::new("dT").monospace().strong());
                ui.add_space(8.0);
                ui.label(RichText::new("Hz").monospace().strong());
                ui.add_space(8.0);
                ui.label(RichText::new("Message").monospace().strong());
            });
            ui.separator();

            egui::ScrollArea::vertical()
                .id_salt("ft8_log")
                .stick_to_bottom(self.ft8_follow_log)
                .show(ui, |ui| {
                    if self.ft8_log.is_empty() {
                        ui.add_space(10.0);
                        ui.centered_and_justified(|ui| {
                            ui.label(
                                RichText::new("🌌 Listening… the band is quiet for the moment.")
                                    .color(Color32::from_gray(100)),
                            );
                        });
                        return;
                    }

                    let selected = self.ft8_selected;
                    let mut new_sel = selected;
                    let mut prev_utc: Option<&str> = None;
                    let mut reply_target_from_double_click: Option<String> = None;
                    let mut compose_from_double_click: Option<String> = None;
                    let mut picked_freq_from_double_click: Option<u32> = None;
                    let mut picked_period_from_double_click: Option<u64> = None;
                    let mut move_tx_from_double_click: Option<bool> = None;
                    let mut session_from_double_click: Option<QsoSession> = None;
                    for (i, entry) in self.ft8_log.iter().enumerate() {
                        if self.ft8_cq_only_view && !entry.is_cq {
                            continue;
                        }
                        if let Some(prev) = prev_utc {
                            if prev != entry.utc {
                                ui.separator();
                            }
                        }
                        prev_utc = Some(&entry.utc);

                        let is_sel = selected == Some(i);
                        let call_hit = operator_call_hit(&entry.message, &operator_call);
                        let worked_call = parse_message(&entry.message).map(|parsed| parsed.from);
                        let is_worked = worked_call.as_ref().is_some_and(|call| {
                            !call.is_empty()
                                && !active_band.is_empty()
                                && self.has_logged_contact_with(call, "FT8", active_band)
                        });
                        let text_color = if let Some(hit) = call_hit {
                            call_hit_badge(hit).1
                        } else if entry.is_cq {
                            Color32::from_rgb(100, 220, 100)
                        } else if entry.snr_db >= -5 {
                            Color32::from_rgb(220, 220, 140)
                        } else {
                            Color32::LIGHT_GRAY
                        };
                        let row = RichText::new(format!(
                            "{:12}  {:+3}  {:5.1}  {:>5}  {}",
                            entry.utc, entry.snr_db, entry.dt_s, entry.freq_hz, entry.message
                        ))
                        .monospace()
                        .color(text_color);

                        let resp = if let Some(hit) = call_hit {
                            let (badge, accent, fill) = call_hit_badge(hit);
                            egui::Frame::group(ui.style())
                                .fill(fill)
                                .stroke(egui::Stroke::new(1.5_f32, accent))
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(RichText::new(badge).strong().color(accent));
                                        ui.selectable_label(is_sel, row)
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
                                            RichText::new("✅ WORKED")
                                                .strong()
                                                .color(Color32::from_rgb(120, 220, 145)),
                                        );
                                        ui.selectable_label(is_sel, row)
                                    })
                                    .inner
                                })
                                .inner
                        } else {
                            ui.selectable_label(is_sel, row)
                        };
                        if resp.clicked() {
                            let now = Instant::now();
                            let synthetic_double = self
                                .ft8_last_click
                                .map(|(idx, t)| {
                                    idx == i && now.duration_since(t) <= Duration::from_millis(500)
                                })
                                .unwrap_or(false);
                            self.ft8_last_click = Some((i, now));

                            new_sel = if is_sel { None } else { Some(i) };

                            if synthetic_double || resp.double_clicked() {
                                // Pre-fill compose with a reply to this call.
                                if let Some(parsed) = parse_message(&entry.message) {
                                    let call = parsed.from.clone();
                                    let my = self.station_callsign_or_default();
                                    let grid = self.station_grid_or_default();
                                    let mut session = QsoSession::start(call.clone(), entry.period);
                                    if let Some(response) = session.response_to(
                                        &parsed,
                                        my,
                                        grid,
                                        entry.snr_db,
                                        entry.period,
                                    ) {
                                        compose_from_double_click = Some(response);
                                        reply_target_from_double_click = Some(call);
                                        session_from_double_click = Some(session);
                                        picked_period_from_double_click = Some(entry.period);
                                        picked_freq_from_double_click = Some(entry.freq_hz);
                                        // Only answering a CQ deliberately moves TX. A caller
                                        // answering our CQ is received on their offset while we
                                        // keep transmitting where we called.
                                        move_tx_from_double_click =
                                            Some(should_move_tx_to_decode(&parsed, false));
                                    }
                                }
                            }
                        }
                    }
                    self.ft8_selected = new_sel;
                    if let (
                        Some(compose),
                        Some(target),
                        Some(session),
                        Some(freq_hz),
                        Some(period),
                        Some(move_tx_to_remote),
                    ) = (
                        compose_from_double_click,
                        reply_target_from_double_click,
                        session_from_double_click,
                        picked_freq_from_double_click,
                        picked_period_from_double_click,
                        move_tx_from_double_click,
                    ) {
                        let reply = PendingManualFt8Reply {
                            compose,
                            target: target.clone(),
                            session,
                            freq_hz,
                            source_period: period,
                            move_tx_to_remote,
                        };
                        let tx_scheduled = self.ft8_tx_active.load(Ordering::Acquire)
                            || self.ft8_tx_queued_period.is_some();
                        let same_target = self
                            .ft8_seq_target
                            .as_deref()
                            .is_some_and(|current| super::exchange::callsign_eq(current, &target));
                        if tx_scheduled && same_target {
                            self.ft8_seq_status = format!("Reply to {target} is already queued");
                        } else if tx_scheduled
                            && (snapshot.ptt_on || self.ft8_tx_started_period.is_some())
                        {
                            self.ft8_seq_status =
                                "Current TX is already on air; target was not changed".to_string();
                        } else if tx_scheduled {
                            self.cancel_ft8_sequence(format!(
                                "Canceling prior reply; switching to {target}"
                            ));
                            self.ft8_pending_manual_reply = Some(reply);
                        } else {
                            self.arm_manual_ft8_reply(reply);
                        }
                    }
                });
        });

        ui.add_space(4.0);

        // QSO and selected-callsign traffic belongs below global band activity,
        // in the mode workspace rather than the universal monitoring rail.
        self.draw_ft8_conversation(ui, snapshot, conversation_h);
        ui.add_space(4.0);

        // ── TX compose ───────────────────────────────────────────────────────
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_min_height(tx_h);
            ui.horizontal(|ui| {
                ui.label(RichText::new("📣 TX DECK").strong());
                ui.separator();
                let ptt_color = if snapshot.ptt_on {
                    Color32::from_rgb(200, 60, 60)
                } else {
                    Color32::from_gray(80)
                };
                if ui
                    .button(
                        RichText::new(if snapshot.ptt_on {
                            "● PTT ON"
                        } else {
                            "○ PTT"
                        })
                        .color(ptt_color),
                    )
                    .clicked()
                {
                    if snapshot.ptt_on
                        || self.ft8_tx_active.load(Ordering::Acquire)
                        || self.digital_tx_active.load(Ordering::Acquire)
                    {
                        self.disarm_all_tx("TX/PTT stopped and all modes disarmed");
                    } else {
                        self.send_command(GuiCommand::TogglePtt);
                    }
                }
                let tx_active = self.ft8_tx_active.load(Ordering::Relaxed);
                if ui
                    .button(
                        RichText::new("⛔ STOP + DISARM ALL")
                            .strong()
                            .color(if tx_active {
                                Color32::from_rgb(255, 130, 130)
                            } else {
                                Color32::from_gray(120)
                            }),
                    )
                    .on_hover_text(
                        "Drop PTT, cancel queued TX, and disarm FT8/FT4 automatic operation",
                    )
                    .clicked()
                {
                    self.disarm_all_tx("All TX stopped and disarmed by operator");
                }
            });

            ui.horizontal(|ui| {
                let available = ui.available_width() - 70.0;
                ui.add(
                    egui::TextEdit::singleline(&mut self.ft8_compose)
                        .desired_width(available)
                        .hint_text("CQ W1AW FN20")
                        .font(egui::TextStyle::Monospace),
                );
                if ui
                    .button(
                        RichText::new("SEND")
                            .strong()
                            .color(Color32::from_rgb(80, 180, 80)),
                    )
                    .clicked()
                    && !self.ft8_compose.is_empty()
                {
                    self.queue_ft8_tx_from_compose(Ft8TxQueuePolicy::Standard, None);
                }
            });

            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                let my = self.station_callsign_or_default().to_string();
                let grid = self.station_grid_or_default().to_string();
                let target = self.ft8_seq_target.clone();
                if ui.small_button("CALL CQ").clicked() {
                    self.ft8_compose = format!("CQ {my} {grid}");
                    self.ft8_autoseq = true;
                    self.ft8_seq_state = Ft8SeqState::CqArmed;
                    self.ft8_seq_target = None;
                    self.ft8_seq_status = format!(
                        "CQ armed (waiting for next slot) · serial {} · {}",
                        self.contest_serial_current.max(1),
                        self.contest_guidance_text()
                    );
                    self.ft8_session = None;
                    self.queue_ft8_tx_from_compose(Ft8TxQueuePolicy::NextSlotOnly, None);
                    self.profile_dirty = true;
                    self.persist_profile("Auto-saved");
                }
                if ui
                    .add_enabled(target.is_some(), egui::Button::new("EXCH"))
                    .on_hover_text("Insert the current contest exchange preview")
                    .clicked()
                {
                    if let Some(target_call) = target.as_deref() {
                        self.ft8_compose = self.contest_exchange_preview(target_call);
                    }
                }
                for (label, exchange) in [("Grid", grid.as_str()), ("RR73", "RR73"), ("73", "73")] {
                    if ui
                        .add_enabled(target.is_some(), egui::Button::new(label))
                        .on_hover_text("Requires an active or selected QSO target")
                        .clicked()
                    {
                        self.ft8_compose =
                            format!("{} {my} {exchange}", target.as_deref().unwrap_or_default());
                    }
                }

                if let Some(target_call) = parse_tx_target_from_compose(&self.ft8_compose, &my) {
                    if let Some(frequency_hz) = snapshot.frequency_hz {
                        let band = band_for_frequency(frequency_hz);
                        if !band.is_empty()
                            && self.has_logged_contact_with(&target_call, "FT8", band)
                        {
                            ui.label(
                                RichText::new(format!(
                                    "⚠ Dupe risk: {target_call} already worked on {band} FT8"
                                ))
                                .small()
                                .color(Color32::from_rgb(255, 180, 82)),
                            );
                        }
                    }
                }

                if self.contest_enabled {
                    ui.label(
                        RichText::new(format!(
                            "Contest serial next: {:03} · exchange preview: {}",
                            self.contest_serial_current.max(1),
                            self.contest_exchange_preview(target.as_deref().unwrap_or(&my))
                        ))
                        .small()
                        .color(Color32::from_rgb(132, 228, 255)),
                    );
                }
            });

            ui.label(
                RichText::new(&self.ft8_seq_status)
                    .small()
                    .color(Color32::GRAY),
            );
            ui.horizontal(|ui| {
                ui.label("PTT lead");
                if ui
                    .add(
                        egui::DragValue::new(&mut self.ptt_lead_ms)
                            .range(100..=1500)
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
                            .range(0..=1000)
                            .suffix(" ms"),
                    )
                    .changed()
                {
                    self.profile_dirty = true;
                    self.persist_profile("Auto-saved");
                }
            });
        });

        ui.add_space(4.0);

        // ── Selected decode detail ───────────────────────────────────────────
        if let Some(idx) = self.ft8_selected {
            if let Some(e) = self.ft8_log.get(idx) {
                let e = e.clone();
                ui.group(|ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(&e.message).monospace().strong());
                        ui.separator();
                        ui.label(format!(
                            "{}  {:+}dB  {}Hz  Δt{:.1}s",
                            e.utc, e.snr_db, e.freq_hz, e.dt_s
                        ));
                        if e.is_cq {
                            ui.label(RichText::new("CQ").color(Color32::LIGHT_GREEN).strong());
                        }
                    });
                    ui.horizontal_wrapped(|ui| {
                        let my = self.station_callsign_or_default();
                        let grid = self.station_grid_or_default();
                        if let Some(call) = parse_message(&e.message).map(|message| message.from) {
                            if ui.small_button(format!("Reply → {call}")).clicked() {
                                self.ft8_compose = format!("{call} {my} {grid}");
                                self.ft8_seq_target = Some(call.clone());
                            }
                        }
                    });
                });
            }
        }
    }
}
