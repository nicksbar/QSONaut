use super::*;
use qsonaut_third_party::wsjt::{
    synthesize_fst4_standard, synthesize_ft4_standard, synthesize_ft8_standard,
    synthesize_jt65_standard, synthesize_jt9_standard, synthesize_q65_standard,
    synthesize_wspr_type1, Fst4Submode, Q65Submode,
};

const FT8_TX_AMPLITUDE_I16: i16 = 18_000;
const FT8_TX_SAMPLE_RATE_HZ: u32 = 12_000;
const FT8_TX_MONITOR_FFT_SIZE: usize = 2_048;
const FT8_TX_MONITOR_HOP_SAMPLES: usize = 500;
pub(super) const FT8_TX_AUDIO_START_S: f64 = modes::exchange::AUDIO_START_SECONDS;
const FT8_MAX_AUDIO_LATE_S: f64 = 1.75;
const DIGITAL_MAX_AUDIO_LATE_S: f64 = 1.0;

pub(super) fn build_ft8_tx_pcm(compose: &str, tx_tone_hz: u32) -> Result<Vec<i16>> {
    synthesize_ft8_standard(compose, tx_tone_hz as f32, FT8_TX_AMPLITUDE_I16)
        .ok_or_else(|| anyhow!("unable to pack FT8 standard message: {compose}"))
}

pub(super) fn build_native_digital_tx_pcm(
    mode: WorkspaceMode,
    compose: &str,
    tx_tone_hz: u32,
    fst4_submode: crate::modes::fst4::Submode,
    cw_wpm: u8,
    cw_tone_hz: u16,
) -> Result<(Vec<i16>, f64)> {
    let tokens: Vec<&str> = compose.split_whitespace().collect();
    if mode != WorkspaceMode::Cw && tokens.len() != 3 {
        anyhow::bail!("{} TX needs exactly 3 message fields", mode.label());
    }
    let tone = tx_tone_hz as f32;
    match mode {
        WorkspaceMode::Ft4 | WorkspaceMode::Fst4 => {
            if mode == WorkspaceMode::Ft4 {
                Ok((
                    synthesize_ft4_standard(compose, tone, FT8_TX_AMPLITUDE_I16)
                        .ok_or_else(|| anyhow!("unable to pack FT4 message"))?,
                    0.5,
                ))
            } else {
                let submode = match fst4_submode {
                    crate::modes::fst4::Submode::S15 => Fst4Submode::S15,
                    crate::modes::fst4::Submode::S30 => Fst4Submode::S30,
                    crate::modes::fst4::Submode::S60 => Fst4Submode::S60,
                    crate::modes::fst4::Submode::S120 => Fst4Submode::S120,
                    crate::modes::fst4::Submode::S300 => Fst4Submode::S300,
                };
                let pcm = synthesize_fst4_standard(compose, submode, tone, FT8_TX_AMPLITUDE_I16)
                    .ok_or_else(|| anyhow!("unable to pack FST4 message"))?;
                Ok((
                    pcm,
                    match fst4_submode {
                        crate::modes::fst4::Submode::S15 => 0.5,
                        _ => 1.0,
                    },
                ))
            }
        }
        WorkspaceMode::Jt9 => synthesize_jt9_standard(compose, tone, FT8_TX_AMPLITUDE_I16)
            .map(|audio| (audio, 0.0))
            .ok_or_else(|| anyhow!("unable to pack JT9 message")),
        WorkspaceMode::Jt65 => synthesize_jt65_standard(compose, tone, FT8_TX_AMPLITUDE_I16)
            .map(|audio| (audio, 0.0))
            .ok_or_else(|| anyhow!("unable to pack JT65 message")),
        WorkspaceMode::Q65 => {
            synthesize_q65_standard(compose, Q65Submode::A30, tone, FT8_TX_AMPLITUDE_I16)
                .map(|audio| (audio, 1.0))
                .ok_or_else(|| anyhow!("unable to pack Q65 message"))
        }
        WorkspaceMode::Wspr => {
            tokens
                .get(2)
                .ok_or_else(|| anyhow!("WSPR TX requires CALL GRID POWER_DBM"))?
                .parse::<i32>()
                .map_err(|_| anyhow!("WSPR power must be an integer dBm value"))?;
            let audio = synthesize_wspr_type1(compose, tone, FT8_TX_AMPLITUDE_I16)
                .ok_or_else(|| anyhow!("invalid WSPR callsign, locator, or power"))?;
            Ok((audio, 0.0))
        }
        WorkspaceMode::Cw => {
            let text = compose.trim().to_ascii_uppercase();
            if text.is_empty() {
                anyhow::bail!("CW TX requires text");
            }
            if let Some(character) = text
                .chars()
                .find(|character| !character.is_ascii_alphanumeric() && !character.is_whitespace())
            {
                anyhow::bail!(
                    "CW TX does not support '{character}'; use A-Z, 0-9, and spaces only"
                );
            }
            let pcm = synthesize_cw_pcm(
                &text,
                f32::from(cw_tone_hz.clamp(200, 3_000)),
                f32::from(cw_wpm.clamp(5, 40)),
            )?;
            if pcm.is_empty() {
                anyhow::bail!("CW TX did not produce audio");
            }
            // CW uses explicit immediate timing, not a slot-relative offset.
            Ok((pcm, 0.0))
        }
        _ => anyhow::bail!("{} transmit synthesis is not available", mode.label()),
    }
}

fn synthesize_cw_pcm(text: &str, tone_hz: f32, wpm: f32) -> Result<Vec<i16>> {
    let dot_samples = (FT8_TX_SAMPLE_RATE_HZ as f32 * 1.2 / wpm).round() as usize;
    let mut pcm = Vec::new();
    for (word_index, word) in text.split_whitespace().enumerate() {
        for (char_index, character) in word.chars().enumerate() {
            let pattern = morse_pattern(character)
                .ok_or_else(|| anyhow!("unsupported CW character '{character}'"))?;
            for (element_index, element) in pattern.chars().enumerate() {
                let length = if element == '-' {
                    dot_samples * 3
                } else {
                    dot_samples
                };
                for index in 0..length {
                    let phase = 2.0 * std::f32::consts::PI * tone_hz * index as f32
                        / FT8_TX_SAMPLE_RATE_HZ as f32;
                    pcm.push((phase.sin() * FT8_TX_AMPLITUDE_I16 as f32).round() as i16);
                }
                if element_index + 1 < pattern.len() {
                    pcm.extend(std::iter::repeat_n(0, dot_samples));
                }
            }
            if char_index + 1 < word.len() {
                pcm.extend(std::iter::repeat_n(0, dot_samples * 3));
            }
        }
        if word_index + 1 < text.split_whitespace().count() {
            pcm.extend(std::iter::repeat_n(0, dot_samples * 7));
        }
    }
    // Give the streaming decoder enough trailing carrier-off time to close
    // the final character. Without this tail, the last character remains in
    // the decoder's pending run (for example, N7U instead of N7UF).
    pcm.extend(std::iter::repeat_n(0, FT8_TX_SAMPLE_RATE_HZ as usize * 2));
    Ok(pcm)
}

fn morse_pattern(character: char) -> Option<&'static str> {
    Some(match character {
        'A' => ".-",
        'B' => "-...",
        'C' => "-.-.",
        'D' => "-..",
        'E' => ".",
        'F' => "..-.",
        'G' => "--.",
        'H' => "....",
        'I' => "..",
        'J' => ".---",
        'K' => "-.-",
        'L' => ".-..",
        'M' => "--",
        'N' => "-.",
        'O' => "---",
        'P' => ".--.",
        'Q' => "--.-",
        'R' => ".-.",
        'S' => "...",
        'T' => "-",
        'U' => "..-",
        'V' => "...-",
        'W' => ".--",
        'X' => "-..-",
        'Y' => "-.--",
        'Z' => "--..",
        '0' => "-----",
        '1' => ".----",
        '2' => "..---",
        '3' => "...--",
        '4' => "....-",
        '5' => ".....",
        '6' => "-....",
        '7' => "--...",
        '8' => "---..",
        '9' => "----.",
        _ => return None,
    })
}

fn play_ft8_tx_pcm(pcm: &[i16], abort: Arc<AtomicBool>, output_device: Option<&str>) -> Result<()> {
    if output_device.is_some_and(|device| device.starts_with("hostbridge://")) {
        if abort.load(Ordering::Relaxed) {
            anyhow::bail!("TX aborted by operator");
        }
        return super::hostbridge_radio::send_remote_pcm(pcm, FT8_TX_SAMPLE_RATE_HZ)
            .context("HostBridge audio output failed");
    }
    play_pcm_blocking(pcm, FT8_TX_SAMPLE_RATE_HZ, output_device, abort)
        .context("native audio output failed")
}

fn wait_until_epoch(target_s: f64, abort: &AtomicBool) -> Result<()> {
    loop {
        if abort.load(Ordering::Relaxed) {
            anyhow::bail!("TX aborted by operator");
        }
        let now_s = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs_f64())
            .unwrap_or(0.0);
        let remaining = target_s - now_s;
        if remaining <= 0.0 {
            return Ok(());
        }
        thread::sleep(Duration::from_secs_f64(remaining.min(0.025)));
    }
}

fn request_ptt(
    command_tx: &mpsc::Sender<GuiCommand>,
    enabled: bool,
    timeout: Duration,
) -> Result<()> {
    let (ack_tx, ack_rx) = mpsc::channel();
    command_tx
        .send(GuiCommand::SetPttWithAck(enabled, ack_tx))
        .context("radio command worker is unavailable")?;
    match ack_rx.recv_timeout(timeout) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(anyhow!(error)),
        Err(_) => anyhow::bail!(
            "radio did not confirm PTT {}",
            if enabled { "ON" } else { "OFF" }
        ),
    }
}

pub(super) struct Ft8TxJob {
    pub(super) period: u64,
    pub(super) pcm: Arc<Vec<i16>>,
    pub(super) ptt_lead: Duration,
    pub(super) ptt_tail: Duration,
    pub(super) output_device: Option<String>,
    pub(super) abort: Arc<AtomicBool>,
    pub(super) active: Arc<AtomicBool>,
    pub(super) command_tx: mpsc::Sender<GuiCommand>,
    pub(super) event_tx: mpsc::Sender<Ft8TxEvent>,
    pub(super) state: Arc<Mutex<GuiState>>,
    pub(super) repaint_ctx: Arc<OnceLock<egui::Context>>,
}

pub(super) fn run_ft8_tx_job(job: Ft8TxJob) {
    let slot_start_s = job.period as f64 * modes::exchange::SLOT_SECONDS;
    let audio_start_s = slot_start_s + FT8_TX_AUDIO_START_S;
    let ptt_start_s = audio_start_s - job.ptt_lead.as_secs_f64();

    let result = (|| -> Result<()> {
        wait_until_epoch(ptt_start_s, &job.abort)?;
        request_ptt(&job.command_tx, true, Duration::from_secs(2))?;
        let _ = job.event_tx.send(Ft8TxEvent::PttConfirmed);

        wait_until_epoch(audio_start_s, &job.abort)?;
        let now_s = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs_f64())
            .unwrap_or(audio_start_s);
        let audio_late_s = now_s - audio_start_s;
        if audio_late_s > FT8_MAX_AUDIO_LATE_S {
            anyhow::bail!("PTT confirmation arrived too late for a valid FT8 frame");
        }
        info!(
            period = job.period,
            audio_late_ms = (audio_late_s.max(0.0) * 1_000.0).round() as u64,
            "FT8 TX audio starting"
        );

        let _ = job.event_tx.send(Ft8TxEvent::AudioStarted);
        let monitor_stop = Arc::new(AtomicBool::new(false));
        let monitor_handle = {
            let pcm = job.pcm.clone();
            let stop = monitor_stop.clone();
            let abort = job.abort.clone();
            let state = job.state.clone();
            let repaint_ctx = job.repaint_ctx.clone();
            thread::spawn(move || monitor_ft8_tx_waterfall(pcm, stop, abort, state, repaint_ctx))
        };
        let playback_result =
            play_ft8_tx_pcm(&job.pcm, job.abort.clone(), job.output_device.as_deref());
        monitor_stop.store(true, Ordering::Release);
        let _ = monitor_handle.join();
        playback_result?;
        if !job.ptt_tail.is_zero() {
            thread::sleep(job.ptt_tail);
        }
        Ok(())
    })();

    // PTT release is unconditional, including playback, timeout, and abort failures.
    let unkey_result = request_ptt(&job.command_tx, false, Duration::from_secs(2));
    job.active.store(false, Ordering::Release);

    match (result, unkey_result) {
        (Ok(()), Ok(())) => {
            let _ = job.event_tx.send(Ft8TxEvent::Complete);
        }
        (Err(error), Ok(())) => {
            let _ = job.event_tx.send(Ft8TxEvent::Failed(error.to_string()));
        }
        (Ok(()), Err(error)) => {
            let _ = job.event_tx.send(Ft8TxEvent::Failed(format!(
                "TX audio completed but PTT release failed: {error}"
            )));
        }
        (Err(error), Err(unkey_error)) => {
            let _ = job.event_tx.send(Ft8TxEvent::Failed(format!(
                "{error}; PTT release also failed: {unkey_error}"
            )));
        }
    }
}

fn monitor_ft8_tx_waterfall(
    pcm: Arc<Vec<i16>>,
    stop: Arc<AtomicBool>,
    abort: Arc<AtomicBool>,
    state: Arc<Mutex<GuiState>>,
    repaint_ctx: Arc<OnceLock<egui::Context>>,
) {
    let mut planner = FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(FT8_TX_MONITOR_FFT_SIZE);
    let mut fft_buf = vec![Complex::<f32>::new(0.0, 0.0); FT8_TX_MONITOR_FFT_SIZE];
    let started = Instant::now();

    for start in (0..pcm.len()).step_by(FT8_TX_MONITOR_HOP_SAMPLES) {
        let target = Duration::from_secs_f64(start as f64 / FT8_TX_SAMPLE_RATE_HZ as f64);
        while started.elapsed() < target {
            if stop.load(Ordering::Acquire) || abort.load(Ordering::Relaxed) {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        if stop.load(Ordering::Acquire) || abort.load(Ordering::Relaxed) {
            return;
        }

        for (offset, sample) in fft_buf.iter_mut().enumerate() {
            let value =
                pcm.get(start + offset).copied().unwrap_or_default() as f32 / i16::MAX as f32;
            let window =
                0.5 - 0.5 * (2.0 * PI * offset as f32 / (FT8_TX_MONITOR_FFT_SIZE - 1) as f32).cos();
            *sample = Complex::new(value * window, 0.0);
        }
        fft.process(&mut fft_buf);
        let bins = fft_buffer_to_display_bins(&fft_buf, AUDIO_BINS, FT8_TX_SAMPLE_RATE_HZ);
        let mut snapshot = state.lock().expect("ui state lock poisoned");
        if snapshot.audio_waterfall_rows.len() >= AUDIO_WF_HEIGHT {
            snapshot.audio_waterfall_rows.pop_front();
        }
        snapshot.audio_waterfall_rows.push_back(bins);
        snapshot.audio_waterfall_revision = snapshot.audio_waterfall_revision.wrapping_add(1);
        snapshot.audio_spectrum_status = "TX OUTPUT".to_string();
        drop(snapshot);
        if let Some(ctx) = repaint_ctx.get() {
            ctx.request_repaint();
        }
    }
}

pub(super) struct DigitalTxJob {
    pub(super) mode: WorkspaceMode,
    pub(super) period: u64,
    pub(super) slot_seconds: f64,
    pub(super) audio_offset_s: f64,
    pub(super) audio_start_s: Option<f64>,
    pub(super) pcm: Arc<Vec<i16>>,
    pub(super) ptt_lead: Duration,
    pub(super) ptt_tail: Duration,
    pub(super) output_device: Option<String>,
    pub(super) abort: Arc<AtomicBool>,
    pub(super) active: Arc<AtomicBool>,
    pub(super) command_tx: mpsc::Sender<GuiCommand>,
    pub(super) event_tx: mpsc::Sender<DigitalTxEvent>,
    pub(super) state: Arc<Mutex<GuiState>>,
    pub(super) repaint_ctx: Arc<OnceLock<egui::Context>>,
}

pub(super) fn run_digital_tx_job(job: DigitalTxJob) {
    let audio_start_s = job
        .audio_start_s
        .unwrap_or(job.period as f64 * job.slot_seconds + job.audio_offset_s);
    let ptt_start_s = audio_start_s - job.ptt_lead.as_secs_f64();
    let result = (|| -> Result<()> {
        wait_until_epoch(ptt_start_s, &job.abort)?;
        request_ptt(&job.command_tx, true, Duration::from_secs(2))?;
        wait_until_epoch(audio_start_s, &job.abort)?;
        let now_s = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs_f64())
            .unwrap_or(audio_start_s);
        let audio_late_s = now_s - audio_start_s;
        if !matches!(job.mode, WorkspaceMode::Cw | WorkspaceMode::Sstv)
            && audio_late_s > DIGITAL_MAX_AUDIO_LATE_S
        {
            anyhow::bail!(
                "{} audio arrived too late for a valid slot ({:.0} ms)",
                job.mode.label(),
                audio_late_s * 1_000.0
            );
        }
        info!(
            mode = job.mode.label(),
            period = job.period,
            audio_late_ms = (audio_late_s.max(0.0) * 1_000.0).round() as u64,
            "digital TX audio starting"
        );
        let _ = job
            .event_tx
            .send(DigitalTxEvent::AudioStarted(job.mode, job.period));
        let monitor_stop = Arc::new(AtomicBool::new(false));
        let monitor_handle = {
            let pcm = job.pcm.clone();
            let stop = monitor_stop.clone();
            let abort = job.abort.clone();
            let state = job.state.clone();
            let repaint_ctx = job.repaint_ctx.clone();
            thread::spawn(move || monitor_ft8_tx_waterfall(pcm, stop, abort, state, repaint_ctx))
        };
        let playback_result =
            play_ft8_tx_pcm(&job.pcm, job.abort.clone(), job.output_device.as_deref());
        monitor_stop.store(true, Ordering::Release);
        let _ = monitor_handle.join();
        playback_result?;
        if !job.ptt_tail.is_zero() {
            thread::sleep(job.ptt_tail);
        }
        Ok(())
    })();

    let unkey_result = request_ptt(&job.command_tx, false, Duration::from_secs(2));
    job.active.store(false, Ordering::Release);
    match (result, unkey_result) {
        (Ok(()), Ok(())) => {
            let _ = job.event_tx.send(DigitalTxEvent::Complete);
        }
        (Err(error), Ok(())) => {
            let _ = job.event_tx.send(DigitalTxEvent::Failed(error.to_string()));
        }
        (Ok(()), Err(error)) => {
            let _ = job.event_tx.send(DigitalTxEvent::Failed(format!(
                "audio completed but PTT release failed: {error}"
            )));
        }
        (Err(error), Err(unkey_error)) => {
            let _ = job.event_tx.send(DigitalTxEvent::Failed(format!(
                "{error}; PTT release also failed: {unkey_error}"
            )));
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct Ft8TxChatEntry {
    pub(super) period: u64,
    pub(super) utc: String,
    pub(super) message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Ft8ChatDirection {
    Rx,
    Tx,
}

#[derive(Debug, Clone)]
pub(super) struct Ft8ChatLine {
    pub(super) period: u64,
    pub(super) utc: String,
    pub(super) message: String,
    pub(super) detail: String,
    pub(super) direction: Ft8ChatDirection,
}

#[derive(Debug, Clone)]
pub(super) struct DigitalTxChatEntry {
    pub(super) mode: WorkspaceMode,
    pub(super) period: u64,
    pub(super) utc: String,
    pub(super) message: String,
}

#[derive(Debug)]
pub(super) enum Ft8TxEvent {
    PttConfirmed,
    AudioStarted,
    Complete,
    Failed(String),
}

#[derive(Debug)]
pub(super) enum DigitalTxEvent {
    AudioStarted(WorkspaceMode, u64),
    Complete,
    Failed(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use qsonaut_modems::AudioBlock;
    use qsonaut_third_party::wsjt::{decode as decode_wsjt, WsjtDecodeConfig, WsjtMode};

    #[test]
    fn morse_table_covers_alphanumeric_operator_text() {
        for character in "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789".chars() {
            assert!(morse_pattern(character).is_some(), "missing {character}");
        }
        assert_eq!(morse_pattern('?'), None);
        assert_eq!(morse_pattern(' '), None);
    }

    #[test]
    fn native_tx_validation_rejects_bad_shapes_and_unsupported_modes() {
        assert!(build_native_digital_tx_pcm(
            WorkspaceMode::Ft4,
            "CQ K1ABC",
            700,
            crate::modes::fst4::Submode::default(),
            20,
            600
        )
        .is_err());
        assert!(build_native_digital_tx_pcm(
            WorkspaceMode::Wspr,
            "K1ABC FN42 nope",
            1_000,
            crate::modes::fst4::Submode::default(),
            20,
            600
        )
        .is_err());
        assert!(build_native_digital_tx_pcm(
            WorkspaceMode::Cw,
            "   ",
            700,
            crate::modes::fst4::Submode::default(),
            20,
            600
        )
        .is_err());
        assert!(build_native_digital_tx_pcm(
            WorkspaceMode::Voice,
            "CQ K1ABC FN42",
            700,
            crate::modes::fst4::Submode::default(),
            20,
            600
        )
        .is_err());
    }

    #[test]
    fn native_digital_modes_produce_nonempty_fixtures() {
        for mode in [
            WorkspaceMode::Fst4,
            WorkspaceMode::Jt9,
            WorkspaceMode::Jt65,
            WorkspaceMode::Q65,
        ] {
            let (pcm, audio_start_s) = build_native_digital_tx_pcm(
                mode,
                "CQ W1AW AA00",
                1_500,
                crate::modes::fst4::Submode::S15,
                20,
                600,
            )
            .unwrap_or_else(|error| panic!("{} synthesis failed: {error}", mode.label()));
            assert!(!pcm.is_empty(), "{} produced no PCM", mode.label());
            assert!(audio_start_s >= 0.0, "{} has invalid start", mode.label());
        }
    }

    #[test]
    #[ignore = "slow native TX/RX round-trip; run in release mode"]
    fn native_tx_waveforms_decode_back_through_the_adapter() {
        for mode in [
            WorkspaceMode::Fst4,
            WorkspaceMode::Jt9,
            WorkspaceMode::Jt65,
            WorkspaceMode::Q65,
        ] {
            let (pcm, _) = build_native_digital_tx_pcm(
                mode,
                "CQ W1AW AA00",
                1_500,
                crate::modes::fst4::Submode::S15,
                20,
                600,
            )
            .unwrap_or_else(|error| panic!("{} synthesis failed: {error}", mode.label()));
            let samples = pcm
                .into_iter()
                .map(|sample| sample as f32 / i16::MAX as f32 * 0.06)
                .collect::<Vec<_>>();
            let (wsjt_mode, config) = match mode {
                WorkspaceMode::Fst4 => (
                    WsjtMode::Fst4(Fst4Submode::S15),
                    WsjtDecodeConfig {
                        frequency_min_hz: 100.0,
                        frequency_max_hz: 3_000.0,
                        sync_min: 0.8,
                        max_candidates: 50,
                        frequency_hint_hz: Some(1_500.0),
                        ..WsjtDecodeConfig::default()
                    },
                ),
                WorkspaceMode::Jt9 => (WsjtMode::Jt9, WsjtDecodeConfig::default()),
                WorkspaceMode::Jt65 => (WsjtMode::Jt65, WsjtDecodeConfig::default()),
                WorkspaceMode::Q65 => (
                    WsjtMode::Q65(Q65Submode::A30),
                    WsjtDecodeConfig {
                        score_threshold: 0.05,
                        max_candidates: 8,
                        time_tolerance_sec: 1.0,
                        ..WsjtDecodeConfig::default()
                    },
                ),
                _ => unreachable!(),
            };
            let batch = decode_wsjt(
                &AudioBlock::new(12_000, samples).expect("native TX audio block"),
                wsjt_mode,
                &config,
            )
            .unwrap_or_else(|error| panic!("{} decode failed: {error}", mode.label()));
            assert!(
                batch
                    .events
                    .iter()
                    .any(|event| event.message.contains("W1AW")),
                "{} TX waveform did not decode: {:?}",
                mode.label(),
                batch.events
            );
        }
    }

    #[test]
    fn wait_until_epoch_honors_abort_and_accepts_elapsed_deadlines() {
        let abort = AtomicBool::new(true);
        assert!(wait_until_epoch(
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_secs_f64()
                + 10.0,
            &abort
        )
        .is_err());
        let clear = AtomicBool::new(false);
        assert!(wait_until_epoch(0.0, &clear).is_ok());
    }

    #[test]
    fn request_ptt_distinguishes_acknowledgement_errors_timeouts_and_disconnects() {
        let (tx, rx) = mpsc::channel();
        let receiver = thread::spawn(move || {
            let GuiCommand::SetPttWithAck(enabled, ack) = rx.recv().expect("PTT command") else {
                panic!("unexpected command");
            };
            assert!(enabled);
            ack.send(Ok(())).expect("ack receiver");
        });
        assert!(request_ptt(&tx, true, Duration::from_secs(1)).is_ok());
        receiver.join().expect("ack thread");

        let (tx, rx) = mpsc::channel();
        let receiver = thread::spawn(move || {
            let GuiCommand::SetPttWithAck(_, ack) = rx.recv().expect("PTT command") else {
                panic!("unexpected command");
            };
            ack.send(Err("PTT rejected".to_string()))
                .expect("ack receiver");
        });
        let error = request_ptt(&tx, false, Duration::from_secs(1)).expect_err("rejected PTT");
        assert!(error.to_string().contains("PTT rejected"));
        receiver.join().expect("ack thread");

        let (tx, _rx) = mpsc::channel();
        assert!(request_ptt(&tx, true, Duration::from_millis(1)).is_err());
        let (tx, rx) = mpsc::channel();
        drop(rx);
        assert!(request_ptt(&tx, false, Duration::from_secs(1)).is_err());
    }
}
