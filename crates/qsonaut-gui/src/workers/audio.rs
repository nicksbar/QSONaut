use super::super::*;
use super::cwdit_adapter::CwDitChannel;
use super::decode::{
    prepare_early_digital_slot, prepare_early_ft8_slot, run_ft8_decode_worker,
    run_native_digital_decode, warm_ft8_decoder,
};
use hound::{SampleFormat, WavSpec, WavWriter};
use qsonaut_audio::resample::Decimator;
use serde_json::json;
use std::fs::File;
use std::io::BufWriter;
use std::path::PathBuf;
use std::sync::atomic::AtomicU32;

type CwRecording = (WavWriter<BufWriter<File>>, BufWriter<File>, PathBuf, u64);

#[allow(clippy::too_many_arguments)]
pub(in super::super) fn spawn_audio_spectrum_worker(
    state: Arc<Mutex<GuiState>>,
    stop: Arc<AtomicBool>,
    tx_active: Arc<AtomicBool>,
    digital_tx_active: Arc<AtomicBool>,
    enabled: bool,
    sample_rate_hz: u32,
    channels: u8,
    preferred_device: Option<String>,
    monitor_enabled: bool,
    monitor_output_device: Option<String>,
    monitor_volume: Arc<AtomicU32>,
    repaint_ctx: Arc<OnceLock<egui::Context>>,
    display_tuning: Arc<Mutex<DisplayTuning>>,
) -> std::thread::JoinHandle<()> {
    thread::spawn(move || {
        if !enabled {
            let mut s = state.lock().expect("ui state lock poisoned");
            s.audio_spectrum_status = "DISABLED".to_string();
            return;
        }

        let audio_service = AudioService::new(preferred_device, true);
        let mut stream = match audio_service.open_stream(sample_rate_hz, channels as u16) {
            Ok(stream) => stream,
            Err(err) => {
                let mut s = state.lock().expect("ui state lock poisoned");
                s.audio_spectrum_status = format!("NO INPUT ({err})");
                return;
            }
        };
        let (monitor, monitor_status) = if monitor_enabled {
            match qsonaut_audio::AudioMonitor::open(
                sample_rate_hz,
                monitor_output_device.as_deref(),
            ) {
                Ok(monitor) => (Some(monitor), " · MONITOR ACTIVE".to_string()),
                Err(err) => {
                    let message = format!(" · MONITOR ERROR ({err})");
                    tracing::error!(error = %err, "failed to start RX audio monitor");
                    (None, message)
                }
            }
        } else {
            (None, String::new())
        };
        if let Some(monitor) = &monitor {
            monitor.set_volume(f32::from_bits(monitor_volume.load(Ordering::Relaxed)));
        }

        let mut fft_planner = FftPlanner::<f32>::new();
        let audio_fft = fft_planner.plan_fft_forward(FFT_SIZE);
        let mut fft_buf = vec![Complex::<f32>::new(0.0, 0.0); FFT_SIZE];
        let mut ring: VecDeque<f32> = VecDeque::with_capacity(FFT_SIZE);
        let decode_in_progress = Arc::new(AtomicBool::new(false));
        let deferred_decode: Arc<Mutex<Option<PendingFt8Decode>>> = Arc::new(Mutex::new(None));
        let digital_decode_in_progress = Arc::new(AtomicBool::new(false));

        // 12 kHz decimation pipeline for FT8 decode
        let can_decode = sample_rate_hz == 48_000;
        let mut decimator = if can_decode {
            Some(Decimator::new(sample_rate_hz))
        } else {
            None
        };
        if can_decode {
            state
                .lock()
                .expect("ui state lock poisoned")
                .ft8_decode_status = "WARMING DECODER".to_string();
            warm_ft8_decoder();
            state
                .lock()
                .expect("ui state lock poisoned")
                .ft8_decode_status = "READY: waiting for full slot".to_string();
        } else {
            state
                .lock()
                .expect("ui state lock poisoned")
                .ft8_decode_status =
                format!("UNAVAILABLE: FT8 requires 48 kHz input (configured {sample_rate_hz} Hz)");
        }
        // 15-second accumulation buffer at 12 kHz (180 000 samples)
        // Retain pre-boundary audio so adaptive timing can compensate for a
        // clock that is behind UTC without discarding the start of a frame.
        let mut ft8_buf: Vec<f32> = Vec::with_capacity(12_000 * 18);
        let mut ft8_slot_gate = Ft8SlotGate::default();
        let mut digital_buf: Vec<f32> = Vec::with_capacity(12_000 * 120);
        // Built lazily on the first CW chunk so the search window always
        // brackets the operator's currently selected audio tone.
        let mut cw_stream_decoder: Option<CwDitChannel> = None;
        let mut cw_stream_tone_hz = 0_u32;
        let mut cw_stream_wpm = 0_u8;
        let mut last_cw_diagnostics = Instant::now();
        let mut last_cw_status = Instant::now() - Duration::from_secs(1);
        let mut cw_recording: Option<CwRecording> = None;
        let mut digital_slot_gate = DigitalSlotGate::default();
        let mut ft4_slot_gate = Ft8SlotGate::default();
        let mut decode_workspace_last: Option<WorkspaceMode> = None;
        // Waterfall rows arrive far faster than a human can see. Redrawing the
        // whole UI on every chunk is what pins the GPU, so cap the repaint rate
        // and let egui coalesce the rest.
        let repaint_interval = Duration::from_millis(66);
        let mut last_repaint = Instant::now() - repaint_interval;
        let mut monitor_runtime_error: Option<String> = None;

        while !stop.load(Ordering::Relaxed) {
            let chunk_samples = {
                let t = display_tuning.lock().expect("tuning lock poisoned");
                let mode = {
                    let s = state.lock().expect("ui state lock poisoned");
                    s.mode.clone()
                };
                let interval_ms = effective_visual_profile(&t, &mode).0;
                ((sample_rate_hz as u64 * interval_ms / 1_000) as usize).max(256)
            };
            let chunk_bytes = (chunk_samples * 2).max(512);
            match stream.read_chunk(chunk_bytes) {
                Ok(samples) => {
                    let monitor_raw_audio = {
                        let shared = state.lock().expect("ui state lock poisoned");
                        shared.workspace_mode != WorkspaceMode::Cw || !can_decode
                    };
                    if let Some(monitor) = &monitor {
                        monitor.set_volume(f32::from_bits(monitor_volume.load(Ordering::Relaxed)));
                        if monitor_raw_audio {
                            monitor.push(&samples);
                        }
                        let dropped_chunks = monitor.take_dropped_chunks();
                        if dropped_chunks > 0 {
                            tracing::warn!(
                                dropped_chunks,
                                "RX audio monitor dropped queued chunks"
                            );
                        }
                        if let Some(error) = monitor.take_error() {
                            tracing::error!(error = %error, "RX audio monitor failed during playback");
                            monitor_runtime_error = Some(error);
                        }
                    }
                    let samples_f32: Vec<f32> = samples
                        .iter()
                        .map(|&s| s as f32 / i16::MAX as f32)
                        .collect();
                    let rms = if samples_f32.is_empty() {
                        0.0
                    } else {
                        (samples_f32
                            .iter()
                            .map(|sample| sample * sample)
                            .sum::<f32>()
                            / samples_f32.len() as f32)
                            .sqrt()
                    };
                    let clip_percent = if samples_f32.is_empty() {
                        0.0
                    } else {
                        samples_f32
                            .iter()
                            .filter(|sample| sample.abs() >= 0.99)
                            .count() as f32
                            * 100.0
                            / samples_f32.len() as f32
                    };
                    // ── Display ring buffer + FFT ──────────────────────────
                    for &x in &samples_f32 {
                        ring.push_back(x);
                    }
                    while ring.len() > FFT_SIZE {
                        ring.pop_front();
                    }
                    let nfill = ring.len();
                    for (i, b) in fft_buf.iter_mut().enumerate() {
                        *b = if i < nfill {
                            let w =
                                0.5 - 0.5 * (2.0 * PI * i as f32 / (nfill.max(2) - 1) as f32).cos();
                            Complex::new(ring[i] * w, 0.0)
                        } else {
                            Complex::new(0.0, 0.0)
                        };
                    }
                    audio_fft.process(&mut fft_buf);
                    let bins = fft_buffer_to_display_bins(&fft_buf, AUDIO_BINS, sample_rate_hz);
                    {
                        let mut s = state.lock().expect("ui state lock poisoned");
                        if !s.ptt_on {
                            if s.audio_waterfall_rows.len() >= AUDIO_WF_HEIGHT {
                                s.audio_waterfall_rows.pop_front();
                            }
                            s.audio_waterfall_rows.push_back(bins);
                            s.audio_waterfall_revision = s.audio_waterfall_revision.wrapping_add(1);
                            s.audio_spectrum_status = monitor_runtime_error
                                .as_deref()
                                .map(|error| format!("LIVE RX · MONITOR ERROR ({error})"))
                                .unwrap_or_else(|| format!("LIVE RX{monitor_status}"));
                        }
                        s.audio_level_dbfs = Some(20.0 * rms.max(1e-9).log10());
                        s.audio_clip_percent = clip_percent;
                    }

                    // ── Slot-aligned native digital decoders ──────────────
                    if let Some(ref mut dec) = decimator {
                        let active_workspace_mode =
                            state.lock().expect("ui state lock poisoned").workspace_mode;
                        if decode_workspace_last != Some(active_workspace_mode) {
                            decode_workspace_last = Some(active_workspace_mode);
                            ft8_buf.clear();
                            digital_buf.clear();
                            cw_stream_decoder = None;
                            cw_stream_tone_hz = 0;
                            cw_stream_wpm = 0;
                            ft8_slot_gate.reset();
                            ft4_slot_gate.reset();
                            digital_slot_gate.reset();
                            *dec = Decimator::new(sample_rate_hz);
                            let mut s = state.lock().expect("ui state lock poisoned");
                            if active_workspace_mode == WorkspaceMode::Ft8 {
                                s.ft8_decode_status =
                                    "READY: collecting a fresh FT8 slot".to_string();
                            } else if active_workspace_mode == WorkspaceMode::Cw {
                                s.digital_decode_status =
                                    "READY: live CW decode starts after 3 seconds".to_string();
                                s.cw_live_text.clear();
                            } else if active_workspace_mode.has_native_decoder() {
                                s.digital_decode_status = format!(
                                    "READY: collecting a fresh {} slot",
                                    active_workspace_mode.label()
                                );
                            }
                        }
                        let ds = dec.process(&samples_f32);
                        if active_workspace_mode == WorkspaceMode::Ft8 {
                            ft8_buf.extend_from_slice(&ds);
                            // Keep a full slot plus ±2.5 s timing headroom.
                            let max_buf = 12_000 * 18;
                            if ft8_buf.len() > max_buf {
                                ft8_buf.drain(..ft8_buf.len() - max_buf);
                            }
                            // Arm at a UTC boundary, then decode as soon as the FT8 waveform has
                            // ended. Startup mid-slot is intentionally ignored.
                            let now_s = SystemTime::now()
                                .duration_since(UNIX_EPOCH)
                                .map(|d| d.as_secs_f64())
                                .unwrap_or(0.0);
                            let current_period = (now_s / 15.0) as u64;
                            let slot_position_s = now_s % 15.0;
                            let captured_samples = (slot_position_s * 12_000.0).round() as usize;
                            let alignment_s = state
                                .lock()
                                .expect("ui state lock poisoned")
                                .ft8_clock_offset_s
                                .unwrap_or(0.0)
                                .clamp(-FT8_ADAPTIVE_OFFSET_LIMIT_S, FT8_ADAPTIVE_OFFSET_LIMIT_S);
                            let adaptive_decode_s =
                                (FT8_EARLY_DECODE_S + alignment_s.max(0.0) as f64).min(14.6);
                            let buffer_ready = captured_samples >= (12_000 * 12)
                                && ft8_buf.len() >= captured_samples;
                            if (tx_active.load(Ordering::Acquire)
                                || digital_tx_active.load(Ordering::Acquire))
                                && slot_position_s >= FT8_EARLY_DECODE_S
                            {
                                ft8_slot_gate.skip(current_period);
                                state
                                    .lock()
                                    .expect("ui state lock poisoned")
                                    .ft8_decode_status = "TX SLOT: decode skipped".to_string();
                            } else if ft8_slot_gate.observe_at(
                                current_period,
                                slot_position_s,
                                adaptive_decode_s,
                                buffer_ready,
                            ) {
                                let decoded_period = current_period;
                                let slot_start_s = decoded_period as f64 * 15.0;
                                let utc = utc_hhmmss_millis(slot_start_s);
                                let deep_decode = state
                                    .lock()
                                    .expect("ui state lock poisoned")
                                    .ft8_deep_decode;
                                let pending = PendingFt8Decode {
                                    samples: prepare_early_ft8_slot(
                                        &ft8_buf,
                                        captured_samples,
                                        alignment_s,
                                    ),
                                    utc,
                                    period: decoded_period,
                                    deep_decode,
                                    alignment_s,
                                };
                                let in_progress = decode_in_progress.clone();
                                if in_progress
                                    .compare_exchange(
                                        false,
                                        true,
                                        Ordering::AcqRel,
                                        Ordering::Relaxed,
                                    )
                                    .is_ok()
                                {
                                    let rms = (pending
                                        .samples
                                        .iter()
                                        .map(|sample| sample * sample)
                                        .sum::<f32>()
                                        / pending.samples.len().max(1) as f32)
                                        .sqrt();
                                    let peak = pending
                                        .samples
                                        .iter()
                                        .map(|sample| sample.abs())
                                        .fold(0.0_f32, f32::max);
                                    info!(
                                        buf_samples = pending.samples.len(),
                                        utc = %pending.utc,
                                        slot_position_ms = (slot_position_s * 1_000.0).round() as u64,
                                        captured_samples,
                                        slot_rms_dbfs = 20.0 * rms.max(1e-9).log10(),
                                        slot_peak_dbfs = 20.0 * peak.max(1e-9).log10(),
                                        deep_decode = pending.deep_decode,
                                        "FT8 decode triggered"
                                    );
                                    let state_d = state.clone();
                                    let deferred_decode_d = deferred_decode.clone();
                                    thread::spawn(move || {
                                        run_ft8_decode_worker(pending, state_d, deferred_decode_d);
                                        in_progress.store(false, Ordering::Release);
                                    });
                                } else {
                                    *deferred_decode
                                        .lock()
                                        .expect("deferred decode lock poisoned") = Some(pending);
                                    info!(
                                        "FT8 decode deferred: previous decode pass still running"
                                    );
                                }
                            }
                        } else if active_workspace_mode == WorkspaceMode::Cw {
                            if tx_active.load(Ordering::Acquire)
                                || digital_tx_active.load(Ordering::Acquire)
                            {
                                let mut shared = state.lock().expect("ui state lock poisoned");
                                shared.cw_live_text.clear();
                                shared.digital_decode_status =
                                    "CW TX active; receive window reset".to_string();
                                continue;
                            }
                            let selected_tone_hz = state
                                .lock()
                                .expect("ui state lock poisoned")
                                .selected_audio_hz;
                            let cw_wpm = {
                                let shared = state.lock().expect("ui state lock poisoned");
                                shared.cw_wpm
                            };
                            let record_rx =
                                state.lock().expect("ui state lock poisoned").cw_record_rx;
                            if record_rx && cw_recording.is_none() {
                                if let Ok((writer, metadata, wav_path)) = open_cw_recording(
                                    selected_tone_hz,
                                    snapshot_recording_frequency(&state),
                                ) {
                                    cw_recording = Some((writer, metadata, wav_path.clone(), 0));
                                    state
                                        .lock()
                                        .expect("ui state lock poisoned")
                                        .cw_recording_status =
                                        format!("Recording {}", wav_path.display());
                                }
                            } else if !record_rx {
                                finish_cw_recording(&mut cw_recording, &state);
                            }
                            if let Some((writer, metadata, _, recorded_samples)) =
                                cw_recording.as_mut()
                            {
                                for sample in &ds {
                                    let value = (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
                                    let _ = writer.write_sample(value);
                                }
                                *recorded_samples =
                                    recorded_samples.saturating_add(ds.len() as u64);
                                let _ = serde_json::to_writer(
                                    &mut *metadata,
                                    &json!({
                                        "type": "audio_block",
                                        "samples": ds.len(),
                                        "selected_tone_hz": selected_tone_hz,
                                    }),
                                );
                                use std::io::Write;
                                let _ = metadata.write_all(b"\n");
                                if *recorded_samples >= 12_000 * 600 {
                                    finish_cw_recording(&mut cw_recording, &state);
                                }
                            }
                            if selected_tone_hz != cw_stream_tone_hz || cw_wpm != cw_stream_wpm {
                                cw_stream_decoder =
                                    Some(CwDitChannel::new(12_000, selected_tone_hz, cw_wpm));
                                cw_stream_tone_hz = selected_tone_hz;
                                cw_stream_wpm = cw_wpm;
                            }
                            if let Some(decoder) = cw_stream_decoder.as_mut() {
                                let (events, channel_audio) = decoder.push_samples_with_audio(&ds);
                                if let Some(monitor) = &monitor {
                                    let channel_audio = channel_audio
                                        .iter()
                                        .map(|sample| {
                                            (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16
                                        })
                                        .collect::<Vec<_>>();
                                    monitor.push_at_sample_rate(&channel_audio, 12_000);
                                }
                                for event in events {
                                    let text = match event {
                                        cwdit_morse::Decoded::Char(character) => {
                                            character.to_string()
                                        }
                                        cwdit_morse::Decoded::WordBreak => " ".to_string(),
                                        cwdit_morse::Decoded::Unknown => continue,
                                    };
                                    let mut shared = state.lock().expect("ui state lock poisoned");
                                    shared.cw_live_text.push_str(&text);
                                    shared.digital_decode_status =
                                        format!("LIVE CW · cw-dit · +{}", text);
                                    shared.digital_decodes.push_back(DigitalDecodeEntry {
                                        mode: WorkspaceMode::Cw,
                                        period: SystemTime::now()
                                            .duration_since(UNIX_EPOCH)
                                            .map(|d| d.as_secs())
                                            .unwrap_or_default(),
                                        utc: utc_hhmmss_millis(
                                            SystemTime::now()
                                                .duration_since(UNIX_EPOCH)
                                                .map(|d| d.as_secs_f64())
                                                .unwrap_or_default(),
                                        ),
                                        snr_db: 0.0,
                                        dt_s: 0.0,
                                        freq_hz: selected_tone_hz,
                                        message: text,
                                    });
                                }
                                if last_cw_status.elapsed() >= Duration::from_secs(1) {
                                    let status = format!(
                                        "LIVE CW · cw-dit · {} chars · {:.1} env/s",
                                        decoder.text().len(),
                                        decoder.envelope_rate(),
                                    );
                                    state
                                        .lock()
                                        .expect("ui state lock poisoned")
                                        .digital_decode_status = status;
                                    last_cw_status = Instant::now();
                                }
                                if last_cw_diagnostics.elapsed() >= Duration::from_secs(10) {
                                    let selected_level = {
                                        let shared = state.lock().expect("ui state lock poisoned");
                                        audio_cursor_level(
                                            &shared.audio_waterfall_rows,
                                            selected_tone_hz,
                                        )
                                    };
                                    tracing::info!(
                                        tone_hz = selected_tone_hz,
                                        selected_level,
                                        input_rms_dbfs = 20.0 * rms.max(1e-9).log10(),
                                        text_len = decoder.text().len(),
                                        envelope_rate = decoder.envelope_rate(),
                                        block_len = decoder.block_len(),
                                        "CW stream diagnostics"
                                    );
                                    last_cw_diagnostics = Instant::now();
                                }
                            }
                        } else if active_workspace_mode == WorkspaceMode::Ft4 {
                            digital_buf.extend_from_slice(&ds);
                            let max_buf = 12_000 * 10;
                            if digital_buf.len() > max_buf {
                                digital_buf.drain(..digital_buf.len() - max_buf);
                            }
                            let now_s = SystemTime::now()
                                .duration_since(UNIX_EPOCH)
                                .map(|duration| duration.as_secs_f64())
                                .unwrap_or(0.0);
                            let current_period = (now_s / FT4_SLOT_SECONDS) as u64;
                            let slot_position_s = now_s % FT4_SLOT_SECONDS;
                            let captured_samples = (slot_position_s * 12_000.0).round() as usize;
                            let alignment_s = state
                                .lock()
                                .expect("ui state lock poisoned")
                                .ft4_clock_offset_s
                                .unwrap_or(0.0)
                                .clamp(-FT4_ADAPTIVE_OFFSET_LIMIT_S, FT4_ADAPTIVE_OFFSET_LIMIT_S);
                            let decode_at_s =
                                (FT4_EARLY_DECODE_S + alignment_s.max(0.0) as f64).min(7.1);
                            let buffer_ready = captured_samples >= 12_000 * 5
                                && digital_buf.len() >= captured_samples;
                            if (tx_active.load(Ordering::Acquire)
                                || digital_tx_active.load(Ordering::Acquire))
                                && slot_position_s >= FT4_EARLY_DECODE_S
                            {
                                ft4_slot_gate.skip(current_period);
                                state
                                    .lock()
                                    .expect("ui state lock poisoned")
                                    .digital_decode_status =
                                    "FT4 TX SLOT: decode skipped".to_string();
                            } else if ft4_slot_gate.observe_at(
                                current_period,
                                slot_position_s,
                                decode_at_s,
                                buffer_ready,
                            ) {
                                let decoded_period = current_period;
                                let skip_own_tx = {
                                    let mut shared = state.lock().expect("ui state lock poisoned");
                                    let skip = shared.digital_tx_period
                                        == Some((WorkspaceMode::Ft4, decoded_period));
                                    if skip {
                                        shared.digital_tx_period = None;
                                        shared.digital_decode_status =
                                            "FT4 TX slot complete; receiving".to_string();
                                    }
                                    skip
                                };
                                if skip_own_tx {
                                    continue;
                                }
                                let samples = prepare_early_digital_slot(
                                    &digital_buf,
                                    captured_samples,
                                    FT4_SLOT_SAMPLES,
                                    alignment_s,
                                );
                                let in_progress = digital_decode_in_progress.clone();
                                if in_progress
                                    .compare_exchange(
                                        false,
                                        true,
                                        Ordering::AcqRel,
                                        Ordering::Relaxed,
                                    )
                                    .is_ok()
                                {
                                    let state_d = state.clone();
                                    let utc =
                                        utc_hhmmss_millis(decoded_period as f64 * FT4_SLOT_SECONDS);
                                    let selected_audio_hz = state
                                        .lock()
                                        .expect("ui state lock poisoned")
                                        .selected_audio_hz;
                                    let deep_decode = state
                                        .lock()
                                        .expect("ui state lock poisoned")
                                        .ft4_deep_decode;
                                    let rms =
                                        (samples.iter().map(|sample| sample * sample).sum::<f32>()
                                            / samples.len().max(1) as f32)
                                            .sqrt();
                                    let peak = samples
                                        .iter()
                                        .map(|sample| sample.abs())
                                        .fold(0.0_f32, f32::max);
                                    info!(
                                        mode = "FT4",
                                        period = decoded_period,
                                        slot_position_ms =
                                            (slot_position_s * 1_000.0).round() as u64,
                                        alignment_s,
                                        deep_decode,
                                        slot_rms_dbfs = 20.0 * rms.max(1e-9).log10(),
                                        slot_peak_dbfs = 20.0 * peak.max(1e-9).log10(),
                                        "FT4 early decode triggered"
                                    );
                                    thread::spawn(move || {
                                        run_native_digital_decode(
                                            WorkspaceMode::Ft4,
                                            crate::modes::fst4::Submode::default(),
                                            samples,
                                            decoded_period,
                                            utc,
                                            selected_audio_hz,
                                            deep_decode,
                                            state_d,
                                        );
                                        in_progress.store(false, Ordering::Release);
                                    });
                                } else {
                                    state
                                        .lock()
                                        .expect("ui state lock poisoned")
                                        .digital_decode_status =
                                        "FT4 decode deferred: previous pass still running"
                                            .to_string();
                                }
                            }
                        } else if let Some(slot_seconds) = active_workspace_mode.slot_seconds(
                            state.lock().expect("ui state lock poisoned").fst4_submode,
                        ) {
                            digital_buf.extend_from_slice(&ds);
                            let slot_samples = (slot_seconds * 12_000.0).round() as usize;
                            if digital_buf.len() > slot_samples {
                                digital_buf.drain(..digital_buf.len() - slot_samples);
                            }
                            let now_s = SystemTime::now()
                                .duration_since(UNIX_EPOCH)
                                .map(|duration| duration.as_secs_f64())
                                .unwrap_or(0.0);
                            let current_period = (now_s / slot_seconds) as u64;
                            let buffer_ready =
                                digital_buf.len() >= slot_samples.saturating_sub(12_000 / 2);
                            if digital_slot_gate.boundary(current_period, buffer_ready) {
                                let decoded_period = current_period.saturating_sub(1);
                                let skip_own_tx = {
                                    let mut shared = state.lock().expect("ui state lock poisoned");
                                    let skip = shared.digital_tx_period
                                        == Some((active_workspace_mode, decoded_period));
                                    if skip {
                                        shared.digital_tx_period = None;
                                        shared.digital_decode_status = format!(
                                            "{} TX slot complete; receiving",
                                            active_workspace_mode.label()
                                        );
                                    }
                                    skip
                                };
                                if skip_own_tx {
                                    continue;
                                }
                                let mut samples = vec![0.0f32; slot_samples];
                                let copy_len = digital_buf.len().min(slot_samples);
                                samples[slot_samples - copy_len..]
                                    .copy_from_slice(&digital_buf[digital_buf.len() - copy_len..]);
                                let state_d = state.clone();
                                let in_progress = digital_decode_in_progress.clone();
                                if in_progress
                                    .compare_exchange(
                                        false,
                                        true,
                                        Ordering::AcqRel,
                                        Ordering::Relaxed,
                                    )
                                    .is_ok()
                                {
                                    let utc =
                                        utc_hhmmss_millis(decoded_period as f64 * slot_seconds);
                                    let selected_audio_hz = state
                                        .lock()
                                        .expect("ui state lock poisoned")
                                        .selected_audio_hz;
                                    info!(
                                        mode = active_workspace_mode.label(),
                                        period = decoded_period,
                                        buf_samples = samples.len(),
                                        utc,
                                        "digital decode triggered"
                                    );
                                    let fst4_submode =
                                        state.lock().expect("ui state lock poisoned").fst4_submode;
                                    thread::spawn(move || {
                                        run_native_digital_decode(
                                            active_workspace_mode,
                                            fst4_submode,
                                            samples,
                                            decoded_period,
                                            utc,
                                            selected_audio_hz,
                                            false,
                                            state_d,
                                        );
                                        in_progress.store(false, Ordering::Release);
                                    });
                                } else {
                                    state
                                        .lock()
                                        .expect("ui state lock poisoned")
                                        .digital_decode_status = format!(
                                        "{} decode skipped: previous pass still running",
                                        active_workspace_mode.label()
                                    );
                                }
                            }
                        }
                    }

                    if let Some(ctx) = repaint_ctx.get() {
                        let elapsed = last_repaint.elapsed();
                        if elapsed >= repaint_interval {
                            last_repaint = Instant::now();
                            ctx.request_repaint();
                        } else {
                            ctx.request_repaint_after(repaint_interval - elapsed);
                        }
                    }
                }
                Err(err) => {
                    state
                        .lock()
                        .expect("ui state lock poisoned")
                        .audio_spectrum_status = format!("NO INPUT ({err})");
                    thread::sleep(Duration::from_millis(100));
                }
            }
        }
    })
}

fn snapshot_recording_frequency(state: &Arc<Mutex<GuiState>>) -> Option<u64> {
    state.lock().expect("ui state lock poisoned").frequency_hz
}

fn open_cw_recording(
    tone_hz: u32,
    frequency_hz: Option<u64>,
) -> anyhow::Result<(WavWriter<BufWriter<File>>, BufWriter<File>, PathBuf)> {
    let directory = qsonaut_log::app_config_dir().join("cw-recordings");
    std::fs::create_dir_all(&directory)?;
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    let wav_path = directory.join(format!("cw-{stamp}-{tone_hz}hz.wav"));
    let metadata_path = wav_path.with_extension("jsonl");
    let spec = WavSpec {
        channels: 1,
        sample_rate: 12_000,
        bits_per_sample: 16,
        sample_format: SampleFormat::Int,
    };
    let writer = WavWriter::create(&wav_path, spec)?;
    let mut metadata = BufWriter::new(File::create(&metadata_path)?);
    use std::io::Write;
    writeln!(
        metadata,
        "{}",
        serde_json::to_string(&json!({
            "type": "session",
            "sample_rate_hz": 12_000,
            "tone_hz": tone_hz,
            "frequency_hz": frequency_hz,
        }))?
    )?;
    Ok((writer, metadata, wav_path))
}

fn finish_cw_recording(recording: &mut Option<CwRecording>, state: &Arc<Mutex<GuiState>>) {
    if let Some((writer, mut metadata, path, samples)) = recording.take() {
        let _ = writer.finalize();
        use std::io::Write;
        let _ = writeln!(
            metadata,
            "{}",
            serde_json::json!({
                "type": "end",
                "samples": samples,
                "duration_s": samples as f64 / 12_000.0,
            })
        );
        let _ = metadata.flush();
        state
            .lock()
            .expect("ui state lock poisoned")
            .cw_recording_status = format!(
            "Saved {} ({:.1}s)",
            path.display(),
            samples as f64 / 12_000.0
        );
    }
}
