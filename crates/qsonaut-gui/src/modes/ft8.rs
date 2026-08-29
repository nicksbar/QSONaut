use super::super::*;

pub(crate) const BAND_PLAN: &[(&str, u64)] = &[
    ("160m", 1_840_000),
    ("80m", 3_573_000),
    ("60m", 5_357_000),
    ("40m", 7_074_000),
    ("30m", 10_136_000),
    ("20m", 14_074_000),
    ("17m", 18_100_000),
    ("15m", 21_074_000),
    ("12m", 24_924_000),
    ("10m", 28_074_000),
    ("6m", 50_313_000),
    ("2m", 144_174_000),
    ("70cm", 432_074_000),
];

fn ft8_phase_label(
    ptt_on: bool,
    queued_period: Option<u64>,
    autoseq: bool,
    has_session: bool,
) -> &'static str {
    if ptt_on {
        "TX NOW"
    } else if queued_period.is_some() {
        "TX QUEUED"
    } else if autoseq && has_session {
        "AUTO TX ARMED"
    } else {
        "RX"
    }
}

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
                operator_call_hit(&entry.message, &operator_call)
                    == Some(OperatorCallHit::DirectedToMe)
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
            let belongs = super::digital_conversation::tx_message_belongs_to_conversation(
                &entry.message,
                &operator_call,
                target.as_deref(),
            );
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
                        .unwrap_or_else(|| "💬 RECENT ACTIVITY · SELECT A CALLSIGN".to_string());
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
                                        "✨ No transmitted messages yet. Select a decode to track a callsign."
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
            let phase_label = ft8_phase_label(
                snapshot.ptt_on,
                self.ft8_tx_queued_period,
                self.ft8_autoseq,
                self.ft8_session.is_some(),
            );
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
                    theme_warning(ui)
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
                    egui::Button::new(RichText::new(auto_label).strong().color(auto_stroke))
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
            if self.automation_unlocked
                && ui
                    .checkbox(&mut self.ft8_auto_answer_cq, "Answer unattended CQs")
                    .on_hover_text(
                        "When armed, automatically reply to stations calling CQ even if you are not \
                         actively watching. QSONaut picks the strongest/nearest caller and starts the \
                         exchange on its own. Leave OFF if you want to choose every caller manually.",
                    )
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
                    theme_warning(ui)
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
                    theme_warning(ui)
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

        let operator_call = self.station_callsign_or_default().to_string();
        let active_band = snapshot.frequency_hz.map(band_for_frequency).unwrap_or("");
        let (decode_h, tx_h) = Self::split_decode_workspace_height(ui.available_height());
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_min_height(decode_h);
            ui.set_max_height(decode_h);
            let available_rect = ui.available_rect_before_wrap();
            let deck_rect = egui::Rect::from_min_size(
                available_rect.min,
                egui::vec2(available_rect.width(), decode_h),
            );
            ui.allocate_rect(deck_rect, egui::Sense::hover());
            let mut deck_ui = ui.new_child(
                egui::UiBuilder::new()
                    .id_salt("ft8_decode_deck_contents")
                    .max_rect(deck_rect)
                    .layout(egui::Layout::top_down(egui::Align::Min)),
            );
            deck_ui.set_clip_rect(deck_rect);
            let panel_h = deck_ui.available_height();
            let conversation_h = panel_h;
            let decode_h = panel_h;
            deck_ui.columns(2, |columns| {
                let left = &mut columns[0];
                left.set_min_width(0.0);
                // ── Global decode log ───────────────────────────────────────
                egui::Frame::dark_canvas(left.style()).show(left, |ui| {
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
                                        RichText::new(
                                            "🌌 Listening… the band is quiet for the moment.",
                                        )
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
                                let directed_to_me =
                                    operator_call_hit(&entry.message, &operator_call)
                                        == Some(OperatorCallHit::DirectedToMe);
                                if self.ft8_cq_only_view && !entry.is_cq && !directed_to_me {
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
                                let worked_call =
                                    parse_message(&entry.message).map(|parsed| parsed.from);
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
                                let park_marker =
                                    if entry.message.to_ascii_uppercase().contains("POTA") {
                                        "🌲 "
                                    } else {
                                        ""
                                    };
                                let pota_detail = if park_marker.is_empty() {
                                    None
                                } else {
                                    parse_message(&entry.message).and_then(|message| {
                                        self.pota_spots
                                            .iter()
                                            .filter(|spot| {
                                                spot.activator.eq_ignore_ascii_case(&message.from)
                                                    && spot.mode == "FT8"
                                            })
                                            .min_by_key(|spot| {
                                                spot.frequency_hz.abs_diff(
                                                    snapshot.frequency_hz.unwrap_or_default()
                                                        + u64::from(entry.freq_hz),
                                                )
                                            })
                                            .map(|spot| {
                                                format!(" · {} {}", spot.reference, spot.name)
                                            })
                                    })
                                };
                                let pota_detail = pota_detail.unwrap_or_default();
                                let row = RichText::new(format!(
                                    "{:12}  {:+3}  {:5.1}  {:>5}  {park_marker}{}{}",
                                    entry.utc,
                                    entry.snr_db,
                                    entry.dt_s,
                                    entry.freq_hz,
                                    entry.message,
                                    pota_detail
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
                                                ui.label(
                                                    RichText::new(badge).strong().color(accent),
                                                );
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
                                            idx == i
                                                && now.duration_since(t)
                                                    <= Duration::from_millis(500)
                                        })
                                        .unwrap_or(false);
                                    self.ft8_last_click = Some((i, now));

                                    new_sel = if is_sel { None } else { Some(i) };

                                    if synthetic_double || resp.double_clicked() {
                                        // Pre-fill compose with a reply to this call.
                                        if let Some(parsed) = parse_message(&entry.message) {
                                            let call = parsed.from.clone();
                                            let my = self.station_callsign_or_default();
                                            let grid = self.station_grid_for_ft8();
                                            let mut session =
                                                QsoSession::start(call.clone(), entry.period);
                                            if let Some(response) = session.response_to(
                                                &parsed,
                                                my,
                                                &grid,
                                                entry.snr_db,
                                                entry.period,
                                            ) {
                                                compose_from_double_click = Some(response);
                                                reply_target_from_double_click = Some(call);
                                                session_from_double_click = Some(session);
                                                picked_period_from_double_click =
                                                    Some(entry.period);
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
                                let same_target =
                                    self.ft8_seq_target.as_deref().is_some_and(|current| {
                                        super::exchange::callsign_eq(current, &target)
                                    });
                                if tx_scheduled && same_target {
                                    self.ft8_seq_status =
                                        format!("Reply to {target} is already queued");
                                } else if tx_scheduled
                                    && (snapshot.ptt_on || self.ft8_tx_started_period.is_some())
                                {
                                    self.ft8_seq_status =
                                        "Current TX is already on air; target was not changed"
                                            .to_string();
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
                let right = &mut columns[1];
                right.set_min_width(0.0);
                // ── Active channel / selected callsign ──────────────────────
                self.draw_ft8_conversation(right, snapshot, conversation_h);
            });
        });
        ui.add_space(4.0);

        // ── TX compose ───────────────────────────────────────────────────────
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_min_height(tx_h);
            ui.set_max_height(tx_h);
            ui.horizontal(|ui| {
                ui.label(RichText::new("📣 TX DECK").strong());
                ui.separator();
                ui.label(
                    RichText::new("Compose and queue a transmission")
                        .small()
                        .color(Color32::GRAY),
                );
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
                let grid = self.station_grid_for_ft8();
                let target = self.ft8_seq_target.clone();
                if ui.small_button("CALL CQ").clicked() {
                    self.ft8_compose = format!("CQ {my} {grid}");
                    self.ft8_selected = None;
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
                    .color(status_color(ui, &self.ft8_seq_status)),
            );
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
                        let grid = self.station_grid_for_ft8();
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

#[cfg(test)]
mod tests {
    use super::{ft8_phase_label, BAND_PLAN};

    #[test]
    fn prioritizes_ft8_transmit_state_in_the_phase_label() {
        assert_eq!(ft8_phase_label(true, Some(4), true, true), "TX NOW");
        assert_eq!(ft8_phase_label(false, Some(4), true, true), "TX QUEUED");
        assert_eq!(ft8_phase_label(false, None, true, true), "AUTO TX ARMED");
        assert_eq!(ft8_phase_label(false, None, true, false), "RX");
    }

    #[test]
    fn exposes_the_ft8_band_plan_in_frequency_order() {
        assert_eq!(BAND_PLAN.len(), 13);
        assert_eq!(BAND_PLAN.first(), Some(&("160m", 1_840_000)));
        assert_eq!(BAND_PLAN.last(), Some(&("70cm", 432_074_000)));
        assert!(BAND_PLAN.windows(2).all(|bands| bands[0].1 < bands[1].1));
    }
}
