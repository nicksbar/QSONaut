use super::super::*;
use super::request_gui_repaint;
use std::sync::atomic::AtomicUsize;

const RADIO_CORE_POLL_INTERVAL: Duration = Duration::from_millis(100);
const RADIO_LEVEL_POLL_INTERVAL: Duration = Duration::from_millis(500);
const RADIO_COMMAND_WAKE_INTERVAL: Duration = Duration::from_millis(50);
const RADIO_SCOPE_READ_SLICE: Duration = Duration::from_millis(25);
const RADIO_SCOPE_STALL_TIMEOUT: Duration = Duration::from_secs(2);
const RADIO_SIGNAL_METER_INTERVAL: Duration = Duration::from_millis(400);
const RADIO_TX_METER_INTERVAL: Duration = Duration::from_millis(300);
const RADIO_AUX_METER_INTERVAL: Duration = Duration::from_millis(1_500);
const RADIO_METER_RESPONSE_TIMEOUT: Duration = Duration::from_millis(150);

// The first radio core poll is instrumented at INFO level so a hardware startup
// stall is visible in the normal application log without logging every 100 ms
// poll forever.
static FIRST_RADIO_CORE_POLL: AtomicBool = AtomicBool::new(true);
static REMOTE_LEVEL_POLL_INDEX: AtomicUsize = AtomicUsize::new(0);
static REMOTE_LEVEL_CONTROL_INDEX: AtomicUsize = AtomicUsize::new(0);
static REMOTE_CORE_POLL_INDEX: AtomicUsize = AtomicUsize::new(0);

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
            let meter = match self.tx_index % 6 {
                0 => ScheduledMeter::Power,
                1 => ScheduledMeter::Swr,
                2 => ScheduledMeter::Power,
                3 => ScheduledMeter::Alc,
                4 => ScheduledMeter::Power,
                _ => ScheduledMeter::Compression,
            };
            self.tx_index = self.tx_index.wrapping_add(1);
            return Some(meter);
        }
        if now >= self.next_aux {
            // Signal is an RX reading. Keep the last useful RX value visible
            // during TX instead of replacing it with a radio's often-zero
            // transmit-side response.
            const AUXILIARY: [ScheduledMeter; 3] = [
                ScheduledMeter::Current,
                ScheduledMeter::Voltage,
                ScheduledMeter::Temperature,
            ];
            let meter = AUXILIARY[self.aux_index % AUXILIARY.len()];
            self.aux_index = self.aux_index.wrapping_add(1);
            self.next_aux = now + RADIO_AUX_METER_INTERVAL;
            return Some(meter);
        }
        None
    }
}

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

fn wire_scope_config(
    config: &RadioScopeStreamConfig,
) -> qsonaut_hostbridge_protocol::ScopeConfiguration {
    let (center_mode, span_hz, fixed_edges_hz, fixed_edge_number) = match config.view {
        RadioScopeView::Narrow => match config.edges {
            Some(edges) => (Some(true), None, Some(edges), Some(4)),
            None => (
                Some(false),
                Some(scope_span_hz(config.span_code)),
                None,
                None,
            ),
        },
        RadioScopeView::Overview => (Some(true), None, config.edges, Some(1)),
    };
    qsonaut_hostbridge_protocol::ScopeConfiguration {
        span_hz,
        fixed_edges_hz,
        fixed_edge_number,
        hold: Some(config.hold),
        reference_level_tenths_db: Some(config.reference_tenths_db),
        sweep_speed: Some(config.sweep_code),
        center_mode,
        vbw_wide: Some(scope_vbw_wide_for_view(config.view, config.vbw_wide)),
    }
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
    radio: impl Into<RadioHandle>,
    state: Arc<Mutex<GuiState>>,
    stop: Arc<AtomicBool>,
    swr_sweep_abort: Arc<AtomicBool>,
    display_tuning: Arc<Mutex<DisplayTuning>>,
    rx: mpsc::Receiver<GuiCommand>,
    repaint_ctx: Arc<OnceLock<egui::Context>>,
    ptt_allowed: Arc<AtomicBool>,
) -> std::thread::JoinHandle<()> {
    let radio = radio.into();
    thread::spawn(move || {
        {
            let mut s = state.lock().expect("ui state lock poisoned");
            if let RadioHandle::Remote(remote) = &radio {
                s.radio_waterfall_status = format!(
                    "CONNECTED · HostBridge {} · {}",
                    remote.host_name(),
                    remote.device_id()
                );
                info!(
                    host = remote.host_name(),
                    radio = remote.device_id(),
                    audio_sources = remote.capabilities_advertised().audio_sources.len(),
                    audio_outputs = remote.capabilities_advertised().audio_outputs.len(),
                    "HostBridge radio selected from negotiated catalog"
                );
            }
            s.radio_power_supported = radio.capabilities().can_set_power;
            s.supported_controls = radio.supported_controls().into_iter().collect();
            s.supported_meters = if matches!(radio, RadioHandle::Local(ConfiguredRadio::Null(_))) {
                // NullRadio intentionally has no physical meter source. Keep
                // the UI deterministic for offline decoding by exposing
                // zero-valued synthetic meters rather than leaving stale or
                // blank readings from another tab.
                MeterId::ALL.iter().copied().collect()
            } else {
                radio.supported_meters().into_iter().collect()
            };
            if matches!(radio, RadioHandle::Local(ConfiguredRadio::Null(_))) {
                s.signal_meter = Some(0);
                s.power_meter = Some(0);
                s.swr = Some(0);
                s.alc_meter = Some(0);
                s.compression_meter = Some(0);
                s.current_meter = Some(0);
                s.voltage_meter = Some(0);
                s.temperature_meter = Some(0);
            }
            info!(
                supported_meters = ?s.supported_meters,
                temperature_supported = radio.supports_meter(MeterId::Temperature),
                "Radio meter capabilities initialized"
            );
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
        let remote_scope_queue = radio.remote_scope_queue();
        let remote_scope_client = radio.remote_scope_client();
        let remote_scope_supported = radio.remote_scope_supported();
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
                if let (Some(queue), Some(client), true) = (
                    remote_scope_queue,
                    remote_scope_client,
                    remote_scope_supported,
                ) {
                    let mut last_remote_row = Instant::now() - Duration::from_millis(66);
                    let mut last_remote_start = Instant::now() - Duration::from_secs(2);
                    let mut remote_scope_started = false;
                    let mut last_remote_config: Option<RadioScopeStreamConfig> = None;
                    while !stream_stop.load(Ordering::Relaxed) {
                        let (
                            desired,
                            power_on,
                            power_settling,
                            power_command_pending,
                            radio_error,
                            scope_config,
                            settings_dirty,
                        ) = {
                            // Snapshot GUI state before touching DisplayTuning. The audio worker
                            // uses the opposite resources in its hot path (DisplayTuning, then
                            // GuiState); acquiring them in one expression here creates a classic
                            // cross-worker deadlock when both workers wake together.
                            let (
                                mode,
                                desired,
                                power_on,
                                power_settling,
                                power_command_pending,
                                radio_error,
                                view,
                                span_code,
                                vbw_wide,
                                hold,
                                reference_tenths_db,
                                settings_dirty,
                            ) = {
                                let s = stream_state.lock().expect("ui state lock poisoned");
                                (
                                    s.mode.clone(),
                                    s.radio_spectrum_desired,
                                    s.radio_power_on,
                                    s.radio_power_settling,
                                    s.radio_power_command_pending,
                                    s.last_error.is_some(),
                                    s.radio_scope_view,
                                    s.radio_scope_span_code,
                                    s.radio_scope_vbw_wide,
                                    s.radio_scope_hold,
                                    s.radio_scope_reference_tenths_db,
                                    s.radio_scope_settings_dirty,
                                )
                            };
                            let sweep_code = {
                                let tuning =
                                    stream_display_tuning.lock().expect("tuning lock poisoned");
                                effective_visual_profile(&tuning, &mode, true).1
                            };
                            let config = RadioScopeStreamConfig {
                                view,
                                span_code,
                                vbw_wide,
                                // The remote IC-7300 USB scope path is
                                // centered by span. Do not replay QSONaut's
                                // local mode-aware fixed-edge projection;
                                // that emits 27 1E and can move the radio's
                                // scope away from the live dial frequency.
                                edges: None,
                                sweep_code,
                                hold,
                                reference_tenths_db,
                            };
                            (
                                desired,
                                power_on,
                                power_settling,
                                power_command_pending,
                                radio_error,
                                config,
                                settings_dirty,
                            )
                        };
                        if !desired {
                            if remote_scope_started {
                                let _ = client.stop_scope(None);
                                remote_scope_started = false;
                            }
                            if let Ok(mut s) = stream_state.lock() {
                                s.radio_spectrum_enabled = false;
                                s.radio_waterfall_status = "OFF".to_string();
                            }
                            last_remote_config = None;
                            thread::sleep(Duration::from_millis(100));
                            continue;
                        }
                        if power_on != Some(true) || power_settling || power_command_pending {
                            if let Ok(mut s) = stream_state.lock() {
                                s.radio_spectrum_enabled = false;
                                s.radio_waterfall_status = if power_on == Some(false) {
                                    "OFF (radio off)".to_string()
                                } else if radio_error {
                                    "CONFIG ERROR".to_string()
                                } else {
                                    "WAITING FOR RADIO".to_string()
                                };
                            }
                            thread::sleep(Duration::from_millis(250));
                            continue;
                        }
                        // Scope startup must not synchronously replay every
                        // CI-V setting. The host radio already has a valid
                        // native scope configuration; apply changes when the
                        // client explicitly dirties the settings. This keeps
                        // startup audio/control traffic responsive and avoids
                        // moving the scope away from the live dial.
                        if settings_dirty {
                            if let Err(error) =
                                client.configure_scope(None, wire_scope_config(&scope_config))
                            {
                                warn!(%error, "failed to send remote scope configuration");
                            } else {
                                last_remote_config = Some(scope_config);
                                if let Ok(mut s) = stream_state.lock() {
                                    s.radio_scope_settings_dirty = false;
                                }
                            }
                        } else if last_remote_config.is_none() {
                            last_remote_config = Some(scope_config);
                        }
                        if remote_scope_started
                            && last_remote_start.elapsed() >= Duration::from_secs(2)
                            && queue.lock().map(|rows| rows.is_empty()).unwrap_or(true)
                        {
                            remote_scope_started = false;
                        }
                        if !remote_scope_started
                            && last_remote_start.elapsed() >= Duration::from_secs(1)
                        {
                            if let Err(error) = client.start_scope(None) {
                                warn!(%error, "failed to start remote scope");
                            } else {
                                remote_scope_started = true;
                                last_remote_start = Instant::now();
                                if let Ok(mut s) = stream_state.lock() {
                                    s.radio_spectrum_enabled = true;
                                    s.radio_waterfall_status =
                                        "STARTING · remote scope".to_string();
                                }
                            }
                        }
                        if desired && last_remote_row.elapsed() >= Duration::from_millis(66) {
                            let bins = queue.lock().ok().and_then(|mut rows| rows.pop_back());
                            if let Some(bins) = bins {
                                let mut s = stream_state.lock().expect("ui state lock poisoned");
                                s.radio_spectrum_enabled = true;
                                apply_waterfall_bins(&mut s, &bins);
                                s.radio_waterfall_status = format!("REMOTE · {} bins", bins.len());
                                last_remote_row = Instant::now();
                                request_gui_repaint(&stream_repaint);
                            }
                        }
                        thread::sleep(Duration::from_millis(10));
                    }
                    if remote_scope_started {
                        let _ = client.stop_scope(None);
                    }
                    return;
                }
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
                    radio_error,
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
                        s.last_error.is_some(),
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
                    } else if radio_error {
                        "CONFIG ERROR".to_string()
                    } else {
                        "WAITING FOR RADIO".to_string()
                    };
                    drop(s);
                    thread::sleep(Duration::from_millis(250));
                    continue;
                }

                if spectrum_desired {
                    let scope_edges = scope_edges_for_view(
                        scope_view,
                        workspace_mode,
                        frequency_hz,
                        span_code,
                        &mode,
                    );
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
                        let (geometry_changed, hold_update, reference_update) =
                            scope_change_updates(last_scope_config, config);
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
                                drop(s);
                                warn!(error = %err, "Radio spectrum stream configuration failed; core radio remains available");
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

                // A scope connection can remain open while the radio stops
                // producing complete sweeps. Let Rigwright distinguish that
                // stalled-stream case from a stream that has never started,
                // then cycle the stream so the next loop re-arms it.
                let scope_health = stream_radio.scope_stream_health();
                if scope_health.is_stalled(RADIO_SCOPE_STALL_TIMEOUT) {
                    warn!(
                        completed_sweeps = scope_health.completed_sweeps,
                        last_sweep_age = ?scope_health.last_sweep_age,
                        "Radio scope stream stalled; re-arming"
                    );
                    let mut s = stream_state.lock().expect("ui state lock poisoned");
                    s.radio_spectrum_enabled = false;
                    s.radio_waterfall_status = "STALLED · retrying".to_string();
                    latest_scope_bins = None;
                    continue;
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
        // schedule. Do not perform the full Icom level/control sweep here:
        // that is many serial transactions and can leave the newly-started
        // worker looking offline while the radio is still settling. The
        // normal level poll will populate controls and meters shortly after
        // the lightweight core probe completes.
        poll_radio_core_state(&rt, &radio, &state, false);
        info!("Initial radio poll completed");
        let mut next_core_poll = Instant::now() + RADIO_CORE_POLL_INTERVAL;
        let mut next_level_poll = Instant::now() + RADIO_LEVEL_POLL_INTERVAL;
        let mut radio_power_settle_until = Instant::now();
        let mut hostbridge_was_connected = true;

        while !stop.load(Ordering::Relaxed) {
            if let Some(hostbridge_connected) = radio.pump_events() {
                if !hostbridge_connected {
                    ptt_allowed.store(false, Ordering::Release);
                    if hostbridge_was_connected {
                        let mut s = state.lock().expect("ui state lock poisoned");
                        s.ptt_on = false;
                        s.radio_waterfall_status =
                            "DISCONNECTED · HostBridge (TX disarmed)".to_string();
                        s.last_error = Some("HostBridge disconnected; TX disarmed".to_string());
                        drop(s);
                        request_gui_repaint(&repaint_ctx);
                    }
                } else if !hostbridge_was_connected {
                    let mut s = state.lock().expect("ui state lock poisoned");
                    s.radio_waterfall_status =
                        "CONNECTED · HostBridge (TX remains disarmed)".to_string();
                    drop(s);
                    request_gui_repaint(&repaint_ctx);
                }
                hostbridge_was_connected = hostbridge_connected;
            }
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
                                    state.lock().expect("ui state lock poisoned").frequency_hz =
                                        Some(target);
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
                                state.lock().expect("ui state lock poisoned").frequency_hz =
                                    Some(target);
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
                                accept_power_command(&mut s, target, Instant::now());
                            }
                            Err(error) => {
                                error!(power_on = target, error = %error, "Radio power command failed");
                                let mut s = state.lock().expect("ui state lock poisoned");
                                reject_power_command(&mut s, error.to_string());
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
                        let data_mode_result = if radio.supports_control_write(ControlId::DataMode)
                        {
                            rt.block_on(radio.set_control(
                                ControlId::DataMode,
                                ControlValue::Bool(preset.data_mode),
                            ))
                        } else {
                            Ok(())
                        };
                        let filter_result = if radio.supports_control_write(ControlId::Filter) {
                            rt.block_on(
                                radio.set_control(ControlId::Filter, ControlValue::U8(filter)),
                            )
                        } else {
                            Ok(())
                        };
                        if let Err(error) = frequency_result
                            .and(mode_result)
                            .and(data_mode_result)
                            .and(filter_result)
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
                            let mut s = state.lock().expect("ui state lock poisoned");
                            s.mode = icom_base_mode_label(preset.base_mode).to_string();
                            s.data_mode = Some(preset.data_mode);
                            s.frequency_hz = Some(frequency_hz);
                            s.radio_power_on = Some(true);
                            s.last_error = None;
                            info!(workspace = %workspace_mode.label(), frequency_hz, mode = ?preset.base_mode, data_mode = preset.data_mode, filter, "Radio workspace preset accepted");
                        }
                        poll_radio_core_state(&rt, &radio, &state, true);
                    }
                    GuiCommand::SetFilter(n) => {
                        let workspace_mode =
                            state.lock().expect("ui state lock poisoned").workspace_mode;
                        let target_filter = n.clamp(1, 3);
                        info!(filter = target_filter, workspace = %workspace_mode.label(), "Radio filter change requested");
                        let result =
                            if radio.supports_control_write(ControlId::Filter) {
                                rt.block_on(radio.set_control(
                                    ControlId::Filter,
                                    ControlValue::U8(target_filter),
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
                                project_control_write(
                                    &mut state.lock().expect("ui state lock poisoned"),
                                    id,
                                    &value,
                                );
                                if id == ControlId::RfPower {
                                    if let ControlValue::U8(level) = &value {
                                        let mut s = state.lock().expect("ui state lock poisoned");
                                        if s.rf_power_write_pending == Some(*level) {
                                            s.rf_power = Some(*level);
                                            s.rf_power_write_pending = None;
                                        }
                                    }
                                }
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
                                let mut s = state.lock().expect("ui state lock poisoned");
                                if id == ControlId::RfPower {
                                    if let ControlValue::U8(level) = &value {
                                        if s.rf_power_write_pending == Some(*level) {
                                            s.rf_power_write_pending = None;
                                        }
                                    }
                                }
                                s.last_error = Some(error.to_string());
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
                        let Some(sweep_setup) = radio.swr_sweep_setup() else {
                            let mut s = state.lock().expect("ui state lock poisoned");
                            s.swr_sweep_active = false;
                            s.swr_sweep_status = "Unsupported by radio driver".to_string();
                            s.last_error = Some(
                                "SWR sweep setup is not documented by this radio driver"
                                    .to_string(),
                            );
                            continue;
                        };
                        if let Err(error) = rt.block_on(radio.set_mode(sweep_setup.carrier_mode)) {
                            error!(error = %error, mode = ?sweep_setup.carrier_mode, "SWR sweep could not select carrier mode");
                            if original_tuner.is_some_and(|status| status.enabled) {
                                let _ = rt.block_on(
                                    radio.set_control(ControlId::Tuner, ControlValue::Bool(true)),
                                );
                            }
                            let mut s = state.lock().expect("ui state lock poisoned");
                            s.swr_sweep_active = false;
                            s.swr_sweep_status = "Carrier mode setup failed".to_string();
                            s.last_error =
                                Some(format!("SWR sweep could not select carrier mode: {error}"));
                            continue;
                        }
                        if let Err(error) = rt.block_on(radio.set_control(
                            ControlId::RfPower,
                            ControlValue::U8(sweep_setup.rf_power),
                        )) {
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
                            mode = ?sweep_setup.carrier_mode,
                            test_power = sweep_setup.rf_power,
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
                if expire_power_on_wake(&mut s, now) {
                    warn!("Radio did not respond after power-on wake window");
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
    radio: &RadioHandle,
    state: &Arc<Mutex<GuiState>>,
    poll_levels: bool,
) {
    let link_health = radio.link_health();
    if link_health.is_degraded() {
        warn!(
            consecutive_timeouts = ?link_health.consecutive_timeouts,
            response_timeouts = ?link_health.response_timeouts,
            "Radio link health degraded"
        );
        let mut s = state.lock().expect("ui state lock poisoned");
        s.radio_waterfall_status = format!(
            "LINK DEGRADED · {} consecutive timeouts",
            link_health.consecutive_timeouts.unwrap_or(0)
        );
    }
    // VFO is already part of the pushed radio state for remote sessions.
    // A request here every 100 ms serializes behind CI-V scope traffic and
    // can starve the HostBridge media event pump.
    let reported_vfo = (!radio.is_remote())
        .then(|| read_vfo_control(rt, radio))
        .flatten();
    if radio.as_icom().is_none() {
        // Refresh remote core state at a bounded 500 ms cadence. The remote
        // scheduler permits only one outstanding read, so sending GetState on
        // a level tick would starve every meter and control read.
        let remote_core_tick = REMOTE_CORE_POLL_INDEX.fetch_add(1, Ordering::Relaxed);
        if !poll_levels && radio.is_remote() && remote_core_tick.is_multiple_of(5) {
            let _ = radio.refresh_state();
        }
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
                        Mode::Data => "USB",
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
            if radio.is_remote() {
                // Reserve this tick for one level read. The remote scheduler
                // permits only one outstanding read, so refreshing GetState
                // immediately before this call would starve every meter and
                // control read indefinitely.
                poll_remote_level_state(rt, radio, state);
            } else {
                poll_radio_level_state(rt, radio, state);
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
        if s.rf_power_write_pending.is_none() {
            s.rf_power = Some(v);
        }
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
    let value = match rt.block_on(radio.get_meter_with_timeout(id, RADIO_METER_RESPONSE_TIMEOUT)) {
        Ok(value) => value,
        Err(error) => {
            debug!(meter = ?id, error = %error, "Scheduled CI-V meter read failed");
            None
        }
    };
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
            MeterId::Voltage => {
                s.voltage_meter = Some(value);
                record_voltage_sample(&mut s.voltage_history, value);
            }
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

fn project_control_write(state: &mut GuiState, id: ControlId, value: &ControlValue) {
    match (id, value) {
        (ControlId::AfGain, ControlValue::U8(value)) => state.af_gain = Some(*value),
        (ControlId::RfGain, ControlValue::U8(value)) => state.rf_gain = Some(*value),
        (ControlId::Squelch, ControlValue::U8(value)) => state.squelch = Some(*value),
        (ControlId::RfPower, ControlValue::U8(value)) => state.rf_power = Some(*value),
        (ControlId::Preamp, ControlValue::U8(value)) => state.preamp = Some(*value),
        (ControlId::Attenuator, ControlValue::U8(value)) => state.attenuator = Some(*value),
        (ControlId::Agc, ControlValue::U8(value)) => state.agc = Some(*value),
        (ControlId::NoiseReductionLevel, ControlValue::U8(value)) => {
            state.noise_reduction_level = Some(*value)
        }
        (ControlId::Filter, ControlValue::U8(value)) => state.filter = Some(*value),
        (ControlId::NoiseBlanker, ControlValue::Bool(value)) => state.noise_blank = Some(*value),
        (ControlId::NoiseReduction, ControlValue::Bool(value)) => {
            state.noise_reduction = Some(*value)
        }
        (ControlId::IpPlus, ControlValue::Bool(value)) => state.ip_plus = Some(*value),
        (ControlId::Notch, ControlValue::Bool(value)) => state.notch_auto = Some(*value),
        (ControlId::ManualNotch, ControlValue::Bool(value)) => state.notch_manual = Some(*value),
        (ControlId::Tuner, ControlValue::Bool(value)) => {
            state.tuner_status = Some(qsonaut_radio::TunerStatus {
                enabled: *value,
                tuning: false,
            });
        }
        (ControlId::Vfo, value) => {
            if let Some(vfo) = control_vfo_value(value) {
                state.active_vfo = vfo;
            }
        }
        _ => {}
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

fn scope_change_updates(
    previous: Option<RadioScopeStreamConfig>,
    current: RadioScopeStreamConfig,
) -> (bool, Option<bool>, Option<i16>) {
    let geometry_changed = previous.is_none_or(|previous| {
        previous.view != current.view
            || previous.span_code != current.span_code
            || previous.edges != current.edges
    });
    let hold_update = previous
        .filter(|previous| previous.hold != current.hold)
        .map(|_| current.hold);
    let reference_update = previous
        .filter(|previous| previous.reference_tenths_db != current.reference_tenths_db)
        .map(|_| current.reference_tenths_db);
    (geometry_changed, hold_update, reference_update)
}

fn expire_power_on_wake(state: &mut GuiState, now: Instant) -> bool {
    if state.radio_power_command_pending
        && state
            .radio_power_wake_deadline
            .is_some_and(|deadline| now >= deadline)
    {
        state.radio_power_on = Some(false);
        state.radio_power_command_pending = false;
        state.radio_power_settling = false;
        state.radio_power_wake_deadline = None;
        state.last_error = Some("radio did not respond after power-on command".to_string());
        true
    } else {
        false
    }
}

fn accept_power_command(state: &mut GuiState, target: bool, now: Instant) {
    // A successful write only means the command was accepted. Do not show ON
    // until a status probe confirms that the radio has actually woken.
    state.radio_power_on = if target { None } else { Some(false) };
    state.radio_power_command_pending = target;
    state.radio_power_settling = target;
    state.radio_power_wake_deadline = target.then_some(now + Duration::from_secs(12));
    state.last_error = None;
}

fn reject_power_command(state: &mut GuiState, error: String) {
    state.last_error = Some(error);
    state.radio_power_command_pending = false;
    state.radio_power_settling = false;
    state.radio_power_wake_deadline = None;
}

fn apply_icom_mode_details(
    state: &mut GuiState,
    details: OperatingMode,
    data_mode_override: Option<bool>,
) {
    // Keep the base mode independent from the data flag. On the IC-7300 the
    // ordinary 0x04 response can carry a stale or misleading data byte;
    // DataMode is read back separately when the profile supports it. If that
    // dedicated read is temporarily unavailable, retain the last confirmed
    // value instead of treating the failed read as a real false response.
    state.mode = icom_base_mode_label(details.base).to_string();
    state.data_mode = Some(
        data_mode_override
            .or(state.data_mode)
            .unwrap_or(details.data_mode),
    );
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
    radio: &dyn Radio,
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
                ControlId::RfPower if s.rf_power_write_pending == Some(value) => {
                    s.rf_power = Some(value);
                    s.rf_power_write_pending = None;
                }
                ControlId::RfPower if s.rf_power_write_pending.is_none() => {
                    s.rf_power = Some(value)
                }
                ControlId::RfPower => {}
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
                MeterId::Voltage => {
                    s.voltage_meter = Some(value);
                    record_voltage_sample(&mut s.voltage_history, value);
                }
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

fn poll_remote_level_state(
    rt: &tokio::runtime::Runtime,
    radio: &RadioHandle,
    state: &Arc<Mutex<GuiState>>,
) {
    // A HostBridge radio shares one physical CI-V link with scope traffic.
    // Send one request per level cycle; the event pump applies the reply and
    // the next cycle observes it. A bulk read here starves the service's
    // media sender and makes the remote waterfall/audio appear degraded.
    let cycle = REMOTE_LEVEL_POLL_INDEX.fetch_add(1, Ordering::Relaxed);
    // Reserve two non-signal slots in each eight-cycle window while leaving
    // the remaining odd cycles available for the rotating control/meter
    // reads. The previous four-cycle pattern returned on every odd cycle,
    // making the 20-slot rotation unreachable.
    if cycle % 8 == 1 {
        if let Ok(Some(status)) = rt.block_on(radio.get_tuner_status()) {
            state.lock().expect("ui state lock poisoned").tuner_status = Some(status);
        }
        return;
    }
    if cycle % 8 == 3 {
        if let Ok(Some(ControlValue::U8(filter))) =
            rt.block_on(radio.get_control(ControlId::Filter))
        {
            state.lock().expect("ui state lock poisoned").filter = Some(filter);
        }
        return;
    }
    // Signal is the primary RX meter. Sample it every second while still
    // giving the remaining advertised controls/meters a bounded rotation.
    if cycle.is_multiple_of(2) {
        update_remote_meter(rt, radio, state, MeterId::Signal, |s, v| {
            s.signal_meter = Some(v)
        });
        return;
    }
    // Count only actual rotating-read opportunities. Deriving this from the
    // global poll tick would skip slots whenever the priority tuner/filter
    // reads occupy a tick.
    let slot = REMOTE_LEVEL_CONTROL_INDEX.fetch_add(1, Ordering::Relaxed) % 20;
    match slot {
        0 => update_remote_u8(rt, radio, state, ControlId::AfGain, |s, v| {
            s.af_gain = Some(v)
        }),
        1 => update_remote_u8(rt, radio, state, ControlId::RfGain, |s, v| {
            s.rf_gain = Some(v)
        }),
        2 => update_remote_u8(rt, radio, state, ControlId::Squelch, |s, v| {
            s.squelch = Some(v)
        }),
        3 => update_remote_u8(rt, radio, state, ControlId::RfPower, |s, v| {
            s.rf_power = Some(v)
        }),
        4 => update_remote_u8(rt, radio, state, ControlId::Preamp, |s, v| {
            s.preamp = Some(v)
        }),
        5 => update_remote_u8(rt, radio, state, ControlId::Attenuator, |s, v| {
            s.attenuator = Some(v)
        }),
        6 => update_remote_u8(rt, radio, state, ControlId::Agc, |s, v| s.agc = Some(v)),
        7 => update_remote_u8(rt, radio, state, ControlId::NoiseReductionLevel, |s, v| {
            s.noise_reduction_level = Some(v)
        }),
        8 => update_remote_bool(rt, radio, state, ControlId::NoiseBlanker, |s, v| {
            s.noise_blank = Some(v)
        }),
        9 => update_remote_bool(rt, radio, state, ControlId::NoiseReduction, |s, v| {
            s.noise_reduction = Some(v)
        }),
        10 => update_remote_bool(rt, radio, state, ControlId::IpPlus, |s, v| {
            s.ip_plus = Some(v)
        }),
        11 => update_remote_bool(rt, radio, state, ControlId::Notch, |s, v| {
            s.notch_auto = Some(v)
        }),
        12 => update_remote_bool(rt, radio, state, ControlId::ManualNotch, |s, v| {
            s.notch_manual = Some(v)
        }),
        13 => update_remote_u8(rt, radio, state, ControlId::Filter, |s, v| {
            s.filter = Some(v)
        }),
        14 => update_remote_meter(rt, radio, state, MeterId::Power, |s, v| {
            s.power_meter = Some(v)
        }),
        15 => update_remote_meter(rt, radio, state, MeterId::Swr, |s, v| s.swr = Some(v)),
        16 => update_remote_meter(rt, radio, state, MeterId::Alc, |s, v| s.alc_meter = Some(v)),
        17 => update_remote_meter(rt, radio, state, MeterId::Compression, |s, v| {
            s.compression_meter = Some(v)
        }),
        18 => update_remote_meter(rt, radio, state, MeterId::Current, |s, v| {
            s.current_meter = Some(v)
        }),
        19 => update_remote_meter(rt, radio, state, MeterId::Voltage, |s, v| {
            s.voltage_meter = Some(v);
            record_voltage_sample(&mut s.voltage_history, v);
        }),
        _ => unreachable!(),
    }
}

fn update_remote_u8<F>(
    rt: &tokio::runtime::Runtime,
    radio: &RadioHandle,
    state: &Arc<Mutex<GuiState>>,
    id: ControlId,
    apply: F,
) where
    F: FnOnce(&mut GuiState, u8),
{
    if let Ok(Some(ControlValue::U8(value))) = rt.block_on(radio.get_control(id)) {
        apply(&mut state.lock().expect("ui state lock poisoned"), value);
    }
}

fn update_remote_bool<F>(
    rt: &tokio::runtime::Runtime,
    radio: &RadioHandle,
    state: &Arc<Mutex<GuiState>>,
    id: ControlId,
    apply: F,
) where
    F: FnOnce(&mut GuiState, bool),
{
    if let Ok(Some(ControlValue::Bool(value))) = rt.block_on(radio.get_control(id)) {
        apply(&mut state.lock().expect("ui state lock poisoned"), value);
    }
}

fn update_remote_meter<F>(
    rt: &tokio::runtime::Runtime,
    radio: &RadioHandle,
    state: &Arc<Mutex<GuiState>>,
    id: MeterId,
    apply: F,
) where
    F: FnOnce(&mut GuiState, u8),
{
    if let Ok(Some(value)) = rt.block_on(radio.get_meter(id)) {
        apply(&mut state.lock().expect("ui state lock poisoned"), value);
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
    let (span_hz, fixed_edges_hz, fixed_edge_number, center_mode) = match config.view {
        RadioScopeView::Narrow => match config.edges {
            Some(edges) => (None, Some(edges), Some(4), Some(true)),
            None => (
                Some(scope_span_hz(config.span_code)),
                None,
                None,
                Some(false),
            ),
        },
        RadioScopeView::Overview => (
            None,
            Some(config.edges.context("active band edges unavailable")?),
            Some(1),
            Some(true),
        ),
    };
    rt.block_on(
        radio.set_scope_configuration(qsonaut_radio::ScopeConfiguration {
            span_hz,
            fixed_edges_hz,
            fixed_edge_number,
            hold: hold_update,
            reference_level_tenths_db: reference_tenths_db_update,
            sweep_speed: Some(config.sweep_code),
            center_mode,
            vbw_wide: Some(scope_vbw_wide_for_view(config.view, config.vbw_wide)),
        }),
    )?;
    Ok(())
}

fn scope_vbw_wide_for_view(_view: RadioScopeView, user_vbw_wide: bool) -> bool {
    user_vbw_wide
}

fn scope_edges_for_view(
    scope_view: RadioScopeView,
    workspace_mode: WorkspaceMode,
    frequency_hz: Option<u64>,
    span_code: u8,
    mode: &str,
) -> Option<(u64, u64)> {
    match (scope_view, workspace_mode) {
        // Voice does not need a mode-aware passband. Keep the radio scope in
        // its normal centered-span mode so a voice transition does not rewrite
        // fixed edge memory.
        (RadioScopeView::Narrow, WorkspaceMode::Voice) => None,
        (RadioScopeView::Overview, _) => frequency_hz
            .and_then(|hz| band_edges_for_frequency(Some(hz)))
            .map(|(low, high, _)| (low, high)),
        (RadioScopeView::Narrow, _) => frequency_hz.and_then(|hz| {
            sideband_scope_edges(
                hz,
                scope_span_hz(span_code),
                scope_projection_for_mode(mode),
            )
        }),
    }
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
        scheduler.next_aux = now;
        assert_eq!(scheduler.next_due(now, true), Some(ScheduledMeter::Voltage));
        scheduler.next_aux = now;
        assert_eq!(
            scheduler.next_due(now, true),
            Some(ScheduledMeter::Temperature)
        );
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
        scheduler.next_aux = now;
        assert_eq!(
            scheduler.next_due(now, true),
            Some(ScheduledMeter::Temperature)
        );
    }

    #[test]
    fn meter_scheduler_waits_when_no_class_is_due_and_wraps_auxiliary_cycles() {
        let now = Instant::now();
        let mut scheduler = MeterPollScheduler::new();
        scheduler.next_signal = now + Duration::from_secs(1);
        scheduler.next_tx = now + Duration::from_secs(1);
        scheduler.next_aux = now + Duration::from_secs(1);
        assert_eq!(scheduler.next_due(now, false), None);

        scheduler.next_aux = now;
        for expected in [
            ScheduledMeter::Current,
            ScheduledMeter::Voltage,
            ScheduledMeter::Temperature,
        ] {
            assert_eq!(scheduler.next_due(now, false), Some(expected));
            scheduler.next_aux = now;
        }
        assert_eq!(scheduler.aux_index, 3);
    }

    #[test]
    fn scheduled_meter_ids_cover_every_hal_meter() {
        let meters = [
            (ScheduledMeter::Signal, MeterId::Signal),
            (ScheduledMeter::Power, MeterId::Power),
            (ScheduledMeter::Swr, MeterId::Swr),
            (ScheduledMeter::Alc, MeterId::Alc),
            (ScheduledMeter::Compression, MeterId::Compression),
            (ScheduledMeter::Current, MeterId::Current),
            (ScheduledMeter::Voltage, MeterId::Voltage),
            (ScheduledMeter::Temperature, MeterId::Temperature),
        ];

        for (scheduled, expected) in meters {
            assert_eq!(scheduled.meter_id(), expected);
        }
    }

    #[test]
    fn scope_vbw_respects_checkbox_in_narrow_and_overview_views() {
        for view in [RadioScopeView::Narrow, RadioScopeView::Overview] {
            assert!(!scope_vbw_wide_for_view(view, false));
            assert!(scope_vbw_wide_for_view(view, true));
        }
    }

    #[test]
    fn scope_edges_project_voice_overview_and_mode_aware_narrow_views() {
        assert_eq!(
            scope_edges_for_view(
                RadioScopeView::Narrow,
                WorkspaceMode::Voice,
                Some(14_074_000),
                3,
                "USB"
            ),
            None
        );
        assert!(scope_edges_for_view(
            RadioScopeView::Overview,
            WorkspaceMode::Ft8,
            Some(14_074_000),
            3,
            "USB"
        )
        .is_some());
        assert!(
            scope_edges_for_view(RadioScopeView::Overview, WorkspaceMode::Ft8, None, 3, "USB")
                .is_none()
        );
        assert!(scope_edges_for_view(
            RadioScopeView::Narrow,
            WorkspaceMode::Ft8,
            Some(14_074_000),
            3,
            "USB"
        )
        .is_some());
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
    fn scope_change_updates_separate_geometry_hold_and_reference_changes() {
        let baseline = RadioScopeStreamConfig {
            view: RadioScopeView::Narrow,
            span_code: 3,
            vbw_wide: false,
            edges: None,
            sweep_code: 1,
            hold: false,
            reference_tenths_db: 0,
        };

        assert_eq!(
            scope_change_updates(
                Some(baseline),
                RadioScopeStreamConfig {
                    hold: true,
                    reference_tenths_db: 10,
                    ..baseline
                }
            ),
            (false, Some(true), Some(10))
        );
        assert_eq!(
            scope_change_updates(
                Some(baseline),
                RadioScopeStreamConfig {
                    span_code: 4,
                    ..baseline
                }
            ),
            (true, None, None)
        );
        assert_eq!(scope_change_updates(None, baseline), (true, None, None));
    }

    #[test]
    fn power_on_wake_timeout_only_expires_an_outstanding_command() {
        let now = Instant::now();
        let mut expired = GuiState {
            radio_power_command_pending: true,
            radio_power_settling: true,
            radio_power_on: None,
            radio_power_wake_deadline: Some(now - Duration::from_secs(1)),
            ..GuiState::default()
        };

        assert!(expire_power_on_wake(&mut expired, now));
        assert_eq!(expired.radio_power_on, Some(false));
        assert!(!expired.radio_power_command_pending);
        assert!(!expired.radio_power_settling);
        assert!(expired.radio_power_wake_deadline.is_none());
        assert_eq!(
            expired.last_error.as_deref(),
            Some("radio did not respond after power-on command")
        );

        let mut waiting = GuiState {
            radio_power_command_pending: true,
            radio_power_wake_deadline: Some(now + Duration::from_secs(1)),
            ..GuiState::default()
        };
        assert!(!expire_power_on_wake(&mut waiting, now));
        assert!(waiting.radio_power_command_pending);
        assert!(waiting.last_error.is_none());
    }

    #[test]
    fn accepted_power_command_projects_on_and_off_states_conservatively() {
        let now = Instant::now();
        let mut waking = GuiState::default();
        accept_power_command(&mut waking, true, now);
        assert_eq!(waking.radio_power_on, None);
        assert!(waking.radio_power_command_pending);
        assert!(waking.radio_power_settling);
        assert_eq!(
            waking
                .radio_power_wake_deadline
                .expect("wake deadline")
                .duration_since(now),
            Duration::from_secs(12)
        );

        let mut off = GuiState {
            radio_power_on: Some(true),
            last_error: Some("stale error".to_string()),
            ..GuiState::default()
        };
        accept_power_command(&mut off, false, now);
        assert_eq!(off.radio_power_on, Some(false));
        assert!(!off.radio_power_command_pending);
        assert!(!off.radio_power_settling);
        assert!(off.radio_power_wake_deadline.is_none());
        assert!(off.last_error.is_none());
    }

    #[test]
    fn rejected_power_command_clears_all_pending_wake_state() {
        let mut state = GuiState {
            radio_power_command_pending: true,
            radio_power_settling: true,
            radio_power_wake_deadline: Some(Instant::now() + Duration::from_secs(12)),
            ..GuiState::default()
        };

        reject_power_command(&mut state, "transport unavailable".to_string());

        assert_eq!(state.last_error.as_deref(), Some("transport unavailable"));
        assert!(!state.radio_power_command_pending);
        assert!(!state.radio_power_settling);
        assert!(state.radio_power_wake_deadline.is_none());
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

    #[test]
    fn icom_mode_readback_preserves_data_state_when_dedicated_read_is_unavailable() {
        let mut state = GuiState {
            data_mode: Some(true),
            ..GuiState::default()
        };
        apply_icom_mode_details(
            &mut state,
            OperatingMode {
                base: BaseMode::Usb,
                data_mode: false,
                filter: Some(2),
            },
            None,
        );

        assert_eq!(state.mode, "USB");
        assert_eq!(state.data_mode, Some(true));
    }

    #[test]
    fn control_vfo_value_accepts_only_the_two_hal_vfos() {
        assert_eq!(control_vfo_value(&ControlValue::U8(0)), Some(0));
        assert_eq!(control_vfo_value(&ControlValue::Vfo(1)), Some(1));
        assert_eq!(control_vfo_value(&ControlValue::U8(2)), None);
        assert_eq!(control_vfo_value(&ControlValue::Bool(true)), None);
    }

    #[test]
    fn icom_base_mode_labels_cover_known_and_unknown_modes() {
        let modes = [
            (BaseMode::Lsb, "LSB"),
            (BaseMode::Usb, "USB"),
            (BaseMode::Am, "AM"),
            (BaseMode::Cw, "CW"),
            (BaseMode::Rtty, "RTTY"),
            (BaseMode::Fm, "FM"),
            (BaseMode::Wfm, "WFM"),
            (BaseMode::CwR, "CW-R"),
            (BaseMode::RttyR, "RTTY-R"),
            (BaseMode::Unknown(99), "UNKNOWN"),
        ];

        for (mode, label) in modes {
            assert_eq!(icom_base_mode_label(mode), label);
        }
    }

    #[test]
    fn waterfall_application_downsamples_scales_and_caps_history() {
        let mut state = GuiState {
            radio_scope_contrast: 2.0,
            ..GuiState::default()
        };

        for _ in 0..(RADIO_WF_HEIGHT + 1) {
            apply_waterfall_bins(
                &mut state,
                &(0..1_400).map(|value| value as u8).collect::<Vec<_>>(),
            );
        }

        assert_eq!(state.radio_waterfall_rows.len(), RADIO_WF_HEIGHT);
        assert_eq!(
            state.radio_waterfall_rows.back().map(Vec::len),
            Some(MAX_RADIO_WF_BINS)
        );
        assert_eq!(state.radio_waterfall_revision, (RADIO_WF_HEIGHT + 1) as u64);
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

#[cfg(test)]
mod level_poll_tests {
    use super::*;
    use async_trait::async_trait;
    use qsonaut_radio::RadioCapabilities;
    use std::collections::{HashMap, VecDeque};
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;

    struct CoverageRadio {
        controls: HashMap<ControlId, ControlValue>,
        meters: HashMap<MeterId, u8>,
    }

    struct ErrorRadio;

    struct ScriptedCiVTransport {
        reads: VecDeque<Vec<u8>>,
    }

    impl ScriptedCiVTransport {
        fn with_frames(frames: impl IntoIterator<Item = Vec<u8>>) -> Self {
            Self {
                reads: frames.into_iter().collect(),
            }
        }

        fn acknowledgements(count: usize) -> Self {
            Self::with_frames((0..count).map(|_| vec![0xFE, 0xFE, 0xE0, 0x94, 0xFB, 0xFD]))
        }

        fn meter_response(command: u8) -> Vec<u8> {
            vec![0xFE, 0xFE, 0xE0, 0x94, 0x15, command, 0x00, 0x48, 0xFD]
        }
    }

    impl Read for ScriptedCiVTransport {
        fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
            let Some(mut frame) = self.reads.pop_front() else {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "scripted CI-V transport has no response",
                ));
            };
            let count = buffer.len().min(frame.len());
            buffer[..count].copy_from_slice(&frame[..count]);
            if count < frame.len() {
                frame.drain(..count);
                self.reads.push_front(frame);
            }
            Ok(count)
        }
    }

    impl Write for ScriptedCiVTransport {
        fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
            Ok(buffer.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl qsonaut_radio::RadioTransport for ScriptedCiVTransport {
        fn set_timeout(&mut self, _timeout: Duration) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[async_trait]
    impl Radio for CoverageRadio {
        async fn get_frequency_hz(&self) -> Result<u64> {
            Ok(7_074_000)
        }

        async fn set_frequency_hz(&self, _hz: u64) -> Result<()> {
            Ok(())
        }

        async fn get_mode(&self) -> Result<Mode> {
            Ok(Mode::Usb)
        }

        async fn set_mode(&self, _mode: Mode) -> Result<()> {
            Ok(())
        }

        async fn set_control(&self, _id: ControlId, _value: ControlValue) -> Result<()> {
            Ok(())
        }

        async fn set_ptt(&self, _enabled: bool) -> Result<()> {
            Ok(())
        }

        async fn get_control(&self, id: ControlId) -> Result<Option<ControlValue>> {
            Ok(self.controls.get(&id).cloned())
        }

        async fn get_meter(&self, id: MeterId) -> Result<Option<u8>> {
            Ok(self.meters.get(&id).copied())
        }

        fn supports_control(&self, id: ControlId) -> bool {
            self.controls.contains_key(&id)
        }

        fn supports_meter(&self, id: MeterId) -> bool {
            self.meters.contains_key(&id)
        }

        fn swr_sweep_setup(&self) -> Option<qsonaut_radio::SwrSweepSetup> {
            Some(qsonaut_radio::SwrSweepSetup {
                carrier_mode: Mode::Rtty,
                rf_power: 77,
            })
        }

        fn capabilities(&self) -> RadioCapabilities {
            RadioCapabilities::default()
        }
    }

    #[async_trait]
    impl Radio for ErrorRadio {
        async fn get_frequency_hz(&self) -> Result<u64> {
            Err(anyhow::anyhow!("frequency read failed"))
        }

        async fn set_frequency_hz(&self, _hz: u64) -> Result<()> {
            Err(anyhow::anyhow!("frequency write failed"))
        }

        async fn get_mode(&self) -> Result<Mode> {
            Err(anyhow::anyhow!("mode read failed"))
        }

        async fn set_mode(&self, _mode: Mode) -> Result<()> {
            Err(anyhow::anyhow!("mode write failed"))
        }

        async fn set_ptt(&self, _enabled: bool) -> Result<()> {
            Err(anyhow::anyhow!("PTT write failed"))
        }

        async fn get_control(&self, _id: ControlId) -> Result<Option<ControlValue>> {
            Err(anyhow::anyhow!("control read failed"))
        }

        async fn get_meter(&self, _id: MeterId) -> Result<Option<u8>> {
            Err(anyhow::anyhow!("meter read failed"))
        }

        fn capabilities(&self) -> RadioCapabilities {
            RadioCapabilities::default()
        }
    }

    #[test]
    fn level_poll_applies_supported_controls_meters_and_voltage_history() {
        let controls = HashMap::from([
            (ControlId::AfGain, ControlValue::U8(11)),
            (ControlId::RfGain, ControlValue::U8(22)),
            (ControlId::Squelch, ControlValue::U8(33)),
            (ControlId::RfPower, ControlValue::U8(44)),
            (ControlId::Preamp, ControlValue::U8(55)),
            (ControlId::Attenuator, ControlValue::U8(66)),
            (ControlId::Agc, ControlValue::U8(77)),
            (ControlId::NoiseReductionLevel, ControlValue::U8(88)),
            (ControlId::NoiseBlanker, ControlValue::Bool(true)),
            (ControlId::NoiseReduction, ControlValue::Bool(true)),
            (ControlId::IpPlus, ControlValue::Bool(true)),
            (ControlId::Notch, ControlValue::Bool(true)),
            (ControlId::ManualNotch, ControlValue::Bool(true)),
        ]);
        let meters = HashMap::from([
            (MeterId::Signal, 101),
            (MeterId::Power, 102),
            (MeterId::Swr, 103),
            (MeterId::Alc, 104),
            (MeterId::Compression, 105),
            (MeterId::Current, 106),
            (MeterId::Voltage, 107),
            (MeterId::Temperature, 108),
        ]);
        let radio = CoverageRadio { controls, meters };
        let state = Arc::new(Mutex::new(GuiState::default()));
        let rt = tokio::runtime::Runtime::new().expect("test runtime");

        poll_radio_level_state(&rt, &radio, &state);

        let state = state.lock().expect("state lock");
        assert_eq!(state.af_gain, Some(11));
        assert_eq!(state.rf_gain, Some(22));
        assert_eq!(state.squelch, Some(33));
        assert_eq!(state.rf_power, Some(44));
        assert_eq!(state.preamp, Some(55));
        assert_eq!(state.attenuator, Some(66));
        assert_eq!(state.agc, Some(77));
        assert_eq!(state.noise_reduction_level, Some(88));
        assert_eq!(state.noise_blank, Some(true));
        assert_eq!(state.noise_reduction, Some(true));
        assert_eq!(state.ip_plus, Some(true));
        assert_eq!(state.notch_auto, Some(true));
        assert_eq!(state.notch_manual, Some(true));
        assert_eq!(state.signal_meter, Some(101));
        assert_eq!(state.power_meter, Some(102));
        assert_eq!(state.swr, Some(103));
        assert_eq!(state.alc_meter, Some(104));
        assert_eq!(state.compression_meter, Some(105));
        assert_eq!(state.current_meter, Some(106));
        assert_eq!(state.voltage_meter, Some(107));
        assert_eq!(state.temperature_meter, Some(108));
        assert_eq!(state.voltage_history.back(), Some(&107));
    }

    #[test]
    fn level_poll_gates_unsupported_controls_and_meters() {
        let radio = CoverageRadio {
            controls: HashMap::new(),
            meters: HashMap::new(),
        };
        let state = Arc::new(Mutex::new(GuiState::default()));
        {
            let mut state = state.lock().expect("state lock");
            state.af_gain = Some(99);
            state.signal_meter = Some(88);
        }
        let rt = tokio::runtime::Runtime::new().expect("test runtime");

        poll_radio_level_state(&rt, &radio, &state);

        let state = state.lock().expect("state lock");
        assert_eq!(state.af_gain, Some(99));
        assert_eq!(state.signal_meter, Some(88));
        assert!(state.tuner_status.is_none());
        assert!(state.voltage_history.is_empty());
        assert_eq!(read_u8_control(&rt, &radio, ControlId::Filter), None);
        assert_eq!(
            read_bool_control(&rt, &radio, ControlId::NoiseBlanker),
            None
        );
        assert_eq!(read_vfo_control(&rt, &radio), None);
    }

    #[test]
    fn control_read_helpers_reject_wrong_value_types_and_invalid_vfos() {
        let radio = CoverageRadio {
            controls: HashMap::from([
                (ControlId::Filter, ControlValue::Bool(true)),
                (ControlId::NoiseBlanker, ControlValue::U8(1)),
                (ControlId::Vfo, ControlValue::U8(2)),
            ]),
            meters: HashMap::new(),
        };
        let rt = tokio::runtime::Runtime::new().expect("test runtime");

        assert_eq!(read_u8_control(&rt, &radio, ControlId::Filter), None);
        assert_eq!(
            read_bool_control(&rt, &radio, ControlId::NoiseBlanker),
            None
        );
        assert_eq!(read_vfo_control(&rt, &radio), None);
    }

    #[test]
    fn core_poll_maps_every_null_radio_mode_and_marks_connection_ready() {
        let modes = [
            (Mode::Usb, "USB"),
            (Mode::Lsb, "LSB"),
            (Mode::Cw, "CW"),
            (Mode::Data, "USB"),
            (Mode::Am, "AM"),
            (Mode::Fm, "FM"),
            (Mode::Wfm, "WFM"),
            (Mode::Rtty, "RTTY"),
            (Mode::CwReverse, "CW-R"),
            (Mode::RttyReverse, "RTTY-R"),
        ];
        let rt = tokio::runtime::Runtime::new().expect("test runtime");

        for (mode, label) in modes {
            let radio = ConfiguredRadio::Null(qsonaut_radio::NullRadio::with_frequency_mode(
                7_074_000, mode,
            ));
            let state = Arc::new(Mutex::new(GuiState::default()));
            poll_radio_core_state(&rt, &radio.into(), &state, true);

            let state = state.lock().expect("state lock");
            assert_eq!(state.frequency_hz, Some(7_074_000));
            assert_eq!(state.mode, label);
            assert_eq!(state.data_mode, Some(mode == Mode::Data));
            assert_eq!(state.radio_power_on, Some(true));
            assert!(state.last_update.is_some());
            assert!(state.last_error.is_none());
        }
    }

    #[test]
    fn level_poll_handles_failed_control_and_meter_reads_without_state_corruption() {
        let rt = tokio::runtime::Runtime::new().expect("test runtime");
        let state = Arc::new(Mutex::new(GuiState::default()));
        let radio = ConfiguredRadio::Null(qsonaut_radio::NullRadio::new());
        {
            let mut state = state.lock().expect("state lock");
            state.radio_power_on = Some(true);
            state.frequency_hz = Some(14_074_000);
        }
        // NullRadio is healthy and confirms that the ready state is populated
        // before the error contract is checked below.
        poll_radio_core_state(&rt, &radio.into(), &state, true);
        assert_eq!(state.lock().expect("state lock").radio_power_on, Some(true));

        // The configured wrapper's non-Icom path is driven by the underlying
        // Radio implementation; exercise the error policy with a direct fake
        // through the same level-poll boundary.
        let failed_state = Arc::new(Mutex::new(GuiState::default()));
        poll_radio_level_state(&rt, &ErrorRadio, &failed_state);
        let failed_state = failed_state.lock().expect("state lock");
        assert!(failed_state.frequency_hz.is_none());
        assert!(failed_state.signal_meter.is_none());
    }

    #[test]
    fn level_poll_preserves_pending_rf_power_until_matching_readback() {
        let rt = tokio::runtime::Runtime::new().expect("test runtime");
        let radio = CoverageRadio {
            controls: HashMap::from([(ControlId::RfPower, ControlValue::U8(44))]),
            meters: HashMap::from([(MeterId::Voltage, 120)]),
        };
        let state = Arc::new(Mutex::new(GuiState::default()));
        {
            let mut state = state.lock().expect("state lock");
            state.rf_power = Some(12);
            state.rf_power_write_pending = Some(44);
        }

        poll_radio_level_state(&rt, &radio, &state);
        {
            let state = state.lock().expect("state lock");
            assert_eq!(state.rf_power, Some(44));
            assert_eq!(state.rf_power_write_pending, None);
            assert_eq!(state.voltage_meter, Some(120));
            assert_eq!(state.voltage_history.back(), Some(&120));
        }

        {
            let mut state = state.lock().expect("state lock");
            state.rf_power = Some(44);
            state.rf_power_write_pending = Some(55);
        }
        poll_radio_level_state(&rt, &radio, &state);
        let state = state.lock().expect("state lock");
        assert_eq!(state.rf_power, Some(44));
        assert_eq!(state.rf_power_write_pending, Some(55));
    }

    #[test]
    fn core_poll_marks_external_transport_failure_as_unavailable() {
        let rt = tokio::runtime::Runtime::new().expect("test runtime");
        let state = Arc::new(Mutex::new(GuiState::default()));
        let radio =
            ConfiguredRadio::Rigctld(qsonaut_radio::rigctld::RigctldRadio::new("127.0.0.1:1"));

        poll_radio_core_state(&rt, &radio.into(), &state, false);

        let state = state.lock().expect("state lock");
        assert_eq!(state.radio_power_on, Some(false));
        assert!(state.last_error.is_some());
    }

    #[test]
    fn icom_core_poll_isolates_serial_probe_failure() {
        let rt = tokio::runtime::Runtime::new().expect("test runtime");
        let state = Arc::new(Mutex::new(GuiState::default()));
        let radio = ConfiguredRadio::Icom(IcomCiVRadio::new_generic(
            "/definitely-not-a-real-serial-device",
            115_200,
            0xE0,
            0x94,
        ));

        poll_radio_core_state(&rt, &radio.into(), &state, false);

        let state = state.lock().expect("state lock");
        assert_eq!(state.radio_power_on, Some(false));
        assert!(state.last_error.is_some());
        assert!(state.frequency_hz.is_none());
    }

    #[test]
    fn icom_core_poll_projects_frequency_mode_and_power_from_civ_status() {
        let transport = ScriptedCiVTransport::with_frames([
            vec![
                0xFE, 0xFE, 0xE0, 0x94, 0x03, 0x00, 0x40, 0x07, 0x14, 0x00, 0xFD,
            ],
            vec![0xFE, 0xFE, 0xE0, 0x94, 0x04, 0x01, 0xFD],
        ]);
        let radio = ConfiguredRadio::Icom(IcomCiVRadio::with_transport(
            Some(qsonaut_radio::IcomCivModel::Ic7300),
            0xE0,
            0x94,
            transport,
        ));
        let state = Arc::new(Mutex::new(GuiState::default()));
        let rt = tokio::runtime::Runtime::new().expect("test runtime");

        poll_radio_core_state(&rt, &radio.into(), &state, false);

        let state = state.lock().expect("state lock");
        assert_eq!(state.frequency_hz, Some(14_074_000));
        assert_eq!(state.mode, "USB");
        assert_eq!(state.radio_power_on, Some(true));
        assert!(state.last_update.is_some());
        assert!(state.last_error.is_none());
    }

    #[test]
    fn icom_scope_worker_waits_when_radio_is_known_off() {
        let state = Arc::new(Mutex::new(GuiState::default()));
        {
            let mut state = state.lock().expect("state lock");
            state.radio_power_on = Some(false);
            state.radio_spectrum_desired = true;
            state.radio_spectrum_enabled = true;
        }
        let stop = Arc::new(AtomicBool::new(false));
        let sweep_abort = Arc::new(AtomicBool::new(false));
        let display_tuning = Arc::new(Mutex::new(DisplayTuning::default()));
        let repaint = Arc::new(OnceLock::new());
        let ptt_allowed = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let handle = spawn_radio_worker(
            ConfiguredRadio::Icom(IcomCiVRadio::new_generic(
                "/definitely-not-a-real-serial-device",
                115_200,
                0xE0,
                0x94,
            )),
            state.clone(),
            stop,
            sweep_abort,
            display_tuning,
            rx,
            repaint,
            ptt_allowed,
        );

        for _ in 0..100 {
            if state.lock().expect("state lock").radio_waterfall_status == "OFF (radio off)" {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        tx.send(GuiCommand::Quit).expect("quit scope worker");
        handle.join().expect("scope worker join");

        let state = state.lock().expect("state lock");
        assert_eq!(state.radio_waterfall_status, "OFF (radio off)");
        assert!(!state.radio_spectrum_enabled);
    }

    #[test]
    fn icom_scope_worker_reports_configuration_failure_without_hardware() {
        let state = Arc::new(Mutex::new(GuiState::default()));
        {
            let mut state = state.lock().expect("state lock");
            state.radio_power_on = Some(true);
            state.radio_spectrum_desired = true;
            state.radio_spectrum_enabled = true;
            state.radio_scope_settings_dirty = true;
        }
        let stop = Arc::new(AtomicBool::new(false));
        let sweep_abort = Arc::new(AtomicBool::new(false));
        let display_tuning = Arc::new(Mutex::new(DisplayTuning::default()));
        let repaint = Arc::new(OnceLock::new());
        let ptt_allowed = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let handle = spawn_radio_worker(
            ConfiguredRadio::Icom(IcomCiVRadio::new_generic(
                "/definitely-not-a-real-serial-device",
                115_200,
                0xE0,
                0x94,
            )),
            state.clone(),
            stop,
            sweep_abort,
            display_tuning,
            rx,
            repaint,
            ptt_allowed,
        );

        for _ in 0..250 {
            let status = state
                .lock()
                .expect("state lock")
                .radio_waterfall_status
                .clone();
            if status == "CONFIG ERROR" || status == "ENABLE RETRY" || status == "OFF (radio off)" {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        tx.send(GuiCommand::Quit).expect("quit scope worker");
        handle.join().expect("scope worker join");

        let state = state.lock().expect("state lock");
        assert!(
            state.radio_waterfall_status == "CONFIG ERROR"
                || state.radio_waterfall_status == "ENABLE RETRY"
                || state.radio_waterfall_status == "OFF (radio off)"
        );
        assert!(!state.ptt_on);
    }

    #[test]
    fn icom_scope_worker_reports_disable_failure_without_hardware() {
        let state = Arc::new(Mutex::new(GuiState::default()));
        {
            let mut state = state.lock().expect("state lock");
            state.radio_power_on = Some(true);
            state.radio_spectrum_desired = false;
            state.radio_spectrum_enabled = true;
        }
        let stop = Arc::new(AtomicBool::new(false));
        let sweep_abort = Arc::new(AtomicBool::new(false));
        let display_tuning = Arc::new(Mutex::new(DisplayTuning::default()));
        let repaint = Arc::new(OnceLock::new());
        let ptt_allowed = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let handle = spawn_radio_worker(
            ConfiguredRadio::Icom(IcomCiVRadio::new_generic(
                "/definitely-not-a-real-serial-device",
                115_200,
                0xE0,
                0x94,
            )),
            state.clone(),
            stop,
            sweep_abort,
            display_tuning,
            rx,
            repaint,
            ptt_allowed,
        );

        for _ in 0..250 {
            let status = state
                .lock()
                .expect("state lock")
                .radio_waterfall_status
                .clone();
            if status == "DISABLE ERROR" || status == "OFF (radio off)" {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        tx.send(GuiCommand::Quit).expect("quit scope worker");
        handle.join().expect("scope worker join");

        let state = state.lock().expect("state lock");
        assert!(
            state.radio_waterfall_status == "DISABLE ERROR"
                || state.radio_waterfall_status == "OFF (radio off)"
        );
        assert!(!state.ptt_on);
    }

    #[test]
    fn icom_scope_worker_reports_enable_failure_for_unavailable_radio() {
        let state = Arc::new(Mutex::new(GuiState::default()));
        {
            let mut state = state.lock().expect("state lock");
            state.radio_power_on = Some(true);
            state.radio_spectrum_desired = true;
            state.radio_spectrum_enabled = false;
        }
        let stop = Arc::new(AtomicBool::new(false));
        let sweep_abort = Arc::new(AtomicBool::new(false));
        let display_tuning = Arc::new(Mutex::new(DisplayTuning::default()));
        let repaint = Arc::new(OnceLock::new());
        let ptt_allowed = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let handle = spawn_radio_worker(
            ConfiguredRadio::Icom(IcomCiVRadio::new_generic(
                "/definitely-not-a-real-serial-device",
                115_200,
                0xE0,
                0x94,
            )),
            state.clone(),
            stop,
            sweep_abort,
            display_tuning,
            rx,
            repaint,
            ptt_allowed,
        );

        for _ in 0..250 {
            let status = state
                .lock()
                .expect("state lock")
                .radio_waterfall_status
                .clone();
            if status == "ENABLE RETRY" || status == "OFF (radio off)" {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        tx.send(GuiCommand::Quit).expect("quit scope worker");
        handle.join().expect("scope worker join");

        let state = state.lock().expect("state lock");
        assert!(
            state.radio_waterfall_status == "ENABLE RETRY"
                || state.radio_waterfall_status == "OFF (radio off)"
        );
        assert!(!state.ptt_on);
    }

    #[test]
    fn scope_configuration_validates_each_view_shape_before_hardware_use() {
        let radio =
            IcomCiVRadio::new_generic("/definitely-not-a-real-serial-device", 115_200, 0xE0, 0x94);
        let rt = tokio::runtime::Runtime::new().expect("test runtime");
        let base = RadioScopeStreamConfig {
            view: RadioScopeView::Narrow,
            span_code: 1,
            vbw_wide: false,
            edges: None,
            sweep_code: 1,
            hold: false,
            reference_tenths_db: 0,
        };

        let narrow = configure_radio_scope(&rt, &radio, &base, None, None);
        assert!(narrow.is_err(), "invalid serial path must fail safely");

        let mut fixed = base;
        fixed.edges = Some((14_070_000, 14_080_000));
        assert!(configure_radio_scope(&rt, &radio, &fixed, Some(true), Some(-20)).is_err());

        let overview = RadioScopeStreamConfig {
            view: RadioScopeView::Overview,
            edges: None,
            ..base
        };
        let error = configure_radio_scope(&rt, &radio, &overview, None, None)
            .expect_err("overview requires band edges");
        assert!(error.to_string().contains("active band edges unavailable"));
    }

    #[test]
    fn scope_configuration_applies_narrow_fixed_and_overview_profiles() {
        let rt = tokio::runtime::Runtime::new().expect("test runtime");
        let narrow_radio = IcomCiVRadio::with_transport(
            Some(qsonaut_radio::IcomCivModel::Ic7300),
            0xE0,
            0x94,
            ScriptedCiVTransport::acknowledgements(4),
        );
        let narrow = RadioScopeStreamConfig {
            view: RadioScopeView::Narrow,
            span_code: 1,
            vbw_wide: true,
            edges: None,
            sweep_code: 2,
            hold: false,
            reference_tenths_db: 0,
        };
        assert!(configure_radio_scope(&rt, &narrow_radio, &narrow, None, None).is_ok());

        let fixed_radio = IcomCiVRadio::with_transport(
            Some(qsonaut_radio::IcomCivModel::Ic7300),
            0xE0,
            0x94,
            ScriptedCiVTransport::acknowledgements(7),
        );
        let fixed = RadioScopeStreamConfig {
            edges: Some((14_070_000, 14_080_000)),
            ..narrow
        };
        assert!(configure_radio_scope(&rt, &fixed_radio, &fixed, Some(true), Some(10)).is_ok());

        let overview_radio = IcomCiVRadio::with_transport(
            Some(qsonaut_radio::IcomCivModel::Ic7300),
            0xE0,
            0x94,
            ScriptedCiVTransport::acknowledgements(6),
        );
        let overview = RadioScopeStreamConfig {
            view: RadioScopeView::Overview,
            edges: Some((14_070_000, 14_080_000)),
            ..narrow
        };
        assert!(configure_radio_scope(&rt, &overview_radio, &overview, Some(false), None).is_ok());
    }

    #[test]
    fn remote_level_scheduler_rotates_every_control_and_meter_without_blocking() {
        let radio = RadioHandle::Local(ConfiguredRadio::Null(
            qsonaut_radio::NullRadio::with_frequency_mode(14_074_000, Mode::Data),
        ));
        let state = Arc::new(Mutex::new(GuiState::default()));
        let rt = tokio::runtime::Runtime::new().expect("test runtime");

        // Exercise complete scheduler rotations, including the tuner/filter
        // priority slots and every advertised control and meter destination.
        for _ in 0..48 {
            poll_remote_level_state(&rt, &radio, &state);
        }

        let state = state.lock().expect("state lock");
        assert!(state.voltage_history.len() <= 20);
    }

    #[test]
    fn remote_level_scheduler_uses_nonblocking_hostbridge_requests() {
        let (remote, client) = test_remote_radio();
        remote.mark_disconnected_for_test();
        let radio = RadioHandle::Remote(Box::new(remote));
        let state = Arc::new(Mutex::new(GuiState::default()));
        let rt = tokio::runtime::Runtime::new().expect("test runtime");

        // A disconnected client still exercises the complete remote request
        // path. Every operation must return promptly and leave cached state
        // untouched until the event pump receives a real reply.
        for _ in 0..80 {
            poll_remote_level_state(&rt, &radio, &state);
        }

        let state = state.lock().expect("state lock");
        assert!(state.signal_meter.is_none());
        assert!(state.voltage_history.is_empty());
        drop(state);
        client.shutdown().ok();
    }

    #[test]
    fn level_scheduler_projects_all_cached_typed_values() {
        let controls = ControlId::ALL
            .iter()
            .copied()
            .map(|id| {
                let value = match id {
                    ControlId::NoiseBlanker
                    | ControlId::NoiseReduction
                    | ControlId::IpPlus
                    | ControlId::Notch
                    | ControlId::ManualNotch
                    | ControlId::Tuner => ControlValue::Bool(true),
                    ControlId::Vfo => ControlValue::Vfo(1),
                    _ => ControlValue::U8(48),
                };
                (id, value)
            })
            .collect();
        let meters = MeterId::ALL.iter().copied().map(|id| (id, 49)).collect();
        let radio = RadioHandle::Test(Arc::new(CoverageRadio { controls, meters }));
        let state = Arc::new(Mutex::new(GuiState::default()));
        let rt = tokio::runtime::Runtime::new().expect("test runtime");

        for _ in 0..400 {
            poll_remote_level_state(&rt, &radio, &state);
        }

        let state = state.lock().expect("state lock");
        assert_eq!(state.signal_meter, Some(49));
        assert_eq!(state.voltage_meter, Some(49));
        assert!(!state.voltage_history.is_empty());
        assert_eq!(state.ip_plus, Some(true));
        assert_eq!(state.notch_manual, Some(true));
    }

    #[test]
    fn remote_core_poll_refreshes_on_its_bounded_cadence() {
        let (remote, client) = test_remote_radio();
        remote.mark_disconnected_for_test();
        let radio = RadioHandle::Remote(Box::new(remote));
        let state = Arc::new(Mutex::new(GuiState::default()));
        let rt = tokio::runtime::Runtime::new().expect("test runtime");

        for _ in 0..7 {
            poll_radio_core_state(&rt, &radio, &state, false);
        }
        poll_radio_core_state(&rt, &radio, &state, true);

        let state = state.lock().expect("state lock");
        assert_eq!(state.radio_power_on, Some(false));
        assert!(state.last_error.is_some());
        drop(state);
        client.shutdown().ok();
    }

    #[test]
    fn worker_swr_sweep_completes_with_a_deterministic_hal_fixture() {
        let controls = HashMap::from([
            (ControlId::AfGain, ControlValue::U8(50)),
            (ControlId::RfGain, ControlValue::U8(50)),
            (ControlId::Squelch, ControlValue::U8(0)),
            (ControlId::RfPower, ControlValue::U8(44)),
            (ControlId::Preamp, ControlValue::U8(0)),
            (ControlId::Attenuator, ControlValue::U8(0)),
            (ControlId::Agc, ControlValue::U8(0)),
            (ControlId::NoiseReductionLevel, ControlValue::U8(0)),
            (ControlId::Filter, ControlValue::U8(2)),
            (ControlId::NoiseBlanker, ControlValue::Bool(false)),
            (ControlId::NoiseReduction, ControlValue::Bool(false)),
            (ControlId::IpPlus, ControlValue::Bool(false)),
            (ControlId::Notch, ControlValue::Bool(false)),
            (ControlId::ManualNotch, ControlValue::Bool(false)),
            (ControlId::Vfo, ControlValue::Vfo(0)),
        ]);
        let meters = HashMap::from([(MeterId::Swr, 12)]);
        let radio = Arc::new(CoverageRadio { controls, meters });
        let state = Arc::new(Mutex::new(GuiState::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let sweep_abort = Arc::new(AtomicBool::new(false));
        let display_tuning = Arc::new(Mutex::new(DisplayTuning::default()));
        let repaint = Arc::new(OnceLock::new());
        let ptt_allowed = Arc::new(AtomicBool::new(true));
        let (tx, rx) = mpsc::channel();
        let handle = spawn_radio_worker(
            RadioHandle::Test(radio),
            state.clone(),
            stop,
            sweep_abort,
            display_tuning,
            rx,
            repaint,
            ptt_allowed,
        );

        tx.send(GuiCommand::TuneDelta(1_000)).expect("tune delta");
        tx.send(GuiCommand::TuneTo(7_100_000)).expect("tune to");
        tx.send(GuiCommand::CycleMode).expect("cycle mode");
        tx.send(GuiCommand::SetRadioMode(Mode::Data))
            .expect("set mode");
        for workspace in [
            WorkspaceMode::Voice,
            WorkspaceMode::Ft8,
            WorkspaceMode::Ft4,
            WorkspaceMode::Fst4,
            WorkspaceMode::Wspr,
            WorkspaceMode::Jt9,
            WorkspaceMode::Jt65,
            WorkspaceMode::Q65,
            WorkspaceMode::Msk144,
            WorkspaceMode::Cw,
            WorkspaceMode::Sstv,
        ] {
            tx.send(GuiCommand::ApplyWorkspace {
                mode: workspace,
                frequency_hz: 7_101_000,
            })
            .expect("apply supported workspace");
        }
        tx.send(GuiCommand::SetFilter(3)).expect("set filter");
        for (id, value) in [
            (ControlId::AfGain, ControlValue::U8(10)),
            (ControlId::RfGain, ControlValue::U8(20)),
            (ControlId::Squelch, ControlValue::U8(30)),
            (ControlId::RfPower, ControlValue::U8(40)),
            (ControlId::Preamp, ControlValue::U8(1)),
            (ControlId::Attenuator, ControlValue::U8(1)),
            (ControlId::Agc, ControlValue::U8(1)),
            (ControlId::NoiseReductionLevel, ControlValue::U8(1)),
            (ControlId::Filter, ControlValue::U8(2)),
            (ControlId::NoiseBlanker, ControlValue::Bool(true)),
            (ControlId::NoiseReduction, ControlValue::Bool(true)),
            (ControlId::IpPlus, ControlValue::Bool(true)),
            (ControlId::Notch, ControlValue::Bool(true)),
            (ControlId::ManualNotch, ControlValue::Bool(true)),
            (ControlId::Vfo, ControlValue::Vfo(1)),
        ] {
            tx.send(GuiCommand::SetControl(id, value))
                .expect("set supported control");
        }
        tx.send(GuiCommand::AfGainDelta(-5))
            .expect("adjust AF gain");
        tx.send(GuiCommand::StartTuner).expect("start tuner");
        tx.send(GuiCommand::SetPtt(false)).expect("PTT off");
        let (ack_tx, ack_rx) = mpsc::channel();
        tx.send(GuiCommand::SetPttWithAck(false, ack_tx))
            .expect("acknowledged PTT off");
        assert_eq!(ack_rx.recv_timeout(Duration::from_secs(1)).unwrap(), Ok(()));

        tx.send(GuiCommand::StartSwrSweep {
            start_hz: 7_074_000,
            stop_hz: 7_074_000,
            step_hz: 1_000,
            interval_ms: 100,
        })
        .expect("start deterministic sweep");
        for _ in 0..500 {
            if state
                .lock()
                .expect("state lock")
                .swr_sweep_status
                .contains("1 points")
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        tx.send(GuiCommand::Quit).expect("quit deterministic sweep");
        handle.join().expect("deterministic sweep worker join");

        let state = state.lock().expect("state lock");
        assert_eq!(state.swr, Some(12));
        assert_eq!(state.swr_sweep_points, vec![(7_074_000, 12)]);
        assert_eq!(state.swr_sweep_status, "1 points");
        assert_eq!(state.rf_power, Some(40));
        assert!(!state.ptt_on);
    }

    #[test]
    fn worker_error_fixture_exercises_nonfatal_command_failures() {
        let state = Arc::new(Mutex::new(GuiState::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let sweep_abort = Arc::new(AtomicBool::new(false));
        let display_tuning = Arc::new(Mutex::new(DisplayTuning::default()));
        let repaint = Arc::new(OnceLock::new());
        let ptt_allowed = Arc::new(AtomicBool::new(true));
        let (tx, rx) = mpsc::channel();
        let handle = spawn_radio_worker(
            RadioHandle::Test(Arc::new(ErrorRadio)),
            state.clone(),
            stop,
            sweep_abort,
            display_tuning,
            rx,
            repaint,
            ptt_allowed,
        );

        // Initial failure marks the link unavailable. Restore the known-live
        // state in the fixture so each command's independent error handling
        // can be exercised without a hardware timeout.
        for _ in 0..100 {
            if state.lock().expect("state lock").radio_power_on == Some(false) {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        state.lock().expect("state lock").radio_power_on = Some(true);
        tx.send(GuiCommand::TuneDelta(1_000))
            .expect("failing tune delta");
        tx.send(GuiCommand::TuneTo(7_100_000))
            .expect("failing tune");
        tx.send(GuiCommand::CycleMode).expect("failing mode cycle");
        tx.send(GuiCommand::SetRadioMode(Mode::Data))
            .expect("failing mode write");
        tx.send(GuiCommand::SetPtt(false)).expect("failing PTT");
        let (ack_tx, ack_rx) = mpsc::channel();
        tx.send(GuiCommand::SetPttWithAck(false, ack_tx))
            .expect("failing acknowledged PTT");
        tx.send(GuiCommand::SetPower(true)).expect("failing power");
        tx.send(GuiCommand::ApplyWorkspace {
            mode: WorkspaceMode::Voice,
            frequency_hz: 7_101_000,
        })
        .expect("failing workspace");
        tx.send(GuiCommand::SetFilter(2)).expect("failing filter");
        tx.send(GuiCommand::SetControl(
            ControlId::AfGain,
            ControlValue::U8(20),
        ))
        .expect("failing control");
        tx.send(GuiCommand::StartTuner).expect("failing tuner");
        tx.send(GuiCommand::AfGainDelta(1))
            .expect("failing AF gain");
        assert!(ack_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("failing PTT acknowledgement")
            .is_err());
        tx.send(GuiCommand::Quit).expect("quit error fixture");
        handle.join().expect("error fixture worker join");

        assert!(state.lock().expect("state lock").last_error.is_some());
    }

    #[test]
    fn hostbridge_scope_projection_preserves_each_view_contract() {
        let narrow = RadioScopeStreamConfig {
            view: RadioScopeView::Narrow,
            span_code: 2,
            vbw_wide: true,
            edges: None,
            sweep_code: 1,
            hold: true,
            reference_tenths_db: -30,
        };
        let wire = wire_scope_config(&narrow);
        assert_eq!(wire.center_mode, Some(false));
        assert_eq!(wire.span_hz, Some(scope_span_hz(2)));
        assert_eq!(wire.fixed_edges_hz, None);
        assert_eq!(wire.hold, Some(true));
        assert_eq!(wire.reference_level_tenths_db, Some(-30));

        let overview = RadioScopeStreamConfig {
            view: RadioScopeView::Overview,
            edges: Some((7_000_000, 7_300_000)),
            ..narrow
        };
        let wire = wire_scope_config(&overview);
        assert_eq!(wire.center_mode, Some(true));
        assert_eq!(wire.span_hz, None);
        assert_eq!(wire.fixed_edges_hz, overview.edges);
        assert_eq!(wire.fixed_edge_number, Some(1));
    }

    #[test]
    fn control_projection_covers_every_typed_radio_setting() {
        let mut state = GuiState::default();
        for (id, value) in [
            (ControlId::AfGain, ControlValue::U8(1)),
            (ControlId::RfGain, ControlValue::U8(2)),
            (ControlId::Squelch, ControlValue::U8(3)),
            (ControlId::RfPower, ControlValue::U8(4)),
            (ControlId::Preamp, ControlValue::U8(5)),
            (ControlId::Attenuator, ControlValue::U8(6)),
            (ControlId::Agc, ControlValue::U8(7)),
            (ControlId::NoiseReductionLevel, ControlValue::U8(8)),
            (ControlId::Filter, ControlValue::U8(9)),
            (ControlId::NoiseBlanker, ControlValue::Bool(true)),
            (ControlId::NoiseReduction, ControlValue::Bool(true)),
            (ControlId::IpPlus, ControlValue::Bool(true)),
            (ControlId::Notch, ControlValue::Bool(true)),
            (ControlId::ManualNotch, ControlValue::Bool(true)),
            (ControlId::Tuner, ControlValue::Bool(true)),
            (ControlId::Vfo, ControlValue::Vfo(1)),
        ] {
            project_control_write(&mut state, id, &value);
        }
        assert_eq!(state.af_gain, Some(1));
        assert_eq!(state.rf_gain, Some(2));
        assert_eq!(state.squelch, Some(3));
        assert_eq!(state.rf_power, Some(4));
        assert_eq!(state.preamp, Some(5));
        assert_eq!(state.attenuator, Some(6));
        assert_eq!(state.agc, Some(7));
        assert_eq!(state.noise_reduction_level, Some(8));
        assert_eq!(state.filter, Some(9));
        assert_eq!(state.noise_blank, Some(true));
        assert_eq!(state.noise_reduction, Some(true));
        assert_eq!(state.ip_plus, Some(true));
        assert_eq!(state.notch_auto, Some(true));
        assert_eq!(state.notch_manual, Some(true));
        assert_eq!(state.tuner_status.map(|status| status.enabled), Some(true));
        assert_eq!(state.active_vfo, 1);
    }

    #[test]
    fn icom_mode_labels_cover_all_hal_modes() {
        let modes = [
            (BaseMode::Lsb, "LSB"),
            (BaseMode::Usb, "USB"),
            (BaseMode::Am, "AM"),
            (BaseMode::Cw, "CW"),
            (BaseMode::Rtty, "RTTY"),
            (BaseMode::Fm, "FM"),
            (BaseMode::Wfm, "WFM"),
            (BaseMode::CwR, "CW-R"),
            (BaseMode::RttyR, "RTTY-R"),
            (BaseMode::Unknown(0x7f), "UNKNOWN"),
        ];
        for (mode, expected) in modes {
            assert_eq!(icom_base_mode_label(mode), expected);
        }
    }

    #[test]
    fn icom_mode_projection_preserves_confirmed_data_and_filter_state() {
        let mut state = GuiState {
            data_mode: Some(true),
            filter: Some(3),
            ..Default::default()
        };
        apply_icom_mode_details(
            &mut state,
            OperatingMode {
                base: BaseMode::Usb,
                data_mode: false,
                filter: None,
            },
            None,
        );
        assert_eq!(state.mode, "USB");
        assert_eq!(state.data_mode, Some(true));
        assert_eq!(state.filter, Some(3));

        apply_icom_mode_details(
            &mut state,
            OperatingMode {
                base: BaseMode::Cw,
                data_mode: false,
                filter: Some(1),
            },
            Some(false),
        );
        assert_eq!(state.mode, "CW");
        assert_eq!(state.data_mode, Some(false));
        assert_eq!(state.filter, Some(1));
    }

    #[test]
    fn vfo_projection_accepts_only_supported_hal_values() {
        let rt = tokio::runtime::Runtime::new().expect("test runtime");
        let radio = CoverageRadio {
            controls: HashMap::from([(ControlId::Vfo, ControlValue::Vfo(1))]),
            meters: HashMap::new(),
        };
        assert_eq!(read_vfo_control(&rt, &radio), Some(1));

        let invalid = CoverageRadio {
            controls: HashMap::from([(ControlId::Vfo, ControlValue::U8(2))]),
            meters: HashMap::new(),
        };
        assert_eq!(read_vfo_control(&rt, &invalid), None);
    }

    #[test]
    fn scope_change_detection_distinguishes_geometry_and_live_parameters() {
        let base = RadioScopeStreamConfig {
            view: RadioScopeView::Narrow,
            span_code: 2,
            vbw_wide: false,
            edges: None,
            sweep_code: 1,
            hold: false,
            reference_tenths_db: -20,
        };
        assert!(!scope_config_changed(None, base));
        assert!(scope_config_changed(
            Some(base),
            RadioScopeStreamConfig { hold: true, ..base }
        ));
        assert_eq!(
            scope_change_updates(Some(base), RadioScopeStreamConfig { hold: true, ..base }),
            (false, Some(true), None)
        );
        assert_eq!(
            scope_change_updates(
                Some(base),
                RadioScopeStreamConfig {
                    span_code: 3,
                    reference_tenths_db: -10,
                    ..base
                }
            ),
            (true, None, Some(-10))
        );
    }

    #[test]
    fn remote_scope_worker_handles_startup_without_blocking_core_control() {
        let (remote, client) = test_remote_radio();
        let state = Arc::new(Mutex::new(GuiState::default()));
        {
            let mut state = state.lock().expect("state lock");
            state.radio_power_on = Some(true);
            state.radio_spectrum_desired = true;
            state.radio_scope_settings_dirty = true;
        }
        let stop = Arc::new(AtomicBool::new(false));
        let sweep_abort = Arc::new(AtomicBool::new(false));
        let display_tuning = Arc::new(Mutex::new(DisplayTuning::default()));
        let repaint = Arc::new(OnceLock::new());
        let ptt_allowed = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let handle = spawn_radio_worker(
            RadioHandle::Remote(Box::new(remote)),
            state.clone(),
            stop.clone(),
            sweep_abort,
            display_tuning,
            rx,
            repaint,
            ptt_allowed,
        );

        std::thread::sleep(Duration::from_millis(1_150));
        tx.send(GuiCommand::Quit).expect("quit remote worker");
        handle.join().expect("remote worker join");
        client.shutdown().ok();

        // Joining is the assertion here: startup scope work must not strand
        // the core worker, regardless of the disconnected test endpoint's
        // final status text.
    }

    #[test]
    fn scheduled_icom_meters_normalize_each_hal_meter_destination() {
        let meters = [
            (ScheduledMeter::Signal, MeterId::Signal, 0x02),
            (ScheduledMeter::Power, MeterId::Power, 0x11),
            (ScheduledMeter::Swr, MeterId::Swr, 0x12),
            (ScheduledMeter::Alc, MeterId::Alc, 0x13),
            (ScheduledMeter::Compression, MeterId::Compression, 0x14),
            (ScheduledMeter::Current, MeterId::Current, 0x16),
            (ScheduledMeter::Voltage, MeterId::Voltage, 0x15),
            (ScheduledMeter::Temperature, MeterId::Temperature, 0x17),
        ];
        let rt = tokio::runtime::Runtime::new().expect("test runtime");

        for (scheduled, id, command) in meters {
            let model = if id == MeterId::Temperature {
                qsonaut_radio::IcomCivModel::Ic7610
            } else {
                qsonaut_radio::IcomCivModel::Ic7300
            };
            let radio = IcomCiVRadio::with_transport(
                Some(model),
                0xE0,
                0x94,
                ScriptedCiVTransport::with_frames([ScriptedCiVTransport::meter_response(command)]),
            );
            let state = Arc::new(Mutex::new(GuiState::default()));

            poll_scheduled_icom_meter(&rt, &radio, &state, scheduled);

            let state = state.lock().expect("state lock");
            let value = match id {
                MeterId::Signal => state.signal_meter,
                MeterId::Power => state.power_meter,
                MeterId::Swr => state.swr,
                MeterId::Alc => state.alc_meter,
                MeterId::Compression => state.compression_meter,
                MeterId::Current => state.current_meter,
                MeterId::Voltage => state.voltage_meter,
                MeterId::Temperature => state.temperature_meter,
            };
            assert_eq!(value, Some(48), "scheduled meter destination for {id:?}");
        }
    }

    #[test]
    fn scheduled_icom_meter_failures_and_unsupported_profiles_are_non_fatal() {
        let rt = tokio::runtime::Runtime::new().expect("test runtime");
        let failed_radio = IcomCiVRadio::with_transport(
            Some(qsonaut_radio::IcomCivModel::Ic7300),
            0xE0,
            0x94,
            ScriptedCiVTransport::with_frames([]),
        );
        let failed_state = Arc::new(Mutex::new(GuiState::default()));
        poll_scheduled_icom_meter(&rt, &failed_radio, &failed_state, ScheduledMeter::Signal);
        assert!(failed_state
            .lock()
            .expect("state lock")
            .signal_meter
            .is_none());

        let unsupported_radio =
            IcomCiVRadio::with_transport(None, 0xE0, 0x94, ScriptedCiVTransport::with_frames([]));
        let unsupported_state = Arc::new(Mutex::new(GuiState::default()));
        poll_scheduled_icom_meter(
            &rt,
            &unsupported_radio,
            &unsupported_state,
            ScheduledMeter::Signal,
        );
        assert!(unsupported_state
            .lock()
            .expect("state lock")
            .signal_meter
            .is_none());
    }

    #[test]
    fn rigctld_loopback_exercises_worker_hal_commands_without_transmit() {
        let listener = match TcpListener::bind("127.0.0.1:0") {
            Ok(listener) => listener,
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                if std::env::var_os("CI").is_some() {
                    panic!("CI runner denied loopback listener: {error}");
                }
                eprintln!("skipping loopback test: TCP listeners are unavailable");
                return;
            }
            Err(error) => panic!("loopback listener: {error}"),
        };
        listener
            .set_nonblocking(true)
            .expect("nonblocking listener");
        let address = listener.local_addr().expect("loopback address");
        let server_stop = Arc::new(AtomicBool::new(false));
        let server_stop_thread = server_stop.clone();
        let server = std::thread::spawn(move || {
            while !server_stop_thread.load(Ordering::Relaxed) {
                let Ok((mut stream, _)) = listener.accept() else {
                    std::thread::sleep(Duration::from_millis(1));
                    continue;
                };
                let mut command = String::new();
                if BufReader::new(stream.try_clone().expect("clone loopback stream"))
                    .read_line(&mut command)
                    .is_ok()
                {
                    let response = match command.trim() {
                        "f" => "14074000\n",
                        "m" => "USB 0\n",
                        _ => "RPRT 0\n",
                    };
                    stream
                        .write_all(response.as_bytes())
                        .expect("loopback response");
                }
            }
        });

        let state = Arc::new(Mutex::new(GuiState::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let sweep_abort = Arc::new(AtomicBool::new(false));
        let display_tuning = Arc::new(Mutex::new(DisplayTuning::default()));
        let repaint = Arc::new(OnceLock::new());
        let ptt_allowed = Arc::new(AtomicBool::new(true));
        let (tx, rx) = mpsc::channel();
        let handle = spawn_radio_worker(
            ConfiguredRadio::Rigctld(qsonaut_radio::rigctld::RigctldRadio::new(
                address.to_string(),
            )),
            state.clone(),
            stop,
            sweep_abort,
            display_tuning,
            rx,
            repaint,
            ptt_allowed,
        );

        tx.send(GuiCommand::TuneDelta(1_000)).expect("tune delta");
        tx.send(GuiCommand::TuneTo(14_075_000)).expect("tune to");
        tx.send(GuiCommand::CycleMode).expect("cycle mode");
        tx.send(GuiCommand::SetRadioMode(Mode::Usb))
            .expect("set mode");
        tx.send(GuiCommand::SetFilter(2))
            .expect("unsupported filter is handled");
        tx.send(GuiCommand::SetControl(ControlId::Vfo, ControlValue::Vfo(1)))
            .expect("unsupported VFO control is handled");
        tx.send(GuiCommand::AfGainDelta(5))
            .expect("unsupported AF gain is handled");
        tx.send(GuiCommand::StartTuner)
            .expect("unsupported tuner is handled");
        tx.send(GuiCommand::SetPower(true))
            .expect("unsupported power-on is handled");
        tx.send(GuiCommand::SetPtt(false)).expect("safe PTT off");
        let (ack_tx, ack_rx) = mpsc::channel();
        tx.send(GuiCommand::SetPttWithAck(false, ack_tx))
            .expect("safe acknowledged PTT off");
        tx.send(GuiCommand::SetPower(false)).expect("power command");
        assert_eq!(
            ack_rx
                .recv_timeout(Duration::from_secs(2))
                .expect("PTT ack"),
            Ok(())
        );
        tx.send(GuiCommand::Quit).expect("quit loopback worker");
        handle.join().expect("loopback worker join");
        server_stop.store(true, Ordering::Relaxed);
        server.join().expect("loopback server join");

        let state = state.lock().expect("state lock");
        assert_eq!(state.frequency_hz, Some(14_074_000));
        assert_eq!(state.mode, "USB");
        assert!(!state.ptt_on);
    }

    #[test]
    fn rigctld_loopback_surfaces_failures_without_dropping_live_state() {
        let listener = match TcpListener::bind("127.0.0.1:0") {
            Ok(listener) => listener,
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                if std::env::var_os("CI").is_some() {
                    panic!("CI runner denied loopback listener: {error}");
                }
                eprintln!("skipping loopback test: TCP listeners are unavailable");
                return;
            }
            Err(error) => panic!("loopback listener: {error}"),
        };
        listener
            .set_nonblocking(true)
            .expect("nonblocking listener");
        let address = listener.local_addr().expect("loopback address");
        let server = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(2);
            let mut index = 0;
            while Instant::now() < deadline {
                let Ok((mut stream, _)) = listener.accept() else {
                    std::thread::sleep(Duration::from_millis(1));
                    continue;
                };
                let mut command = String::new();
                BufReader::new(stream.try_clone().expect("clone loopback stream"))
                    .read_line(&mut command)
                    .expect("loopback command");
                let response = match index {
                    0 => "14074000\n",
                    1 => "USB 0\n",
                    _ => "RPRT -1\n",
                };
                stream
                    .write_all(response.as_bytes())
                    .expect("loopback error response");
                index += 1;
            }
        });

        let state = Arc::new(Mutex::new(GuiState::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let sweep_abort = Arc::new(AtomicBool::new(false));
        let display_tuning = Arc::new(Mutex::new(DisplayTuning::default()));
        let repaint = Arc::new(OnceLock::new());
        let ptt_allowed = Arc::new(AtomicBool::new(true));
        let (tx, rx) = mpsc::channel();
        let handle = spawn_radio_worker(
            ConfiguredRadio::Rigctld(qsonaut_radio::rigctld::RigctldRadio::new(
                address.to_string(),
            )),
            state.clone(),
            stop,
            sweep_abort,
            display_tuning,
            rx,
            repaint,
            ptt_allowed,
        );

        for _ in 0..100 {
            if state.lock().expect("state lock").radio_power_on == Some(true) {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        tx.send(GuiCommand::TuneTo(14_075_000))
            .expect("failing tune command");
        for _ in 0..100 {
            if state.lock().expect("state lock").last_error.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        // Exercise the worker's error handling for every non-transmit CAT
        // command while the loopback server is returning RPRT -1.  This is
        // deliberately a local rigctld endpoint; no hardware and no PTT-on
        // operation are involved.
        tx.send(GuiCommand::TuneDelta(1_000))
            .expect("failing tune delta");
        tx.send(GuiCommand::CycleMode).expect("failing mode cycle");
        tx.send(GuiCommand::SetRadioMode(Mode::Usb))
            .expect("failing mode write");
        tx.send(GuiCommand::SetPtt(false)).expect("failing PTT off");
        let (ack_tx, ack_rx) = mpsc::channel();
        tx.send(GuiCommand::SetPttWithAck(false, ack_tx))
            .expect("failing acknowledged PTT off");
        tx.send(GuiCommand::SetPower(true))
            .expect("failing power command");
        tx.send(GuiCommand::SetFilter(2))
            .expect("failing filter command");
        tx.send(GuiCommand::SetControl(
            ControlId::AfGain,
            ControlValue::U8(20),
        ))
        .expect("failing control command");
        tx.send(GuiCommand::StartTuner)
            .expect("failing tuner command");
        tx.send(GuiCommand::AfGainDelta(1))
            .expect("failing AF gain command");
        assert!(ack_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("failing PTT ack")
            .is_err());
        tx.send(GuiCommand::StartSwrSweep {
            start_hz: 14_074_000,
            stop_hz: 14_074_000,
            step_hz: 1_000,
            interval_ms: 100,
        })
        .expect("failing SWR setup");
        tx.send(GuiCommand::Quit).expect("quit error worker");
        handle.join().expect("error worker join");
        server.join().expect("error server join");

        let state = state.lock().expect("state lock");
        // A transient failed response after a known-good connection does not
        // falsely report a power-off transition; the error remains visible.
        assert_eq!(state.radio_power_on, Some(true));
        assert!(state.last_error.is_some());
        assert!(!state.ptt_on);
    }

    #[test]
    fn null_radio_worker_handles_safe_control_commands_without_transmit() {
        let state = Arc::new(Mutex::new(GuiState::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let sweep_abort = Arc::new(AtomicBool::new(false));
        let display_tuning = Arc::new(Mutex::new(DisplayTuning::default()));
        let repaint = Arc::new(OnceLock::new());
        let ptt_allowed = Arc::new(AtomicBool::new(true));
        let (tx, rx) = mpsc::channel();
        let handle = spawn_radio_worker(
            ConfiguredRadio::Null(qsonaut_radio::NullRadio::with_frequency_mode(
                7_074_000,
                Mode::Usb,
            )),
            state.clone(),
            stop,
            sweep_abort,
            display_tuning,
            rx,
            repaint,
            ptt_allowed,
        );

        tx.send(GuiCommand::TuneDelta(1_000)).expect("tune delta");
        tx.send(GuiCommand::TuneTo(7_100_000)).expect("tune to");
        tx.send(GuiCommand::CycleMode).expect("cycle mode");
        tx.send(GuiCommand::SetControl(
            ControlId::RfPower,
            ControlValue::U8(50),
        ))
        .expect("set control");
        tx.send(GuiCommand::ApplyWorkspace {
            mode: WorkspaceMode::Voice,
            frequency_hz: 7_100_000,
        })
        .expect("apply workspace");
        for mode in [
            WorkspaceMode::Ft8,
            WorkspaceMode::Ft4,
            WorkspaceMode::Fst4,
            WorkspaceMode::Wspr,
            WorkspaceMode::Jt9,
            WorkspaceMode::Jt65,
            WorkspaceMode::Q65,
            WorkspaceMode::Msk144,
            WorkspaceMode::Cw,
            WorkspaceMode::Sstv,
        ] {
            tx.send(GuiCommand::ApplyWorkspace {
                mode,
                frequency_hz: 7_101_000,
            })
            .expect("apply workspace mode");
        }
        tx.send(GuiCommand::SetRadioMode(Mode::Data))
            .expect("set mode");
        tx.send(GuiCommand::SetFilter(3)).expect("set filter");
        tx.send(GuiCommand::AfGainDelta(-10))
            .expect("adjust AF gain");
        tx.send(GuiCommand::SetPtt(false)).expect("disable ptt");
        let (ack_tx, ack_rx) = mpsc::channel();
        tx.send(GuiCommand::SetPttWithAck(false, ack_tx))
            .expect("disable ptt with ack");
        tx.send(GuiCommand::StartSwrSweep {
            start_hz: 7_100_000,
            stop_hz: 7_100_000,
            step_hz: 0,
            interval_ms: 100,
        })
        .expect("reject invalid sweep");
        tx.send(GuiCommand::StartSwrSweep {
            start_hz: 7_101_000,
            stop_hz: 7_100_000,
            step_hz: 1_000,
            interval_ms: 100,
        })
        .expect("reject reversed sweep");
        tx.send(GuiCommand::StartTuner).expect("start tuner");
        tx.send(GuiCommand::SetPower(false)).expect("power off");
        assert_eq!(ack_rx.recv().expect("PTT acknowledgement"), Ok(()));
        tx.send(GuiCommand::Quit).expect("quit");
        handle.join().expect("radio worker should stop cleanly");

        let state = state.lock().expect("state lock");
        assert_eq!(state.frequency_hz, Some(7_101_000));
        assert_eq!(state.mode, "USB");
        assert!(!state.ptt_on);
    }

    #[test]
    fn null_radio_worker_exits_when_session_command_channel_disconnects() {
        let state = Arc::new(Mutex::new(GuiState::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let sweep_abort = Arc::new(AtomicBool::new(false));
        let display_tuning = Arc::new(Mutex::new(DisplayTuning::default()));
        let repaint = Arc::new(OnceLock::new());
        let ptt_allowed = Arc::new(AtomicBool::new(false));
        let (tx, rx) = mpsc::channel();
        let handle = spawn_radio_worker(
            ConfiguredRadio::Null(qsonaut_radio::NullRadio::new()),
            state.clone(),
            stop,
            sweep_abort,
            display_tuning,
            rx,
            repaint,
            ptt_allowed,
        );
        drop(tx);

        handle
            .join()
            .expect("worker should stop when its session closes");
        let state = state.lock().expect("state lock");
        assert!(!state.ptt_on);
        assert!(state.supported_controls.is_empty());
    }

    #[test]
    fn null_radio_swr_sweep_restores_state_without_hardware_transmit() {
        let state = Arc::new(Mutex::new(GuiState::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let sweep_abort = Arc::new(AtomicBool::new(false));
        let display_tuning = Arc::new(Mutex::new(DisplayTuning::default()));
        let repaint = Arc::new(OnceLock::new());
        let ptt_allowed = Arc::new(AtomicBool::new(true));
        let (tx, rx) = mpsc::channel();
        let handle = spawn_radio_worker(
            ConfiguredRadio::Null(qsonaut_radio::NullRadio::with_frequency_mode(
                7_074_000,
                Mode::Usb,
            )),
            state.clone(),
            stop,
            sweep_abort,
            display_tuning,
            rx,
            repaint,
            ptt_allowed,
        );

        tx.send(GuiCommand::StartSwrSweep {
            start_hz: 7_074_000,
            stop_hz: 7_074_000,
            step_hz: 1_000,
            interval_ms: 100,
        })
        .expect("simulated SWR sweep");
        for _ in 0..500 {
            if state
                .lock()
                .expect("state lock")
                .swr_sweep_status
                .contains("read failures")
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        tx.send(GuiCommand::Quit).expect("quit simulated sweep");
        handle.join().expect("simulated sweep worker join");

        let state = state.lock().expect("state lock");
        assert_eq!(state.frequency_hz, Some(7_074_000));
        assert_eq!(state.mode, "USB");
        assert!(!state.ptt_on);
        assert!(!state.swr_sweep_active);
        assert_eq!(state.swr_sweep_status, "Unsupported by radio driver");
    }

    #[test]
    fn worker_rejects_ptt_when_radio_is_unavailable_or_profile_is_inactive() {
        let state = Arc::new(Mutex::new(GuiState::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let sweep_abort = Arc::new(AtomicBool::new(false));
        let display_tuning = Arc::new(Mutex::new(DisplayTuning::default()));
        let repaint = Arc::new(OnceLock::new());
        let ptt_allowed = Arc::new(AtomicBool::new(true));
        let (tx, rx) = mpsc::channel();
        let handle = spawn_radio_worker(
            ConfiguredRadio::Null(qsonaut_radio::NullRadio::with_frequency_mode(
                7_074_000,
                Mode::Usb,
            )),
            state.clone(),
            stop,
            sweep_abort,
            display_tuning,
            rx,
            repaint,
            ptt_allowed.clone(),
        );

        for _ in 0..100 {
            if state.lock().expect("state lock").radio_power_on == Some(true) {
                break;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        state.lock().expect("state lock").radio_power_on = Some(false);
        let (unavailable_tx, unavailable_rx) = mpsc::channel();
        tx.send(GuiCommand::SetPttWithAck(true, unavailable_tx))
            .expect("unavailable PTT command");
        assert_eq!(
            unavailable_rx
                .recv_timeout(Duration::from_secs(1))
                .expect("unavailable PTT response")
                .expect_err("unavailable radio must reject PTT"),
            "radio is powered off"
        );
        assert_eq!(
            state.lock().expect("state lock").last_error.as_deref(),
            Some("radio command skipped: radio is unavailable")
        );

        state.lock().expect("state lock").radio_power_on = Some(true);
        ptt_allowed.store(false, Ordering::Release);
        let (inactive_tx, inactive_rx) = mpsc::channel();
        tx.send(GuiCommand::SetPttWithAck(true, inactive_tx))
            .expect("inactive profile PTT command");
        assert_eq!(
            inactive_rx
                .recv_timeout(Duration::from_secs(1))
                .expect("inactive PTT response")
                .expect_err("inactive profile must reject PTT"),
            "PTT is disabled while this radio profile is inactive"
        );
        tx.send(GuiCommand::Quit).expect("quit worker");
        handle.join().expect("worker join");
        assert!(!state.lock().expect("state lock").ptt_on);
    }
}
