use super::*;

const FT8_TX_AMPLITUDE_I16: i16 = 18_000;
const FT8_TX_SAMPLE_RATE_HZ: u32 = 12_000;
const FT8_TX_MONITOR_FFT_SIZE: usize = 2_048;
const FT8_TX_MONITOR_HOP_SAMPLES: usize = 500;
pub(super) const FT8_TX_AUDIO_START_S: f64 = modes::exchange::AUDIO_START_SECONDS;
const FT8_MAX_AUDIO_LATE_S: f64 = 1.75;
const DIGITAL_MAX_AUDIO_LATE_S: f64 = 1.0;

pub(super) fn build_ft8_tx_pcm(compose: &str, tx_tone_hz: u32) -> Result<Vec<i16>> {
    let tokens: Vec<&str> = compose.split_whitespace().collect();
    if tokens.len() != 3 {
        anyhow::bail!("standard FT8 TX needs exactly 3 fields (DESTINATION SOURCE GRID/REPORT)");
    }
    let msg77 = if tokens[0].eq_ignore_ascii_case("CQ") {
        pack77("CQ", tokens[1], tokens[2])
            .ok_or_else(|| anyhow!("unable to pack FT8 CQ message: {compose}"))?
    } else {
        pack77(tokens[0], tokens[1], tokens[2])
            .ok_or_else(|| anyhow!("unable to pack FT8 standard message: {compose}"))?
    };
    let tones = ft8_message_to_tones(&msg77);
    Ok(ft8_tones_to_i16(
        &tones,
        tx_tone_hz as f32,
        FT8_TX_AMPLITUDE_I16,
    ))
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
    let to_i16 = |audio: Vec<f32>| {
        audio
            .into_iter()
            .map(|sample| (sample.clamp(-1.0, 1.0) * FT8_TX_AMPLITUDE_I16 as f32).round() as i16)
            .collect::<Vec<_>>()
    };
    let tone = tx_tone_hz as f32;
    match mode {
        WorkspaceMode::Ft4 | WorkspaceMode::Fst4 => {
            let bits = pack77(tokens[0], tokens[1], tokens[2])
                .ok_or_else(|| anyhow!("unable to pack {} message", mode.label()))?;
            if mode == WorkspaceMode::Ft4 {
                let tones = mfsk_core::ft4::encode::message_to_tones(&bits);
                Ok((
                    mfsk_core::ft4::encode::tones_to_i16(&tones, tone, FT8_TX_AMPLITUDE_I16),
                    0.5,
                ))
            } else {
                let tones = mfsk_core::fst4::encode::message_to_tones(&bits);
                let pcm = match fst4_submode {
                    crate::modes::fst4::Submode::S15 => {
                        mfsk_core::fst4::encode::tones_to_i16_with_gfsk(
                            &tones,
                            tone,
                            FT8_TX_AMPLITUDE_I16,
                            &mfsk_core::fst4::encode::FST4_15_GFSK,
                        )
                    }
                    crate::modes::fst4::Submode::S30 => {
                        mfsk_core::fst4::encode::tones_to_i16_with_gfsk(
                            &tones,
                            tone,
                            FT8_TX_AMPLITUDE_I16,
                            &mfsk_core::fst4::encode::FST4_30_GFSK,
                        )
                    }
                    crate::modes::fst4::Submode::S60 => {
                        mfsk_core::fst4::encode::tones_to_i16_with_gfsk(
                            &tones,
                            tone,
                            FT8_TX_AMPLITUDE_I16,
                            &mfsk_core::fst4::encode::FST4_60A_GFSK,
                        )
                    }
                    crate::modes::fst4::Submode::S120 => {
                        mfsk_core::fst4::encode::tones_to_i16_with_gfsk(
                            &tones,
                            tone,
                            FT8_TX_AMPLITUDE_I16,
                            &mfsk_core::fst4::encode::FST4_120_GFSK,
                        )
                    }
                    crate::modes::fst4::Submode::S300 => {
                        mfsk_core::fst4::encode::tones_to_i16_with_gfsk(
                            &tones,
                            tone,
                            FT8_TX_AMPLITUDE_I16,
                            &mfsk_core::fst4::encode::FST4_300_GFSK,
                        )
                    }
                };
                Ok((
                    pcm,
                    match fst4_submode {
                        crate::modes::fst4::Submode::S15 => 0.5,
                        _ => 1.0,
                    },
                ))
            }
        }
        WorkspaceMode::Jt9 => {
            mfsk_core::jt9::synthesize_standard(tokens[0], tokens[1], tokens[2], 12_000, tone, 1.0)
                .map(|audio| (to_i16(audio), 0.0))
                .ok_or_else(|| anyhow!("unable to pack JT9 message"))
        }
        WorkspaceMode::Jt65 => {
            mfsk_core::jt65::synthesize_standard(tokens[0], tokens[1], tokens[2], 12_000, tone, 1.0)
                .map(|audio| (to_i16(audio), 0.0))
                .ok_or_else(|| anyhow!("unable to pack JT65 message"))
        }
        WorkspaceMode::Q65 => {
            mfsk_core::q65::synthesize_standard(tokens[0], tokens[1], tokens[2], 12_000, tone, 1.0)
                .map(|audio| (to_i16(audio), 1.0))
                .ok_or_else(|| anyhow!("unable to pack Q65 message"))
        }
        WorkspaceMode::Wspr => {
            let callsign = tokens
                .first()
                .copied()
                .ok_or_else(|| anyhow!("WSPR TX requires CALL GRID POWER_DBM"))?;
            let grid = tokens
                .get(1)
                .copied()
                .ok_or_else(|| anyhow!("WSPR TX requires CALL GRID POWER_DBM"))?;
            let power_dbm = tokens
                .get(2)
                .ok_or_else(|| anyhow!("WSPR TX requires CALL GRID POWER_DBM"))?
                .parse::<i32>()
                .map_err(|_| anyhow!("WSPR power must be an integer dBm value"))?;
            let audio = mfsk_core::wspr::synthesize_type1(
                callsign,
                grid,
                power_dbm,
                FT8_TX_SAMPLE_RATE_HZ,
                tone,
                0.8,
            )
            .ok_or_else(|| anyhow!("invalid WSPR callsign, locator, or power"))?;
            Ok((to_i16(audio), 0.0))
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
        if job.mode != WorkspaceMode::Cw && audio_late_s > DIGITAL_MAX_AUDIO_LATE_S {
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
