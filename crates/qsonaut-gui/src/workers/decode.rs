use super::super::*;
use qsonaut_modems::AudioBlock;
use qsonaut_third_party::wsjt::{decode as decode_wsjt, Fst4Submode, WsjtDecodeConfig, WsjtMode};

const FT8_SLOT_MS: u128 = 15_000;
const FT8_DEEP_RUNTIME_BUDGET_MS: u128 = 12_000;
const FT8_DEEP_SYNC_MIN: f32 = 1.3;
const FT8_DEEP_MAX_CAND: usize = 120;

pub(in super::super) fn warm_ft8_decoder() {
    let warmup_audio = vec![0i16; FT8_SLOT_SAMPLES];
    let started = Instant::now();
    let _ = DecodeRequest::<Ft8>::wsjtx_depth(
        &warmup_audio,
        100.0,
        3_000.0,
        1.6,
        16,
        WsjtxDepth::D1,
        None,
    )
    .decode();
    info!(
        elapsed_ms = started.elapsed().as_millis() as u64,
        "FT8 decoder warmup complete"
    );
}

pub(in super::super) fn prepare_early_ft8_slot(
    rolling: &[f32],
    captured_samples: usize,
    alignment_s: f32,
) -> Vec<f32> {
    let mut slot = vec![0.0; FT8_SLOT_SAMPLES];
    let local_boundary = rolling.len() as isize - captured_samples.min(rolling.len()) as isize;
    let alignment_samples = (alignment_s * 12_000.0).round() as isize;
    let requested_start = local_boundary + alignment_samples;
    let source_start = requested_start.max(0) as usize;
    let destination_start = requested_start.min(0).unsigned_abs().min(FT8_SLOT_SAMPLES);
    let copy_len = rolling
        .len()
        .saturating_sub(source_start)
        .min(FT8_SLOT_SAMPLES.saturating_sub(destination_start));
    if copy_len > 0 {
        slot[destination_start..destination_start + copy_len]
            .copy_from_slice(&rolling[source_start..source_start + copy_len]);
    }
    slot
}

pub(in super::super) fn prepare_early_digital_slot(
    rolling: &[f32],
    captured_samples: usize,
    slot_samples: usize,
    alignment_s: f32,
) -> Vec<f32> {
    let mut slot = vec![0.0; slot_samples];
    let local_boundary = rolling.len() as isize - captured_samples.min(rolling.len()) as isize;
    let alignment_samples = (alignment_s * 12_000.0).round() as isize;
    let requested_start = local_boundary + alignment_samples;
    let source_start = requested_start.max(0) as usize;
    let destination_start = requested_start.min(0).unsigned_abs().min(slot_samples);
    let copy_len = rolling
        .len()
        .saturating_sub(source_start)
        .min(slot_samples.saturating_sub(destination_start));
    if copy_len > 0 {
        slot[destination_start..destination_start + copy_len]
            .copy_from_slice(&rolling[source_start..source_start + copy_len]);
    }
    slot
}

#[allow(clippy::too_many_arguments)]
pub(in super::super) fn run_native_digital_decode(
    mode: WorkspaceMode,
    fst4_submode: crate::modes::fst4::Submode,
    samples: Vec<f32>,
    period: u64,
    utc: String,
    selected_audio_hz: u32,
    deep_decode: bool,
    state: Arc<Mutex<GuiState>>,
) {
    let backend = state
        .lock()
        .expect("ui state lock poisoned")
        .compute_backend;
    let budget = Duration::from_secs_f64(mode.core_slot_seconds().unwrap_or(15.0));
    let mut trace = DecodeTrace::new(mode.label(), backend, samples.len(), budget);
    let mut decoded = Vec::new();
    let mut push = |snr_db: f32, dt_s: f32, freq_hz: f32, message: String| {
        decoded.push(DigitalDecodeEntry {
            mode,
            period,
            utc: utc.clone(),
            snr_db,
            dt_s,
            freq_hz: freq_hz.max(0.0).round() as u32,
            message,
        });
    };

    trace.measure("protocol decode", || match mode {
        WorkspaceMode::Ft4 | WorkspaceMode::Fst4 => {
            let audio: Vec<i16> = samples
                .iter()
                .map(|sample| (sample.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16)
                .collect();
            if mode == WorkspaceMode::Ft4 {
                let sync_min = if deep_decode { 0.45 } else { 0.6 };
                let request = DecodeRequest::<mfsk_core::ft4::Ft4>::new(
                    &audio, 100.0, 3_000.0, sync_min, 160,
                )
                .freq_hint(selected_audio_hz as f32);
                let outcome = if deep_decode {
                    request.sic_rounds(3).decode()
                } else {
                    request.decode()
                };
                for result in outcome.results {
                    if let Some(message) = unpack77(result.message77()) {
                        push(result.snr_db, result.dt_sec, result.freq_hz, message);
                    }
                }
            } else if let Ok(batch) = decode_wsjt(
                &AudioBlock::new(12_000, samples.clone()).expect("normalized audio is valid"),
                WsjtMode::Fst4(match fst4_submode {
                    crate::modes::fst4::Submode::S15 => Fst4Submode::S15,
                    crate::modes::fst4::Submode::S30 => Fst4Submode::S30,
                    crate::modes::fst4::Submode::S60 => Fst4Submode::S60,
                    crate::modes::fst4::Submode::S120 => Fst4Submode::S120,
                    crate::modes::fst4::Submode::S300 => Fst4Submode::S300,
                }),
                &WsjtDecodeConfig {
                    frequency_min_hz: 100.0,
                    frequency_max_hz: 3_000.0,
                    sync_min: 0.8,
                    max_candidates: 50,
                    frequency_hint_hz: Some(selected_audio_hz as f32),
                    deep_decode,
                    ..WsjtDecodeConfig::default()
                },
            ) {
                for event in batch.events {
                    push(
                        event.snr_db.unwrap_or_default(),
                        event.delta_time_seconds.unwrap_or_default(),
                        event.audio_frequency_hz.unwrap_or_default(),
                        event.message,
                    );
                }
            }
        }
        WorkspaceMode::Wspr => {
            if let Ok(batch) = decode_wsjt(
                &AudioBlock::new(12_000, samples.clone()).expect("normalized audio is valid"),
                WsjtMode::Wspr,
                &WsjtDecodeConfig::default(),
            ) {
                for event in batch.events {
                    push(
                        event.snr_db.unwrap_or_default(),
                        event.delta_time_seconds.unwrap_or_default(),
                        event.audio_frequency_hz.unwrap_or_default(),
                        event.message,
                    );
                }
            }
        }
        WorkspaceMode::Jt9 => {
            if let Ok(batch) = decode_wsjt(
                &AudioBlock::new(12_000, samples.clone()).expect("normalized audio is valid"),
                WsjtMode::Jt9,
                &WsjtDecodeConfig::default(),
            ) {
                for event in batch.events {
                    push(
                        event.snr_db.unwrap_or_default(),
                        event.delta_time_seconds.unwrap_or_default(),
                        event.audio_frequency_hz.unwrap_or_default(),
                        event.message,
                    );
                }
            }
        }
        WorkspaceMode::Jt65 => {
            if let Ok(batch) = decode_wsjt(
                &AudioBlock::new(12_000, samples.clone()).expect("normalized audio is valid"),
                WsjtMode::Jt65,
                &WsjtDecodeConfig::default(),
            ) {
                for event in batch.events {
                    push(
                        event.snr_db.unwrap_or_default(),
                        event.delta_time_seconds.unwrap_or_default(),
                        event.audio_frequency_hz.unwrap_or_default(),
                        event.message,
                    );
                }
            }
        }
        WorkspaceMode::Q65 => {
            if let Ok(batch) = decode_wsjt(
                &AudioBlock::new(12_000, samples.clone()).expect("normalized audio is valid"),
                WsjtMode::Q65,
                &WsjtDecodeConfig::default(),
            ) {
                for event in batch.events {
                    push(
                        event.snr_db.unwrap_or_default(),
                        event.delta_time_seconds.unwrap_or_default(),
                        event.audio_frequency_hz.unwrap_or_default(),
                        event.message,
                    );
                }
            }
        }
        WorkspaceMode::Msk144 => {
            if let Ok(batch) = decode_wsjt(
                &AudioBlock::new(12_000, samples.clone()).expect("normalized audio is valid"),
                WsjtMode::Msk144,
                &WsjtDecodeConfig {
                    frequency_hint_hz: Some(selected_audio_hz as f32),
                    ..WsjtDecodeConfig::default()
                },
            ) {
                for event in batch.events {
                    push(
                        event.snr_db.unwrap_or_default(),
                        event.delta_time_seconds.unwrap_or_default(),
                        event.audio_frequency_hz.unwrap_or_default(),
                        event.message,
                    );
                }
            }
        }
        WorkspaceMode::Ft8
        | WorkspaceMode::Cw
        | WorkspaceMode::Voice
        | WorkspaceMode::Sstv
        | WorkspaceMode::Fldigi => {}
    });

    let telemetry = trace.finish(decoded.len());
    let elapsed_ms = telemetry.total.as_millis();
    info!(
        mode = mode.label(),
        decoded = decoded.len(),
        elapsed_ms = elapsed_ms as u64,
        "digital decode pass complete"
    );
    let (psk_sender, dial_frequency_hz) = {
        let shared = state.lock().expect("ui state lock poisoned");
        (shared.psk_report_sender.clone(), shared.frequency_hz)
    };
    let received_at = (period as f64 * mode.core_slot_seconds().unwrap_or(15.0)) as u32;
    for result in &decoded {
        submit_psk_report(
            &psk_sender,
            dial_frequency_hz,
            result.freq_hz,
            result.snr_db,
            &result.message,
            mode.label(),
            received_at,
        );
    }
    let mut shared = state.lock().expect("ui state lock poisoned");
    shared.digital_compute_telemetry = Some(telemetry);
    if mode == WorkspaceMode::Ft4 {
        shared.ft4_last_decode_period = Some(period);
    }
    if mode == WorkspaceMode::Ft4 && !decoded.is_empty() {
        let mut offsets: Vec<f32> = decoded.iter().map(|result| result.dt_s).collect();
        offsets.sort_by(f32::total_cmp);
        let measured = offsets[offsets.len() / 2]
            .clamp(-FT4_ADAPTIVE_OFFSET_LIMIT_S, FT4_ADAPTIVE_OFFSET_LIMIT_S);
        shared.ft4_clock_offset_s = Some(
            shared
                .ft4_clock_offset_s
                .map_or(measured, |previous| previous + 0.35 * (measured - previous)),
        );
    }
    shared.digital_decode_status = if decoded.is_empty() {
        format!("LIVE: no {} decodes in {elapsed_ms} ms", mode.label())
    } else {
        let timing = if mode == WorkspaceMode::Ft4 {
            shared
                .ft4_clock_offset_s
                .map(|offset| format!(" | adaptive dT {offset:+.2}s"))
                .unwrap_or_default()
        } else {
            String::new()
        };
        format!(
            "LIVE: {} {} decoded in {elapsed_ms} ms{timing}",
            decoded.len(),
            mode.label()
        )
    };
    shared.digital_decodes.extend(decoded);
    while shared.digital_decodes.len() > 300 {
        shared.digital_decodes.pop_front();
    }
}

pub(in super::super) fn run_ft8_decode_worker(
    mut pending: PendingFt8Decode,
    state: Arc<Mutex<GuiState>>,
    deferred_decode: Arc<Mutex<Option<PendingFt8Decode>>>,
) {
    loop {
        let elapsed_ms = run_ft8_decode(
            pending.samples,
            state.clone(),
            pending.utc,
            pending.period,
            pending.deep_decode,
            pending.alignment_s,
        );
        let next = deferred_decode
            .lock()
            .expect("deferred decode lock poisoned")
            .take();
        if let Some(mut next_pending) = next {
            if next_pending.deep_decode && elapsed_ms > FT8_DEEP_RUNTIME_BUDGET_MS {
                next_pending.deep_decode = false;
                info!(
                    elapsed_ms = elapsed_ms as u64,
                    budget_ms = FT8_DEEP_RUNTIME_BUDGET_MS as u64,
                    "FT8 deep decode exceeded realtime budget; switching deferred pass to FAST"
                );
            }
            info!(
                buf_samples = next_pending.samples.len(),
                utc = %next_pending.utc,
                deep_decode = next_pending.deep_decode,
                "FT8 running deferred decode"
            );
            pending = next_pending;
        } else {
            break;
        }
    }
}

/// Background FT8 decode — runs in its own thread, one per period.
fn run_ft8_decode(
    samples: Vec<f32>,
    state: Arc<Mutex<GuiState>>,
    utc: String,
    period: u64,
    deep_decode: bool,
    alignment_s: f32,
) -> u128 {
    let backend = state
        .lock()
        .expect("ui state lock poisoned")
        .compute_backend;
    let mut trace = DecodeTrace::new(
        "FT8",
        backend,
        samples.len(),
        Duration::from_millis(FT8_SLOT_MS as u64),
    );
    let audio_i16: Vec<i16> = trace.measure("prepare PCM", || {
        samples
            .iter()
            .map(|&x| {
                let s = x.clamp(-1.0, 1.0);
                (s * i16::MAX as f32).round() as i16
            })
            .collect()
    });

    // mfsk-core FT8 decode (12 kHz slot-aligned audio), mapped to the
    // library's WSJT-X depth presets for clearer latency/recall behavior.
    let outcome = trace.measure("protocol decode", || {
        if deep_decode {
            // D2: staged early decode (`sic_early`) with WSJT-X-style profile.
            DecodeRequest::<Ft8>::wsjtx_depth(
                &audio_i16,
                100.0,
                3_000.0,
                FT8_DEEP_SYNC_MIN,
                FT8_DEEP_MAX_CAND,
                WsjtxDepth::D2,
                None,
            )
            .decode()
        } else {
            // D1: non-early SIC (`sic_rounds(2)`) for lower latency.
            DecodeRequest::<Ft8>::wsjtx_depth(
                &audio_i16,
                100.0,
                3_000.0,
                FT8_FAST_SYNC_MIN,
                FT8_FAST_MAX_CAND,
                WsjtxDepth::D1,
                None,
            )
            .decode()
        }
    });

    let results: Vec<Ft8DecodeEntry> = trace.measure("unpack results", || {
        let mut results = Vec::new();
        for r in &outcome.results {
            if let Some(msg) = unpack77(r.message77()) {
                let is_cq = msg.starts_with("CQ");
                let snr = r.snr_db.round() as i8;
                let absolute_dt_s = alignment_s + r.dt_sec;
                debug!(
                    freq = r.freq_hz,
                    dt_s = absolute_dt_s,
                    snr,
                    msg,
                    "FT8 decode OK"
                );
                results.push(Ft8DecodeEntry {
                    period,
                    utc: utc.clone(),
                    snr_db: snr,
                    dt_s: absolute_dt_s,
                    freq_hz: r.freq_hz.max(0.0).round() as u32,
                    message: msg,
                    is_cq,
                });
            }
        }
        results
    });

    let telemetry = trace.finish(results.len());
    let elapsed_ms = telemetry.total.as_millis();
    info!(
        deep_decode,
        decoded = results.len(),
        elapsed_ms = elapsed_ms as u64,
        over_slot = elapsed_ms > FT8_SLOT_MS,
        "FT8 decode pass complete"
    );

    let (psk_sender, dial_frequency_hz) = {
        let shared = state.lock().expect("ui state lock poisoned");
        (shared.psk_report_sender.clone(), shared.frequency_hz)
    };
    for result in &results {
        submit_psk_report(
            &psk_sender,
            dial_frequency_hz,
            result.freq_hz,
            f32::from(result.snr_db),
            &result.message,
            "FT8",
            period.saturating_mul(15).min(u64::from(u32::MAX)) as u32,
        );
    }

    let mut s = state.lock().expect("ui state lock poisoned");
    s.ft8_compute_telemetry = Some(telemetry);
    s.ft8_last_decode_period = Some(period);
    if !results.is_empty() {
        let mut offsets: Vec<f32> = results.iter().map(|result| result.dt_s).collect();
        offsets.sort_by(f32::total_cmp);
        let measured_offset = offsets[offsets.len() / 2]
            .clamp(-FT8_ADAPTIVE_OFFSET_LIMIT_S, FT8_ADAPTIVE_OFFSET_LIMIT_S);
        let adaptive_offset = s.ft8_clock_offset_s.map_or(measured_offset, |previous| {
            previous + 0.35 * (measured_offset - previous)
        });
        s.ft8_clock_offset_s = Some(adaptive_offset);
        s.ft8_decode_status = format!(
            "LIVE: {} decoded in {} ms | adaptive dT {adaptive_offset:+.2}s",
            results.len(),
            elapsed_ms
        );
        s.ft8_pending.extend(results);
    } else {
        s.ft8_decode_status = format!("LIVE: no decodes in {elapsed_ms} ms");
    }

    elapsed_ms
}
