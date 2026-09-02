use super::super::*;

impl QsonautGuiApp {
    pub(crate) fn restart_psk_reporter(&mut self) {
        self.psk_reporter = None;
        self.psk_reporter = start_psk_reporter(
            self.psk_reporter_enabled,
            &self.config.radio.backend,
            self.station_callsign.trim(),
            self.station_grid.trim(),
            ReporterTuning {
                batch_interval_secs: self.psk_batch_interval_secs,
                repeat_cache_secs: self.psk_repeat_cache_secs,
                max_pending: self.psk_max_pending,
            },
            &self.state,
        );
        let sender = self.psk_reporter.as_ref().map(Reporter::sender);
        for session in self.parked_radio_sessions.values() {
            if let Ok(mut state) = session.state.lock() {
                state.psk_report_sender = (!matches!(
                    session.config.backend.trim().to_ascii_lowercase().as_str(),
                    "null" | "mock" | "none"
                ))
                .then(|| sender.clone())
                .flatten();
            }
        }
    }
}

pub(crate) fn start_psk_reporter(
    enabled: bool,
    backend: &str,
    callsign: &str,
    grid: &str,
    tuning: ReporterTuning,
    state: &Arc<Mutex<GuiState>>,
) -> Option<Reporter> {
    state
        .lock()
        .expect("ui state lock poisoned")
        .psk_report_sender = None;
    if !enabled
        || matches!(
            backend.trim().to_ascii_lowercase().as_str(),
            "null" | "mock" | "none"
        )
        || callsign.trim().is_empty()
        || callsign == "N0CALL"
        || grid == "AA00"
    {
        info!(enabled, backend, callsign = %callsign, grid = %grid, "PSK Reporter not started: reporting is disabled, simulated, or station identity is incomplete");
        return None;
    }
    let mut config = ReporterConfig::production(callsign, grid);
    config.tuning = tuning;
    let reporter = Reporter::start(config);
    info!(callsign = %callsign, grid = %grid, "PSK Reporter started");
    state
        .lock()
        .expect("ui state lock poisoned")
        .psk_report_sender = Some(reporter.sender());
    Some(reporter)
}

pub(crate) fn submit_psk_report(
    sender: &Option<ReportSender>,
    dial_frequency_hz: Option<u64>,
    audio_frequency_hz: u32,
    snr_db: f32,
    message: &str,
    mode: &str,
    received_at: u32,
) {
    let Some(sender) = sender else { return };
    let parsed = parse_message(message);
    let sender_locator = parsed
        .as_ref()
        .and_then(|parsed| match &parsed.exchange {
            Exchange::Grid(grid) => Some(grid.clone()),
            _ => None,
        })
        .unwrap_or_default();
    let callsign = parsed.map(|parsed| parsed.from).or_else(|| {
        message
            .split_whitespace()
            .find(|token| is_probable_callsign(token))
            .map(|token| token.trim_matches(['<', '>']).to_ascii_uppercase())
    });
    let (Some(callsign), Some(dial)) = (callsign, dial_frequency_hz) else {
        return;
    };
    let frequency_hz = dial.saturating_add(u64::from(audio_frequency_hz));
    let callsign_for_log = callsign.clone();
    let queued = sender.submit(ReceptionReport {
        sender_callsign: callsign,
        frequency_hz,
        snr_db: snr_db.round().clamp(-127.0, 127.0) as i8,
        mode: mode.to_string(),
        sender_locator,
        received_at,
    });
    if queued {
        debug!(callsign = %callsign_for_log, frequency_hz, mode = %mode, "PSK Reporter reception report queued");
    } else {
        warn!(callsign = %callsign_for_log, mode = %mode, "PSK Reporter reception report rejected: worker unavailable");
    }
}
