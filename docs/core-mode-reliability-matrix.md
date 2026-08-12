# Core mode reliability matrix (Phase 1)

Status: draft-in-use  
Date: 2026-08-12  
Scope: FT8, FT4, JT9

## Reliability SLOs (local/CI gate targets)

- **Decode slot arming correctness:** 100% in deterministic unit tests.
- **Startup behavior:** No mid-slot startup decode trigger for slot-gated modes.
- **TX-slot self-decode suppression:** 100% for known local-TX periods.
- **Reply scheduling determinism (FT8):** boundary behavior fixed at configured deadline and PTT lead assumptions.
- **Retry safety:** no period arithmetic wrap hazards in retry decisions.

## Mode matrix

| Mode | Slot model | Decode trigger model | TX model | Current hardening status |
|---|---:|---|---|---|
| FT8 | 15.0 s | Boundary + early decode window (adaptive dT) | Timed slot TX queue with parity/reply policies | Active (tests expanded) |
| FT4 | 7.5 s | Boundary + early decode window (adaptive dT) | Timed slot TX queue | Active (tests expanded) |
| JT9 | 60.0 s | Slot-length decode pass (native path) | Native synthesis path available | Active (decode-path test added) |

## Test mapping (implemented)

### FT8 timing + scheduler

- `tests::ft8_slot_gate_never_decodes_mid_slot_after_startup`
- `tests::ft8_slot_gate_honors_adaptive_decode_threshold`
- `tests::ft8_slot_gate_waits_for_buffer_readiness_at_decode_time`
- `ft8_ops::tests::reply_deadline_boundary_is_inclusive_then_rolls_forward`
- `ft8_ops::tests::next_tx_period_rejects_candidate_once_ptt_window_opens`
- `ft8_ops::tests::retry_guard_is_saturating_and_never_wraps_period_math`

### FT4 timing + decode path

- `tests::ft4_workspace_adapter_decodes_generated_audio`
- `tests::early_ft4_capture_contains_a_deliberately_late_decodable_waveform`

### JT9 decode path

- `tests::jt9_workspace_adapter_decodes_generated_audio`

### Cross-mode guardrails

- `tests::phase1_target_modes_have_slot_and_decoder_support`
- `tests::digital_slot_gate_requires_a_complete_period_after_startup`
- `tests::digital_slot_gate_reset_requires_new_boundary_again`

## Upstream behavior anchors (WSJT-X)

These files are used as behavioral references when flow questions arise:

- `lib/ft8_decode.f90`
- `lib/ft4_decode.f90`
- `lib/jt9_decode.f90`
- `lib/decoder.f90`

Reference source mirror used in this cycle:

- `https://git.code.sf.net/p/wsjt/wsjtx`
- clone head observed: `b4f9a43`
