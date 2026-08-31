use super::super::*;
use super::cwdit_adapter::CwDitChannel;
use super::decode::{
    prepare_early_digital_slot, prepare_early_ft8_slot, run_ft8_decode_worker,
    run_native_digital_decode, warm_ft8_decoder,
};
use super::request_gui_repaint;
use hound::{SampleFormat, WavSpec, WavWriter};
use qsonaut_audio::{CANONICAL_CHANNELS, CANONICAL_SAMPLE_RATE_HZ};
use qsonaut_modems::AudioNormalizer;
use qsonaut_third_party::cw::CwDecode;
use qsonaut_third_party::sstv as qsonaut_sstv;
use serde_json::json;
use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU32;

const CW_RETARGET_TIMEOUT_S: u64 = 10;

struct SignalRecording {
    full_width: Option<WavWriter<BufWriter<File>>>,
    stream: Option<WavWriter<BufWriter<File>>>,
    metadata: BufWriter<File>,
    path: PathBuf,
    samples: u64,
}

enum RecordingMessage {
    Start {
        mode: WorkspaceMode,
        sample_rate_hz: u32,
        tone_hz: u32,
        frequency_hz: Option<u64>,
        full_width: bool,
        stream: bool,
    },
    Audio {
        full_width: Vec<f32>,
        stream: Vec<f32>,
        samples: u64,
        mode: WorkspaceMode,
    },
    Stop,
}

fn spawn_recording_writer(
    state: Arc<Mutex<GuiState>>,
) -> std::sync::mpsc::SyncSender<RecordingMessage> {
    let (tx, rx) = std::sync::mpsc::sync_channel(8);
    thread::spawn(move || {
        let mut recording = None;
        while let Ok(message) = rx.recv() {
            match message {
                RecordingMessage::Start {
                    mode,
                    sample_rate_hz,
                    tone_hz,
                    frequency_hz,
                    full_width,
                    stream,
                } => {
                    finish_signal_recording(&mut recording, &state);
                    match open_signal_recording(
                        mode,
                        sample_rate_hz,
                        tone_hz,
                        frequency_hz,
                        full_width,
                        stream,
                    ) {
                        Ok(next) => {
                            let path = next.path.clone();
                            recording = Some(next);
                            if let Ok(mut shared) = state.lock() {
                                shared.cw_recording_status =
                                    format!("Recording {}", path.display());
                            }
                        }
                        Err(error) => {
                            tracing::warn!(%error, "signal recording could not start");
                        }
                    }
                }
                RecordingMessage::Audio {
                    full_width,
                    stream,
                    samples,
                    mode,
                } => {
                    let mut should_finish = false;
                    if let Some(recording) = recording.as_mut() {
                        if let Some(writer) = recording.full_width.as_mut() {
                            for sample in full_width {
                                let value = (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
                                let _ = writer.write_sample(value);
                            }
                        }
                        if let Some(writer) = recording.stream.as_mut() {
                            for sample in stream {
                                let value = (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
                                let _ = writer.write_sample(value);
                            }
                        }
                        recording.samples = recording.samples.saturating_add(samples);
                        let _ = serde_json::to_writer(
                            &mut recording.metadata,
                            &json!({ "type": "audio_block", "samples": samples, "mode": mode.label() }),
                        );
                        use std::io::Write;
                        let _ = recording.metadata.write_all(b"\n");
                        should_finish = recording.samples >= 12_000 * 600;
                    }
                    if should_finish {
                        finish_signal_recording(&mut recording, &state);
                    }
                }
                RecordingMessage::Stop => finish_signal_recording(&mut recording, &state),
            }
        }
        finish_signal_recording(&mut recording, &state);
    });
    tx
}

fn strongest_cw_tone_hz(buffer: &[Complex<f32>], sample_rate_hz: u32) -> Option<u32> {
    if buffer.len() < 4 || sample_rate_hz == 0 {
        return None;
    }
    let low = ((300.0 * buffer.len() as f32) / sample_rate_hz as f32).ceil() as usize;
    let high = ((3_000.0 * buffer.len() as f32) / sample_rate_hz as f32).floor() as usize;
    let high = high.min(buffer.len() / 2);
    if low >= high {
        return None;
    }
    let magnitudes: Vec<f32> = (low..=high).map(|index| buffer[index].norm()).collect();
    let average = magnitudes.iter().sum::<f32>() / magnitudes.len() as f32;
    let (peak_offset, peak) = magnitudes
        .iter()
        .enumerate()
        .max_by(|(_, left), (_, right)| left.total_cmp(right))?;
    if !peak.is_finite() || *peak < 1e-4 || *peak < average * 8.0 {
        return None;
    }
    let bin = low + peak_offset;
    Some((bin as f32 * sample_rate_hz as f32 / buffer.len() as f32).round() as u32)
}

fn null_sim_stations(mode: WorkspaceMode) -> Vec<QsonautPerson> {
    let mut stations = qsonaut_demo_people()
        .into_iter()
        .filter(|person| {
            person.modes.is_empty()
                || person
                    .modes
                    .iter()
                    .any(|supported| supported.eq_ignore_ascii_case(mode.label()))
        })
        .filter(|person| person.callsign.as_deref().is_some_and(is_probable_callsign))
        .collect::<Vec<_>>();
    if stations.is_empty() {
        stations.push(QsonautPerson {
            name: Some("QSONaut".to_string()),
            callsign: Some("N7UF".to_string()),
            grid: Some("CN87".to_string()),
            power_dbm: Some(30),
            ..QsonautPerson::default()
        });
    }
    if stations.len() < 2 {
        stations.push(QsonautPerson {
            name: Some("Test Station".to_string()),
            callsign: Some("W1AW".to_string()),
            grid: Some("FN31".to_string()),
            power_dbm: Some(30),
            ..QsonautPerson::default()
        });
    }
    if stations.len() < 3 {
        stations.push(QsonautPerson {
            name: Some("Demo Station".to_string()),
            callsign: Some("K6ABC".to_string()),
            grid: Some("DM13".to_string()),
            power_dbm: Some(30),
            ..QsonautPerson::default()
        });
    }
    stations
}

struct NullAudioGenerator {
    mode: Option<WorkspaceMode>,
    waveforms: Vec<Vec<f32>>,
    period_s: f64,
    start_s: f64,
    next_deadline: Option<Instant>,
}

impl Default for NullAudioGenerator {
    fn default() -> Self {
        Self {
            mode: None,
            waveforms: Vec::new(),
            period_s: 15.0,
            start_s: 0.5,
            next_deadline: None,
        }
    }
}

impl NullAudioGenerator {
    fn rebuild(&mut self, mode: WorkspaceMode, state: &GuiState) {
        self.mode = Some(mode);
        self.period_s = mode.slot_seconds(state.fst4_submode).unwrap_or(match mode {
            WorkspaceMode::Sstv => 60.0,
            _ => 15.0,
        });
        self.start_s = if mode == WorkspaceMode::Sstv {
            1.0
        } else {
            0.5
        };
        self.waveforms.clear();

        let stations = null_sim_stations(mode);
        let first = &stations[0];
        let second = &stations[1];
        let first_call = first.callsign.as_deref().unwrap_or("N7UF");
        let second_call = second.callsign.as_deref().unwrap_or("W1AW");
        let first_grid = first.grid.as_deref().unwrap_or("CN87");
        let second_grid = second.grid.as_deref().unwrap_or("FN31");
        let first_power = first.power_dbm.unwrap_or(30);
        let second_power = second.power_dbm.unwrap_or(30);
        let messages: Vec<(String, u32)> = match mode {
            WorkspaceMode::Sstv => stations
                .iter()
                .take(3)
                .map(|station| {
                    (
                        format!(
                            "{} {}",
                            station.callsign.as_deref().unwrap_or("N7UF"),
                            station.name.as_deref().unwrap_or("IMAGE")
                        ),
                        1_500,
                    )
                })
                .collect(),
            WorkspaceMode::Cw => vec![
                (format!("CQ {first_call}"), 700),
                (format!("{first_call} DE {second_call}"), 1_500),
            ],
            WorkspaceMode::Wspr => vec![
                (format!("{first_call} {first_grid} {first_power}"), 1_000),
                (format!("{second_call} {second_grid} {second_power}"), 2_000),
            ],
            _ => vec![
                (format!("CQ {first_call} {first_grid}"), 700),
                (format!("{first_call} {second_call} -10"), 1_400),
                (format!("{second_call} {first_call} R-07"), 2_100),
                (format!("{first_call} {second_call} RR73"), 700),
                (format!("{second_call} {first_call} 73"), 1_400),
                (format!("CQ {second_call} {second_grid}"), 2_100),
            ],
        };
        for (frame_index, (message, tone_hz)) in messages.into_iter().enumerate() {
            let mut waveform = Vec::new();
            let pcm = match mode {
                WorkspaceMode::Ft8 => build_ft8_tx_pcm(&message, tone_hz),
                WorkspaceMode::Ft4
                | WorkspaceMode::Fst4
                | WorkspaceMode::Jt9
                | WorkspaceMode::Jt65
                | WorkspaceMode::Q65
                | WorkspaceMode::Wspr => build_native_digital_tx_pcm(
                    mode,
                    &message,
                    tone_hz,
                    state.fst4_submode,
                    state.cw_wpm,
                    state.selected_audio_hz as u16,
                )
                .map(|(pcm, _)| pcm),
                WorkspaceMode::Cw => build_native_digital_tx_pcm(
                    mode,
                    &message,
                    tone_hz,
                    state.fst4_submode,
                    state.cw_wpm,
                    tone_hz as u16,
                )
                .map(|(pcm, _)| pcm),
                WorkspaceMode::Sstv => {
                    let width = 320_usize;
                    let height = 256_usize;
                    let mut rgb = vec![0_u8; width * height * 3];
                    for y in 0..height {
                        for x in 0..width {
                            let offset = (y * width + x) * 3;
                            rgb[offset] = ((x + frame_index * 47) % 256) as u8;
                            rgb[offset + 1] = ((y + frame_index * 71) % 256) as u8;
                            rgb[offset + 2] = ((x + y + frame_index * 29) % 256) as u8;
                        }
                    }
                    qsonaut_sstv::encode_rgb_mode_12k(
                        qsonaut_sstv::SstvMode::MartinM1,
                        width as u32,
                        height as u32,
                        &rgb,
                    )
                    .map_err(|error| anyhow::anyhow!(error.to_string()))
                }
                _ => Err(anyhow::anyhow!("unsupported null audio mode")),
            };
            let Ok(pcm) = pcm else { continue };
            if waveform.len() < pcm.len() {
                waveform.resize(pcm.len(), 0.0);
            }
            for (target, sample) in waveform.iter_mut().zip(pcm) {
                *target += sample as f32 / i16::MAX as f32
                    * if mode == WorkspaceMode::Sstv {
                        0.04
                    } else {
                        0.06
                    };
            }
            self.waveforms.push(waveform);
        }
        if mode == WorkspaceMode::Sstv {
            // Martin M1 is approximately 114 seconds at 320x256. Leave a
            // small guard interval so the next image cannot truncate the
            // current VIS/image transmission.
            let longest_s = self
                .waveforms
                .iter()
                .map(|waveform| waveform.len() as f64 / f64::from(CANONICAL_SAMPLE_RATE_HZ))
                .fold(0.0, f64::max);
            self.period_s = self
                .period_s
                .max(
                    f64::from(qsonaut_sstv::mode_duration_seconds(
                        qsonaut_sstv::SstvMode::MartinM1,
                    )) + 2.0,
                )
                .max(longest_s + 2.0);
        }
    }

    fn status(&self) -> String {
        let now_s = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs_f64())
            .unwrap_or_default();
        let cycle = (now_s / self.period_s).floor() as usize;
        let frame_count = self.waveforms.len().max(1);
        let frame = cycle % frame_count + 1;
        let until_next = self.period_s - now_s.rem_euclid(self.period_s);
        format!(
            "SIMULATED RX · {} · frame {frame}/{frame_count} · next slot in {:.1}s",
            self.mode.map(WorkspaceMode::label).unwrap_or("starting"),
            until_next
        )
    }

    fn read_chunk(&mut self, sample_count: usize, state: &Arc<Mutex<GuiState>>) -> Vec<f32> {
        // A real CPAL stream blocks until the requested audio exists. Without
        // the same pacing, the worker can consume the null source thousands
        // of times faster than real time, making the waterfall race and
        // over-driving the monitor output.
        let chunk_duration =
            Duration::from_secs_f64(sample_count as f64 / f64::from(CANONICAL_SAMPLE_RATE_HZ));
        let now = Instant::now();
        let deadline = self.next_deadline.get_or_insert(now);
        if *deadline > now {
            std::thread::sleep(*deadline - now);
        }
        let started = Instant::now();
        self.next_deadline = Some(started + chunk_duration);
        let snapshot = state.lock().expect("ui state lock poisoned").clone();
        if self.mode != Some(snapshot.workspace_mode) {
            self.rebuild(snapshot.workspace_mode, &snapshot);
        }
        let now_s = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs_f64())
            .unwrap_or_default();
        let phase_s = now_s.rem_euclid(self.period_s);
        let cycle = (now_s / self.period_s).floor() as usize;
        let waveform = self
            .waveforms
            .get(cycle % self.waveforms.len().max(1))
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        (0..sample_count)
            .map(|offset| {
                let sample_phase = phase_s + offset as f64 / f64::from(CANONICAL_SAMPLE_RATE_HZ);
                let waveform_index = ((sample_phase - self.start_s) * 12_000.0).round() as isize;
                if waveform_index < 0 || waveform_index as usize >= waveform.len() {
                    0.0
                } else {
                    waveform[waveform_index as usize]
                }
            })
            .collect()
    }
}

fn save_received_sstv_image(
    image: &qsonaut_sstv::DecodedImage,
    mode: Option<qsonaut_sstv::SstvMode>,
    radio_frequency_hz: Option<u64>,
) -> anyhow::Result<PathBuf> {
    let directory = qsonaut_log::app_config_dir().join("sstv-images");
    std::fs::create_dir_all(&directory)?;
    let timestamp_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let mode_name = mode
        .map(|value| value.name().to_ascii_lowercase().replace(' ', "-"))
        .unwrap_or_else(|| "unknown".to_string());
    let frequency = radio_frequency_hz
        .map(|value| value.to_string())
        .unwrap_or_else(|| "unknown-rf".to_string());
    let path = directory.join(format!("rx-{timestamp_ms}-{frequency}-{mode_name}.png"));
    let rgb = image::RgbImage::from_raw(image.width as u32, image.height as u32, image.rgb.clone())
        .ok_or_else(|| anyhow::anyhow!("decoded SSTV dimensions do not match the RGB buffer"))?;
    rgb.save(&path)?;
    Ok(path)
}

fn save_sstv_debug_capture(
    samples: &[f32],
    sample_rate_hz: u32,
    received_id: &str,
    mode: Option<qsonaut_sstv::SstvMode>,
    frequency_hz: Option<u64>,
    frequency_offset_hz: f32,
    received_unix_ms: u128,
) -> anyhow::Result<(PathBuf, PathBuf)> {
    let directory = qsonaut_log::app_config_dir().join("sstv-images");
    std::fs::create_dir_all(&directory)?;
    let stem = format!("{received_id}-debug");
    let wav_path = directory.join(format!("{stem}.wav"));
    let metadata_path = directory.join(format!("{stem}.jsonl"));
    let spec = WavSpec {
        channels: 1,
        sample_rate: sample_rate_hz,
        bits_per_sample: 16,
        sample_format: SampleFormat::Int,
    };
    let mut writer = WavWriter::create(&wav_path, spec)?;
    for &sample in samples {
        writer.write_sample((sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)?;
    }
    writer.finalize()?;
    let metadata = json!({
        "type": "sstv_debug_capture",
        "received_id": received_id,
        "received_unix_ms": received_unix_ms,
        "sample_rate_hz": sample_rate_hz,
        "sample_count": samples.len(),
        "duration_s": samples.len() as f64 / sample_rate_hz.max(1) as f64,
        "mode": mode.map(|value| value.name()),
        "frequency_hz": frequency_hz,
        "frequency_offset_hz": frequency_offset_hz,
        "wav_path": wav_path,
    });
    std::fs::write(&metadata_path, serde_json::to_vec_pretty(&metadata)?)?;
    Ok((wav_path, metadata_path))
}

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
        {
            let mut shared = state.lock().expect("ui state lock poisoned");
            shared.audio_device_sample_rate_hz = None;
            shared.audio_device_channels = None;
            shared.audio_device_sample_format = None;
            shared.audio_input_fallback_attempts.clear();
            shared.audio_monitor_adjustment_ppm = 0;
            shared.audio_monitor_buffered_ms = 0;
            shared.audio_monitor_underruns = 0;
        }
        if !enabled {
            info!("Audio worker disabled by configuration");
            let mut s = state.lock().expect("ui state lock poisoned");
            s.audio_spectrum_status = "DISABLED".to_string();
            return;
        }

        let configured_sample_rate_hz = sample_rate_hz;
        let sample_rate_hz = CANONICAL_SAMPLE_RATE_HZ;
        if configured_sample_rate_hz != sample_rate_hz || u16::from(channels) != CANONICAL_CHANNELS
        {
            info!(
                configured_sample_rate_hz,
                configured_channels = channels,
                canonical_sample_rate_hz = sample_rate_hz,
                canonical_channels = CANONICAL_CHANNELS,
                "Normalizing configured audio processing format"
            );
        }
        let null_audio = preferred_device.as_deref() == Some(NULL_INPUT_DEVICE);
        let mut null_generator = null_audio.then(NullAudioGenerator::default);
        let audio_service = AudioService::new(preferred_device, true);
        let mut stream = if null_audio {
            None
        } else {
            match audio_service.open_stream(sample_rate_hz, CANONICAL_CHANNELS) {
                Ok(stream) => Some(stream),
                Err(err) => {
                    tracing::error!(sample_rate_hz, channels = CANONICAL_CHANNELS, error = %err, "Audio input stream failed to open");
                    let mut s = state.lock().expect("ui state lock poisoned");
                    s.audio_spectrum_status = format!("NO INPUT ({err})");
                    return;
                }
            }
        };
        let device_sample_rate_hz = stream
            .as_ref()
            .map(|stream| stream.device_sample_rate_hz())
            .unwrap_or(sample_rate_hz);
        let device_channels = stream
            .as_ref()
            .map(|stream| stream.device_channels())
            .unwrap_or(CANONICAL_CHANNELS);
        let device_sample_format = stream
            .as_ref()
            .map(|stream| format!("{:?}", stream.device_sample_format()))
            .unwrap_or_else(|| "F32 (simulated)".to_string());
        let resampling_input = !null_audio
            && stream
                .as_ref()
                .is_some_and(|stream| device_sample_rate_hz != stream.output_sample_rate_hz());
        info!(
            canonical_sample_rate_hz = sample_rate_hz,
            device_sample_rate_hz,
            device_channels,
            device_sample_format = %device_sample_format,
            resampling_input,
            null_audio,
            monitor_enabled,
            "Audio input worker started"
        );
        if let Some(stream) = &stream {
            for attempt in stream.fallback_attempts() {
                warn!(attempt, "Audio input configuration fallback");
            }
        }
        {
            let mut shared = state.lock().expect("ui state lock poisoned");
            shared.audio_device_sample_rate_hz = Some(device_sample_rate_hz);
            shared.audio_device_channels = Some(device_channels);
            shared.audio_device_sample_format = Some(device_sample_format);
            shared.audio_input_fallback_attempts = stream
                .as_ref()
                .map(|stream| stream.fallback_attempts().to_vec())
                .unwrap_or_default();
        }
        let null_output = monitor_output_device.as_deref() == Some(NULL_OUTPUT_DEVICE);
        let (monitor, monitor_status) = if monitor_enabled && !null_output {
            match qsonaut_audio::AudioMonitor::open(
                sample_rate_hz,
                monitor_output_device.as_deref(),
            ) {
                Ok(monitor) => {
                    info!(
                        output_sample_rate_hz = monitor.output_sample_rate_hz(),
                        output_channels = monitor.output_channels(),
                        output_sample_format = ?monitor.output_sample_format(),
                        "Audio monitor output opened"
                    );
                    for attempt in monitor.fallback_attempts() {
                        warn!(attempt, "Audio monitor configuration fallback");
                    }
                    (Some(monitor), " · MONITOR ACTIVE".to_string())
                }
                Err(err) => {
                    let message = format!(" · MONITOR ERROR ({err})");
                    tracing::error!(error = %err, "failed to start RX audio monitor");
                    (None, message)
                }
            }
        } else if null_output {
            (None, " · NULL OUTPUT".to_string())
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
            Some(AudioNormalizer::new(sample_rate_hz).expect("validated 48 kHz input"))
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
            let mut shared = state.lock().expect("ui state lock poisoned");
            shared.ft8_decode_status =
                format!("UNAVAILABLE: FT8 requires 48 kHz input (configured {sample_rate_hz} Hz)");
            shared.sstv_status =
                format!("UNAVAILABLE: SSTV requires 48 kHz input (configured {sample_rate_hz} Hz)");
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
        let mut cw_auto_target_candidate: Option<u32> = None;
        let mut cw_auto_target_observations = 0_u8;
        let mut cw_retarget_last_signal = Instant::now();
        let mut cw_filtered_signal_present = false;
        let mut cw_silence_samples = 0_usize;
        let mut last_cw_diagnostics = Instant::now();
        let mut last_cw_status = Instant::now() - Duration::from_secs(1);
        let recording_tx = spawn_recording_writer(state.clone());
        let mut recording_active = false;
        let mut sstv_receiver = qsonaut_sstv::MultiModeReceiver::default();
        let mut sstv_tuning_offset_hz = 0_i32;
        let mut last_sstv_vis: Option<u8> = None;
        let mut sstv_receive_started: Option<Instant> = None;
        let mut sstv_progress_bucket = 0_u8;
        let mut last_sstv_reset_generation = 0_u64;
        let mut sstv_session_power_sum = 0.0_f64;
        let mut sstv_session_chunks = 0_u64;
        let mut sstv_session_max_clip_percent = 0.0_f32;
        let mut sstv_debug_recording = false;
        let mut sstv_debug_samples = Vec::<f32>::new();
        let mut sstv_tail_decoder: Option<CwDitChannel> = None;
        let mut sstv_tail_samples_remaining = 0_usize;
        let mut last_sstv_no_header_log = Instant::now() - Duration::from_secs(10);
        let mut digital_slot_gate = DigitalSlotGate::default();
        let mut ft4_slot_gate = Ft8SlotGate::default();
        let mut decode_workspace_last: Option<WorkspaceMode> = None;
        // Waterfall rows arrive far faster than a human can see. Redrawing the
        // whole UI on every chunk is what pins the GPU, so cap the repaint rate
        // and let egui coalesce the rest.
        let repaint_interval = Duration::from_millis(66);
        let mut last_repaint = Instant::now() - repaint_interval;
        let mut monitor_runtime_error: Option<String> = None;
        let mut last_audio_read_error: Option<String> = None;
        let mut last_monitor_clock_log = Instant::now() - Duration::from_secs(30);

        while !stop.load(Ordering::Relaxed) {
            let chunk_samples = {
                let t = display_tuning.lock().expect("tuning lock poisoned");
                let mode = {
                    let s = state.lock().expect("ui state lock poisoned");
                    s.mode.clone()
                };
                let interval_ms = effective_visual_profile(&t, &mode, false).0;
                ((sample_rate_hz as u64 * interval_ms / 1_000) as usize).max(256)
            };
            match if let Some(stream) = stream.as_mut() {
                stream.read_frames_f32_until_stopped(chunk_samples, &stop)
            } else {
                Ok(Some(
                    null_generator
                        .as_mut()
                        .expect("null audio generator initialized")
                        .read_chunk(chunk_samples, &state),
                ))
            } {
                Ok(Some(samples)) => {
                    if last_audio_read_error.take().is_some() {
                        info!("Audio input stream recovered");
                    }
                    let monitor_raw_audio = {
                        let shared = state.lock().expect("ui state lock poisoned");
                        shared.workspace_mode != WorkspaceMode::Cw || !can_decode
                    };
                    let mut monitor_clock_status = String::new();
                    if let Some(monitor) = &monitor {
                        monitor.set_volume(f32::from_bits(monitor_volume.load(Ordering::Relaxed)));
                        if monitor_raw_audio {
                            monitor.push_f32(&samples);
                        }
                        let dropped_chunks = monitor.take_dropped_chunks();
                        if dropped_chunks > 0 {
                            tracing::warn!(
                                dropped_chunks,
                                "RX audio monitor dropped queued chunks"
                            );
                        }
                        let underruns = monitor.take_underruns();
                        if underruns > 0 {
                            tracing::warn!(
                                underruns,
                                buffered_ms = monitor.buffered_latency_ms(),
                                adjustment_ppm = monitor.clock_adjustment_ppm(),
                                "RX audio monitor clock buffer underrun; rebuffering"
                            );
                        }
                        let adjustment_ppm = monitor.clock_adjustment_ppm();
                        let buffered_ms = monitor.buffered_latency_ms();
                        {
                            let mut shared = state.lock().expect("ui state lock poisoned");
                            shared.audio_monitor_adjustment_ppm = adjustment_ppm;
                            shared.audio_monitor_buffered_ms = buffered_ms;
                            shared.audio_monitor_underruns =
                                shared.audio_monitor_underruns.saturating_add(underruns);
                        }
                        monitor_clock_status =
                            format!(" · SYNC {adjustment_ppm:+} ppm · {buffered_ms} ms");
                        if last_monitor_clock_log.elapsed() >= Duration::from_secs(30) {
                            info!(
                                adjustment_ppm,
                                buffered_ms, "RX audio monitor clock synchronization"
                            );
                            last_monitor_clock_log = Instant::now();
                        }
                        if let Some(error) = monitor.take_error() {
                            tracing::error!(error = %error, "RX audio monitor failed during playback");
                            monitor_runtime_error = Some(error);
                        }
                    }
                    let samples_f32 = samples;
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
                                .unwrap_or_else(|| {
                                    if null_audio {
                                        null_generator
                                            .as_ref()
                                            .map(NullAudioGenerator::status)
                                            .unwrap_or_else(|| {
                                                "SIMULATED RX · NULL SOUND CARD".to_string()
                                            })
                                    } else if resampling_input {
                                        format!(
                                            "LIVE RX · RESAMPLED {} → {} Hz{monitor_status}{monitor_clock_status}",
                                            device_sample_rate_hz, sample_rate_hz
                                        )
                                    } else {
                                        format!("LIVE RX{monitor_status}{monitor_clock_status}")
                                    }
                                });
                        }
                        s.audio_level_dbfs = Some(20.0 * rms.max(1e-9).log10());
                        s.audio_clip_percent = clip_percent;
                    }

                    // ── Slot-aligned native digital decoders ──────────────
                    if let Some(ref mut dec) = decimator {
                        let active_workspace_mode =
                            state.lock().expect("ui state lock poisoned").workspace_mode;
                        if decode_workspace_last != Some(active_workspace_mode) {
                            info!(workspace = %active_workspace_mode.label(), "Audio decoder workspace changed");
                            if recording_active {
                                let _ = recording_tx.try_send(RecordingMessage::Stop);
                                recording_active = false;
                            }
                            decode_workspace_last = Some(active_workspace_mode);
                            ft8_buf.clear();
                            digital_buf.clear();
                            cw_stream_decoder = None;
                            cw_stream_tone_hz = 0;
                            cw_stream_wpm = 0;
                            cw_auto_target_candidate = None;
                            cw_auto_target_observations = 0;
                            cw_retarget_last_signal = Instant::now();
                            cw_filtered_signal_present = false;
                            cw_silence_samples = 0;
                            ft8_slot_gate.reset();
                            ft4_slot_gate.reset();
                            digital_slot_gate.reset();
                            sstv_receiver.reset();
                            last_sstv_vis = None;
                            sstv_receive_started = None;
                            sstv_progress_bucket = 0;
                            *dec = AudioNormalizer::new(sample_rate_hz)
                                .expect("validated 48 kHz input");
                            let mut s = state.lock().expect("ui state lock poisoned");
                            if active_workspace_mode == WorkspaceMode::Ft8 {
                                s.ft8_decode_status =
                                    "READY: collecting a fresh FT8 slot".to_string();
                            } else if active_workspace_mode == WorkspaceMode::Cw {
                                s.digital_decode_status =
                                    "READY: live CW decode starts after 3 seconds".to_string();
                                s.cw_live_text.clear();
                                s.cw_retarget_remaining_s = None;
                            } else if active_workspace_mode == WorkspaceMode::Sstv {
                                s.sstv_status = format!(
                                    "READY: {} · {}",
                                    s.sstv_rx_mode
                                        .map(|mode| mode.name())
                                        .unwrap_or("Auto (VIS)"),
                                    if s.sstv_auto_target {
                                        "auto-target scanning baseband"
                                    } else {
                                        "waiting at the manual target"
                                    }
                                );
                                s.sstv_progress = None;
                                s.sstv_locked_offset_hz = None;
                                info!(
                                    auto_target = s.sstv_auto_target,
                                    scan_min_offset_hz = qsonaut_sstv::AUTO_TARGET_MIN_OFFSET_HZ,
                                    scan_max_offset_hz = qsonaut_sstv::AUTO_TARGET_MAX_OFFSET_HZ,
                                    manual_offset_hz = s.sstv_tuning_offset_hz,
                                    receive_mode = s
                                        .sstv_rx_mode
                                        .map(|mode| mode.name())
                                        .unwrap_or("Auto (VIS)"),
                                    "SSTV receiver ready"
                                );
                            } else if active_workspace_mode.has_native_decoder() {
                                s.digital_decode_status = format!(
                                    "READY: collecting a fresh {} slot",
                                    active_workspace_mode.label()
                                );
                            }
                        }
                        let ds = dec
                            .process_f32_mono(&samples_f32)
                            .expect("audio capture samples are finite");
                        let (
                            recording_enabled,
                            recording_mode,
                            recording_full_width,
                            recording_stream,
                        ) = {
                            let shared = state.lock().expect("ui state lock poisoned");
                            (
                                shared.recording_enabled,
                                shared.recording_modes.contains(&active_workspace_mode),
                                shared.recording_full_width,
                                shared.recording_stream,
                            )
                        };
                        if !recording_enabled
                            || !recording_mode
                            || (!recording_full_width && !recording_stream)
                        {
                            if recording_active {
                                let _ = recording_tx.try_send(RecordingMessage::Stop);
                                recording_active = false;
                            }
                        } else {
                            if !recording_active {
                                let shared = state.lock().expect("ui state lock poisoned");
                                let _ = recording_tx.try_send(RecordingMessage::Start {
                                    mode: active_workspace_mode,
                                    sample_rate_hz,
                                    tone_hz: shared.selected_audio_hz,
                                    frequency_hz: shared.frequency_hz,
                                    full_width: recording_full_width,
                                    stream: recording_stream,
                                });
                                recording_active = true;
                            }
                            let _ = recording_tx.try_send(RecordingMessage::Audio {
                                full_width: if recording_full_width {
                                    samples_f32.clone()
                                } else {
                                    Vec::new()
                                },
                                stream: if active_workspace_mode == WorkspaceMode::Cw
                                    || !recording_stream
                                {
                                    Vec::new()
                                } else {
                                    ds.clone()
                                },
                                samples: ds.len() as u64,
                                mode: active_workspace_mode,
                            });
                        }
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
                        } else if active_workspace_mode == WorkspaceMode::Sstv {
                            if tx_active.load(Ordering::Acquire)
                                || digital_tx_active.load(Ordering::Acquire)
                            {
                                sstv_receiver.reset();
                                let mut shared = state.lock().expect("ui state lock poisoned");
                                shared.sstv_status = "SSTV TX active; RX reset".to_string();
                                shared.sstv_progress = None;
                                shared.sstv_detected_mode = None;
                                shared.sstv_locked_offset_hz = None;
                                sstv_receive_started = None;
                                sstv_progress_bucket = 0;
                            } else {
                                let (debug_requested, selected_tone_hz, cw_wpm) = {
                                    let shared = state.lock().expect("ui state lock poisoned");
                                    (
                                        shared.sstv_debug_capture_requested,
                                        shared.selected_audio_hz,
                                        shared.cw_wpm,
                                    )
                                };
                                if debug_requested && !sstv_debug_recording {
                                    sstv_debug_recording = true;
                                    sstv_debug_samples.clear();
                                    let mut shared = state.lock().expect("ui state lock poisoned");
                                    shared.sstv_debug_capture_requested = false;
                                    shared.sstv_debug_status =
                                        "Debug capture armed; waiting for SSTV completion"
                                            .to_string();
                                    info!("SSTV debug capture started");
                                }
                                if sstv_debug_recording {
                                    sstv_debug_samples.extend_from_slice(&ds);
                                }
                                if sstv_tail_samples_remaining > 0 {
                                    if let Some(decoder) = sstv_tail_decoder.as_mut() {
                                        let (events, _) = decoder.push_samples_with_audio(&ds);
                                        let mut shared =
                                            state.lock().expect("ui state lock poisoned");
                                        for event in events {
                                            let text = match event {
                                                CwDecode::Character(character) => {
                                                    character.to_string()
                                                }
                                                CwDecode::WordBreak => " ".to_string(),
                                                CwDecode::Unknown => continue,
                                            };
                                            shared.cw_live_text.push_str(&text);
                                            shared.digital_decode_status =
                                                format!("SSTV TAIL CW · cw-dit · +{text}");
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
                                    }
                                    sstv_tail_samples_remaining =
                                        sstv_tail_samples_remaining.saturating_sub(ds.len());
                                    if sstv_tail_samples_remaining == 0 {
                                        sstv_tail_decoder = None;
                                        info!("SSTV tail CW decode window ended");
                                    }
                                }
                                let (
                                    requested_offset_hz,
                                    requested_mode,
                                    requested_auto_target,
                                    radio_frequency_hz,
                                    reset_generation,
                                ) = {
                                    let shared = state.lock().expect("ui state lock poisoned");
                                    (
                                        shared.sstv_tuning_offset_hz,
                                        shared.sstv_rx_mode,
                                        shared.sstv_auto_target,
                                        shared.frequency_hz,
                                        shared.sstv_reset_generation,
                                    )
                                };
                                if reset_generation != last_sstv_reset_generation {
                                    sstv_receiver.reset();
                                    last_sstv_reset_generation = reset_generation;
                                    last_sstv_vis = None;
                                    sstv_receive_started = None;
                                    sstv_progress_bucket = 0;
                                    info!("SSTV receiver reset; scanning for a new complete VIS header");
                                }
                                let targeting_changed = sstv_receiver.selected_mode()
                                    != requested_mode
                                    || sstv_receiver.auto_target() != requested_auto_target;
                                sstv_receiver.set_selected_mode(requested_mode);
                                sstv_receiver.set_auto_target(requested_auto_target);
                                if targeting_changed {
                                    last_sstv_vis = None;
                                    sstv_receive_started = None;
                                    sstv_progress_bucket = 0;
                                }
                                if !requested_auto_target
                                    && requested_offset_hz != sstv_tuning_offset_hz
                                {
                                    sstv_tuning_offset_hz = requested_offset_hz;
                                    sstv_receiver
                                        .set_tuning_offset_hz(sstv_tuning_offset_hz as f32);
                                    last_sstv_vis = None;
                                    sstv_receive_started = None;
                                    sstv_progress_bucket = 0;
                                }
                                let active_mode_before_push = sstv_receiver.active_mode();
                                let locked_offset_before_push = sstv_receiver.locked_offset_hz();
                                let decoded = sstv_receiver.push(&ds);
                                let completed_mode = sstv_receiver.take_completed_mode();
                                let decode_error = sstv_receiver.take_decode_error();
                                let auto_reacquired = sstv_receiver.take_auto_reacquired();
                                let progress = sstv_receiver.progress();
                                let active_mode = sstv_receiver.active_mode();
                                let detected_vis = sstv_receiver.detected_vis();
                                let scan_candidate_offset_hz =
                                    sstv_receiver.scan_candidate_offset_hz();
                                let scan_prominence_db = sstv_receiver.scan_prominence_db();
                                let locked_offset_hz =
                                    sstv_receiver.locked_offset_hz().or_else(|| {
                                        decoded.as_ref().map(|_| {
                                            locked_offset_before_push
                                                .unwrap_or(sstv_tuning_offset_hz as f32)
                                        })
                                    });
                                let frequency_offset_hz = sstv_receiver
                                    .frequency_offset_hz()
                                    .or(locked_offset_hz)
                                    .unwrap_or_default();
                                let afc_residual_hz = if requested_auto_target {
                                    0.0
                                } else {
                                    frequency_offset_hz - sstv_tuning_offset_hz as f32
                                };
                                let rms_dbfs = 20.0 * rms.max(1e-9).log10();
                                let new_vis =
                                    detected_vis.filter(|vis| Some(*vis) != last_sstv_vis);
                                if detected_vis != last_sstv_vis {
                                    if let Some(vis) = detected_vis {
                                        info!(
                                            vis,
                                            mode = qsonaut_sstv::vis_mode_name(vis),
                                            supported = qsonaut_sstv::mode_from_vis(vis).is_some(),
                                            frequency_offset_hz,
                                            input_rms_dbfs = rms_dbfs,
                                            input_clip_percent = clip_percent,
                                            leader_prominence_db = scan_prominence_db,
                                            auto_target = requested_auto_target,
                                            radio_frequency_hz,
                                            "SSTV target acquired from VIS header"
                                        );
                                    }
                                    last_sstv_vis = detected_vis;
                                }
                                if auto_reacquired {
                                    sstv_receive_started = None;
                                    sstv_progress_bucket = 0;
                                    info!(
                                        timeout_s = 2.0,
                                        "SSTV unresolved VIS acquisition expired; auto-target scanning resumed"
                                    );
                                }
                                let mut shared = state.lock().expect("ui state lock poisoned");
                                shared.sstv_progress = progress;
                                shared.sstv_locked_offset_hz = requested_auto_target
                                    .then(|| locked_offset_hz.map(|offset| offset.round() as i32))
                                    .flatten();
                                if let Some(mode) =
                                    detected_vis.and_then(qsonaut_sstv::mode_from_vis)
                                {
                                    shared.sstv_detected_mode = Some(mode);
                                }
                                if auto_reacquired {
                                    shared.sstv_detected_mode = None;
                                    shared.sstv_locked_offset_hz = None;
                                }
                                if let Some(vis) = new_vis {
                                    let detected = qsonaut_sstv::mode_from_vis(vis);
                                    let accepted = detected.is_some()
                                        && (requested_mode.is_none() || requested_mode == detected);
                                    if accepted {
                                        sstv_receive_started = Some(Instant::now());
                                        sstv_progress_bucket = 0;
                                        sstv_session_power_sum = 0.0;
                                        sstv_session_chunks = 0;
                                        sstv_session_max_clip_percent = 0.0;
                                    } else {
                                        info!(
                                            vis,
                                            mode = qsonaut_sstv::vis_mode_name(vis),
                                            target_offset_hz = frequency_offset_hz,
                                            receive_filter = requested_mode
                                                .map(|mode| mode.name())
                                                .unwrap_or("Auto (VIS)"),
                                            "SSTV VIS header ignored by receive-mode filter"
                                        );
                                    }
                                }
                                if let (Some(value), Some(mode)) = (progress, active_mode) {
                                    sstv_session_power_sum += f64::from(rms * rms);
                                    sstv_session_chunks = sstv_session_chunks.saturating_add(1);
                                    sstv_session_max_clip_percent =
                                        sstv_session_max_clip_percent.max(clip_percent);
                                    let bucket = ((value * 100.0) / 25.0).floor() as u8;
                                    if bucket > sstv_progress_bucket && bucket < 4 {
                                        sstv_progress_bucket = bucket;
                                        let percent = bucket * 25;
                                        info!(
                                            mode = mode.name(),
                                            percent,
                                            target_offset_hz = frequency_offset_hz,
                                            "SSTV receive progress"
                                        );
                                    }
                                }
                                if detected_vis.is_none()
                                    && active_mode.is_none()
                                    && rms >= 0.005
                                    && last_sstv_no_header_log.elapsed() >= Duration::from_secs(10)
                                {
                                    last_sstv_no_header_log = Instant::now();
                                    info!(
                                        input_rms_dbfs = rms_dbfs,
                                        auto_target = requested_auto_target,
                                        manual_offset_hz = sstv_tuning_offset_hz,
                                        scan_candidate_offset_hz,
                                        scan_prominence_db,
                                        "SSTV audio present without a complete VIS header"
                                    );
                                }
                                shared.sstv_status = if auto_reacquired {
                                    "AUTO REACQUIRE: unresolved VIS timed out after 2.0s · scanning baseband"
                                        .to_string()
                                } else if let Some(error) = decode_error {
                                    let mode = active_mode_before_push
                                        .map(|mode| mode.name())
                                        .unwrap_or("SSTV");
                                    tracing::warn!(mode, error = %error, "SSTV image decode failed");
                                    sstv_receive_started = None;
                                    sstv_progress_bucket = 0;
                                    format!("SSTV DECODE FAILED: {error}")
                                } else if let (Some(value), Some(mode)) = (progress, active_mode) {
                                    format!(
                                        "RECEIVING {} · {:.0}% · target {frequency_offset_hz:+.0} Hz · AFC residual {afc_residual_hz:+.0} Hz",
                                        mode.name(),
                                        value * 100.0,
                                    )
                                } else if let Some(vis) = detected_vis {
                                    let detected = qsonaut_sstv::mode_from_vis(vis);
                                    if requested_mode.is_some() && requested_mode != detected {
                                        format!(
                                            "VIS {vis}: {} ignored · RX filter is {}",
                                            qsonaut_sstv::vis_mode_name(vis),
                                            requested_mode
                                                .map(|mode| mode.name())
                                                .unwrap_or("Auto (VIS)"),
                                        )
                                    } else {
                                        format!(
                                            "VIS {vis}: {} detected · preparing decoder",
                                            qsonaut_sstv::vis_mode_name(vis),
                                        )
                                    }
                                } else if rms >= 0.005 {
                                    format!(
                                        "AUDIO PRESENT ({:.0} dBFS) · no complete VIS header; start RX before the image",
                                        20.0 * rms.max(1e-9).log10()
                                    )
                                } else if requested_auto_target {
                                    match (scan_candidate_offset_hz, scan_prominence_db) {
                                        (Some(candidate), Some(prominence)) => format!(
                                            "SCANNING: strongest 1900 Hz leader candidate {candidate:+.0} Hz · prominence {prominence:.1} dB · waiting for complete VIS",
                                        ),
                                        _ => format!(
                                            "SCANNING: VIS offsets {:+}..{:+} Hz · waiting for a complete header",
                                            qsonaut_sstv::AUTO_TARGET_MIN_OFFSET_HZ,
                                            qsonaut_sstv::AUTO_TARGET_MAX_OFFSET_HZ,
                                        ),
                                    }
                                } else {
                                    format!(
                                        "LISTENING: decoder {}–{} Hz ({sstv_tuning_offset_hz:+} Hz) · waiting for VIS",
                                        1_100 + sstv_tuning_offset_hz,
                                        2_300 + sstv_tuning_offset_hz,
                                    )
                                };
                                if let Some(image) = decoded {
                                    let received_unix_ms = SystemTime::now()
                                        .duration_since(UNIX_EPOCH)
                                        .unwrap_or_default()
                                        .as_millis();
                                    let saved_path = match save_received_sstv_image(
                                        &image,
                                        completed_mode,
                                        radio_frequency_hz,
                                    ) {
                                        Ok(path) => {
                                            info!(path = %path.display(), "SSTV received image saved");
                                            Some(path.display().to_string())
                                        }
                                        Err(error) => {
                                            tracing::warn!(error = %error, "failed to save received SSTV image");
                                            None
                                        }
                                    };
                                    let rgb = image.rgb;
                                    let received_id = saved_path
                                        .clone()
                                        .unwrap_or_else(|| format!("rx-{received_unix_ms}"));
                                    shared.sstv_width = image.width;
                                    shared.sstv_height = image.height;
                                    shared.sstv_rgb = rgb.clone();
                                    shared.sstv_revision = shared.sstv_revision.wrapping_add(1);
                                    shared.sstv_saved_path = saved_path.clone();
                                    shared.sstv_received_images.push_front(ReceivedSstvImage {
                                        id: received_id.clone(),
                                        path: saved_path.clone(),
                                        mode: completed_mode,
                                        frequency_hz: radio_frequency_hz,
                                        width: image.width,
                                        height: image.height,
                                        rgb,
                                        received_unix_ms,
                                        analysis: None,
                                        debug_audio_path: None,
                                        debug_metadata_path: None,
                                    });
                                    info!(
                                        received_image_id = %received_id,
                                        path = ?saved_path,
                                        carousel_size = shared.sstv_received_images.len(),
                                        "SSTV received image added to carousel"
                                    );
                                    while shared.sstv_received_images.len() > 24 {
                                        shared.sstv_received_images.pop_back();
                                    }
                                    shared.sstv_received_revision =
                                        shared.sstv_received_revision.wrapping_add(1);
                                    shared.sstv_progress = None;
                                    shared.sstv_detected_mode = completed_mode;
                                    shared.sstv_status = format!(
                                        "RECEIVED: {} image complete · {}×{} · target {frequency_offset_hz:+.0} Hz",
                                        completed_mode.map(|mode| mode.name()).unwrap_or("SSTV"),
                                        shared.sstv_width,
                                        shared.sstv_height,
                                    );
                                    let elapsed_s = sstv_receive_started
                                        .take()
                                        .map(|started| started.elapsed().as_secs_f32())
                                        .unwrap_or_default();
                                    let mode =
                                        completed_mode.map(|mode| mode.name()).unwrap_or("SSTV");
                                    let average_rms_dbfs = if sstv_session_chunks == 0 {
                                        rms_dbfs
                                    } else {
                                        20.0 * ((sstv_session_power_sum
                                            / sstv_session_chunks as f64)
                                            .sqrt()
                                            .max(1e-9)
                                            .log10()
                                            as f32)
                                    };
                                    info!(
                                        mode,
                                        width = shared.sstv_width,
                                        height = shared.sstv_height,
                                        target_offset_hz = frequency_offset_hz,
                                        elapsed_s,
                                        radio_frequency_hz,
                                        average_input_rms_dbfs = average_rms_dbfs,
                                        max_input_clip_percent = sstv_session_max_clip_percent,
                                        leader_prominence_db = scan_prominence_db,
                                        "SSTV image receive complete"
                                    );
                                    sstv_tail_decoder =
                                        Some(CwDitChannel::new(12_000, selected_tone_hz, cw_wpm));
                                    sstv_tail_samples_remaining = 12_000 * 12;
                                    if sstv_debug_recording {
                                        match save_sstv_debug_capture(
                                            &sstv_debug_samples,
                                            12_000,
                                            &received_id,
                                            completed_mode,
                                            radio_frequency_hz,
                                            frequency_offset_hz,
                                            received_unix_ms,
                                        ) {
                                            Ok((wav_path, metadata_path)) => {
                                                if let Some(saved) =
                                                    shared.sstv_received_images.front_mut()
                                                {
                                                    saved.debug_audio_path =
                                                        Some(wav_path.display().to_string());
                                                    saved.debug_metadata_path =
                                                        Some(metadata_path.display().to_string());
                                                }
                                                shared.sstv_debug_status = format!(
                                                    "Saved debug capture {}",
                                                    wav_path.display()
                                                );
                                                info!(path = %wav_path.display(), metadata = %metadata_path.display(), "SSTV debug capture saved");
                                            }
                                            Err(error) => {
                                                shared.sstv_debug_status =
                                                    format!("Debug capture save failed: {error}");
                                                tracing::warn!(error = %error, "failed to save SSTV debug capture");
                                            }
                                        }
                                        sstv_debug_recording = false;
                                        sstv_debug_samples.clear();
                                    }
                                    sstv_progress_bucket = 0;
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
                            let (mut auto_target, auto_retarget) = {
                                let shared = state.lock().expect("ui state lock poisoned");
                                (shared.cw_auto_target, shared.cw_auto_retarget)
                            };
                            let candidate = strongest_cw_tone_hz(&fft_buf, sample_rate_hz);
                            if auto_retarget && !auto_target {
                                if cw_filtered_signal_present {
                                    cw_retarget_last_signal = Instant::now();
                                } else if cw_retarget_last_signal.elapsed()
                                    >= Duration::from_secs(CW_RETARGET_TIMEOUT_S)
                                {
                                    if recording_active {
                                        let _ = recording_tx.try_send(RecordingMessage::Stop);
                                        recording_active = false;
                                    }
                                    let mut shared = state.lock().expect("ui state lock poisoned");
                                    shared.cw_auto_target = true;
                                    shared.cw_retarget_remaining_s = None;
                                    shared.cw_auto_target_tone_hz = None;
                                    shared.digital_decode_status =
                                        "CW AUTO TARGET: no signal for 10 seconds; scanning"
                                            .to_string();
                                    auto_target = true;
                                    cw_auto_target_candidate = None;
                                    cw_auto_target_observations = 0;
                                    cw_retarget_last_signal = Instant::now();
                                    info!(
                                        "CW auto target resumed after 10 seconds without a signal"
                                    );
                                }
                            }
                            if auto_target {
                                if candidate == cw_auto_target_candidate {
                                    cw_auto_target_observations =
                                        cw_auto_target_observations.saturating_add(1);
                                } else {
                                    cw_auto_target_candidate = candidate;
                                    cw_auto_target_observations = candidate.map_or(0, |_| 1);
                                }
                                if let Some(tone_hz) =
                                    candidate.filter(|_| cw_auto_target_observations >= 3)
                                {
                                    let mut shared = state.lock().expect("ui state lock poisoned");
                                    shared.selected_audio_hz = tone_hz;
                                    shared.cw_auto_target = false;
                                    shared.cw_retarget_remaining_s = if auto_retarget {
                                        Some(CW_RETARGET_TIMEOUT_S as u8)
                                    } else {
                                        None
                                    };
                                    shared.cw_auto_target_tone_hz = Some(tone_hz);
                                    shared.digital_decode_status =
                                        format!("CW AUTO TARGET: locked {tone_hz} Hz");
                                    cw_auto_target_candidate = None;
                                    cw_auto_target_observations = 0;
                                    cw_retarget_last_signal = Instant::now();
                                }
                            } else {
                                cw_auto_target_candidate = None;
                                cw_auto_target_observations = 0;
                            }
                            let selected_tone_hz = state
                                .lock()
                                .expect("ui state lock poisoned")
                                .selected_audio_hz;
                            let cw_wpm = {
                                let shared = state.lock().expect("ui state lock poisoned");
                                shared.cw_wpm
                            };
                            if selected_tone_hz != cw_stream_tone_hz || cw_wpm != cw_stream_wpm {
                                if cw_stream_tone_hz != 0 && recording_active {
                                    let _ = recording_tx.try_send(RecordingMessage::Stop);
                                    recording_active = false;
                                }
                                cw_stream_decoder =
                                    Some(CwDitChannel::new(12_000, selected_tone_hz, cw_wpm));
                                cw_stream_tone_hz = selected_tone_hz;
                                cw_stream_wpm = cw_wpm;
                            }
                            if let Some(decoder) = cw_stream_decoder.as_mut() {
                                let (mut events, channel_audio) =
                                    decoder.push_samples_with_audio(&ds);
                                if let Some(monitor) = &monitor {
                                    monitor.push_f32_at_sample_rate(&channel_audio, 12_000);
                                }
                                if recording_active {
                                    let _ = recording_tx.try_send(RecordingMessage::Audio {
                                        full_width: Vec::new(),
                                        stream: if recording_stream {
                                            channel_audio.clone()
                                        } else {
                                            Vec::new()
                                        },
                                        samples: 0,
                                        mode: WorkspaceMode::Cw,
                                    });
                                }
                                let channel_rms = if channel_audio.is_empty() {
                                    0.0
                                } else {
                                    (channel_audio
                                        .iter()
                                        .map(|sample| sample * sample)
                                        .sum::<f32>()
                                        / channel_audio.len() as f32)
                                        .sqrt()
                                };
                                cw_filtered_signal_present = channel_rms >= 0.002;
                                if auto_retarget && !auto_target {
                                    let elapsed = cw_retarget_last_signal.elapsed().as_secs();
                                    let remaining = CW_RETARGET_TIMEOUT_S.saturating_sub(elapsed);
                                    let remaining = u8::try_from(remaining).unwrap_or(u8::MAX);
                                    let mut shared = state.lock().expect("ui state lock poisoned");
                                    shared.cw_retarget_remaining_s = Some(remaining);
                                    if remaining > 0 {
                                        shared.digital_decode_status = format!(
                                            "CW AUTO TARGET: locked; retarget in {remaining}s"
                                        );
                                    }
                                }
                                if channel_rms < 0.002 {
                                    cw_silence_samples =
                                        cw_silence_samples.saturating_add(channel_audio.len());
                                    if cw_silence_samples >= 12_000 {
                                        events.extend(decoder.finish());
                                        cw_silence_samples = 0;
                                    }
                                } else {
                                    cw_silence_samples = 0;
                                }
                                for event in events {
                                    let text = match event {
                                        CwDecode::Character(character) => character.to_string(),
                                        CwDecode::WordBreak => " ".to_string(),
                                        CwDecode::Unknown => continue,
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

                    if last_repaint.elapsed() >= repaint_interval {
                        last_repaint = Instant::now();
                        request_gui_repaint(&repaint_ctx);
                    }
                }
                Ok(None) => break,
                Err(err) => {
                    let message = err.to_string();
                    if last_audio_read_error.as_deref() != Some(message.as_str()) {
                        warn!(error = %message, "Audio input stream read failed; retrying");
                        last_audio_read_error = Some(message.clone());
                    }
                    state
                        .lock()
                        .expect("ui state lock poisoned")
                        .audio_spectrum_status = format!("NO INPUT ({message})");
                    thread::sleep(Duration::from_millis(100));
                }
            }
        }
    })
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::{
        finish_signal_recording, open_signal_recording_in, strongest_cw_tone_hz, GuiState,
        NullAudioGenerator, WorkspaceMode,
    };
    use qsonaut_third_party::sstv as qsonaut_sstv;
    use rustfft::num_complex::Complex;
    use std::sync::{Arc, Mutex};
    use tempfile::tempdir;

    #[test]
    fn null_audio_generator_builds_waveforms_for_supported_demo_modes() {
        for mode in [
            WorkspaceMode::Ft8,
            WorkspaceMode::Ft4,
            WorkspaceMode::Fst4,
            WorkspaceMode::Wspr,
            WorkspaceMode::Jt9,
            WorkspaceMode::Jt65,
            WorkspaceMode::Q65,
            WorkspaceMode::Cw,
            WorkspaceMode::Sstv,
        ] {
            let state = GuiState {
                workspace_mode: mode,
                ..GuiState::default()
            };
            let mut generator = NullAudioGenerator::default();
            generator.rebuild(mode, &state);
            assert!(
                !generator.waveforms.is_empty(),
                "null source did not encode {mode:?}"
            );
            assert!(
                generator
                    .waveforms
                    .iter()
                    .flat_map(|waveform| waveform.iter())
                    .any(|sample| *sample != 0.0),
                "null source encoded an empty {mode:?} waveform"
            );
        }
    }

    #[test]
    fn null_audio_generator_uses_mode_slots_and_cycles_exchange_frames() {
        let state = GuiState {
            workspace_mode: WorkspaceMode::Ft8,
            ..GuiState::default()
        };
        let mut generator = NullAudioGenerator::default();
        generator.rebuild(WorkspaceMode::Ft8, &state);
        assert_eq!(generator.waveforms.len(), 6);
        assert_eq!(generator.period_s, 15.0);

        let state = GuiState {
            workspace_mode: WorkspaceMode::Fst4,
            ..GuiState::default()
        };
        generator.rebuild(WorkspaceMode::Fst4, &state);
        assert_eq!(generator.period_s, 60.0);
        assert_eq!(generator.waveforms.len(), 6);

        let state = GuiState {
            workspace_mode: WorkspaceMode::Wspr,
            ..GuiState::default()
        };
        generator.rebuild(WorkspaceMode::Wspr, &state);
        assert_eq!(generator.period_s, 120.0);
        assert_eq!(generator.waveforms.len(), 2);

        let state = GuiState {
            workspace_mode: WorkspaceMode::Sstv,
            ..GuiState::default()
        };
        generator.rebuild(WorkspaceMode::Sstv, &state);
        assert!(generator.period_s > 60.0);
        assert!(generator.period_s >= generator.waveforms[0].len() as f64 / 12_000.0 + 2.0);
        assert_eq!(generator.waveforms.len(), 3);
        assert!(generator.waveforms[0] != generator.waveforms[1]);
        assert!(generator.waveforms[1] != generator.waveforms[2]);
        let status = generator.status();
        assert!(status.contains("SSTV"));
        assert!(
            status.contains("frame 1/3")
                || status.contains("frame 2/3")
                || status.contains("frame 3/3")
        );
        assert!(status.contains("next slot in"));
    }

    #[test]
    fn null_sstv_signal_remains_auto_target_decodable_at_simulation_level() {
        let state = GuiState {
            workspace_mode: WorkspaceMode::Sstv,
            ..GuiState::default()
        };
        let mut generator = NullAudioGenerator::default();
        generator.rebuild(WorkspaceMode::Sstv, &state);

        let mut audio = vec![0.0_f32; 12_000];
        audio.extend_from_slice(&generator.waveforms[0]);
        audio.extend(std::iter::repeat_n(0.0, 12_000));
        let mut receiver = qsonaut_sstv::MultiModeReceiver::default();
        receiver.set_auto_target(true);
        let mut decoded = None;
        for chunk in audio.chunks(4096) {
            decoded = receiver.push(chunk).or(decoded);
        }
        assert!(
            decoded.is_some(),
            "null SSTV signal should auto-target and decode"
        );
        assert_eq!(
            receiver.take_completed_mode(),
            Some(qsonaut_sstv::SstvMode::MartinM1)
        );
    }

    #[test]
    fn strongest_cw_tone_requires_a_prominent_audio_peak() {
        let mut spectrum = vec![Complex::new(0.001, 0.0); 2_048];
        assert_eq!(strongest_cw_tone_hz(&spectrum, 12_000), None);
        spectrum[256] = Complex::new(1.0, 0.0);
        let tone_hz = strongest_cw_tone_hz(&spectrum, 12_000).expect("CW peak");
        assert_eq!(tone_hz, 1_500);
    }

    #[test]
    fn signal_recording_writes_session_audio_and_end_metadata() {
        let directory = tempdir().expect("temporary recording directory");
        let mut recording = Some(
            open_signal_recording_in(
                directory.path(),
                WorkspaceMode::Cw,
                48_000,
                700,
                Some(14_074_000),
                true,
                true,
            )
            .expect("recording should open"),
        );
        let path = recording.as_ref().expect("recording exists").path.clone();
        let signal = recording.as_mut().expect("recording exists");
        if let Some(writer) = signal.full_width.as_mut() {
            writer.write_sample(i16::MAX).expect("full-width sample");
        }
        if let Some(writer) = signal.stream.as_mut() {
            writer.write_sample(i16::MIN).expect("stream sample");
        }
        signal.samples = 12_000;

        let state = Arc::new(Mutex::new(GuiState::default()));
        finish_signal_recording(&mut recording, &state);

        let metadata = std::fs::read_to_string(path.with_extension("jsonl"))
            .expect("recording metadata should exist");
        assert!(metadata.contains("\"type\":\"session\""));
        assert!(metadata.contains("\"type\":\"end\""));
        assert!(metadata.contains("\"samples\":12000"));
        assert!(path.with_extension("full.wav").exists());
        assert!(path.with_extension("stream.wav").exists());
        assert!(state
            .lock()
            .expect("state lock")
            .cw_recording_status
            .starts_with("Saved "));

        std::fs::remove_file(path.with_extension("jsonl")).expect("metadata cleanup");
        std::fs::remove_file(path.with_extension("full.wav")).expect("full cleanup");
        std::fs::remove_file(path.with_extension("stream.wav")).expect("stream cleanup");
    }
}

fn open_signal_recording(
    mode: WorkspaceMode,
    full_width_sample_rate_hz: u32,
    tone_hz: u32,
    frequency_hz: Option<u64>,
    record_full_width: bool,
    record_stream: bool,
) -> anyhow::Result<SignalRecording> {
    let directory = qsonaut_log::app_config_dir().join("signal-recordings");
    open_signal_recording_in(
        &directory,
        mode,
        full_width_sample_rate_hz,
        tone_hz,
        frequency_hz,
        record_full_width,
        record_stream,
    )
}

fn open_signal_recording_in(
    directory: &Path,
    mode: WorkspaceMode,
    full_width_sample_rate_hz: u32,
    tone_hz: u32,
    frequency_hz: Option<u64>,
    record_full_width: bool,
    record_stream: bool,
) -> anyhow::Result<SignalRecording> {
    std::fs::create_dir_all(directory)?;
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    let path = directory.join(format!(
        "{}-{stamp}-{tone_hz}hz",
        mode.label().to_ascii_lowercase()
    ));
    let metadata_path = path.with_extension("jsonl");
    let full_width = record_full_width
        .then(|| {
            WavWriter::create(
                path.with_extension("full.wav"),
                WavSpec {
                    channels: 1,
                    sample_rate: full_width_sample_rate_hz,
                    bits_per_sample: 16,
                    sample_format: SampleFormat::Int,
                },
            )
        })
        .transpose()?;
    let stream = record_stream
        .then(|| {
            WavWriter::create(
                path.with_extension("stream.wav"),
                WavSpec {
                    channels: 1,
                    sample_rate: CANONICAL_SAMPLE_RATE_HZ,
                    bits_per_sample: 16,
                    sample_format: SampleFormat::Int,
                },
            )
        })
        .transpose()?;
    let mut metadata = BufWriter::new(File::create(&metadata_path)?);
    use std::io::Write;
    writeln!(
        metadata,
        "{}",
        serde_json::to_string(&json!({
            "type": "session",
            "mode": mode.label(),
            "full_width_sample_rate_hz": full_width_sample_rate_hz,
            "stream_sample_rate_hz": CANONICAL_SAMPLE_RATE_HZ,
            "tone_hz": tone_hz,
            "frequency_hz": frequency_hz,
            "full_width": record_full_width,
            "stream": record_stream,
        }))?
    )?;
    Ok(SignalRecording {
        full_width,
        stream,
        metadata,
        path,
        samples: 0,
    })
}

fn finish_signal_recording(recording: &mut Option<SignalRecording>, state: &Arc<Mutex<GuiState>>) {
    if let Some(mut recording) = recording.take() {
        if let Some(writer) = recording.full_width.take() {
            let _ = writer.finalize();
        }
        if let Some(writer) = recording.stream.take() {
            let _ = writer.finalize();
        }
        use std::io::Write;
        let _ = writeln!(
            recording.metadata,
            "{}",
            serde_json::json!({
                "type": "end",
                "samples": recording.samples,
                "duration_s": recording.samples as f64 / CANONICAL_SAMPLE_RATE_HZ as f64,
            })
        );
        let _ = recording.metadata.flush();
        state
            .lock()
            .expect("ui state lock poisoned")
            .cw_recording_status = format!(
            "Saved {} ({:.1}s)",
            recording.path.display(),
            recording.samples as f64 / CANONICAL_SAMPLE_RATE_HZ as f64
        );
    }
}
