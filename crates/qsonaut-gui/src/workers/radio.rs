use super::super::*;

const RADIO_CORE_POLL_INTERVAL: Duration = Duration::from_millis(100);
const RADIO_LEVEL_POLL_INTERVAL: Duration = Duration::from_secs(1);
const RADIO_COMMAND_WAKE_INTERVAL: Duration = Duration::from_millis(50);
const RADIO_SCOPE_READ_SLICE: Duration = Duration::from_millis(50);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RadioScopeStreamConfig {
    view: RadioScopeView,
    span_code: u8,
    vbw_wide: bool,
    edges: Option<(u64, u64)>,
    sweep_code: u8,
    hold: bool,
    reference_tenths_db: i16,
}

fn workspace_audio_controls_clear_noise() -> (ControlId, ControlValue, ControlId, ControlValue) {
    (
        ControlId::NoiseReduction,
        ControlValue::Bool(false),
        ControlId::NoiseBlanker,
        ControlValue::Bool(false),
    )
}

pub(crate) fn spawn_radio_worker(
    radio: ConfiguredRadio,
    state: Arc<Mutex<GuiState>>,
    stop: Arc<AtomicBool>,
    display_tuning: Arc<Mutex<DisplayTuning>>,
    rx: mpsc::Receiver<GuiCommand>,
    repaint_ctx: Arc<OnceLock<egui::Context>>,
) -> std::thread::JoinHandle<()> {
    thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(err) => {
                let mut s = state.lock().expect("ui state lock poisoned");
                s.last_error = Some(format!("failed to start GUI runtime: {err}"));
                return;
            }
        };

        let stream_state = state.clone();
        let stream_radio = radio.as_icom().cloned();
        let stream_stop = stop.clone();
        let stream_repaint = repaint_ctx.clone();
        let stream_display_tuning = display_tuning.clone();
        let _stream_handle = thread::spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(err) => {
                    let mut s = stream_state.lock().expect("ui state lock poisoned");
                    s.last_error = Some(format!("failed to start streaming runtime: {err}"));
                    return;
                }
            };

            let mut last_scope_config: Option<RadioScopeStreamConfig> = None;
            let mut cadence_started = Instant::now();
            let mut cadence_sweeps = 0usize;
            let mut cadence_rate = 0.0_f32;
            let mut division_rate = 0.0_f32;
            let mut last_scope_divisions = 0_u64;
            let mut last_dropped_sweeps = 0_u64;
            let mut dropped_sweeps_delta = 0_u64;
            let waterfall_repaint_interval = Duration::from_millis(66);
            let mut last_waterfall_repaint = Instant::now() - waterfall_repaint_interval;

            let Some(stream_radio) = stream_radio else {
                let mut s = stream_state.lock().expect("ui state lock poisoned");
                s.radio_spectrum_enabled = false;
                s.radio_waterfall_status = "UNAVAILABLE (radio has no scope stream)".to_string();
                return;
            };

            while !stream_stop.load(Ordering::Relaxed) {
                let (
                    spectrum_desired,
                    spectrum_enabled,
                    scope_view,
                    frequency_hz,
                    span_code,
                    vbw_wide,
                    mode,
                    scope_hold,
                    scope_reference_tenths_db,
                ) = {
                    let s = stream_state.lock().expect("ui state lock poisoned");
                    (
                        s.radio_spectrum_desired,
                        s.radio_spectrum_enabled,
                        s.radio_scope_view,
                        s.frequency_hz,
                        s.radio_scope_span_code,
                        s.radio_scope_vbw_wide,
                        s.mode.clone(),
                        s.radio_scope_hold,
                        s.radio_scope_reference_tenths_db,
                    )
                };

                if spectrum_desired {
                    let scope_edges = match scope_view {
                        RadioScopeView::Overview => frequency_hz
                            .and_then(|hz| band_edges_for_frequency(Some(hz)))
                            .map(|(low, high, _)| (low, high)),
                        RadioScopeView::Narrow => frequency_hz.and_then(|hz| {
                            sideband_scope_edges(
                                hz,
                                scope_span_hz(span_code),
                                scope_projection_for_mode(&mode),
                            )
                        }),
                    };
                    let sweep_code = {
                        let tuning = stream_display_tuning.lock().expect("tuning lock poisoned");
                        effective_visual_profile(&tuning, &mode).1
                    };
                    let config = RadioScopeStreamConfig {
                        view: scope_view,
                        span_code,
                        vbw_wide,
                        edges: scope_edges,
                        sweep_code,
                        hold: scope_hold,
                        reference_tenths_db: scope_reference_tenths_db,
                    };
                    if last_scope_config != Some(config) {
                        let geometry_changed = last_scope_config.is_none_or(|previous| {
                            previous.view != config.view
                                || previous.span_code != config.span_code
                                || previous.edges != config.edges
                        });
                        let hold_update = last_scope_config
                            .filter(|previous| previous.hold != config.hold)
                            .map(|_| config.hold);
                        let reference_update = last_scope_config
                            .filter(|previous| {
                                previous.reference_tenths_db != config.reference_tenths_db
                            })
                            .map(|_| config.reference_tenths_db);
                        match configure_radio_scope(
                            &rt,
                            &stream_radio,
                            &config,
                            hold_update,
                            reference_update,
                        ) {
                            Ok(()) => {
                                last_scope_config = Some(config);
                                let mut s = stream_state.lock().expect("ui state lock poisoned");
                                if geometry_changed {
                                    s.radio_waterfall_rows.clear();
                                    s.radio_waterfall_revision =
                                        s.radio_waterfall_revision.wrapping_add(1);
                                }
                                s.last_error = None;
                            }
                            Err(err) => {
                                let mut s = stream_state.lock().expect("ui state lock poisoned");
                                s.radio_waterfall_status = "CONFIG ERROR".to_string();
                                s.last_error = Some(err.to_string());
                                drop(s);
                                thread::sleep(Duration::from_millis(500));
                                continue;
                            }
                        }
                    }
                }

                if !spectrum_desired {
                    if spectrum_enabled {
                        let disable_result = rt.block_on(stream_radio.disable_spectrum_stream());
                        let mut s = stream_state.lock().expect("ui state lock poisoned");
                        s.radio_spectrum_enabled = false;
                        match disable_result {
                            Ok(()) => s.radio_waterfall_status = "OFF".to_string(),
                            Err(err) => {
                                s.radio_waterfall_status = "DISABLE ERROR".to_string();
                                s.last_error = Some(err.to_string());
                            }
                        }
                    }
                    thread::sleep(Duration::from_millis(250));
                    continue;
                }

                if !spectrum_enabled {
                    match rt
                        .block_on(stream_radio.enable_spectrum_stream(Duration::from_millis(2_500)))
                    {
                        Ok(bins) => {
                            let mut s = stream_state.lock().expect("ui state lock poisoned");
                            s.radio_spectrum_enabled = true;
                            apply_waterfall_bins(&mut s, &bins);
                            s.radio_waterfall_status = "READY · 475 bins".to_string();
                            s.last_error = None;
                            drop(s);
                            if let Some(ctx) = stream_repaint.get() {
                                ctx.request_repaint();
                            }
                        }
                        Err(err) => {
                            let mut s = stream_state.lock().expect("ui state lock poisoned");
                            s.radio_spectrum_enabled = false;
                            s.radio_waterfall_status = "ENABLE RETRY".to_string();
                            s.last_error = Some(err.to_string());
                            drop(s);
                            thread::sleep(Duration::from_millis(1_000));
                        }
                    }
                    continue;
                }

                // Preserve every complete native sweep delivered in a serial
                // read instead of returning one and discarding the buffered rest.
                match rt.block_on(stream_radio.drain_scope_waveform_sweeps(RADIO_SCOPE_READ_SLICE))
                {
                    Ok(sweeps) if !sweeps.is_empty() => {
                        cadence_sweeps += sweeps.len();
                        let elapsed = cadence_started.elapsed();
                        if elapsed >= Duration::from_secs(1) {
                            cadence_rate = cadence_sweeps as f32 / elapsed.as_secs_f32();
                            let (divisions, _, dropped) = stream_radio.scope_stream_counters();
                            division_rate = divisions.saturating_sub(last_scope_divisions) as f32
                                / elapsed.as_secs_f32();
                            dropped_sweeps_delta = dropped.saturating_sub(last_dropped_sweeps);
                            last_scope_divisions = divisions;
                            last_dropped_sweeps = dropped;
                            cadence_sweeps = 0;
                            cadence_started = Instant::now();
                        }
                        let mut s = stream_state.lock().expect("ui state lock poisoned");
                        for bins in sweeps {
                            if !bins.is_empty() {
                                apply_waterfall_bins(&mut s, &bins);
                            }
                        }
                        s.radio_waterfall_status = format!(
                            "READY · {:.1} sweeps/s · {:.0} div/s · {} dropped",
                            cadence_rate, division_rate, dropped_sweeps_delta
                        );
                        drop(s);
                        if let Some(ctx) = stream_repaint.get() {
                            let elapsed = last_waterfall_repaint.elapsed();
                            if elapsed >= waterfall_repaint_interval {
                                last_waterfall_repaint = Instant::now();
                                ctx.request_repaint();
                            } else {
                                ctx.request_repaint_after(waterfall_repaint_interval - elapsed);
                            }
                        }
                    }
                    Err(err) => {
                        let msg = err.to_string();
                        let mut s = stream_state.lock().expect("ui state lock poisoned");
                        if is_transient_civ_read_error(&msg) {
                            s.radio_waterfall_status = "WAITING FRAME".to_string();
                        } else {
                            s.radio_spectrum_enabled = false;
                            s.radio_waterfall_status = "NO FRAME".to_string();
                            s.last_error = Some(msg);
                        }
                    }
                    _ => {}
                }
            }
        });

        poll_radio_core_state(&rt, &radio, &state, true);
        let mut next_core_poll = Instant::now() + RADIO_CORE_POLL_INTERVAL;
        let mut next_level_poll = Instant::now() + RADIO_LEVEL_POLL_INTERVAL;

        while !stop.load(Ordering::Relaxed) {
            let wait = next_core_poll
                .saturating_duration_since(Instant::now())
                .min(RADIO_COMMAND_WAKE_INTERVAL);
            let cmd = match rx.recv_timeout(wait) {
                Ok(cmd) => Some(cmd),
                Err(mpsc::RecvTimeoutError::Timeout) => None,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            };

            if let Some(cmd) = cmd {
                match cmd {
                    GuiCommand::Quit => return,
                    GuiCommand::TuneDelta(delta) => {
                        let freq = rt.block_on(radio.get_frequency_hz()).ok();
                        if let Some(freq) = freq {
                            let target = if delta.is_negative() {
                                freq.saturating_sub(delta.unsigned_abs())
                            } else {
                                freq.saturating_add(delta as u64)
                            };
                            if let Err(err) = rt.block_on(radio.set_frequency_hz(target)) {
                                let mut s = state.lock().expect("ui state lock poisoned");
                                s.last_error = Some(err.to_string());
                            }
                        }
                        poll_radio_core_state(&rt, &radio, &state, true);
                    }
                    GuiCommand::TuneTo(target) => {
                        if let Err(err) = rt.block_on(radio.set_frequency_hz(target)) {
                            state.lock().expect("ui state lock poisoned").last_error =
                                Some(err.to_string());
                        }
                        poll_radio_core_state(&rt, &radio, &state, true);
                    }
                    GuiCommand::CycleMode => {
                        let current = rt.block_on(radio.get_mode()).unwrap_or(Mode::Usb);
                        let next = match current {
                            Mode::Usb => Mode::Lsb,
                            Mode::Lsb => Mode::Cw,
                            Mode::Cw => Mode::Data,
                            Mode::Data => Mode::Usb,
                            _ => Mode::Usb,
                        };
                        if let Err(err) = rt.block_on(Radio::set_mode(&radio, next)) {
                            let mut s = state.lock().expect("ui state lock poisoned");
                            s.last_error = Some(err.to_string());
                        }
                        poll_radio_core_state(&rt, &radio, &state, true);
                    }
                    GuiCommand::SetPtt(target) => {
                        let result = rt
                            .block_on(radio.set_ptt(target))
                            .map_err(|error| error.to_string());
                        let mut s = state.lock().expect("ui state lock poisoned");
                        match result {
                            Ok(()) => {
                                s.ptt_on = target;
                                s.last_error = None;
                            }
                            Err(error) => s.last_error = Some(error),
                        }
                        drop(s);
                        poll_radio_core_state(&rt, &radio, &state, true);
                    }
                    GuiCommand::SetPttWithAck(target, ack_tx) => {
                        let result = rt
                            .block_on(radio.set_ptt(target))
                            .map_err(|error| error.to_string());
                        {
                            let mut s = state.lock().expect("ui state lock poisoned");
                            match &result {
                                Ok(()) => {
                                    s.ptt_on = target;
                                    s.last_error = None;
                                }
                                Err(error) => s.last_error = Some(error.clone()),
                            }
                        }
                        let _ = ack_tx.send(result);
                        poll_radio_core_state(&rt, &radio, &state, true);
                    }
                    GuiCommand::ApplyWorkspace {
                        mode: workspace_mode,
                        frequency_hz,
                    } => {
                        let preset = workspace_radio_preset(workspace_mode);
                        let filter = preset.filter.clamp(1, 3);
                        let frequency_result = rt.block_on(radio.set_frequency_hz(frequency_hz));
                        let (nr_id, nr_value, nb_id, nb_value) =
                            workspace_audio_controls_clear_noise();
                        let noise_result = rt
                            .block_on(radio.set_control(nr_id, nr_value))
                            .and_then(|_| rt.block_on(radio.set_control(nb_id, nb_value)));
                        if let Some(icom) = radio.as_icom() {
                            let mode_result = rt.block_on(icom.set_operating_mode_details(
                                preset.base_mode,
                                preset.data_mode,
                                filter,
                            ));
                            if let Err(error) = frequency_result.and(mode_result).and(noise_result)
                            {
                                state.lock().expect("ui state lock poisoned").last_error =
                                    Some(error.to_string());
                            }
                        } else {
                            let mode = if preset.data_mode {
                                Mode::Data
                            } else {
                                match preset.base_mode {
                                    BaseMode::Lsb => Mode::Lsb,
                                    BaseMode::Cw | BaseMode::CwR => Mode::Cw,
                                    _ => Mode::Usb,
                                }
                            };
                            let mode_result = rt.block_on(Radio::set_mode(&radio, mode));
                            if let Err(error) = frequency_result.and(mode_result) {
                                state.lock().expect("ui state lock poisoned").last_error =
                                    Some(error.to_string());
                            }
                        }
                        poll_radio_core_state(&rt, &radio, &state, true);
                    }
                    GuiCommand::SetFilter(n) => {
                        let workspace_mode =
                            state.lock().expect("ui state lock poisoned").workspace_mode;
                        let preset = workspace_radio_preset(workspace_mode);
                        let target_filter = n.clamp(1, 3);
                        let result = if let Some(icom) = radio.as_icom() {
                            rt.block_on(icom.set_operating_mode_details(
                                preset.base_mode,
                                preset.data_mode,
                                target_filter,
                            ))
                        } else {
                            Err(anyhow::anyhow!(
                                "filter selection is unavailable for this radio profile"
                            ))
                        };
                        if let Err(err) = result {
                            let mut s = state.lock().expect("ui state lock poisoned");
                            s.last_error = Some(err.to_string());
                        }
                        poll_radio_core_state(&rt, &radio, &state, true);
                    }
                    GuiCommand::SetControl(id, value) => {
                        if let Err(error) = rt.block_on(radio.set_control(id, value)) {
                            state.lock().expect("ui state lock poisoned").last_error =
                                Some(error.to_string());
                        }
                        poll_radio_core_state(&rt, &radio, &state, true);
                    }
                    GuiCommand::AfGainDelta(delta) => {
                        let current = match rt
                            .block_on(radio.get_control(ControlId::AfGain))
                            .ok()
                            .flatten()
                        {
                            Some(ControlValue::U8(v)) => v,
                            _ => 100,
                        };
                        let target = if delta.is_negative() {
                            current.saturating_sub(delta.unsigned_abs() as u8)
                        } else {
                            current.saturating_add(delta as u8)
                        };
                        if let Err(err) = rt.block_on(
                            radio.set_control(ControlId::AfGain, ControlValue::U8(target)),
                        ) {
                            let mut s = state.lock().expect("ui state lock poisoned");
                            s.last_error = Some(err.to_string());
                        }
                        poll_radio_core_state(&rt, &radio, &state, true);
                    }
                }
                next_core_poll = Instant::now() + RADIO_CORE_POLL_INTERVAL;
                next_level_poll = Instant::now() + RADIO_LEVEL_POLL_INTERVAL;
                if let Some(ctx) = repaint_ctx.get() {
                    ctx.request_repaint();
                }
            }

            let now = Instant::now();
            if now >= next_core_poll {
                let poll_levels = now >= next_level_poll;
                poll_radio_core_state(&rt, &radio, &state, poll_levels);
                next_core_poll = Instant::now() + RADIO_CORE_POLL_INTERVAL;
                if poll_levels {
                    next_level_poll = Instant::now() + RADIO_LEVEL_POLL_INTERVAL;
                }
                if let Some(ctx) = repaint_ctx.get() {
                    ctx.request_repaint();
                }
            }
        }
    })
}

fn poll_radio_core_state(
    rt: &tokio::runtime::Runtime,
    radio: &ConfiguredRadio,
    state: &Arc<Mutex<GuiState>>,
    poll_levels: bool,
) {
    if radio.as_icom().is_none() {
        let frequency = rt.block_on(radio.get_frequency_hz());
        let mode = rt.block_on(radio.get_mode());
        let mut s = state.lock().expect("ui state lock poisoned");
        match (frequency, mode) {
            (Ok(frequency_hz), Ok(mode)) => {
                s.frequency_hz = Some(frequency_hz);
                s.mode = match mode {
                    Mode::Usb => "USB",
                    Mode::Lsb => "LSB",
                    Mode::Cw => "CW",
                    Mode::Data => "DATA",
                    Mode::Am => "AM",
                    Mode::Fm => "FM",
                    Mode::Wfm => "WFM",
                    Mode::Rtty => "RTTY",
                    Mode::CwReverse => "CW-R",
                    Mode::RttyReverse => "RTTY-R",
                }
                .to_string();
                s.data_mode = Some(mode == Mode::Data);
                s.last_update = Some(Instant::now());
                s.last_error = None;
            }
            (frequency, mode) => {
                s.last_error = Some(
                    frequency
                        .err()
                        .or_else(|| mode.err())
                        .expect("one CAT operation failed")
                        .to_string(),
                );
            }
        }
        return;
    }
    let radio = radio.as_icom().expect("checked Icom driver");
    // Read what we need under a brief lock, then do all I/O outside it.
    let spectrum_enabled = state
        .lock()
        .expect("ui state lock poisoned")
        .radio_spectrum_enabled;
    let status_result = if spectrum_enabled {
        rt.block_on(radio.probe_stream_status())
    } else {
        radio.probe()
    };
    let (af, rf, pwr) = if poll_levels {
        (
            read_u8_control(rt, radio, ControlId::AfGain),
            read_u8_control(rt, radio, ControlId::RfGain),
            read_u8_control(rt, radio, ControlId::RfPower),
        )
    } else {
        (None, None, None)
    };
    // The IC-7300's regular mode response does not consistently include the
    // active FIL number, so filter state needs its own fast query.
    let filt = read_u8_control(rt, radio, ControlId::Filter);
    let mut s = state.lock().expect("ui state lock poisoned");
    if let Ok(status) = status_result {
        if let Some(freq) = status.frequency_hz {
            s.frequency_hz = Some(freq);
        }
        if let Some(mode) = status.mode {
            s.mode = mode;
        }
        if let Some(details) = status.mode_details {
            s.data_mode = Some(details.data_mode);
            s.filter = details.filter;
        }
        s.last_update = Some(Instant::now());
    }
    if let Some(v) = af {
        s.af_gain = Some(v);
    }
    if let Some(v) = rf {
        s.rf_gain = Some(v);
    }
    if let Some(v) = pwr {
        s.rf_power = Some(v);
    }
    if let Some(v) = filt {
        s.filter = Some(v);
    }
}

pub(crate) fn apply_waterfall_bins(next: &mut GuiState, bins: &[u8]) {
    let mut row = if bins.len() > MAX_RADIO_WF_BINS {
        downsample_bins(bins, MAX_RADIO_WF_BINS)
    } else {
        bins.to_vec()
    };
    row = scale_scope_levels(&row, next.radio_scope_contrast);
    if next.radio_waterfall_rows.len() >= RADIO_WF_HEIGHT {
        next.radio_waterfall_rows.pop_front();
    }
    next.radio_waterfall_rows.push_back(row);
    next.radio_waterfall_revision = next.radio_waterfall_revision.wrapping_add(1);
}

fn configure_radio_scope(
    rt: &tokio::runtime::Runtime,
    radio: &IcomCiVRadio,
    config: &RadioScopeStreamConfig,
    hold_update: Option<bool>,
    reference_tenths_db_update: Option<i16>,
) -> Result<()> {
    rt.block_on(radio.set_scope_sweep_speed(config.sweep_code))?;
    if let Some(reference_tenths_db) = reference_tenths_db_update {
        rt.block_on(radio.set_scope_reference_level_tenths_db(reference_tenths_db))?;
    }
    match config.view {
        RadioScopeView::Narrow => {
            if let Some((low_hz, high_hz)) = config.edges {
                // Edge 4 is reserved for QSONaut's mode-aware passband window,
                // leaving the operator's first three radio edge memories alone.
                rt.block_on(radio.set_scope_fixed_edge_frequencies(4, low_hz, high_hz))?;
                rt.block_on(radio.set_scope_fixed_edge_number(4))?;
                rt.block_on(radio.set_scope_center_fixed_mode(true))?;
            } else {
                rt.block_on(radio.set_scope_center_fixed_mode(false))?;
                rt.block_on(radio.set_scope_span_hz(scope_span_hz(config.span_code)))?;
            }
            rt.block_on(
                radio.set_scope_vbw_wide(scope_vbw_wide_for_view(config.view, config.vbw_wide)),
            )?;
        }
        RadioScopeView::Overview => {
            let (low_hz, high_hz) = config.edges.context("active band edges unavailable")?;
            rt.block_on(radio.set_scope_fixed_edge_frequencies(1, low_hz, high_hz))?;
            rt.block_on(radio.set_scope_fixed_edge_number(1))?;
            rt.block_on(radio.set_scope_center_fixed_mode(true))?;
            rt.block_on(
                radio.set_scope_vbw_wide(scope_vbw_wide_for_view(config.view, config.vbw_wide)),
            )?;
        }
    }
    if let Some(hold) = hold_update {
        rt.block_on(radio.set_scope_hold(hold))?;
    }
    Ok(())
}

fn scope_vbw_wide_for_view(_view: RadioScopeView, user_vbw_wide: bool) -> bool {
    user_vbw_wide
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_vbw_respects_checkbox_in_narrow_and_overview_views() {
        for view in [RadioScopeView::Narrow, RadioScopeView::Overview] {
            assert!(!scope_vbw_wide_for_view(view, false));
            assert!(scope_vbw_wide_for_view(view, true));
        }
    }
}

fn read_u8_control<R: Radio + ?Sized>(
    rt: &tokio::runtime::Runtime,
    radio: &R,
    id: ControlId,
) -> Option<u8> {
    match rt.block_on(radio.get_control(id)).ok().flatten() {
        Some(ControlValue::U8(v)) => Some(v),
        _ => None,
    }
}

#[cfg(test)]
mod workspace_tests {
    use super::*;

    #[test]
    fn software_workspace_clears_noise_processing() {
        let (nr_id, nr_value, nb_id, nb_value) = workspace_audio_controls_clear_noise();
        assert_eq!(nr_id, ControlId::NoiseReduction);
        assert_eq!(nr_value, ControlValue::Bool(false));
        assert_eq!(nb_id, ControlId::NoiseBlanker);
        assert_eq!(nb_value, ControlValue::Bool(false));
    }
}
