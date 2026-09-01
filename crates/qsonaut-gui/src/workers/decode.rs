use super::super::*;
use qsonaut_modems::{extract_aligned_window, AudioBlock};
use qsonaut_third_party::wsjt::{decode as decode_wsjt, Fst4Submode, WsjtDecodeConfig, WsjtMode};

const FT8_SLOT_MS: u128 = 15_000;
const FT8_DEEP_RUNTIME_BUDGET_MS: u128 = 12_000;
const FT8_DEEP_SYNC_MIN: f32 = 1.3;
const FT8_DEEP_MAX_CAND: usize = 120;

fn q65_live_decode_config() -> WsjtDecodeConfig {
    // Q65's coarse search is substantially more expensive than the other
    // native modes. Keep the live worker bounded unless a future explicit
    // deep-decode control opts into a wider pass.
    WsjtDecodeConfig {
        score_threshold: 0.05,
        max_candidates: 8,
        time_tolerance_sec: 1.0,
        ..WsjtDecodeConfig::default()
    }
}

pub(in super::super) fn warm_ft8_decoder() {
    let warmup_audio =
        AudioBlock::new(12_000, vec![0.0; FT8_SLOT_SAMPLES]).expect("normalized audio is valid");
    let started = Instant::now();
    let _ = decode_wsjt(&warmup_audio, WsjtMode::Ft8, &WsjtDecodeConfig::default());
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
    extract_aligned_window(
        rolling,
        captured_samples,
        FT8_SLOT_SAMPLES,
        alignment_s,
        12_000,
    )
}

pub(in super::super) fn prepare_early_digital_slot(
    rolling: &[f32],
    captured_samples: usize,
    slot_samples: usize,
    alignment_s: f32,
) -> Vec<f32> {
    extract_aligned_window(rolling, captured_samples, slot_samples, alignment_s, 12_000)
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
            let wsjt_mode = if mode == WorkspaceMode::Ft4 {
                WsjtMode::Ft4
            } else {
                WsjtMode::Fst4(match fst4_submode {
                    crate::modes::fst4::Submode::S15 => Fst4Submode::S15,
                    crate::modes::fst4::Submode::S30 => Fst4Submode::S30,
                    crate::modes::fst4::Submode::S60 => Fst4Submode::S60,
                    crate::modes::fst4::Submode::S120 => Fst4Submode::S120,
                    crate::modes::fst4::Submode::S300 => Fst4Submode::S300,
                })
            };
            if let Ok(batch) = decode_wsjt(
                &AudioBlock::new(12_000, samples.clone()).expect("normalized audio is valid"),
                wsjt_mode,
                &WsjtDecodeConfig {
                    frequency_min_hz: 100.0,
                    frequency_max_hz: 3_000.0,
                    sync_min: if mode == WorkspaceMode::Ft4 {
                        if deep_decode {
                            0.45
                        } else {
                            0.6
                        }
                    } else {
                        0.8
                    },
                    max_candidates: if mode == WorkspaceMode::Ft4 { 160 } else { 50 },
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
            let config = q65_live_decode_config();
            if let Ok(batch) = decode_wsjt(
                &AudioBlock::new(12_000, samples.clone()).expect("normalized audio is valid"),
                WsjtMode::Q65,
                &config,
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

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::{q65_live_decode_config, run_native_digital_decode, WorkspaceMode};
    use crate::modes::fst4::Submode;
    use crate::tx_audio::build_native_digital_tx_pcm;
    use qsonaut_modems::AudioBlock;
    use qsonaut_third_party::wsjt::{decode as decode_wsjt, WsjtMode};
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    #[test]
    fn q65_live_search_decodes_a_null_fixture_with_a_bounded_search() {
        let (pcm, _) = build_native_digital_tx_pcm(
            WorkspaceMode::Q65,
            "CQ W1AW AA00",
            1_500,
            Submode::default(),
            20,
            600,
        )
        .expect("Q65 fixture synthesis");
        let samples = pcm
            .into_iter()
            .map(|sample| sample as f32 / i16::MAX as f32 * 0.06)
            .collect::<Vec<_>>();
        let started = Instant::now();
        let batch = decode_wsjt(
            &AudioBlock::new(12_000, samples).expect("Q65 audio block"),
            WsjtMode::Q65,
            &q65_live_decode_config(),
        )
        .expect("Q65 decode");
        assert!(
            started.elapsed().as_secs() < 10,
            "Q65 live search exceeded realtime safety bound"
        );
        assert!(
            batch
                .events
                .iter()
                .any(|event| event.message.contains("W1AW")),
            "Q65 fixture did not decode: {:?}",
            batch.events
        );
    }

    #[test]
    fn q65_live_search_handles_a_short_capture_without_hanging() {
        let started = Instant::now();
        let result = decode_wsjt(
            &AudioBlock::new(12_000, vec![0.0; 12_000]).expect("short audio block"),
            WsjtMode::Q65,
            &q65_live_decode_config(),
        );
        assert!(
            started.elapsed().as_secs() < 10,
            "Q65 short-capture handling exceeded realtime safety bound"
        );
        assert!(result.is_ok(), "short Q65 capture should be a valid no-op");
        assert!(result.expect("checked above").events.is_empty());
    }

    #[test]
    fn native_decode_worker_handles_empty_captures_for_each_supported_protocol() {
        for mode in [
            WorkspaceMode::Ft4,
            WorkspaceMode::Fst4,
            WorkspaceMode::Wspr,
            WorkspaceMode::Jt9,
            WorkspaceMode::Jt65,
            WorkspaceMode::Q65,
            WorkspaceMode::Msk144,
        ] {
            let state = Arc::new(Mutex::new(crate::GuiState::default()));
            run_native_digital_decode(
                mode,
                Submode::default(),
                vec![0.0; 12_000],
                42,
                "00:00:42".to_string(),
                1_500,
                false,
                state.clone(),
            );
            let shared = state.lock().expect("state");
            assert!(
                shared.digital_decodes.is_empty(),
                "{mode:?} decoded silence"
            );
            assert!(shared.digital_decode_status.contains(mode.label()));
            if mode == WorkspaceMode::Ft4 {
                assert_eq!(shared.ft4_last_decode_period, Some(42));
            } else {
                assert_eq!(shared.ft4_last_decode_period, None);
            }
        }
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
    let audio = AudioBlock::new(12_000, samples).expect("normalized audio is valid");
    let outcome = trace.measure("protocol decode", || {
        decode_wsjt(
            &audio,
            WsjtMode::Ft8,
            &WsjtDecodeConfig {
                frequency_min_hz: 100.0,
                frequency_max_hz: 3_000.0,
                sync_min: if deep_decode {
                    FT8_DEEP_SYNC_MIN
                } else {
                    FT8_FAST_SYNC_MIN
                },
                max_candidates: if deep_decode {
                    FT8_DEEP_MAX_CAND
                } else {
                    FT8_FAST_MAX_CAND
                },
                deep_decode,
                ..WsjtDecodeConfig::default()
            },
        )
        .expect("normalized FT8 audio and mode are valid")
    });

    let results: Vec<Ft8DecodeEntry> = trace.measure("unpack results", || {
        let mut results = Vec::new();
        for event in &outcome.events {
            let msg = &event.message;
            let is_cq = msg.starts_with("CQ");
            let snr = event.snr_db.unwrap_or_default().round() as i8;
            let absolute_dt_s = alignment_s + event.delta_time_seconds.unwrap_or_default();
            debug!(
                freq = event.audio_frequency_hz.unwrap_or_default(),
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
                freq_hz: event
                    .audio_frequency_hz
                    .unwrap_or_default()
                    .max(0.0)
                    .round() as u32,
                message: msg.clone(),
                is_cq,
            });
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
