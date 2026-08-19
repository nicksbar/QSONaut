use super::super::*;

impl QsonautGuiApp {
    fn log_completed_native_session(&mut self, session: &QsoSession, mode: WorkspaceMode) {
        let frequency_hz = self
            .state
            .lock()
            .expect("ui state lock poisoned")
            .frequency_hz
            .unwrap_or_default();
        let slot_seconds = mode
            .slot_seconds(self.fst4_submode)
            .unwrap_or(FT4_SLOT_SECONDS);
        let started_at = (session.started_period as f64 * slot_seconds) as u64;
        let ended_at = (session.last_rx_period.saturating_add(1) as f64 * slot_seconds) as u64;
        let mut record = QsoRecord::new(
            &session.target,
            mode.label(),
            band_for_frequency(frequency_hz),
            frequency_hz,
            started_at,
            ended_at,
        );
        record.grid = session.remote_grid.clone().unwrap_or_default();
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

    pub(crate) fn handle_ft4_decodes(
        &mut self,
        decodes: &[DigitalDecodeEntry],
        completed_period: Option<u64>,
    ) {
        self.handle_native_sequence(WorkspaceMode::Ft4, decodes, completed_period);
    }

    pub(crate) fn handle_native_sequence(
        &mut self,
        mode: WorkspaceMode,
        decodes: &[DigitalDecodeEntry],
        completed_period: Option<u64>,
    ) {
        let seen = self.native_seen_decodes.entry(mode).or_default();
        let fresh: Vec<DigitalDecodeEntry> = decodes
            .iter()
            .filter(|entry| {
                entry.mode == mode
                    && seen.insert((entry.period, entry.freq_hz, entry.message.clone()))
            })
            .cloned()
            .collect();
        if !fresh.is_empty() {
            self.track_decode_batch(fresh.len());
        }
        if self.ft4_seen_decodes.len() > 1_000 {
            let latest = fresh
                .iter()
                .map(|entry| entry.period)
                .max()
                .unwrap_or_default();
            self.ft4_seen_decodes
                .retain(|(period, _, _)| *period + 100 >= latest);
        }

        let operator_call = self.station_callsign_or_default().to_string();
        for entry in &fresh {
            if let Some(hit) = operator_call_hit(&entry.message, &operator_call) {
                let call = parse_message(&entry.message)
                    .map(|parsed| parsed.from)
                    .unwrap_or_default();
                self.app_events.publish(AppEvent::CallsignHit {
                    mode: mode.label().to_string(),
                    call,
                    snr_db: entry.snr_db,
                    freq_hz: entry.freq_hz,
                    message: entry.message.clone(),
                    directed_to_me: hit == OperatorCallHit::DirectedToMe,
                });
            }
        }

        let auto_sequence = if mode == WorkspaceMode::Ft4 {
            self.ft4_autoseq
        } else {
            self.native_autoseq_mode == Some(mode)
        };
        if !auto_sequence || self.digital_tx_active.load(Ordering::Acquire) {
            return;
        }

        let my_call = self.station_callsign_or_default().to_string();
        let my_grid = self.station_grid_or_default().to_string();
        let awaiting_cq_caller = self
            .digital_last_tx_message
            .as_deref()
            .is_some_and(|message| message.starts_with("CQ "));
        let has_session = if mode == WorkspaceMode::Ft4 {
            self.ft4_session.is_some()
        } else {
            self.native_sessions.contains_key(&mode)
        };
        if !has_session {
            let candidates = fresh.iter().enumerate().filter_map(|(index, entry)| {
                let parsed = parse_message(&entry.message)?;
                let eligible = (awaiting_cq_caller && parsed.directed_to(&my_call))
                    || (self.ft8_auto_answer_cq && parsed.is_cq);
                if !eligible || callsign_eq(&parsed.from, &my_call) {
                    return None;
                }
                Some(ReplyCandidate {
                    index,
                    snr_db: entry.snr_db.round() as i8,
                    freq_hz: entry.freq_hz,
                    parsed,
                })
            });
            if let Some(chosen) =
                select_candidate(candidates, self.ft4_auto_reply_policy, self.rx_tone_hz)
            {
                let session =
                    QsoSession::start(chosen.parsed.from.clone(), fresh[chosen.index].period);
                if mode == WorkspaceMode::Ft4 {
                    self.ft4_session = Some(session);
                } else {
                    self.native_sessions.insert(mode, session);
                }
                self.digital_seq_target = Some(chosen.parsed.from.clone());
                self.digital_tx_status = format!(
                    "🎯 {} selected by {} priority",
                    chosen.parsed.from,
                    self.ft4_auto_reply_policy.label()
                );
            }
        }
        let mut queued_response = false;
        for entry in fresh {
            let Some(parsed) = parse_message(&entry.message) else {
                continue;
            };
            let session = if mode == WorkspaceMode::Ft4 {
                self.ft4_session.as_mut()
            } else {
                self.native_sessions.get_mut(&mode)
            };
            let Some(session) = session else {
                continue;
            };
            let response = session.response_to(
                &parsed,
                &my_call,
                &my_grid,
                entry.snr_db.round() as i8,
                entry.period,
            );
            if session.stage == QsoStage::Complete {
                let completed = session.clone();
                self.log_completed_native_session(&completed, mode);
                self.digital_tx_status = format!(
                    "🏁 FT4 QSO with {} complete · nice contact!",
                    completed.target
                );
                if mode == WorkspaceMode::Ft4 {
                    self.ft4_session = None;
                } else {
                    self.native_sessions.remove(&mode);
                }
                self.digital_seq_target = None;
                let stop_policy = if mode == WorkspaceMode::Ft4 {
                    self.ft4_stop_policy
                } else {
                    self.native_stop_policy
                };
                if stop_policy == AutoTxStopPolicy::AfterCurrentQso {
                    self.ft4_autoseq = false;
                    self.native_autoseq_mode = None;
                    self.ft4_stop_policy = AutoTxStopPolicy::Continuous;
                    self.native_stop_policy = AutoTxStopPolicy::Continuous;
                    self.digital_tx_status.push_str(" · automatic TX stopped");
                }
                break;
            }
            if let Some(response) = response {
                self.digital_compose = response;
                self.rx_tone_hz = entry.freq_hz;
                if !self.ft8_hold_tx_freq {
                    self.tx_tone_hz = entry.freq_hz;
                }
                self.queue_native_digital_tx(mode);
                queued_response = true;
                break;
            }
        }
        if queued_response {
            return;
        }
        let has_session = if mode == WorkspaceMode::Ft4 {
            self.ft4_session.is_some()
        } else {
            self.native_sessions.contains_key(&mode)
        };
        if !has_session {
            if should_repeat_cq(
                self.ft4_autoseq,
                awaiting_cq_caller,
                self.ft4_last_tx_period,
                completed_period,
            ) {
                self.digital_tx_status = "📣 No FT4 caller yet · repeating CQ".to_string();
                self.queue_native_digital_tx(mode);
            }
            return;
        }
        let last_tx_period = if mode == WorkspaceMode::Ft4 {
            self.ft4_last_tx_period
        } else {
            self.native_last_tx_periods.get(&mode).copied()
        };
        if completed_period.is_some_and(|period| {
            last_tx_period.is_some_and(|last_tx| period == last_tx.saturating_add(1))
        }) {
            let attempts = if mode == WorkspaceMode::Ft4 {
                self.ft4_session
                    .as_ref()
                    .map(|s| s.tx_attempts)
                    .unwrap_or_default()
            } else {
                *self.native_attempts.get(&mode).unwrap_or(&0)
            };
            let max_attempts = if mode == WorkspaceMode::Ft4 {
                self.ft4_max_attempts
            } else {
                6
            };
            if attempts >= max_attempts {
                self.ft4_autoseq = false;
                self.ft4_stop_policy = AutoTxStopPolicy::Continuous;
                if mode == WorkspaceMode::Ft4 {
                    self.ft4_session = None;
                } else {
                    self.native_sessions.remove(&mode);
                }
                self.digital_tx_status = format!(
                    "{} stopped after {} unanswered attempts",
                    mode.label(),
                    self.ft4_max_attempts
                );
            } else if !self.digital_compose.trim().is_empty() {
                self.digital_tx_status = format!(
                    "🔁 No {} reply yet · repeating the last exchange",
                    mode.label()
                );
                self.queue_native_digital_tx(mode);
            }
        }
    }

    pub(crate) fn queue_native_digital_tx(&mut self, mode: WorkspaceMode) {
        let compose = self.digital_compose.clone();
        if self.block_duplicate_tx_if_needed(mode, &compose) {
            return;
        }
        if self.digital_suppress_canceled_tx_events {
            self.digital_tx_status = "TX cancellation is still settling; try again".to_string();
            return;
        }
        if self.ft8_tx_active.load(Ordering::Acquire)
            || self.digital_tx_active.load(Ordering::Acquire)
        {
            self.digital_tx_status = "TX not queued: another transmission is active".to_string();
            return;
        }
        let Some(command_tx) = self.command_tx.clone() else {
            self.digital_tx_status = "TX unavailable: radio control is disabled".to_string();
            return;
        };
        let slot_seconds = mode.slot_seconds(self.fst4_submode);
        if slot_seconds.is_none() && mode != WorkspaceMode::Cw {
            self.digital_tx_status = format!("{} TX backend is not available", mode.label());
            return;
        }
        let tx_tone_hz = self.contest_effective_tx_tone_hz();
        match build_native_digital_tx_pcm(
            mode,
            &self.digital_compose,
            tx_tone_hz,
            self.fst4_submode,
            self.cw_wpm,
            self.cw_tone_hz,
        ) {
            Ok((pcm, audio_offset_s)) => {
                let now_s = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|duration| duration.as_secs_f64())
                    .unwrap_or(0.0);
                let (period, job_slot_seconds, audio_start_s) =
                    if let Some(slot_seconds) = slot_seconds {
                        let period = (now_s / slot_seconds).floor() as u64 + 1;
                        (period, slot_seconds, None)
                    } else {
                        (
                            now_s.floor() as u64,
                            1.0,
                            Some(now_s + self.ptt_lead_ms as f64 / 1_000.0 + 0.05),
                        )
                    };
                self.digital_tx_abort.store(false, Ordering::Release);
                self.digital_tx_active.store(true, Ordering::Release);
                self.digital_tx_status = if mode == WorkspaceMode::Cw {
                    format!(
                        "CW queued for {}",
                        utc_hhmmss_millis(audio_start_s.unwrap_or(now_s))
                    )
                } else {
                    format!(
                        "{} queued for {}",
                        mode.label(),
                        utc_hhmmss_millis(period as f64 * job_slot_seconds)
                    )
                };
                self.digital_queued_tx_message = Some(self.digital_compose.trim().to_string());
                if mode != WorkspaceMode::Cw {
                    self.state
                        .lock()
                        .expect("ui state lock poisoned")
                        .digital_tx_period = Some((mode, period));
                }
                let job = DigitalTxJob {
                    mode,
                    period,
                    slot_seconds: job_slot_seconds,
                    audio_offset_s,
                    audio_start_s,
                    pcm: Arc::new(pcm),
                    ptt_lead: Duration::from_millis(self.ptt_lead_ms),
                    ptt_tail: Duration::from_millis(self.ptt_tail_ms),
                    output_device: self.config.audio.output_device.clone(),
                    abort: self.digital_tx_abort.clone(),
                    active: self.digital_tx_active.clone(),
                    command_tx,
                    event_tx: self.digital_tx_event_tx.clone(),
                    state: self.state.clone(),
                    repaint_ctx: self.repaint_ctx.clone(),
                };
                thread::spawn(move || run_digital_tx_job(job));
            }
            Err(error) => {
                self.digital_tx_status = format!("TX encode failed: {error}");
            }
        }
    }

    pub(crate) fn stop_native_digital_tx(&mut self) {
        let had_scheduled_tx = self.digital_tx_active.load(Ordering::Acquire)
            || self.digital_tx_started.is_some()
            || self.digital_queued_tx_message.is_some();
        if had_scheduled_tx {
            self.digital_suppress_canceled_tx_events = true;
        }
        self.digital_tx_abort.store(true, Ordering::Release);
        self.digital_tx_active.store(false, Ordering::Release);
        if let Some(tx) = &self.command_tx {
            let _ = tx.send(GuiCommand::SetPtt(false));
        }
        self.state
            .lock()
            .expect("ui state lock poisoned")
            .digital_tx_period = None;
        self.digital_tx_status = "TX stopped".to_string();
        self.digital_queued_tx_message = None;
    }

    pub(crate) fn process_native_digital_tx_pipeline(&mut self) {
        while let Ok(event) = self.digital_tx_event_rx.try_recv() {
            if self.digital_suppress_canceled_tx_events {
                if matches!(event, DigitalTxEvent::Complete | DigitalTxEvent::Failed(_)) {
                    self.digital_suppress_canceled_tx_events = false;
                    self.digital_tx_abort.store(false, Ordering::Release);
                }
                continue;
            }
            self.digital_tx_status = match event {
                DigitalTxEvent::AudioStarted(mode, period) => {
                    self.digital_tx_started = Some((mode, period));
                    let session = if mode == WorkspaceMode::Ft4 {
                        self.ft4_session.as_mut()
                    } else {
                        self.native_sessions.get_mut(&mode)
                    };
                    if let Some(session) = session {
                        session.tx_attempts = session.tx_attempts.saturating_add(1);
                        self.native_attempts.insert(mode, session.tx_attempts);
                    }
                    if let Some(message) = self.digital_queued_tx_message.clone() {
                        let utc = utc_hhmmss_millis(
                            period as f64 * mode.slot_seconds(self.fst4_submode).unwrap_or(1.0),
                        );
                        self.digital_tx_chat.push_back(DigitalTxChatEntry {
                            mode,
                            period,
                            utc,
                            message,
                        });
                        while self.digital_tx_chat.len() > 100 {
                            self.digital_tx_chat.pop_front();
                        }
                    }
                    format!("🔥 {} waveform on the air", mode.label())
                }
                DigitalTxEvent::Complete => {
                    let completed_mode = self.digital_tx_started.take().map(|(mode, period)| {
                        if mode == WorkspaceMode::Ft4 {
                            self.ft4_last_tx_period = Some(period);
                        } else {
                            self.native_last_tx_periods.insert(mode, period);
                        }
                        mode
                    });
                    let completed_session = completed_mode.and_then(|mode| {
                        if mode == WorkspaceMode::Ft4 {
                            self.ft4_session
                                .as_ref()
                                .filter(|session| should_finalize_after_tx(session.stage))
                                .cloned()
                        } else {
                            self.native_sessions
                                .get(&mode)
                                .filter(|session| should_finalize_after_tx(session.stage))
                                .cloned()
                        }
                    });
                    self.digital_last_tx_message = self.digital_queued_tx_message.take();
                    let mut status = if completed_mode == Some(WorkspaceMode::Ft4)
                        && self.ft4_stop_policy == AutoTxStopPolicy::AfterNextTx
                    {
                        self.ft4_stop_policy = AutoTxStopPolicy::Continuous;
                        self.ft4_autoseq = false;
                        self.profile_dirty = true;
                        self.persist_profile("FT4 automatic TX paused");
                        "🔒 FT4 TX complete · automatic TX is paused".to_string()
                    } else {
                        "📡 TX complete · receiver back on watch".to_string()
                    };
                    if let Some(session) = completed_session {
                        let target = session.target.clone();
                        let mode = completed_mode.expect("session implies completed mode");
                        self.log_completed_native_session(&session, mode);
                        if mode == WorkspaceMode::Ft4 {
                            self.ft4_session = None;
                        } else {
                            self.native_sessions.remove(&mode);
                        }
                        self.digital_seq_target = None;
                        let stop_policy = if mode == WorkspaceMode::Ft4 {
                            self.ft4_stop_policy
                        } else {
                            self.native_stop_policy
                        };
                        if stop_policy == AutoTxStopPolicy::AfterCurrentQso {
                            self.ft4_autoseq = false;
                            self.ft4_stop_policy = AutoTxStopPolicy::Continuous;
                            self.native_autoseq_mode = None;
                            self.native_stop_policy = AutoTxStopPolicy::Continuous;
                        }
                        status =
                            format!("🏁 {} QSO with {target} complete and logged", mode.label());
                        if (mode == WorkspaceMode::Ft4 && !self.ft4_autoseq)
                            || (mode != WorkspaceMode::Ft4 && self.native_autoseq_mode.is_none())
                        {
                            status.push_str(" · automatic TX stopped");
                        }
                    }
                    status
                }
                DigitalTxEvent::Failed(error) => {
                    self.digital_tx_started = None;
                    self.digital_queued_tx_message = None;
                    self.ft4_stop_policy = AutoTxStopPolicy::Continuous;
                    // A failed or canceled waveform must not leave the QSO
                    // session armed for a reply that can no longer arrive.
                    // Otherwise the next automation cycle inherits stale
                    // exchange state and appears permanently stuck.
                    self.ft4_autoseq = false;
                    self.ft4_session = None;
                    self.digital_seq_target = None;
                    self.native_sessions.clear();
                    self.native_autoseq_mode = None;
                    format!("⚠ TX failed · {error}")
                }
            };
        }
    }
}
