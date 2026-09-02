use super::super::*;
use std::sync::mpsc::{Receiver, Sender};
use std::thread::JoinHandle;

impl QsonautGuiApp {
    pub(in super::super) fn cat_connection_result(
        backend: &str,
        model: &str,
        port: &str,
        baud_rate: u32,
        controller_civ_address: u8,
        radio_civ_address: u8,
    ) -> Result<String, String> {
        if !backend.eq_ignore_ascii_case("native") {
            Err(format!(
                "CAT test requires the Native Rigwright backend; selected backend is '{}'.",
                backend
            ))
        } else if port.is_empty() {
            Err(
                "CAT test requires a specific serial port; select a radio USB/serial port first."
                    .into(),
            )
        } else {
            match open_model_with_radio_address(
                model,
                port,
                baud_rate,
                controller_civ_address,
                Some(radio_civ_address),
            ) {
                Ok(radio) => Self::cat_probe_result(model, baud_rate, radio),
                Err(error) => Err(format!(
                    "Could not open CAT port '{}' at {} baud for {}: {error}",
                    port, baud_rate, model
                )),
            }
        }
    }

    pub(in super::super) fn cat_probe_result(
        model: &str,
        baud_rate: u32,
        radio: ConfiguredRadio,
    ) -> Result<String, String> {
        match radio {
            ConfiguredRadio::Yaesu(yaesu) => match yaesu.verify_model() {
                Ok(()) => Ok(format!(
                    "CAT OK: {} answered and matched the selected profile at {} baud.",
                    model, baud_rate
                )),
                Err(error) => Err(format!(
                    "CAT probe failed for {} at {} baud: {error}",
                    model, baud_rate
                )),
            },
            ConfiguredRadio::Kenwood(kenwood) => match kenwood.verify_model() {
                Ok(()) => Ok(format!(
                    "CAT OK: {} answered and matched the selected profile at {} baud.",
                    model, baud_rate
                )),
                Err(error) => Err(format!(
                    "CAT probe failed for {} at {} baud: {error}",
                    model, baud_rate
                )),
            },
            ConfiguredRadio::Icom(icom) => {
                match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => match runtime.block_on(Radio::get_frequency_hz(&icom)) {
                        Ok(frequency_hz) => Ok(format!(
                            "CAT OK: {} answered at {} baud (frequency {} Hz).",
                            model, baud_rate, frequency_hz
                        )),
                        Err(error) => Err(format!(
                            "CAT probe failed for {} at {} baud: {error}",
                            model, baud_rate
                        )),
                    },
                    Err(error) => Err(format!("CAT test runtime failed: {error}")),
                }
            }
            _ => Err(format!(
                "CAT testing is not implemented for the selected native profile '{}'.",
                model
            )),
        }
    }
}

pub(in super::super) fn stop_radio_worker_for_reconnect(
    command_tx: &mut Option<Sender<GuiCommand>>,
    worker_stop: &mut Arc<AtomicBool>,
    worker_handle: &mut Option<JoinHandle<()>>,
) {
    let started = Instant::now();
    if let Some(tx) = command_tx {
        let _ = tx.send(GuiCommand::Quit);
    }
    worker_stop.store(true, Ordering::Relaxed);
    if let Some(handle) = worker_handle.take() {
        // A synchronous CI-V read can outlive the UI frame. Do not make the
        // UI wait indefinitely for a worker that is already being stopped.
        let (done_tx, done_rx) = mpsc::channel();
        thread::spawn(move || {
            let _ = handle.join();
            let _ = done_tx.send(());
        });
        if done_rx.recv_timeout(Duration::from_secs(3)).is_err() {
            warn!(elapsed = ?started.elapsed(), "Radio worker did not stop within reconnect timeout; abandoning join");
        } else {
            info!(elapsed = ?started.elapsed(), "Radio worker stopped for reconnect");
        }
    }
    *command_tx = None;
    *worker_stop = Arc::new(AtomicBool::new(false));
}

pub(in super::super) fn mark_radio_reconnect_disabled(
    state: &Arc<Mutex<GuiState>>,
    radio_init_rx: &mut Option<Receiver<Option<ConfiguredRadio>>>,
    radio_init_attempted: &mut bool,
) {
    *radio_init_rx = None;
    *radio_init_attempted = true;
    let mut state = state.lock().expect("ui state lock poisoned");
    state.radio_waterfall_status = "UNAVAILABLE (radio disabled)".to_string();
}

pub(in super::super) fn apply_radio_reconnect<Init, Restart>(
    radio_init_rx: &mut Option<Receiver<Option<ConfiguredRadio>>>,
    radio_init_attempted: &mut bool,
    device_restart_required: &mut bool,
    state: &Arc<Mutex<GuiState>>,
    should_restart_audio: bool,
    init: Init,
    restart_audio: Restart,
) where
    Init: FnOnce() -> Receiver<Option<ConfiguredRadio>>,
    Restart: FnOnce(),
{
    *radio_init_rx = Some(init());
    *radio_init_attempted = false;
    *device_restart_required = false;
    if should_restart_audio {
        restart_audio();
    }
    let mut state = state.lock().expect("ui state lock poisoned");
    state.radio_waterfall_status = "CONNECTING…".to_string();
    state.last_error = None;
}

pub(in super::super) fn spawn_cat_connection_test(
    backend: String,
    model: String,
    port: String,
    baud_rate: u32,
    controller_civ_address: u8,
    radio_civ_address: u8,
) -> Receiver<Result<String, String>> {
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let result = QsonautGuiApp::cat_connection_result(
            &backend,
            &model,
            &port,
            baud_rate,
            controller_civ_address,
            radio_civ_address,
        );

        match &result {
            Ok(message) => info!(
                model = %model,
                port = %port,
                baud = baud_rate,
                result = %message,
                "CAT connection test succeeded"
            ),
            Err(error) => warn!(
                model = %model,
                port = %port,
                baud = baud_rate,
                error = %error,
                "CAT connection test failed"
            ),
        }
        let _ = tx.send(result);
    });
    rx
}
