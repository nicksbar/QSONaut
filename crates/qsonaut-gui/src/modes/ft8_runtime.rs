use super::super::*;

fn retuned_ft8_tones(
    freq_hz: u32,
    current_tx_hz: u32,
    move_tx_to_remote: bool,
    hold_tx_freq: bool,
) -> (u32, u32, bool) {
    let picked = freq_hz.clamp(100, 3_500);
    let tx_moved = move_tx_to_remote && !hold_tx_freq;
    let tx_hz = if tx_moved { picked } else { current_tx_hz };
    (picked, tx_hz, tx_moved)
}

impl QsonautGuiApp {
    fn log_completed_ft8_session(&mut self, session: &QsoSession) {
        let frequency_hz = self
            .state
            .lock()
            .expect("ui state lock poisoned")
            .frequency_hz
            .unwrap_or_default();
        let started_at = session.started_period.saturating_mul(15);
        let ended_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or_else(|_| session.last_rx_period.saturating_add(1).saturating_mul(15));
        let mut record = QsoRecord::new(
            &session.target,
            "FT8",
            band_for_frequency(frequency_hz),
            frequency_hz,
            started_at,
            ended_at,
        );
        record.grid = session.remote_grid.clone().unwrap_or_default();
        record.operation_mode = if session.pota_reference.is_empty() {
            "General".to_string()
        } else {
            "POTA".to_string()
        };
        record.pota_role = session
            .pota_reference
            .is_empty()
            .then(String::new)
            .unwrap_or_else(|| "Hunter".to_string());
        record.pota_reference = session.pota_reference.clone();
        record.pota_name = session.pota_name.clone();
        record.report_sent = session
            .report_sent
            .map(format_signal_report)
            .unwrap_or_default();
        record.report_received = session
            .report_received
            .map(format_signal_report)
            .unwrap_or_default();
        if self.contest_enabled {
            record.contest_serial_sent = Some(self.contest_serial_current.max(1));
            record.contest_exchange_sent = self.contest_exchange_preview(&session.target);
            record.contest_exchange_received = record.report_received.clone();
            self.advance_contest_serial();
            self.profile_dirty = true;
            self.persist_profile("Auto-saved");
        }
        self.append_qso(record, "Auto-logged");
    }

    pub(crate) fn queue_ft8_tx_from_compose(
        &mut self,
        policy: Ft8TxQueuePolicy,
        rx_period: Option<u64>,
    ) {
        info!(
            ?policy,
            compose = %self.ft8_compose,
            auto_sequence = self.ft8_autoseq,
            radio_available = self.command_tx.is_some(),
            tx_active = self.ft8_tx_active.load(Ordering::Acquire),
            queued_period = ?self.ft8_tx_queued_period,
            suppress_canceled_events = self.ft8_suppress_canceled_tx_events,
            "FT8 TX request received"
        );
        if self.ft8_compose.trim().is_empty() {
            self.ft8_seq_status = "TX not queued: compose is empty".to_string();
            return;
        }
        let compose = self.ft8_compose.clone();
        if self.block_duplicate_tx_if_needed(WorkspaceMode::Ft8, &compose) {
            tracing::warn!(compose = %compose, "FT8 TX blocked by duplicate-contact guard");
            return;
        }
        let Some(command_tx) = self.command_tx.clone() else {
            tracing::warn!(compose = %compose, "FT8 TX blocked: radio command channel unavailable");
            self.ft8_seq_status = "TX unavailable: radio control is disabled".to_string();
            return;
        };
        if self.ft8_tx_active.load(Ordering::Acquire) || self.ft8_tx_queued_period.is_some() {
            tracing::warn!(
                compose = %compose,
                tx_active = self.ft8_tx_active.load(Ordering::Acquire),
                queued_period = ?self.ft8_tx_queued_period,
                "FT8 TX blocked: another transmission is already scheduled"
            );
            self.ft8_seq_status =
                "TX not queued: another transmission is already scheduled".to_string();
            return;
        }
        if self.digital_tx_active.load(Ordering::Acquire) {
            tracing::warn!(compose = %compose, "FT8 TX blocked: another digital mode is active");
            self.ft8_seq_status = "TX not queued: another digital mode is transmitting".to_string();
            return;
        }
        if self.ft8_suppress_canceled_tx_events {
            if self.ft8_tx_active.load(Ordering::Acquire) || self.ft8_tx_queued_period.is_some() {
                tracing::warn!(
                    compose = %compose,
                    tx_active = self.ft8_tx_active.load(Ordering::Acquire),
                    queued_period = ?self.ft8_tx_queued_period,
                    "FT8 TX blocked: cancellation is still settling"
                );
                self.ft8_seq_status = "TX cancellation is still settling; try again".to_string();
                return;
            }
            // A canceled worker can terminate without delivering its terminal
            // event (for example if the radio command channel disappears).
            // Do not leave every later TX request permanently blocked.
            tracing::warn!("clearing stale FT8 TX cancellation gate");
            self.ft8_suppress_canceled_tx_events = false;
        }
        if self
            .ft8_session
            .as_ref()
            .is_some_and(|session| session.tx_attempts >= self.ft8_max_attempts)
        {
            tracing::warn!(
                compose = %compose,
                attempts = self.ft8_session.as_ref().map(|session| session.tx_attempts),
                max_attempts = self.ft8_max_attempts,
                "FT8 TX blocked: maximum unanswered attempts reached"
            );
            self.cancel_ft8_sequence(format!(
                "Stopped after {} unanswered attempts",
                self.ft8_max_attempts
            ));
            return;
        }
        self.ft8_tx_abort.store(false, Ordering::Relaxed);
        let tx_tone_hz = self.contest_effective_tx_tone_hz();
        match build_ft8_tx_pcm(&self.ft8_compose, tx_tone_hz) {
            Ok(pcm) => {
                let now_s = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_secs_f64())
                    .unwrap_or(0.0);
                let period = (now_s / SLOT_SECONDS) as u64;
                let target_period = match policy {
                    Ft8TxQueuePolicy::Standard => {
                        next_tx_period(now_s, None, self.ptt_lead_ms as f64 / 1_000.0)
                    }
                    Ft8TxQueuePolicy::ReplyAsap => rx_period.map_or_else(
                        || next_tx_period(now_s, None, self.ptt_lead_ms as f64 / 1_000.0),
                        |source_period| {
                            next_reply_period(
                                now_s,
                                source_period,
                                self.ptt_lead_ms as f64 / 1_000.0,
                            )
                        },
                    ),
                    Ft8TxQueuePolicy::NextSlotOnly => next_tx_period(
                        now_s,
                        Some(((period + 1) % 2) as u8),
                        self.ptt_lead_ms as f64 / 1_000.0,
                    ),
                };
                info!(
                    ?policy,
                    source_period = ?rx_period,
                    current_period = period,
                    target_period,
                    slot_position_s = now_s % SLOT_SECONDS,
                    "FT8 TX scheduled"
                );
                let pcm = Arc::new(pcm);
                self.ft8_tx_pcm = Some(pcm.clone());
                self.ft8_queued_tx_message = Some(self.ft8_compose.trim().to_string());
                self.ft8_tx_queued_period = Some(target_period);
                self.ft8_tx_started_period = None;
                self.ft8_last_tx_was_cq =
                    parse_message(&self.ft8_compose).is_some_and(|message| message.is_cq);
                self.ft8_seq_state = Ft8SeqState::TxQueued;
                self.ft8_seq_status = match policy {
                    Ft8TxQueuePolicy::ReplyAsap if target_period == period => {
                        format!("Reply STARTING NOW at slot +{:.2}s", now_s % SLOT_SECONDS)
                    }
                    Ft8TxQueuePolicy::ReplyAsap => format!(
                        "Reply queued for future slot {} (period {})",
                        utc_hhmmss_millis(target_period as f64 * 15.0),
                        target_period
                    ),
                    Ft8TxQueuePolicy::NextSlotOnly => format!(
                        "CQ queued for next slot {} (period {})",
                        utc_hhmmss_millis(target_period as f64 * 15.0),
                        target_period
                    ),
                    Ft8TxQueuePolicy::Standard => format!(
                        "TX queued for {} (period {})",
                        utc_hhmmss_millis(target_period as f64 * 15.0),
                        target_period
                    ),
                };

                self.ft8_tx_active.store(true, Ordering::Release);
                let job = Ft8TxJob {
                    period: target_period,
                    pcm,
                    ptt_lead: Duration::from_millis(self.ptt_lead_ms),
                    ptt_tail: Duration::from_millis(self.ptt_tail_ms),
                    output_device: effective_audio_output_device(
                        &self.config.radio.backend,
                        self.config.audio.output_device.clone(),
                    ),
                    abort: self.ft8_tx_abort.clone(),
                    active: self.ft8_tx_active.clone(),
                    command_tx,
                    event_tx: self.ft8_tx_event_tx.clone(),
                    state: self.state.clone(),
                    repaint_ctx: self.repaint_ctx.clone(),
                };
                thread::spawn(move || run_ft8_tx_job(job));
                if let Some(session) = self.ft8_session.as_mut() {
                    session.tx_attempts = session.tx_attempts.saturating_add(1);
                    self.ft8_seq_status.push_str(&format!(
                        " | attempt {}/{}",
                        session.tx_attempts, self.ft8_max_attempts
                    ));
                }
            }
            Err(err) => {
                self.ft8_seq_status = format!("TX encode failed: {err}");
            }
        }
    }

    fn retune_from_decode_pick(&mut self, freq_hz: u32, move_tx_to_remote: bool) -> bool {
        let (rx_hz, tx_hz, tx_moved) = retuned_ft8_tones(
            freq_hz,
            self.tx_tone_hz,
            move_tx_to_remote,
            self.ft8_hold_tx_freq,
        );
        self.rx_tone_hz = rx_hz;
        self.tx_tone_hz = tx_hz;
        self.profile_dirty = true;
        self.persist_profile("Auto-saved");
        tx_moved
    }

    pub(crate) fn force_stop_tx(&mut self) {
        let had_scheduled_tx =
            self.ft8_tx_active.load(Ordering::Acquire) || self.ft8_tx_queued_period.is_some();
        if had_scheduled_tx {
            self.ft8_suppress_canceled_tx_events = true;
        }
        self.ft8_tx_abort.store(true, Ordering::Relaxed);
        self.ft8_tx_active.store(false, Ordering::Relaxed);

        self.ft8_tx_queued_period = None;
        self.ft8_tx_started_period = None;
        self.ft8_tx_pcm = None;
        self.ft8_queued_tx_message = None;
        self.ft8_pending_manual_reply = None;
        self.ft8_seq_state = Ft8SeqState::Idle;
        self.ft8_seq_status = "TX force-stopped".to_string();

        if let Some(tx) = &self.command_tx {
            let _ = tx.send(GuiCommand::SetPtt(false));
        }
    }

    pub(crate) fn cancel_ft8_sequence(&mut self, reason: String) {
        self.force_stop_tx();
        if self.digital_tx_active.load(Ordering::Acquire) {
            self.stop_native_digital_tx();
        }
        self.ft8_autoseq = false;
        self.ft8_stop_policy = AutoTxStopPolicy::Continuous;
        self.ft8_seq_status = reason;
        self.profile_dirty = true;
        self.persist_profile("Automatic operation stopped");
    }

    pub(crate) fn arm_manual_ft8_reply(&mut self, reply: PendingManualFt8Reply) {
        self.ft8_compose = reply.compose;
        let tx_moved = self.retune_from_decode_pick(reply.freq_hz, reply.move_tx_to_remote);
        self.ft8_autoseq = true;
        self.ft8_seq_state = Ft8SeqState::ReplyArmed;
        self.ft8_seq_target = Some(reply.target.clone());
        self.ft8_session = Some(reply.session);
        self.ft8_seq_status = if self.ft8_hold_tx_freq {
            format!(
                "Reply armed for {}; RX moved to {} Hz (TX held)",
                reply.target, self.rx_tone_hz
            )
        } else if tx_moved {
            format!(
                "Reply armed for {}; RX/TX set to {} Hz",
                reply.target, self.rx_tone_hz
            )
        } else {
            format!(
                "Reply armed for {}; RX moved to {} Hz (TX stays at {} Hz)",
                reply.target, self.rx_tone_hz, self.tx_tone_hz
            )
        };
        self.profile_dirty = true;
        self.persist_profile("Auto-saved");
        self.profile_io_status = "Auto-seq armed from decode selection".to_string();
        self.queue_ft8_tx_from_compose(Ft8TxQueuePolicy::ReplyAsap, Some(reply.source_period));
    }

    pub(crate) fn process_ft8_tx_pipeline(&mut self) {
        while let Ok(event) = self.ft8_tx_event_rx.try_recv() {
            if self.ft8_suppress_canceled_tx_events {
                let terminal = matches!(&event, Ft8TxEvent::Complete | Ft8TxEvent::Failed(_));
                if terminal {
                    self.ft8_suppress_canceled_tx_events = false;
                    if let Some(reply) = self.ft8_pending_manual_reply.take() {
                        self.arm_manual_ft8_reply(reply);
                    }
                }
                continue;
            }
            match event {
                Ft8TxEvent::PttConfirmed => {
                    self.ft8_seq_status =
                        "⚡ PTT confirmed · waveform launch is locked in".to_string();
                }
                Ft8TxEvent::AudioStarted => {
                    self.ft8_tx_started_period = self.ft8_tx_queued_period;
                    self.ft8_last_tx_message = self.ft8_queued_tx_message.clone();
                    if let Some(message) = self.ft8_queued_tx_message.clone() {
                        let now_s = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .map(|duration| duration.as_secs_f64())
                            .unwrap_or_default();
                        let period = self
                            .ft8_tx_queued_period
                            .unwrap_or_else(|| (now_s / 15.0).floor() as u64);
                        let duplicate = self.ft8_tx_chat.back().is_some_and(|entry| {
                            entry.period == period && entry.message == message
                        });
                        if !duplicate {
                            self.ft8_tx_chat.push_back(Ft8TxChatEntry {
                                period,
                                utc: utc_hhmmss_millis(now_s),
                                message,
                            });
                            while self.ft8_tx_chat.len() > 100 {
                                self.ft8_tx_chat.pop_front();
                            }
                        }
                    }
                    self.ft8_seq_status = "🔥 FT8 waveform on the air".to_string();
                }
                Ft8TxEvent::Complete => {
                    let completed_session = self
                        .ft8_session
                        .as_ref()
                        .filter(|session| should_finalize_after_tx(session.stage))
                        .cloned();
                    let stop_after_tx = self.ft8_stop_policy == AutoTxStopPolicy::AfterNextTx;
                    if stop_after_tx {
                        self.ft8_stop_policy = AutoTxStopPolicy::Continuous;
                    }
                    self.ft8_seq_status = if stop_after_tx {
                        self.ft8_autoseq = false;
                        "🔒 TX complete · automatic TX is paused".to_string()
                    } else if self.ft8_last_tx_was_cq {
                        "📣 CQ away · listening for callers".to_string()
                    } else {
                        "📡 TX complete · ears open for the reply".to_string()
                    };
                    self.ft8_last_tx_period = self.ft8_tx_started_period;
                    self.ft8_seq_state = if self.ft8_autoseq {
                        if self.ft8_last_tx_was_cq {
                            Ft8SeqState::CqArmed
                        } else {
                            Ft8SeqState::ReplyArmed
                        }
                    } else {
                        Ft8SeqState::Idle
                    };
                    self.ft8_tx_queued_period = None;
                    self.ft8_tx_started_period = None;
                    self.ft8_tx_pcm = None;
                    self.ft8_queued_tx_message = None;
                    self.ft8_tx_abort.store(false, Ordering::Relaxed);
                    if stop_after_tx {
                        self.profile_dirty = true;
                        self.persist_profile("Automatic TX paused and saved");
                    }
                    if let Some(session) = completed_session {
                        let target = session.target.clone();
                        self.log_completed_ft8_session(&session);
                        self.ft8_seq_status =
                            format!("🏁 QSO with {target} complete and logged · beautiful!");
                        self.ft8_seq_target = None;
                        self.ft8_session = None;
                        if self.ft8_stop_policy == AutoTxStopPolicy::AfterCurrentQso {
                            self.ft8_autoseq = false;
                            self.ft8_stop_policy = AutoTxStopPolicy::Continuous;
                            self.ft8_seq_state = Ft8SeqState::Idle;
                            self.ft8_seq_status.push_str(" · automatic TX stopped");
                        }
                    }
                }
                Ft8TxEvent::Failed(error) => {
                    self.ft8_seq_status = format!("⚠ TX failed · {error}");
                    self.ft8_seq_state = Ft8SeqState::Idle;
                    self.ft8_tx_queued_period = None;
                    self.ft8_tx_started_period = None;
                    self.ft8_tx_pcm = None;
                    self.ft8_queued_tx_message = None;
                    self.ft8_autoseq = false;
                    self.ft8_stop_policy = AutoTxStopPolicy::Continuous;
                    self.ft8_seq_target = None;
                    self.ft8_session = None;
                }
            }
        }
    }

    pub(crate) fn handle_ft8_decodes(
        &mut self,
        decodes: &[Ft8DecodeEntry],
        completed_period: Option<u64>,
    ) {
        if decodes.is_empty() && completed_period.is_none() {
            return;
        }
        if !decodes.is_empty() {
            self.track_decode_batch(decodes.len());
        }

        let my_call = self.station_callsign_or_default().to_ascii_uppercase();
        let my_grid = self.station_grid_for_ft8();

        for entry in decodes {
            if let Some(hit) = operator_call_hit(&entry.message, &my_call) {
                let call = parse_message(&entry.message)
                    .map(|parsed| parsed.from)
                    .unwrap_or_default();
                self.app_events.publish(AppEvent::CallsignHit {
                    mode: "FT8".to_string(),
                    call,
                    snr_db: f32::from(entry.snr_db),
                    freq_hz: entry.freq_hz,
                    message: entry.message.clone(),
                    directed_to_me: hit == OperatorCallHit::DirectedToMe,
                });
            }
        }

        if my_call == "N0CALL" {
            self.ft8_seq_status = "Auto reply paused: set a valid operator callsign".to_string();
            return;
        }

        if let Some(target) = self.ft8_seq_target.clone() {
            let working_other = decodes.iter().find_map(|entry| {
                let parsed = parse_message(&entry.message)?;
                (callsign_eq(&parsed.from, &target) && parsed.directed_away_from(&my_call))
                    .then(|| parsed.to.unwrap_or_else(|| "another station".to_string()))
            });
            if let Some(other) = working_other {
                self.cancel_ft8_sequence(format!("Canceled: {target} is responding to {other}"));
                return;
            }
        }

        if !self.ft8_autoseq
            || self.ft8_tx_active.load(Ordering::Acquire)
            || self.ft8_tx_queued_period.is_some()
        {
            return;
        }

        if let Some(session) = self.ft8_session.as_mut() {
            let target = session.target.clone();
            let response = decodes.iter().find_map(|entry| {
                let parsed = parse_message(&entry.message)?;
                let message =
                    session.response_to(&parsed, &my_call, &my_grid, entry.snr_db, entry.period)?;
                Some((message, entry.period, entry.freq_hz))
            });

            if session.stage == QsoStage::Complete {
                let completed_session = session.clone();
                self.ft8_seq_status = format!("QSO with {target} complete; ready for next caller");
                self.ft8_seq_state = Ft8SeqState::Idle;
                self.ft8_seq_target = None;
                self.ft8_session = None;
                self.log_completed_ft8_session(&completed_session);
                if self.ft8_stop_policy == AutoTxStopPolicy::AfterCurrentQso {
                    self.ft8_autoseq = false;
                    self.ft8_stop_policy = AutoTxStopPolicy::Continuous;
                    self.ft8_seq_status.push_str(" · automatic TX stopped");
                }
                return;
            }

            if let Some((message, period, freq_hz)) = response {
                self.ft8_compose = message;
                self.retune_from_decode_pick(freq_hz, false);
                self.queue_ft8_tx_from_compose(Ft8TxQueuePolicy::ReplyAsap, Some(period));
            } else if completed_period
                .is_some_and(|period| should_retry_after_decode(self.ft8_last_tx_period, period))
            {
                let period = completed_period.expect("checked above");
                self.ft8_seq_status = format!("🔁 No reply from {target} yet · trying again");
                self.queue_ft8_tx_from_compose(Ft8TxQueuePolicy::ReplyAsap, Some(period));
            }
            return;
        }

        let candidates = decodes.iter().enumerate().filter_map(|(index, entry)| {
            let parsed = parse_message(&entry.message)?;
            let eligible =
                parsed.directed_to(&my_call) || (self.ft8_auto_answer_cq && parsed.is_cq);
            if !eligible || callsign_eq(&parsed.from, &my_call) {
                return None;
            }
            Some(ReplyCandidate {
                index,
                snr_db: entry.snr_db,
                freq_hz: entry.freq_hz,
                parsed,
            })
        });
        let Some(selected) =
            select_candidate(candidates, self.ft8_auto_reply_policy, self.rx_tone_hz)
        else {
            if should_repeat_cq(
                self.ft8_autoseq,
                self.ft8_last_tx_was_cq,
                self.ft8_last_tx_period,
                completed_period,
            ) {
                self.ft8_seq_status = "📣 No caller yet · repeating CQ".to_string();
                self.queue_ft8_tx_from_compose(Ft8TxQueuePolicy::NextSlotOnly, None);
            }
            return;
        };
        let entry = &decodes[selected.index];
        let mut session = QsoSession::start(selected.parsed.from.clone(), entry.period);
        if selected.parsed.raw.to_ascii_uppercase().contains("POTA") {
            if let Some(spot) = self
                .pota_spots
                .iter()
                .filter(|spot| {
                    spot.activator.eq_ignore_ascii_case(&selected.parsed.from) && spot.mode == "FT8"
                })
                .min_by_key(|spot| {
                    spot.frequency_hz.abs_diff(
                        self.state
                            .lock()
                            .expect("ui state lock poisoned")
                            .frequency_hz
                            .unwrap_or_default()
                            + u64::from(entry.freq_hz),
                    )
                })
            {
                session.pota_reference = spot.reference.clone();
                session.pota_name = spot.name.clone();
            }
        }
        let Some(response) = session.response_to(
            &selected.parsed,
            &my_call,
            &my_grid,
            entry.snr_db,
            entry.period,
        ) else {
            return;
        };

        self.ft8_seq_target = Some(session.target.clone());
        self.ft8_seq_state = Ft8SeqState::ReplyArmed;
        self.ft8_seq_status = format!(
            "{} selected {} at {:+} dB",
            self.ft8_auto_reply_policy.label(),
            session.target,
            entry.snr_db
        );
        self.ft8_session = Some(session);
        self.ft8_compose = response;
        self.retune_from_decode_pick(
            entry.freq_hz,
            should_move_tx_to_decode(&selected.parsed, false),
        );
        self.queue_ft8_tx_from_compose(Ft8TxQueuePolicy::ReplyAsap, Some(entry.period));
    }
}

#[cfg(test)]
mod tests {
    use super::retuned_ft8_tones;

    #[test]
    fn clamps_decode_picks_and_respects_tx_hold() {
        assert_eq!(retuned_ft8_tones(20, 1_500, true, false), (100, 100, true));
        assert_eq!(
            retuned_ft8_tones(4_000, 1_500, true, false),
            (3_500, 3_500, true)
        );
        assert_eq!(
            retuned_ft8_tones(2_000, 1_500, true, true),
            (2_000, 1_500, false)
        );
        assert_eq!(
            retuned_ft8_tones(2_000, 1_500, false, false),
            (2_000, 1_500, false)
        );
    }
}
