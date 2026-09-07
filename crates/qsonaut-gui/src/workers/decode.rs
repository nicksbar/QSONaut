use super::super::*;
use qsonaut_modems::{extract_aligned_window, AudioBlock};
use qsonaut_third_party::wsjt::{
    acquire_ft8_slot_phases, decode as decode_wsjt, Fst4Submode, WsjtDecodeConfig, WsjtMode,
    FT8_SLOT_ACQUISITION_REQUIRED_SAMPLES,
};

const FT8_SLOT_MS: u128 = 15_000;
const FT8_DEEP_RUNTIME_BUDGET_MS: u128 = 12_000;
const FT8_DEEP_SYNC_MIN: f32 = 1.3;
const FT8_DEEP_MAX_CAND: usize = 120;

fn q65_live_decode_config() -> WsjtDecodeConfig {
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
    let q65_submode = state.lock().expect("ui state lock poisoned").q65_submode;
    let js8_controls = state.lock().expect("ui state lock poisoned").js8_controls;
    if mode == WorkspaceMode::Js8 {
        let rms = (samples.iter().map(|sample| sample * sample).sum::<f32>()
            / samples.len().max(1) as f32)
            .sqrt();
        let peak = samples
            .iter()
            .map(|sample| sample.abs())
            .fold(0.0_f32, f32::max);
        info!(
            mode = ?js8_controls.mode,
            waterfall = js8_controls.waterfall,
            selected_audio_hz,
            sample_count = samples.len(),
            slot_rms_dbfs = 20.0 * rms.max(1e-9).log10(),
            slot_peak_dbfs = 20.0 * peak.max(1e-9).log10(),
            "JS8 decode pass starting"
        );
    }
    let budget =
        Duration::from_secs_f64(mode.slot_seconds(fst4_submode, q65_submode).unwrap_or(15.0));
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
            if let Ok(batch) = decode_wsjt(
                &AudioBlock::new(12_000, samples.clone()).expect("normalized audio is valid"),
                WsjtMode::Q65(q65_submode),
                &q65_live_decode_config(),
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
        WorkspaceMode::Js8 => {
            let audio = AudioBlock::new(12_000, samples).expect("normalized audio is valid");
            let rx_config = js8_controls.rx_config(selected_audio_hz as f32);
            let mut focused_decode = false;
            let signal_rms = (audio
                .samples
                .iter()
                .map(|sample| sample * sample)
                .sum::<f32>()
                / audio.samples.len().max(1) as f32)
                .sqrt();
            let waterfall_scan = js8_controls.waterfall && signal_rms >= 1.0e-4;
            // Waterfall mode must be primary: a successful cursor decode must
            // not hide the other carriers in the same JS8 window. Focused
            // decoding remains available when waterfall mode is disabled.
            if !waterfall_scan {
                match qsonaut_js8::decode_audio_block_detailed(&audio, rx_config) {
                    Ok(result) => {
                        focused_decode = true;
                        let message = crate::modes::js8::format_js8_message(&result.message);
                        info!(
                            message = %message,
                            frequency_hz = ?result.event.audio_frequency_hz,
                            snr_db = ?result.event.snr_db,
                            "JS8 focused decode succeeded"
                        );
                        push(
                            result.event.snr_db.unwrap_or_default(),
                            result.event.delta_time_seconds.unwrap_or_default(),
                            result.event.audio_frequency_hz.unwrap_or_default(),
                            message,
                        );
                    }
                    Err(error) => {
                        debug!(error = %error, "JS8 focused decode did not produce a result");
                    }
                }
            }
            // A wide scan is substantially more expensive than a focused
            // decode.  Do not start one for an empty slot: the null modem and
            // real receivers both have quiet periods, and allowing a scan on
            // every quiet slot can keep the single decode worker occupied
            // across the next JS8 boundary.
            if waterfall_scan {
                let scan_started = Instant::now();
                let mut scan_config = js8_controls.scan_config();
                // A JS8 slot contains one protocol frame. The modem scanner
                // supports recording scans with many rolling candidates, but
                // using that policy here would retry the same slot once per
                // second (up to the whole 15/30-second capture) and can run
                // past the next slot boundary. The slot gate already gives us
                // a protocol-aligned window, so perform one bounded waterfall
                // extraction while retaining all configured signal passes.
                scan_config.max_candidates = 1;
                scan_config.step_samples = 1;
                match qsonaut_js8::scan_audio_block_detailed(&audio, rx_config, scan_config) {
                    Ok(results) => {
                        info!(
                            result_count = results.len(),
                            elapsed_ms = scan_started.elapsed().as_millis() as u64,
                            "JS8 waterfall scan complete"
                        );
                        for result in results {
                            let message =
                                crate::modes::js8::format_js8_message(&result.result.message);
                            let event = result.result.event;
                            push(
                                event.snr_db.unwrap_or_default(),
                                event.delta_time_seconds.unwrap_or_default(),
                                event.audio_frequency_hz.unwrap_or_default(),
                                message,
                            );
                        }
                    }
                    Err(error) => {
                        warn!(
                            error = %error,
                            elapsed_ms = scan_started.elapsed().as_millis() as u64,
                            "JS8 waterfall scan failed"
                        );
                    }
                }
            } else if js8_controls.waterfall && !focused_decode {
                debug!(signal_rms, "JS8 waterfall scan skipped for quiet input");
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
        | WorkspaceMode::Rade
        | WorkspaceMode::Sstv => {}
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
    let received_at =
        (period as f64 * mode.slot_seconds(fst4_submode, q65_submode).unwrap_or(15.0)) as u32;
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
    shared.digital_decode_status = format!("{} decode complete", mode.label());
    shared.digital_decodes.extend(decoded);
    while shared.digital_decodes.len() > 300 {
        shared.digital_decodes.pop_front();
    }
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::{
        acquire_ft8_alignment, run_ft8_decode_worker, run_native_digital_decode, PendingFt8Decode,
        WorkspaceMode,
    };
    use crate::modes::fst4::Submode;
    use crate::tx_audio::build_native_digital_tx_pcm;
    use crate::{FT4_EARLY_DECODE_S, FT4_SLOT_SAMPLES};
    use qsonaut_third_party::wsjt::{
        decode as decode_wsjt, WsjtDecodeConfig, WsjtMode, FT8_SLOT_ACQUISITION_REQUIRED_SAMPLES,
    };
    use std::sync::{Arc, Mutex};

    #[test]
    fn ft4_generated_fixture_decodes_with_live_configuration() {
        let pcm = build_native_digital_tx_pcm(
            WorkspaceMode::Ft4,
            "CQ W1AW AA00",
            1_500,
            Submode::default(),
            20,
            600,
        )
        .expect("FT4 fixture synthesis")
        .0;
        let samples = pcm
            .into_iter()
            .map(|sample| sample as f32 / i16::MAX as f32 * 0.06)
            .collect::<Vec<_>>();
        let batch = decode_wsjt(
            &qsonaut_modems::AudioBlock::new(12_000, samples).expect("FT4 audio block"),
            WsjtMode::Ft4,
            &WsjtDecodeConfig {
                frequency_min_hz: 100.0,
                frequency_max_hz: 3_000.0,
                sync_min: 0.6,
                max_candidates: 160,
                frequency_hint_hz: Some(1_500.0),
                ..WsjtDecodeConfig::default()
            },
        )
        .expect("FT4 decode");
        assert!(
            batch
                .events
                .iter()
                .any(|event| event.message.contains("W1AW")),
            "FT4 fixture did not decode: {:?}",
            batch.events
        );
    }

    #[test]
    fn ft4_early_capture_window_decodes_and_publishes_to_shared_log() {
        let pcm = build_native_digital_tx_pcm(
            WorkspaceMode::Ft4,
            "CQ W1AW AA00",
            1_500,
            Submode::default(),
            20,
            600,
        )
        .expect("FT4 fixture synthesis")
        .0;
        let captured_samples = (FT4_EARLY_DECODE_S * 12_000.0).round() as usize;
        let mut rolling = vec![0.0_f32; 12_000 * 10];
        let slot_boundary = rolling.len() - captured_samples;
        let waveform_start = slot_boundary + (0.5 * 12_000.0) as usize;
        let waveform = pcm
            .iter()
            .map(|sample| *sample as f32 / i16::MAX as f32 * 0.06)
            .collect::<Vec<_>>();
        assert!(waveform_start + waveform.len() <= slot_boundary + captured_samples);
        rolling[waveform_start..waveform_start + waveform.len()].copy_from_slice(&waveform);

        let samples =
            super::prepare_early_digital_slot(&rolling, captured_samples, FT4_SLOT_SAMPLES, 0.0);
        let state = Arc::new(Mutex::new(crate::GuiState::default()));
        super::run_native_digital_decode(
            WorkspaceMode::Ft4,
            Submode::default(),
            samples,
            42,
            "00:05:15.000".to_string(),
            1_500,
            false,
            state.clone(),
        );

        let shared = state.lock().expect("state");
        assert_eq!(shared.ft4_last_decode_period, Some(42));
        assert!(
            shared
                .digital_decodes
                .iter()
                .any(|entry| entry.mode == WorkspaceMode::Ft4 && entry.message.contains("W1AW")),
            "early FT4 capture did not publish a decode: {:?}",
            shared.digital_decodes
        );
    }

    #[test]
    fn js8_null_fixture_decodes_multiple_carriers_through_waterfall_path() {
        let first_frequency_hz = 1_071;
        let second_frequency_hz = 1_871;
        let first_pcm = build_native_digital_tx_pcm(
            WorkspaceMode::Js8,
            "CQ+N7UF+++\u{2b}+",
            first_frequency_hz,
            Submode::default(),
            20,
            600,
        )
        .expect("JS8 fixture synthesis")
        .0;
        let second_pcm = build_native_digital_tx_pcm(
            WorkspaceMode::Js8,
            "W1AW FN31",
            second_frequency_hz,
            Submode::default(),
            20,
            600,
        )
        .expect("second JS8 fixture synthesis")
        .0;
        let samples = first_pcm
            .into_iter()
            .zip(second_pcm)
            .map(|(first, second)| (first as f32 + second as f32) / i16::MAX as f32 * 0.06)
            .collect::<Vec<_>>();
        let mut samples = samples;
        samples.resize(15 * 12_000, 0.0);
        let state = Arc::new(Mutex::new(crate::GuiState {
            selected_audio_hz: first_frequency_hz,
            ..crate::GuiState::default()
        }));

        run_native_digital_decode(
            WorkspaceMode::Js8,
            Submode::default(),
            samples,
            42,
            "00:05:15.000".to_string(),
            first_frequency_hz,
            false,
            state.clone(),
        );

        let shared = state.lock().expect("state");
        let js8_decodes: Vec<_> = shared
            .digital_decodes
            .iter()
            .filter(|entry| entry.mode == WorkspaceMode::Js8)
            .collect();
        assert!(
            js8_decodes.len() >= 2,
            "JS8 waterfall did not decode both carriers: {:?}",
            shared.digital_decodes
        );
        assert!(
            js8_decodes
                .iter()
                .any(|entry| (entry.freq_hz as i32 - first_frequency_hz as i32).abs() <= 25)
                && js8_decodes
                    .iter()
                    .any(|entry| (entry.freq_hz as i32 - second_frequency_hz as i32).abs() <= 25),
            "JS8 waterfall frequencies were not recovered: {:?}",
            js8_decodes
        );
    }

    #[test]
    #[ignore = "slow FST4/Q65 end-to-end DSP validation; run in release mode"]
    fn native_generated_signals_decode_through_the_adapter() {
        for (mode, submode, message) in [
            (WorkspaceMode::Fst4, Submode::S15, "CQ W1AW AA00"),
            (WorkspaceMode::Jt9, Submode::default(), "CQ W1AW AA00"),
            (WorkspaceMode::Jt65, Submode::default(), "CQ W1AW AA00"),
            (WorkspaceMode::Q65, Submode::default(), "CQ W1AW AA00"),
        ] {
            let pcm = build_native_digital_tx_pcm(mode, message, 1_500, submode, 20, 600)
                .expect("native fixture synthesis")
                .0;
            let samples = pcm
                .into_iter()
                .map(|sample| sample as f32 / i16::MAX as f32 * 0.06)
                .collect::<Vec<_>>();
            let state = Arc::new(Mutex::new(crate::GuiState::default()));

            run_native_digital_decode(
                mode,
                submode,
                samples,
                42,
                "00:05:15.000".to_string(),
                1_500,
                false,
                state.clone(),
            );

            let shared = state.lock().expect("state");
            assert!(
                shared
                    .digital_decodes
                    .iter()
                    .any(|entry| entry.mode == mode && entry.message.contains("W1AW")),
                "{mode:?} fixture did not decode: {:?}",
                shared.digital_decodes
            );
        }
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

    #[test]
    fn ft8_decode_worker_publishes_telemetry_for_silence_without_results() {
        let state = Arc::new(Mutex::new(crate::GuiState::default()));
        let deferred = Arc::new(Mutex::new(None));
        run_ft8_decode_worker(
            PendingFt8Decode {
                samples: vec![0.0; 12_000],
                acquisition_samples: Vec::new(),
                captured_samples: 12_000,
                utc: "00:00:00.000".to_string(),
                period: 7,
                deep_decode: false,
                alignment_s: 0.25,
            },
            state.clone(),
            deferred,
        );

        let state = state.lock().expect("state");
        assert!(state.ft8_compute_telemetry.is_some());
        assert_eq!(state.ft8_last_decode_period, Some(7));
        assert_eq!(state.ft8_decode_status, "FT8 decode complete");
        assert!(state.ft8_pending.is_empty());
    }

    #[test]
    fn quiet_ft8_acquisition_does_not_claim_a_lock() {
        let (alignment, confidence, candidates) = acquire_ft8_alignment(
            &vec![0.0; FT8_SLOT_ACQUISITION_REQUIRED_SAMPLES],
            FT8_SLOT_ACQUISITION_REQUIRED_SAMPLES,
        );
        assert!(alignment.is_none());
        assert!(confidence.is_none());
        assert_eq!(candidates, 0);
    }
}

pub(in super::super) fn run_ft8_decode_worker(
    mut pending: PendingFt8Decode,
    state: Arc<Mutex<GuiState>>,
    deferred_decode: Arc<Mutex<Option<PendingFt8Decode>>>,
) {
    loop {
        if !pending.acquisition_samples.is_empty() {
            let period = pending.period;
            let (alignment, confidence, candidates) =
                acquire_ft8_alignment(&pending.acquisition_samples, pending.captured_samples);
            let mut state_guard = state.lock().expect("ui state lock poisoned");
            state_guard.ft8_acquisition_candidates = candidates;
            state_guard.ft8_last_acquisition_period = Some(period);
            if let Some((alignment, confidence)) = alignment.zip(confidence) {
                pending.alignment_s = alignment;
                pending.samples = prepare_early_ft8_slot(
                    &pending.acquisition_samples,
                    pending.captured_samples,
                    alignment,
                );
                state_guard.ft8_slot_phase_s = Some(alignment);
                state_guard.ft8_sync_confidence = Some(confidence);
                state_guard.ft8_sync_state = Ft8SyncState::Locked;
                state_guard.ft8_acquisition_generation = state_guard.ft8_reacquire_generation;
            } else {
                state_guard.ft8_sync_confidence = None;
                state_guard.ft8_sync_state = Ft8SyncState::Unlocked;
            }
            drop(state_guard);
            pending.acquisition_samples.clear();
        }
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

fn acquire_ft8_alignment(
    samples: &[f32],
    captured_samples: usize,
) -> (Option<f32>, Option<f32>, usize) {
    if samples.len() < FT8_SLOT_ACQUISITION_REQUIRED_SAMPLES {
        return (None, None, 0);
    }
    let config = WsjtDecodeConfig {
        frequency_min_hz: 100.0,
        frequency_max_hz: 3_000.0,
        sync_min: FT8_FAST_SYNC_MIN,
        max_candidates: FT8_FAST_MAX_CAND,
        ..WsjtDecodeConfig::default()
    };
    let audio = AudioBlock::new(12_000, samples.to_vec()).expect("normalized audio is valid");
    let candidates = acquire_ft8_slot_phases(&audio, &config).unwrap_or_default();
    let candidate_count = candidates.len();
    for candidate in candidates.into_iter().take(8) {
        let window = extract_aligned_window(
            samples,
            captured_samples,
            FT8_SLOT_SAMPLES,
            candidate.delta_time_seconds,
            12_000,
        );
        let Ok(batch) = decode_wsjt(
            &AudioBlock::new(12_000, window).expect("normalized audio is valid"),
            WsjtMode::Ft8,
            &config,
        ) else {
            continue;
        };
        if !batch.events.is_empty() {
            return (
                Some(candidate.delta_time_seconds),
                Some(candidate.weight),
                candidate_count,
            );
        }
    }
    (None, None, candidate_count)
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
        for event in outcome.events {
            let msg = event.message;
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
                message: msg,
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
    s.ft8_decode_status = "FT8 decode complete".to_string();
    if !results.is_empty() {
        s.ft8_pending.extend(results);
    }

    elapsed_ms
}
