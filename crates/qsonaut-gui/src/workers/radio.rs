use super::super::*;
use super::request_gui_repaint;

const RADIO_CORE_POLL_INTERVAL: Duration = Duration::from_millis(100);
const RADIO_LEVEL_POLL_INTERVAL: Duration = Duration::from_millis(500);
const RADIO_COMMAND_WAKE_INTERVAL: Duration = Duration::from_millis(50);
const RADIO_SCOPE_READ_SLICE: Duration = Duration::from_millis(25);
const RADIO_SIGNAL_METER_INTERVAL: Duration = Duration::from_millis(400);
const RADIO_TX_METER_INTERVAL: Duration = Duration::from_millis(300);
const RADIO_AUX_METER_INTERVAL: Duration = Duration::from_millis(1_500);
const RADIO_METER_RESPONSE_TIMEOUT: Duration = Duration::from_millis(150);

// The first radio core poll is instrumented at INFO level so a hardware startup
// stall is visible in the normal application log without logging every 100 ms
// poll forever.
static FIRST_RADIO_CORE_POLL: AtomicBool = AtomicBool::new(true);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScheduledMeter {
    Signal,
    Power,
    Swr,
    Alc,
    Compression,
    Current,
    Voltage,
    Temperature,
}

impl ScheduledMeter {
    fn meter_id(self) -> MeterId {
        match self {
            Self::Signal => MeterId::Signal,
            Self::Power => MeterId::Power,
            Self::Swr => MeterId::Swr,
            Self::Alc => MeterId::Alc,
            Self::Compression => MeterId::Compression,
            Self::Current => MeterId::Current,
            Self::Voltage => MeterId::Voltage,
            Self::Temperature => MeterId::Temperature,
        }
    }
}

struct MeterPollScheduler {
    next_signal: Instant,
    next_tx: Instant,
    next_aux: Instant,
    aux_index: usize,
    tx_index: usize,
}

impl MeterPollScheduler {
    fn new() -> Self {
        let now = Instant::now();
        Self {
            next_signal: now,
            next_tx: now,
            next_aux: now + RADIO_AUX_METER_INTERVAL,
            aux_index: 0,
            tx_index: 0,
        }
    }

    fn next_due(&mut self, now: Instant, transmitting: bool) -> Option<ScheduledMeter> {
        if !transmitting && now >= self.next_signal {
            self.next_signal = now + RADIO_SIGNAL_METER_INTERVAL;
            return Some(ScheduledMeter::Signal);
        }
        if transmitting && now >= self.next_tx {
            self.next_tx = now + RADIO_TX_METER_INTERVAL;
            let meter = match self.tx_index % 4 {
                0 => ScheduledMeter::Power,
                1 => ScheduledMeter::Swr,
                2 => ScheduledMeter::Alc,
                _ => ScheduledMeter::Compression,
            };
            self.tx_index = self.tx_index.wrapping_add(1);
            return Some(meter);
        }
        if now >= self.next_aux {
            const AUXILIARY: [ScheduledMeter; 4] = [
                ScheduledMeter::Current,
                ScheduledMeter::Voltage,
                ScheduledMeter::Temperature,
                ScheduledMeter::Signal,
            ];
            let meter = AUXILIARY[self.aux_index % AUXILIARY.len()];
            self.aux_index = self.aux_index.wrapping_add(1);
            self.next_aux = now + RADIO_AUX_METER_INTERVAL;
            return Some(meter);
        }
        None
    }
}

const RADIO_CONTROL_IDS: &[ControlId] = &[
    ControlId::AfGain,
    ControlId::RfGain,
    ControlId::Squelch,
    ControlId::RfPower,
    ControlId::Preamp,
    ControlId::ExternalPreamp,
    ControlId::Attenuator,
    ControlId::NoiseBlanker,
    ControlId::NoiseReduction,
    ControlId::NoiseReductionLevel,
    ControlId::IpPlus,
    ControlId::Notch,
    ControlId::ManualNotch,
    ControlId::DataMode,
    ControlId::Filter,
    ControlId::Agc,
    ControlId::Rit,
    ControlId::Xit,
    ControlId::Split,
    ControlId::Tuner,
    ControlId::Vfo,
    ControlId::MainSub,
];

const RADIO_METER_IDS: &[MeterId] = &[
    MeterId::Signal,
    MeterId::Power,
    MeterId::Swr,
    MeterId::Alc,
    MeterId::Compression,
    MeterId::Current,
    MeterId::Voltage,
    MeterId::Temperature,
];

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

#[allow(clippy::too_many_arguments)]
pub(crate) fn spawn_radio_worker(
    radio: ConfiguredRadio,
    state: Arc<Mutex<GuiState>>,
    stop: Arc<AtomicBool>,
    swr_sweep_abort: Arc<AtomicBool>,
    display_tuning: Arc<Mutex<DisplayTuning>>,
    rx: mpsc::Receiver<GuiCommand>,
    repaint_ctx: Arc<OnceLock<egui::Context>>,
    ptt_allowed: Arc<AtomicBool>,
) -> std::thread::JoinHandle<()> {
    thread::spawn(move || {
        {
            let mut s = state.lock().expect("ui state lock poisoned");
            s.radio_power_supported = radio.capabilities().can_set_power;
            s.supported_controls = RADIO_CONTROL_IDS
                .iter()
                .copied()
                .filter(|id| radio.supports_control(*id))
                .collect();
            s.supported_meters = RADIO_METER_IDS
                .iter()
                .copied()
                .filter(|id| radio.supports_meter(*id))
                .collect();
        }
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
            let mut display_rows = 0usize;
            let mut display_rate = 0.0_f32;
            let mut division_rate = 0.0_f32;
            let mut last_scope_divisions = 0_u64;
            let mut last_dropped_sweeps = 0_u64;
            let mut dropped_sweeps_delta = 0_u64;
            let mut latest_scope_bins: Option<Vec<u8>> = None;
            let mut last_display_row = Instant::now();
            let waterfall_repaint_interval = Duration::from_millis(66);
            let mut last_waterfall_repaint = Instant::now() - waterfall_repaint_interval;
            let mut meter_scheduler = MeterPollScheduler::new();

            let Some(stream_radio) = stream_radio else {
                warn!("Radio scope worker unavailable: radio has no scope stream");
                let mut s = stream_state.lock().expect("ui state lock poisoned");
                s.radio_spectrum_enabled = false;
                s.radio_waterfall_status = "UNAVAILABLE (radio has no scope stream)".to_string();
                return;
            };

            while !stream_stop.load(Ordering::Relaxed) {
                let (
                    spectrum_desired,
                    spectrum_enabled,
                    power_on,
                    power_settling,
                    power_command_pending,
                    scope_view,
                    frequency_hz,
                    span_code,
                    vbw_wide,
                    workspace_mode,
                    mode,
                    ptt_on,
                    scope_hold,
                    scope_reference_tenths_db,
                    scope_settings_dirty,
                ) = {
                    let s = stream_state.lock().expect("ui state lock poisoned");
                    (
                        s.radio_spectrum_desired,
                        s.radio_spectrum_enabled,
                        s.radio_power_on,
                        s.radio_power_settling,
                        s.radio_power_command_pending,
                        s.radio_scope_view,
                        s.frequency_hz,
                        s.radio_scope_span_code,
                        s.radio_scope_vbw_wide,
                        s.workspace_mode,
                        s.mode.clone(),
                        s.ptt_on,
                        s.radio_scope_hold,
                        s.radio_scope_reference_tenths_db,
                        s.radio_scope_settings_dirty,
                    )
                };

                // Do not send scope traffic while the radio state is unknown,
                // powered off, or still waking. The initial state can be
                // unknown briefly while the core worker performs its first
                // probe, and sending CI-V scope commands during that window
                // can delay a cold power-on.
                if power_on != Some(true) || power_settling || power_command_pending {
                    let mut s = stream_state.lock().expect("ui state lock poisoned");
                    s.radio_spectrum_enabled = false;
                    s.radio_waterfall_status = if power_on == Some(false) {
                        "OFF (radio off)".to_string()
                    } else {
                        "WAITING FOR RADIO".to_string()
                    };
                    drop(s);
                    thread::sleep(Duration::from_millis(250));
                    continue;
                }

                if spectrum_desired {
                    let scope_edges = match (scope_view, workspace_mode) {
                        // Voice does not need a mode-aware passband. Keep the
                        // radio scope in its normal centered-span mode so a
                        // voice transition does not rewrite fixed edge memory.
                        (RadioScopeView::Narrow, WorkspaceMode::Voice) => None,
                        (RadioScopeView::Overview, _) => frequency_hz
                            .and_then(|hz| band_edges_for_frequency(Some(hz)))
                            .map(|(low, high, _)| (low, high)),
                        (RadioScopeView::Narrow, _) => frequency_hz.and_then(|hz| {
                            sideband_scope_edges(
                                hz,
                                scope_span_hz(span_code),
                                scope_projection_for_mode(&mode),
                            )
                        }),
                    };
                    let sweep_code = {
                        let tuning = stream_display_tuning.lock().expect("tuning lock poisoned");
                        effective_visual_profile(&tuning, &mode, true).1
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
                    // The first observed configuration is a baseline, not a
                    // command. Startup must not restore QSONaut defaults onto
                    // the radio. Subsequent changes are operator-driven and
                    // may be applied normally.
                    if !scope_settings_dirty {
                        // Profile hydration, mode readback, and initial
                        // frequency discovery are observations, not operator
                        // requests. Keep the latest values as the baseline.
                        last_scope_config = Some(config);
                    } else if scope_config_changed(last_scope_config, config) {
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
                                info!(
                                    view = ?config.view,
                                    span_code = config.span_code,
                                    vbw_wide = config.vbw_wide,
                                    sweep_code = config.sweep_code,
                                    hold = config.hold,
                                    reference_tenths_db = config.reference_tenths_db,
                                    "Radio scope configured"
                                );
                                let mut s = stream_state.lock().expect("ui state lock poisoned");
                                s.radio_scope_settings_dirty = false;
                                if geometry_changed {
                                    s.radio_waterfall_rows.clear();
                                    s.radio_waterfall_revision =
                                        s.radio_waterfall_revision.wrapping_add(1);
                                }
                                s.last_error = None;
                            }
                            Err(err) => {
                                warn!(error = %err, view = ?config.view, "Radio scope configuration failed");
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
                            Ok(()) => {
                                s.radio_waterfall_status = "OFF".to_string();
                                info!("Radio spectrum stream disabled");
                            }
                            Err(err) => {
                                s.radio_waterfall_status = "DISABLE ERROR".to_string();
                                s.last_error = Some(err.to_string());
                                error!(error = %err, "Radio spectrum stream disable failed");
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
                            let bin_count = bins.len();
                            let mut s = stream_state.lock().expect("ui state lock poisoned");
                            s.radio_spectrum_enabled = true;
                            apply_waterfall_bins(&mut s, &bins);
                            latest_scope_bins = Some(bins);
                            display_rows += 1;
                            last_display_row = Instant::now();
                            s.radio_waterfall_status = "READY · 475 bins".to_string();
                            s.last_error = None;
                            drop(s);
                            info!(
                                bins = bin_count,
                                "Radio spectrum stream enabled without changing scope settings"
                            );
                            request_gui_repaint(&stream_repaint);
                        }
                        Err(err) => {
                            let mut s = stream_state.lock().expect("ui state lock poisoned");
                            s.radio_spectrum_enabled = false;
                            s.radio_waterfall_status = "ENABLE RETRY".to_string();
                            s.last_error = Some(err.to_string());
                            warn!(error = %err, "Radio spectrum stream enable failed; retrying");
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
                        for bins in sweeps {
                            if !bins.is_empty() {
                                latest_scope_bins = Some(bins);
                            }
                        }
                    }
                    Err(err) => {
                        let msg = err.to_string();
                        let mut s = stream_state.lock().expect("ui state lock poisoned");
                        if is_transient_civ_read_error(&msg) {
                            s.radio_waterfall_status = "WAITING FRAME".to_string();
                        } else {
                            error!(error = %msg, "Radio scope waveform read failed");
                            s.radio_spectrum_enabled = false;
                            s.radio_waterfall_status = "NO FRAME".to_string();
                            s.last_error = Some(msg);
                            continue;
                        }
                    }
                    _ => {}
                }

                // CI-V scope traffic owns the connection's fast path. Read at
                // most one meter after servicing the scope, and only when its
                // priority makes it due. This prevents a level batch from
                // monopolizing the serial connection and lets TX meters take
                // precedence over the slower auxiliary rotation.
                if let Some(meter) = meter_scheduler.next_due(Instant::now(), ptt_on) {
                    poll_scheduled_icom_meter(&rt, &stream_radio, &stream_state, meter);
                }

                // A complete IC-7300 sweep consists of 11 CI-V division
                // frames. Keep that native measurement honest while scrolling
                // the display at the selected host cadence by repeating the
                // latest complete sweep between radio updates.
                let display_interval_ms = {
                    let tuning = stream_display_tuning.lock().expect("tuning lock poisoned");
                    effective_visual_profile(&tuning, &mode, true).0
                };
                let display_due =
                    last_display_row.elapsed() >= Duration::from_millis(display_interval_ms);
                if display_due {
                    if let Some(bins) = latest_scope_bins.as_deref() {
                        let mut s = stream_state.lock().expect("ui state lock poisoned");
                        apply_waterfall_bins(&mut s, bins);
                        display_rows += 1;
                        last_display_row = Instant::now();
                    }
                }

                let elapsed = cadence_started.elapsed();
                if elapsed >= Duration::from_secs(1) {
                    cadence_rate = cadence_sweeps as f32 / elapsed.as_secs_f32();
                    display_rate = display_rows as f32 / elapsed.as_secs_f32();
                    let (divisions, _, dropped) = stream_radio.scope_stream_counters();
                    division_rate = divisions.saturating_sub(last_scope_divisions) as f32
                        / elapsed.as_secs_f32();
                    dropped_sweeps_delta = dropped.saturating_sub(last_dropped_sweeps);
                    last_scope_divisions = divisions;
                    last_dropped_sweeps = dropped;
                    cadence_sweeps = 0;
                    display_rows = 0;
                    cadence_started = Instant::now();
                }

                if display_due && latest_scope_bins.is_some() {
                    let mut s = stream_state.lock().expect("ui state lock poisoned");
                    s.radio_waterfall_status = format!(
                        "READY · {:.1} native/s · {:.1} rows/s · {:.0} div/s · {} dropped",
                        cadence_rate, display_rate, division_rate, dropped_sweeps_delta
                    );
                    drop(s);
                    if last_waterfall_repaint.elapsed() >= waterfall_repaint_interval {
                        last_waterfall_repaint = Instant::now();
                        request_gui_repaint(&stream_repaint);
                    }
                }
            }
        });

        info!("Initial radio poll started");
        // Establish frequency/mode state before entering the normal polling
        // schedule. Icom control state is also read here so the capability
        // panel is useful immediately; non-Icom level reads begin on the next
        // scheduled poll and cannot delay startup.
        // The Icom scope worker owns high-rate meter reads, but it must not
        // suppress live state for controls such as TUNE, NB, IP+, and notch.
        poll_radio_core_state(&rt, &radio, &state, radio.as_icom().is_some());
        info!("Initial radio poll completed");
        let mut next_core_poll = Instant::now() + RADIO_CORE_POLL_INTERVAL;
        let mut next_level_poll = Instant::now() + RADIO_LEVEL_POLL_INTERVAL;
        let mut radio_power_settle_until = Instant::now();

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
                let radio_unavailable = {
                    let s = state.lock().expect("ui state lock poisoned");
                    s.radio_power_on == Some(false)
                        || s.radio_power_command_pending
                        || s.radio_power_settling
                };
                if radio_unavailable && !matches!(&cmd, GuiCommand::Quit | GuiCommand::SetPower(_))
                {
                    warn!(command = ?cmd, "Radio command skipped while radio is unavailable");
                    let mut s = state.lock().expect("ui state lock poisoned");
                    s.last_error = Some("radio command skipped: radio is unavailable".to_string());
                    if let GuiCommand::SetPttWithAck(_, ack_tx) = &cmd {
                        let _ = ack_tx.send(Err("radio is powered off".to_string()));
                    }
                    continue;
                }
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
                            info!(
                                delta_hz = delta,
                                from_hz = freq,
                                target_hz = target,
                                "Radio tune command requested"
                            );
                            match rt.block_on(radio.set_frequency_hz(target)) {
                                Ok(()) => {
                                    info!(frequency_hz = target, "Radio tune command accepted")
                                }
                                Err(err) => {
                                    error!(target_hz = target, error = %err, "Radio tune command failed");
                                    let mut s = state.lock().expect("ui state lock poisoned");
                                    s.last_error = Some(err.to_string());
                                }
                            }
                        } else {
                            warn!(
                                delta_hz = delta,
                                "Radio tune command skipped: frequency unavailable"
                            );
                        }
                        poll_radio_core_state(&rt, &radio, &state, true);
                    }
                    GuiCommand::TuneTo(target) => {
                        info!(target_hz = target, "Radio direct tune command requested");
                        match rt.block_on(radio.set_frequency_hz(target)) {
                            Ok(()) => {
                                info!(frequency_hz = target, "Radio direct tune command accepted")
                            }
                            Err(err) => {
                                error!(target_hz = target, error = %err, "Radio direct tune command failed");
                                state.lock().expect("ui state lock poisoned").last_error =
                                    Some(err.to_string());
                            }
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
                        info!(from = ?current, to = ?next, "Radio mode cycle requested");
                        match rt.block_on(Radio::set_mode(&radio, next)) {
                            Ok(()) => info!(mode = ?next, "Radio mode cycle accepted"),
                            Err(err) => {
                                error!(mode = ?next, error = %err, "Radio mode cycle failed");
                                let mut s = state.lock().expect("ui state lock poisoned");
                                s.last_error = Some(err.to_string());
                            }
                        }
                        poll_radio_core_state(&rt, &radio, &state, true);
                    }
                    GuiCommand::SetRadioMode(mode) => {
                        info!(mode = ?mode, "Radio mode command requested");
                        match rt.block_on(Radio::set_mode(&radio, mode)) {
                            Ok(()) => info!(mode = ?mode, "Radio mode command accepted"),
                            Err(err) => {
                                error!(mode = ?mode, error = %err, "Radio mode command failed");
                                state.lock().expect("ui state lock poisoned").last_error =
                                    Some(err.to_string());
                            }
                        }
                        poll_radio_core_state(&rt, &radio, &state, true);
                    }
                    GuiCommand::SetPtt(target) => {
                        if target && !ptt_allowed.load(Ordering::Acquire) {
                            warn!("Radio PTT command blocked while profile is inactive");
                            continue;
                        }
                        info!(ptt = target, "Radio PTT command requested");
                        let result = rt
                            .block_on(radio.set_ptt(target))
                            .map_err(|error| error.to_string());
                        let mut s = state.lock().expect("ui state lock poisoned");
                        match result {
                            Ok(()) => {
                                info!(ptt = target, "Radio PTT command accepted");
                                s.ptt_on = target;
                                s.last_error = None;
                            }
                            Err(error) => {
                                error!(ptt = target, error = %error, "Radio PTT command failed");
                                s.last_error = Some(error)
                            }
                        }
                        drop(s);
                        poll_radio_core_state(&rt, &radio, &state, true);
                    }
                    GuiCommand::SetPttWithAck(target, ack_tx) => {
                        if target && !ptt_allowed.load(Ordering::Acquire) {
                            let _ = ack_tx
                                .send(Err("PTT is disabled while this radio profile is inactive"
                                    .to_string()));
                            warn!("Radio PTT command blocked while profile is inactive");
                            continue;
                        }
                        info!(
                            ptt = target,
                            "Radio PTT command requested with acknowledgement"
                        );
                        let result = rt
                            .block_on(radio.set_ptt(target))
                            .map_err(|error| error.to_string());
                        {
                            let mut s = state.lock().expect("ui state lock poisoned");
                            match &result {
                                Ok(()) => {
                                    info!(ptt = target, "Radio PTT command accepted");
                                    s.ptt_on = target;
                                    s.last_error = None;
                                }
                                Err(error) => {
                                    error!(ptt = target, error = %error, "Radio PTT command failed");
                                    s.last_error = Some(error.clone())
                                }
                            }
                        }
                        let _ = ack_tx.send(result);
                        poll_radio_core_state(&rt, &radio, &state, true);
                    }
                    GuiCommand::SetPower(target) => {
                        info!(power_on = target, "Radio power command requested");
                        match rt.block_on(radio.set_power(target)) {
                            Ok(()) => {
                                info!(power_on = target, "Radio power command accepted");
                                let mut s = state.lock().expect("ui state lock poisoned");
                                // A successful write only means the command was
                                // accepted. Do not show ON until a status probe
                                // confirms that the radio has actually woken.
                                s.radio_power_on = if target { None } else { Some(false) };
                                s.radio_power_command_pending = target;
                                s.radio_power_settling = target;
                                s.radio_power_wake_deadline =
                                    target.then_some(Instant::now() + Duration::from_secs(12));
                                s.last_error = None;
                            }
                            Err(error) => {
                                error!(power_on = target, error = %error, "Radio power command failed");
                                let mut s = state.lock().expect("ui state lock poisoned");
                                s.last_error = Some(error.to_string());
                                s.radio_power_command_pending = false;
                                s.radio_power_settling = false;
                                s.radio_power_wake_deadline = None;
                            }
                        }
                        if target {
                            radio_power_settle_until = Instant::now() + Duration::from_secs(2);
                        }
                    }
                    GuiCommand::ApplyWorkspace {
                        mode: workspace_mode,
                        frequency_hz,
                    } => {
                        swr_sweep_abort.store(false, Ordering::Relaxed);
                        info!(
                            workspace = %workspace_mode.label(),
                            frequency_hz,
                            "Applying radio workspace preset"
                        );
                        // Publish the requested workspace before applying the
                        // radio preset so a following filter/control command
                        // cannot briefly reuse the previous digital mode.
                        state.lock().expect("ui state lock poisoned").workspace_mode =
                            workspace_mode;
                        let preset = workspace_radio_preset_for_frequency(
                            workspace_mode,
                            Some(frequency_hz),
                        );
                        let filter = preset.filter.clamp(1, 3);
                        let frequency_result = rt.block_on(radio.set_frequency_hz(frequency_hz));
                        let (nr_id, nr_value, nb_id, nb_value) =
                            workspace_audio_controls_clear_noise();
                        let noise_result = [(nr_id, nr_value), (nb_id, nb_value)]
                            .into_iter()
                            .filter(|(id, _)| radio.supports_control_write(*id))
                            .try_for_each(|(id, value)| rt.block_on(radio.set_control(id, value)));
                        if let Some(icom) = radio.as_icom() {
                            let mode_result = rt.block_on(icom.set_operating_mode_details(
                                preset.base_mode,
                                preset.data_mode,
                                filter,
                            ));
                            let data_mode_result =
                                if workspace_mode == WorkspaceMode::Voice {
                                    rt.block_on(radio.set_control(
                                        ControlId::DataMode,
                                        ControlValue::Bool(false),
                                    ))
                                } else {
                                    Ok(())
                                };
                            if let Err(error) = frequency_result
                                .and(mode_result)
                                .and(data_mode_result)
                                .and(noise_result)
                            {
                                error!(
                                    workspace = %workspace_mode.label(),
                                    frequency_hz,
                                    error = %error,
                                    "Radio workspace preset failed"
                                );
                                state.lock().expect("ui state lock poisoned").last_error =
                                    Some(error.to_string());
                            } else {
                                info!(workspace = %workspace_mode.label(), frequency_hz, mode = ?preset.base_mode, data_mode = preset.data_mode, filter, "Radio workspace preset accepted");
                            }
                        } else {
                            let mode = if preset.data_mode {
                                Mode::Data
                            } else {
                                match preset.base_mode {
                                    BaseMode::Lsb => Mode::Lsb,
                                    BaseMode::Cw | BaseMode::CwR => Mode::Cw,
                                    BaseMode::Fm => Mode::Fm,
                                    _ => Mode::Usb,
                                }
                            };
                            let mode_result = rt.block_on(Radio::set_mode(&radio, mode));
                            match frequency_result.and(mode_result) {
                                Ok(()) => {
                                    // Reflect a successful write immediately. The follow-up
                                    // CAT read is useful confirmation, but a compatible radio
                                    // may briefly return an incomplete status frame after a
                                    // mode change.
                                    let mut s = state.lock().expect("ui state lock poisoned");
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
                                    s.radio_power_on = Some(true);
                                    s.last_error = None;
                                    info!(workspace = %workspace_mode.label(), frequency_hz, mode = ?preset.base_mode, data_mode = preset.data_mode, "Radio workspace preset accepted");
                                }
                                Err(error) => {
                                    error!(
                                        workspace = %workspace_mode.label(),
                                        frequency_hz,
                                        error = %error,
                                        "Radio workspace preset failed"
                                    );
                                    state.lock().expect("ui state lock poisoned").last_error =
                                        Some(error.to_string());
                                }
                            }
                        }
                        poll_radio_core_state(&rt, &radio, &state, true);
                    }
                    GuiCommand::SetFilter(n) => {
                        let workspace_mode =
                            state.lock().expect("ui state lock poisoned").workspace_mode;
                        let frequency_hz =
                            state.lock().expect("ui state lock poisoned").frequency_hz;
                        let preset =
                            workspace_radio_preset_for_frequency(workspace_mode, frequency_hz);
                        let target_filter = n.clamp(1, 3);
                        info!(filter = target_filter, workspace = %workspace_mode.label(), "Radio filter change requested");
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
                        match result {
                            Ok(()) => {
                                info!(filter = target_filter, workspace = %workspace_mode.label(), "Radio filter change accepted")
                            }
                            Err(err) => {
                                error!(filter = target_filter, error = %err, "Radio filter change failed");
                                let mut s = state.lock().expect("ui state lock poisoned");
                                s.last_error = Some(err.to_string());
                            }
                        }
                        poll_radio_core_state(&rt, &radio, &state, true);
                    }
                    GuiCommand::SetControl(id, value) => {
                        debug!(control = ?id, value = ?value, "Radio control change requested");
                        let result = if radio.supports_control_write(id) {
                            rt.block_on(radio.set_control(id, value.clone()))
                        } else {
                            Err(anyhow::anyhow!(
                                "control {id:?} is not writable by the loaded radio profile"
                            ))
                        };
                        match result {
                            Ok(()) => {
                                if id == ControlId::Vfo {
                                    if let Some(vfo) = control_vfo_value(&value) {
                                        state.lock().expect("ui state lock poisoned").active_vfo =
                                            vfo;
                                    }
                                }
                                info!(control = ?id, "Radio control change accepted")
                            }
                            Err(error) => {
                                error!(control = ?id, error = %error, "Radio control change failed");
                                state.lock().expect("ui state lock poisoned").last_error =
                                    Some(error.to_string());
                            }
                        }
                        poll_radio_core_state(&rt, &radio, &state, true);
                    }
                    GuiCommand::StartTuner => {
                        info!("Radio antenna tuner start requested");
                        match rt.block_on(radio.start_tuner()) {
                            Ok(()) => info!("Radio antenna tuner start accepted"),
                            Err(error) => {
                                error!(error = %error, "Radio antenna tuner start failed");
                                state.lock().expect("ui state lock poisoned").last_error =
                                    Some(error.to_string());
                            }
                        }
                        poll_radio_core_state(&rt, &radio, &state, true);
                    }
                    GuiCommand::StartSwrSweep {
                        start_hz,
                        stop_hz,
                        step_hz,
                        interval_ms,
                    } => {
                        info!(
                            start_hz,
                            stop_hz,
                            step_hz,
                            interval_ms,
                            expected_points = stop_hz
                                .saturating_sub(start_hz)
                                .checked_div(step_hz.max(1))
                                .unwrap_or(0)
                                .saturating_add(1)
                                .min(200),
                            "SWR sweep requested"
                        );
                        if step_hz == 0 || stop_hz < start_hz {
                            state.lock().expect("ui state lock poisoned").last_error = Some(
                                "SWR sweep requires a positive step and stop >= start".to_string(),
                            );
                            continue;
                        }
                        {
                            let mut s = state.lock().expect("ui state lock poisoned");
                            s.swr_sweep_active = true;
                            s.swr_sweep_points.clear();
                            s.swr_sweep_status = "Saving radio state".to_string();
                        }
                        let original_frequency = rt.block_on(radio.get_frequency_hz()).ok();
                        let original_mode = rt.block_on(radio.get_mode()).ok();
                        let original_power = rt
                            .block_on(radio.get_control(ControlId::RfPower))
                            .ok()
                            .flatten();
                        let original_tuner = rt.block_on(radio.get_tuner_status()).ok().flatten();
                        info!(
                            original_frequency_hz = original_frequency,
                            original_mode = ?original_mode,
                            original_power = ?original_power,
                            original_tuner = ?original_tuner,
                            "SWR sweep saved radio state"
                        );
                        if original_tuner.is_some_and(|status| status.tuning) {
                            let mut s = state.lock().expect("ui state lock poisoned");
                            s.swr_sweep_active = false;
                            s.swr_sweep_status = "Tuner is already tuning".to_string();
                            s.last_error = Some(
                                "SWR sweep cannot start while the antenna tuner is tuning"
                                    .to_string(),
                            );
                            continue;
                        }
                        if original_tuner.is_some_and(|status| status.enabled) {
                            if let Err(error) = rt.block_on(
                                radio.set_control(ControlId::Tuner, ControlValue::Bool(false)),
                            ) {
                                error!(error = %error, "SWR sweep could not disable antenna tuner");
                                let mut s = state.lock().expect("ui state lock poisoned");
                                s.swr_sweep_active = false;
                                s.swr_sweep_status = "Tuner disable failed".to_string();
                                s.last_error = Some(format!(
                                    "SWR sweep could not disable the antenna tuner: {error}"
                                ));
                                continue;
                            }
                            info!("SWR sweep disabled antenna tuner");
                        }
                        if let Err(error) = rt.block_on(radio.set_mode(Mode::Rtty)) {
                            error!(error = %error, "SWR sweep could not select RTTY carrier mode");
                            if original_tuner.is_some_and(|status| status.enabled) {
                                let _ = rt.block_on(
                                    radio.set_control(ControlId::Tuner, ControlValue::Bool(true)),
                                );
                            }
                            let mut s = state.lock().expect("ui state lock poisoned");
                            s.swr_sweep_active = false;
                            s.swr_sweep_status = "Carrier mode setup failed".to_string();
                            s.last_error =
                                Some(format!("SWR sweep could not select RTTY mode: {error}"));
                            continue;
                        }
                        // The IC-7300 manual's plot procedure calls for
                        // approximately 30 W so the transmit SWR detector is
                        // active and has enough signal to report a value.
                        const SWR_TEST_POWER_PERCENT: u8 = 30;
                        let test_power = ((u16::from(SWR_TEST_POWER_PERCENT) * 255) / 100) as u8;
                        if let Err(error) = rt.block_on(
                            radio.set_control(ControlId::RfPower, ControlValue::U8(test_power)),
                        ) {
                            error!(error = %error, "SWR sweep could not set low test power");
                            if let Some(mode) = original_mode {
                                let _ = rt.block_on(radio.set_mode(mode));
                            }
                            if original_tuner.is_some_and(|status| status.enabled) {
                                let _ = rt.block_on(
                                    radio.set_control(ControlId::Tuner, ControlValue::Bool(true)),
                                );
                            }
                            let mut s = state.lock().expect("ui state lock poisoned");
                            s.swr_sweep_active = false;
                            s.swr_sweep_status = "Low-power setup failed".to_string();
                            s.last_error =
                                Some(format!("SWR sweep could not set test power: {error}"));
                            continue;
                        }
                        info!(
                            mode = "RTTY",
                            test_power_percent = SWR_TEST_POWER_PERCENT,
                            "SWR sweep configured carrier pipeline"
                        );
                        let mut frequency = start_hz;
                        let mut points = 0_u16;
                        let mut read_failures = 0_u16;
                        let mut tx_keyed = false;
                        while frequency <= stop_hz && points < 200 && !stop.load(Ordering::Relaxed)
                        {
                            // Icom's documented plot measurement keys the transmitter only
                            // for the sample, then returns to receive before retuning.
                            if let Err(error) = rt.block_on(radio.set_frequency_hz(frequency)) {
                                error!(frequency_hz = frequency, error = %error, "SWR sweep frequency change failed");
                                break;
                            }
                            if let Err(error) = rt.block_on(radio.set_ptt(true)) {
                                error!(frequency_hz = frequency, error = %error, "SWR sweep failed to key TX");
                                read_failures += 1;
                                break;
                            }
                            tx_keyed = true;
                            {
                                let mut s = state.lock().expect("ui state lock poisoned");
                                s.ptt_on = true;
                                s.swr_sweep_status =
                                    format!("TX keyed at {frequency} Hz; measuring");
                            }
                            info!(frequency_hz = frequency, "SWR sweep TX keyed");
                            let wait_until = Instant::now()
                                + Duration::from_millis(interval_ms.clamp(100, 10_000));
                            while Instant::now() < wait_until
                                && !swr_sweep_abort.load(Ordering::Relaxed)
                            {
                                let remaining =
                                    wait_until.saturating_duration_since(Instant::now());
                                thread::sleep(remaining.min(Duration::from_millis(50)));
                            }
                            if swr_sweep_abort.load(Ordering::Relaxed) {
                                info!(frequency_hz = frequency, "SWR sweep stop requested");
                                break;
                            }
                            let sample = match rt.block_on(radio.get_meter(MeterId::Swr)) {
                                Ok(value) => value,
                                Err(error) => {
                                    read_failures += 1;
                                    warn!(frequency_hz = frequency, error = %error, "SWR sweep meter read failed");
                                    None
                                }
                            };
                            if let Err(error) = rt.block_on(radio.set_ptt(false)) {
                                error!(frequency_hz = frequency, error = %error, "SWR sweep failed to unkey TX");
                                state.lock().expect("ui state lock poisoned").last_error =
                                    Some(format!("SWR sweep could not unkey TX: {error}"));
                                break;
                            }
                            tx_keyed = false;
                            state.lock().expect("ui state lock poisoned").ptt_on = false;
                            info!(frequency_hz = frequency, "SWR sweep TX unkeyed");
                            match sample {
                                Some(value) => {
                                    info!(
                                        point = points + 1,
                                        frequency_hz = frequency,
                                        swr_level = value,
                                        "SWR sweep sample"
                                    );
                                    let mut s = state.lock().expect("ui state lock poisoned");
                                    s.swr = Some(value);
                                    s.swr_sweep_points.push((frequency, value));
                                }
                                None => {
                                    read_failures += 1;
                                    warn!(frequency_hz = frequency, "SWR sweep meter unavailable");
                                }
                            }
                            frequency = frequency.saturating_add(step_hz);
                            points += 1;
                            if frequency == u64::MAX {
                                break;
                            }
                        }
                        if tx_keyed {
                            if let Err(error) = rt.block_on(radio.set_ptt(false)) {
                                error!(error = %error, "SWR sweep failed to disarm TX");
                                state.lock().expect("ui state lock poisoned").last_error =
                                    Some(format!("SWR sweep could not disarm TX: {error}"));
                            } else {
                                info!("SWR sweep TX disarmed");
                            }
                        }
                        {
                            let mut s = state.lock().expect("ui state lock poisoned");
                            s.ptt_on = false;
                        }
                        if let Some(power) = original_power {
                            if let Err(error) =
                                rt.block_on(radio.set_control(ControlId::RfPower, power))
                            {
                                error!(error = %error, "SWR sweep failed to restore RF power");
                            } else {
                                info!("SWR sweep restored RF power");
                            }
                        }
                        if original_tuner.is_some_and(|status| status.enabled) {
                            if let Err(error) = rt.block_on(
                                radio.set_control(ControlId::Tuner, ControlValue::Bool(true)),
                            ) {
                                error!(error = %error, "SWR sweep failed to restore antenna tuner");
                            } else {
                                info!("SWR sweep restored antenna tuner");
                            }
                        }
                        if let Some(mode) = original_mode {
                            if let Err(error) = rt.block_on(radio.set_mode(mode)) {
                                error!(error = %error, "SWR sweep failed to restore operating mode");
                            } else {
                                info!(mode = ?mode, "SWR sweep restored operating mode");
                            }
                        }
                        if let Some(original_frequency) = original_frequency {
                            if let Err(error) =
                                rt.block_on(radio.set_frequency_hz(original_frequency))
                            {
                                error!(frequency_hz = original_frequency, error = %error, "SWR sweep failed to restore frequency");
                            } else {
                                info!(
                                    frequency_hz = original_frequency,
                                    "SWR sweep restored frequency"
                                );
                            }
                        }
                        let mut s = state.lock().expect("ui state lock poisoned");
                        s.swr_sweep_active = false;
                        s.swr_sweep_status = format!(
                            "{} points{}{}",
                            s.swr_sweep_points.len(),
                            if read_failures > 0 {
                                format!(" · {read_failures} read failures")
                            } else {
                                String::new()
                            },
                            if swr_sweep_abort.load(Ordering::Relaxed) {
                                " · stopped"
                            } else {
                                ""
                            }
                        );
                        info!(
                            points = s.swr_sweep_points.len(),
                            read_failures,
                            restored_frequency_hz = original_frequency,
                            "SWR sweep completed"
                        );
                        drop(s);
                        swr_sweep_abort.store(false, Ordering::Relaxed);
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
                        match rt.block_on(
                            radio.set_control(ControlId::AfGain, ControlValue::U8(target)),
                        ) {
                            Ok(()) => info!(
                                control = "AfGain",
                                value = target,
                                "Radio control change accepted"
                            ),
                            Err(err) => {
                                error!(control = "AfGain", value = target, error = %err, "Radio control change failed");
                                let mut s = state.lock().expect("ui state lock poisoned");
                                s.last_error = Some(err.to_string());
                            }
                        }
                        poll_radio_core_state(&rt, &radio, &state, true);
                    }
                }
                next_core_poll = Instant::now() + RADIO_CORE_POLL_INTERVAL;
                next_level_poll = Instant::now() + RADIO_LEVEL_POLL_INTERVAL;
                request_gui_repaint(&repaint_ctx);
            }

            let now = Instant::now();
            if now >= radio_power_settle_until {
                let mut s = state.lock().expect("ui state lock poisoned");
                s.radio_power_settling = false;
            }
            {
                let mut s = state.lock().expect("ui state lock poisoned");
                if s.radio_power_command_pending
                    && s.radio_power_wake_deadline
                        .is_some_and(|deadline| now >= deadline)
                {
                    warn!("Radio did not respond after power-on wake window");
                    s.radio_power_on = Some(false);
                    s.radio_power_command_pending = false;
                    s.radio_power_settling = false;
                    s.radio_power_wake_deadline = None;
                    s.last_error = Some("radio did not respond after power-on command".to_string());
                }
            }
            let power_off =
                state.lock().expect("ui state lock poisoned").radio_power_on == Some(false);
            let power_settling = now < radio_power_settle_until;
            if now >= next_core_poll && !power_off && !power_settling {
                let poll_levels = now >= next_level_poll;
                poll_radio_core_state(&rt, &radio, &state, poll_levels);
                next_core_poll = Instant::now() + RADIO_CORE_POLL_INTERVAL;
                if poll_levels {
                    next_level_poll = Instant::now() + RADIO_LEVEL_POLL_INTERVAL;
                }
                request_gui_repaint(&repaint_ctx);
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
    let reported_vfo = read_vfo_control(rt, radio);
    if radio.as_icom().is_none() {
        let instrument_poll = FIRST_RADIO_CORE_POLL.swap(false, Ordering::Relaxed);
        if instrument_poll {
            info!("Initial radio core frequency read started");
        }
        let frequency = rt.block_on(radio.get_frequency_hz());
        if instrument_poll {
            info!(
                ok = frequency.is_ok(),
                "Initial radio core frequency read completed"
            );
            info!("Initial radio core mode read started");
        }
        let mode = rt.block_on(radio.get_mode());
        if instrument_poll {
            info!(ok = mode.is_ok(), "Initial radio core mode read completed");
        }
        {
            let mut s = state.lock().expect("ui state lock poisoned");
            match (frequency, mode) {
                (Ok(frequency_hz), Ok(mode)) => {
                    s.frequency_hz = Some(frequency_hz);
                    if let Some(vfo) = reported_vfo {
                        s.active_vfo = vfo;
                    }
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
                    s.radio_power_on = Some(true);
                    s.radio_power_command_pending = false;
                    s.radio_power_settling = false;
                    s.radio_power_wake_deadline = None;
                    s.last_update = Some(Instant::now());
                    s.last_error = None;
                }
                (frequency, mode) => {
                    warn!(
                        initial_poll = instrument_poll,
                        frequency_error = ?frequency.as_ref().err(),
                        mode_error = ?mode.as_ref().err(),
                        "Radio core poll returned an error"
                    );
                    let was_ready = s.radio_power_on == Some(true);
                    let wake_pending = s.radio_power_command_pending
                        && s.radio_power_wake_deadline
                            .is_some_and(|deadline| Instant::now() < deadline);
                    // A single incomplete response immediately after a write is
                    // not proof that the radio powered off. Keep an already-live
                    // connection usable and let the next scheduled poll recover
                    // the displayed state.
                    if !wake_pending && !was_ready {
                        s.radio_power_on = Some(false);
                        s.radio_power_command_pending = false;
                        s.radio_power_wake_deadline = None;
                    }
                    s.last_error = Some(
                        frequency
                            .err()
                            .or_else(|| mode.err())
                            .expect("one CAT operation failed")
                            .to_string(),
                    );
                }
            }
        }
        if poll_levels {
            poll_radio_level_state(rt, radio, state);
        }
        return;
    }
    let radio = radio.as_icom().expect("checked Icom driver");
    // Read what we need under a brief lock, then do all I/O outside it.
    let spectrum_enabled = state
        .lock()
        .expect("ui state lock poisoned")
        .radio_spectrum_enabled;
    // While the scope worker is active it is the sole owner of Icom meter
    // scheduling. This branch only reads optional control state; it does not
    // perform the meter batch used by non-Icom backends.
    let status_result = if spectrum_enabled {
        rt.block_on(radio.probe_stream_status())
    } else {
        radio.probe()
    };
    if let Err(error) = &status_result {
        let mut s = state.lock().expect("ui state lock poisoned");
        let wake_pending = s.radio_power_command_pending
            && s.radio_power_wake_deadline
                .is_some_and(|deadline| Instant::now() < deadline);
        if !wake_pending {
            s.radio_power_on = Some(false);
            s.radio_power_command_pending = false;
            s.radio_power_wake_deadline = None;
            s.last_error = Some(error.to_string());
        }
        return;
    }
    // IC-7300 mode status can report a misleading data byte through the
    // generic 0x04 response. Query the model-specific DataMode control for the
    // flag used by the UI label.
    let data_mode = if poll_levels {
        radio
            .supports_control_read(ControlId::DataMode)
            .then(|| {
                rt.block_on(radio.get_control(ControlId::DataMode))
                    .ok()
                    .flatten()
            })
            .flatten()
            .and_then(|value| match value {
                ControlValue::Bool(enabled) => Some(enabled),
                _ => None,
            })
    } else {
        None
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
    let (
        squelch,
        preamp,
        attenuator,
        noise_blank,
        noise_reduction,
        noise_reduction_level,
        ip_plus,
        notch_auto,
        notch_manual,
        agc,
        tuner_status,
    ) = if poll_levels {
        (
            read_u8_control(rt, radio, ControlId::Squelch),
            read_u8_control(rt, radio, ControlId::Preamp),
            read_u8_control(rt, radio, ControlId::Attenuator),
            read_bool_control(rt, radio, ControlId::NoiseBlanker),
            read_bool_control(rt, radio, ControlId::NoiseReduction),
            read_u8_control(rt, radio, ControlId::NoiseReductionLevel),
            read_bool_control(rt, radio, ControlId::IpPlus),
            read_bool_control(rt, radio, ControlId::Notch),
            read_bool_control(rt, radio, ControlId::ManualNotch),
            read_u8_control(rt, radio, ControlId::Agc),
            radio
                .supports_control_read(ControlId::Tuner)
                .then(|| rt.block_on(radio.get_tuner_status()).ok().flatten())
                .flatten(),
        )
    } else {
        (
            None, None, None, None, None, None, None, None, None, None, None,
        )
    };
    // The IC-7300's regular mode response does not consistently include the
    // active FIL number, so filter state needs its own fast query.
    let filt = poll_levels
        .then(|| read_u8_control(rt, radio, ControlId::Filter))
        .flatten();
    let mut s = state.lock().expect("ui state lock poisoned");
    if let Ok(status) = status_result {
        s.radio_power_on = Some(true);
        s.radio_power_command_pending = false;
        s.radio_power_settling = false;
        s.radio_power_wake_deadline = None;
        if let Some(freq) = status.frequency_hz {
            s.frequency_hz = Some(freq);
        }
        if let Some(vfo) = reported_vfo {
            s.active_vfo = vfo;
        }
        if let Some(mode) = status.mode {
            s.mode = mode;
        }
        if let Some(details) = status.mode_details {
            apply_icom_mode_details(&mut s, details, data_mode);
        }
        if let Some(data_mode) = data_mode {
            s.data_mode = Some(data_mode);
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
    if let Some(v) = squelch {
        s.squelch = Some(v);
    }
    if let Some(v) = preamp {
        s.preamp = Some(v);
    }
    if let Some(v) = attenuator {
        s.attenuator = Some(v);
    }
    if let Some(v) = noise_blank {
        s.noise_blank = Some(v);
    }
    if let Some(v) = noise_reduction {
        s.noise_reduction = Some(v);
    }
    if let Some(v) = noise_reduction_level {
        s.noise_reduction_level = Some(v);
    }
    if let Some(v) = ip_plus {
        s.ip_plus = Some(v);
    }
    if let Some(v) = notch_auto {
        s.notch_auto = Some(v);
    }
    if let Some(v) = notch_manual {
        s.notch_manual = Some(v);
    }
    if let Some(v) = agc {
        s.agc = Some(v);
    }
    if tuner_status.is_some() {
        s.tuner_status = tuner_status;
    }
    if let Some(v) = filt {
        s.filter = Some(v);
    }
}

fn poll_scheduled_icom_meter(
    rt: &tokio::runtime::Runtime,
    radio: &IcomCiVRadio,
    state: &Arc<Mutex<GuiState>>,
    scheduled: ScheduledMeter,
) {
    let id = scheduled.meter_id();
    if !radio.supports_meter(id) {
        return;
    }
    let started = Instant::now();
    let value = rt
        .block_on(radio.get_meter_with_timeout(id, RADIO_METER_RESPONSE_TIMEOUT))
        .ok()
        .flatten();
    let elapsed = started.elapsed();
    if elapsed >= Duration::from_millis(100) {
        debug!(meter = ?id, elapsed_ms = elapsed.as_millis(), "Slow scheduled CI-V meter read");
    }
    if let Some(value) = value {
        let mut s = state.lock().expect("ui state lock poisoned");
        match id {
            MeterId::Signal => s.signal_meter = Some(value),
            MeterId::Power => s.power_meter = Some(value),
            MeterId::Alc => s.alc_meter = Some(value),
            MeterId::Swr => s.swr = Some(value),
            MeterId::Compression => s.compression_meter = Some(value),
            MeterId::Current => s.current_meter = Some(value),
            MeterId::Voltage => s.voltage_meter = Some(value),
            MeterId::Temperature => s.temperature_meter = Some(value),
        }
    }
}

fn control_vfo_value(value: &ControlValue) -> Option<u8> {
    match value {
        ControlValue::U8(value) | ControlValue::Vfo(value) if *value <= 1 => Some(*value),
        _ => None,
    }
}

fn icom_base_mode_label(mode: BaseMode) -> &'static str {
    match mode {
        BaseMode::Lsb => "LSB",
        BaseMode::Usb => "USB",
        BaseMode::Am => "AM",
        BaseMode::Cw => "CW",
        BaseMode::Rtty => "RTTY",
        BaseMode::Fm => "FM",
        BaseMode::Wfm => "WFM",
        BaseMode::CwR => "CW-R",
        BaseMode::RttyR => "RTTY-R",
        BaseMode::Unknown(_) => "UNKNOWN",
    }
}

fn scope_config_changed(
    previous: Option<RadioScopeStreamConfig>,
    current: RadioScopeStreamConfig,
) -> bool {
    previous.is_some_and(|previous| previous != current)
}

fn apply_icom_mode_details(
    state: &mut GuiState,
    details: OperatingMode,
    data_mode_override: Option<bool>,
) {
    // Keep the base mode independent from the data flag. On the IC-7300 the
    // ordinary 0x04 response can carry a stale or misleading data byte;
    // DataMode is read back separately when the profile supports it.
    state.mode = icom_base_mode_label(details.base).to_string();
    state.data_mode = Some(data_mode_override.unwrap_or(details.data_mode));
    // Some Icom responses omit FIL even though the mode is valid. Do not
    // erase a known filter while waiting for the dedicated read.
    if let Some(filter) = details.filter {
        state.filter = Some(filter);
    }
}

fn read_vfo_control<R: Radio + ?Sized>(rt: &tokio::runtime::Runtime, radio: &R) -> Option<u8> {
    if !radio.supports_control_read(ControlId::Vfo) {
        return None;
    }
    control_vfo_value(
        &rt.block_on(radio.get_control(ControlId::Vfo))
            .ok()
            .flatten()?,
    )
}

fn poll_radio_level_state(
    rt: &tokio::runtime::Runtime,
    radio: &ConfiguredRadio,
    state: &Arc<Mutex<GuiState>>,
) {
    let read_meter = |id| {
        radio
            .supports_meter(id)
            .then(|| rt.block_on(radio.get_meter(id)).ok().flatten())
            .flatten()
    };
    let values = [
        (
            ControlId::AfGain,
            read_u8_control(rt, radio, ControlId::AfGain),
        ),
        (
            ControlId::RfGain,
            read_u8_control(rt, radio, ControlId::RfGain),
        ),
        (
            ControlId::Squelch,
            read_u8_control(rt, radio, ControlId::Squelch),
        ),
        (
            ControlId::RfPower,
            read_u8_control(rt, radio, ControlId::RfPower),
        ),
        (
            ControlId::Preamp,
            read_u8_control(rt, radio, ControlId::Preamp),
        ),
        (
            ControlId::Attenuator,
            read_u8_control(rt, radio, ControlId::Attenuator),
        ),
        (ControlId::Agc, read_u8_control(rt, radio, ControlId::Agc)),
        (
            ControlId::NoiseReductionLevel,
            read_u8_control(rt, radio, ControlId::NoiseReductionLevel),
        ),
    ];
    let noise_blank = read_bool_control(rt, radio, ControlId::NoiseBlanker);
    let noise_reduction = read_bool_control(rt, radio, ControlId::NoiseReduction);
    let ip_plus = read_bool_control(rt, radio, ControlId::IpPlus);
    let notch_auto = read_bool_control(rt, radio, ControlId::Notch);
    let notch_manual = read_bool_control(rt, radio, ControlId::ManualNotch);
    let tuner_status = radio
        .supports_control_read(ControlId::Tuner)
        .then(|| rt.block_on(radio.get_tuner_status()).ok().flatten())
        .flatten();
    let meters = [
        (MeterId::Signal, read_meter(MeterId::Signal)),
        (MeterId::Power, read_meter(MeterId::Power)),
        (MeterId::Swr, read_meter(MeterId::Swr)),
        (MeterId::Alc, read_meter(MeterId::Alc)),
        (MeterId::Compression, read_meter(MeterId::Compression)),
        (MeterId::Current, read_meter(MeterId::Current)),
        (MeterId::Voltage, read_meter(MeterId::Voltage)),
        (MeterId::Temperature, read_meter(MeterId::Temperature)),
    ];
    let mut s = state.lock().expect("ui state lock poisoned");
    for (id, value) in values {
        if let Some(value) = value {
            match id {
                ControlId::AfGain => s.af_gain = Some(value),
                ControlId::RfGain => s.rf_gain = Some(value),
                ControlId::Squelch => s.squelch = Some(value),
                ControlId::RfPower => s.rf_power = Some(value),
                ControlId::Preamp => s.preamp = Some(value),
                ControlId::Attenuator => s.attenuator = Some(value),
                ControlId::Agc => s.agc = Some(value),
                ControlId::NoiseReductionLevel => s.noise_reduction_level = Some(value),
                _ => {}
            }
        }
    }
    for (id, value) in meters {
        if let Some(value) = value {
            match id {
                MeterId::Signal => s.signal_meter = Some(value),
                MeterId::Power => s.power_meter = Some(value),
                MeterId::Swr => s.swr = Some(value),
                MeterId::Alc => s.alc_meter = Some(value),
                MeterId::Compression => s.compression_meter = Some(value),
                MeterId::Current => s.current_meter = Some(value),
                MeterId::Voltage => s.voltage_meter = Some(value),
                MeterId::Temperature => s.temperature_meter = Some(value),
            }
        }
    }
    if let Some(value) = noise_blank {
        s.noise_blank = Some(value);
    }
    if let Some(value) = noise_reduction {
        s.noise_reduction = Some(value);
    }
    if let Some(value) = ip_plus {
        s.ip_plus = Some(value);
    }
    if let Some(value) = notch_auto {
        s.notch_auto = Some(value);
    }
    if let Some(value) = notch_manual {
        s.notch_manual = Some(value);
    }
    if tuner_status.is_some() {
        s.tuner_status = tuner_status;
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
    // Geometry and mode changes can restore the radio's stored sweep setting,
    // so make the requested speed the final configuration command.
    rt.block_on(radio.set_scope_sweep_speed(config.sweep_code))?;
    Ok(())
}

fn scope_vbw_wide_for_view(_view: RadioScopeView, user_vbw_wide: bool) -> bool {
    user_vbw_wide
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meter_scheduler_prioritizes_signal_and_one_tx_meter() {
        let now = Instant::now();
        let mut scheduler = MeterPollScheduler::new();
        scheduler.next_signal = now;
        scheduler.next_tx = now;
        scheduler.next_aux = now;

        assert_eq!(scheduler.next_due(now, false), Some(ScheduledMeter::Signal));
        assert_eq!(scheduler.next_due(now, true), Some(ScheduledMeter::Power));
        assert_eq!(scheduler.next_due(now, true), Some(ScheduledMeter::Current));
    }

    #[test]
    fn meter_scheduler_rotates_auxiliary_reads_without_batching() {
        let now = Instant::now();
        let mut scheduler = MeterPollScheduler::new();
        scheduler.next_signal = now + Duration::from_secs(10);
        scheduler.next_tx = now + Duration::from_secs(10);
        scheduler.next_aux = now;

        assert_eq!(
            scheduler.next_due(now, false),
            Some(ScheduledMeter::Current)
        );
        scheduler.next_aux = now;
        assert_eq!(
            scheduler.next_due(now, false),
            Some(ScheduledMeter::Voltage)
        );
    }

    #[test]
    fn scope_vbw_respects_checkbox_in_narrow_and_overview_views() {
        for view in [RadioScopeView::Narrow, RadioScopeView::Overview] {
            assert!(!scope_vbw_wide_for_view(view, false));
            assert!(scope_vbw_wide_for_view(view, true));
        }
    }

    #[test]
    fn first_scope_observation_is_not_a_radio_configuration_change() {
        let config = RadioScopeStreamConfig {
            view: RadioScopeView::Narrow,
            span_code: 3,
            vbw_wide: false,
            edges: None,
            sweep_code: 1,
            hold: false,
            reference_tenths_db: 0,
        };

        assert!(!scope_config_changed(None, config));
        assert!(!scope_config_changed(Some(config), config));
        assert!(scope_config_changed(
            Some(config),
            RadioScopeStreamConfig {
                sweep_code: 2,
                ..config
            }
        ));
    }

    #[test]
    fn icom_mode_readback_preserves_filter_when_mode_frame_omits_it() {
        let mut state = GuiState {
            filter: Some(3),
            ..GuiState::default()
        };
        apply_icom_mode_details(
            &mut state,
            OperatingMode {
                base: BaseMode::Usb,
                data_mode: false,
                filter: None,
            },
            Some(false),
        );

        assert_eq!(state.mode, "USB");
        assert_eq!(state.data_mode, Some(false));
        assert_eq!(state.filter, Some(3));
    }

    #[test]
    fn icom_mode_readback_uses_dedicated_data_state_over_stale_mode_flag() {
        let mut state = GuiState::default();
        apply_icom_mode_details(
            &mut state,
            OperatingMode {
                base: BaseMode::Usb,
                data_mode: true,
                filter: Some(2),
            },
            Some(false),
        );

        assert_eq!(state.mode, "USB");
        assert_eq!(state.data_mode, Some(false));
        assert_eq!(state.filter, Some(2));
    }
}

fn read_u8_control<R: Radio + ?Sized>(
    rt: &tokio::runtime::Runtime,
    radio: &R,
    id: ControlId,
) -> Option<u8> {
    if !radio.supports_control_read(id) {
        return None;
    }
    match rt.block_on(radio.get_control(id)).ok().flatten() {
        Some(ControlValue::U8(v)) => Some(v),
        _ => None,
    }
}

fn read_bool_control<R: Radio + ?Sized>(
    rt: &tokio::runtime::Runtime,
    radio: &R,
    id: ControlId,
) -> Option<bool> {
    if !radio.supports_control_read(id) {
        return None;
    }
    match rt.block_on(radio.get_control(id)).ok().flatten() {
        Some(ControlValue::Bool(v)) => Some(v),
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
