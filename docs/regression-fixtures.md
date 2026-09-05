# v0.4 deterministic regression fixtures

Status: expanded deterministic software catalog

This page is the starting inventory for v0.4 issue #83. A fixture is useful
only when its inputs, generation method or provenance, expected result, and
test command are stable enough for another contributor to reproduce it.

## Current CI-covered fixtures

These are deterministic software fixtures already present in the checkout.
They are generated in memory or built from explicit test data, so they do not
depend on a sound card, radio, wall clock, or external provider.

| Area | Current fixture/test | What it proves | What it does not prove |
|---|---|---|---|
| FT8 | `early_ft8_slot_contains_a_decodable_complete_waveform` | A generated complete waveform can be decoded in the early-slot path. | Reception from a physical radio or noisy band. |
| FT8 | `ft8_fixture_tolerates_level_timing_and_low_noise_variation` | Generated FT8 remains decodable across deterministic level, timing, and low-noise perturbations. | RF fading, multipath, and recordings from a physical receiver. |
| FT4 | `early_ft4_capture_contains_a_deliberately_late_decodable_waveform` | The early capture path handles a deliberately late-decodable waveform. | Timing behavior on a real device clock or RF path. |
| FT4 | `ft4_workspace_adapter_decodes_generated_audio` | The workspace adapter reaches the decoder with generated audio. | Broad weak-signal and interference performance. |
| FT4 | `ft4_null_fixture_tolerates_level_and_timing_variation`, `ft4_fixture_tolerates_deterministic_low_noise` | Generated FT4 survives deterministic amplitude, timing, and low-noise variation. | Real weak-signal propagation and receiver impairments. |
| Carrier rejection | `steady_carrier_fixture_does_not_create_a_digital_decode` | A deterministic unmodulated carrier does not produce a false FT8 decode. | Arbitrary interference mixtures or decoder behavior outside the tested band. |
| JT9 | `jt9_workspace_adapter_decodes_generated_audio` | The JT9 workspace adapter decodes its generated test signal. | A complete JT9 operating workflow or hardware result. |
| Slot guards | `digital_slot_gate_requires_a_complete_period_after_startup`, `digital_slot_gate_reset_requires_new_boundary_again` | Startup and reset cannot trigger an unsafe mid-period decode. | Long-running clock drift and device scheduling behavior. |
| Radio recovery | `unavailable_radio_rejects_queued_ptt_without_touching_driver` | A queued acknowledged PTT command is rejected with an explicit powered-off error, never reaches the driver, and leaves PTT off. | Physical disconnect timing and reconnect behavior on a real transport. |
| TX cancellation/recovery | `worker_disabled_constructor_supports_safe_tx_pipeline_transitions`, `tx_safety_detects_and_clears_every_transmit_path` | Failed TX events and global disarm clear FT8, native digital, SSTV, queued, and active transmit state. | Audio-interface and radio timing during a physical failure. |
| Worker disconnect | `radio_session_stop_contract_signals_every_owned_worker`, `audio_worker_disabled_path_reports_disabled_without_opening_devices` | Session shutdown signals radio/audio/SWR workers and disabled audio cannot open hardware. | Kernel/device unplug timing and reconnect behavior. |
| FT8 scheduling | `reply_deadline_boundary_is_inclusive_then_rolls_forward`, `next_tx_period_rejects_candidate_once_ptt_window_opens`, `retry_guard_is_saturating_and_never_wraps_period_math` | Boundary, PTT-window, and retry arithmetic behavior is deterministic. | Actual PTT latency on a radio. |
| Configuration | Isolated temporary-file fixtures in `qsonaut-core/src/config.rs` and `profile::tests::malformed_profile_fixture_is_rejected_without_partial_state` | Invalid and valid configuration/profile loading is testable without the user profile, including rejection of a malformed persisted baud value. | Every historical migration format or automatic repair of arbitrary corruption. |
| ADIF | `imports_adif_records_and_skips_duplicates`, `imports_contest_pota_grid_and_malformed_fields_without_losing_valid_records` and related tests | Import normalization, duplicate handling, contest/POTA/grid fields, malformed records, and export fields are stable for covered records. | Interoperability with every external logger. |
| Local AI | `local_policy_allows_only_loopback_http`, `assistant_content_and_ndjson_parser_handle_empty_or_malformed_values`, and model-role selection tests | Non-loopback providers, malformed responses, missing models, undownloaded models, and incompatible roles fail safely with actionable errors. | A live provider, model quality, or GPU/VRAM behavior. |

The names above are intentionally linked to source tests rather than copied
audio files. This keeps the CI catalog reviewable and avoids silently claiming
provenance for recordings that are not checked into the repository. Signal
perturbations use fixed arithmetic generators and fixed seeds; they do not use
wall-clock time, a sound card, a radio, or an external provider.

## Reproduction

Run the focused GUI and logging suites from the repository root:

```text
cargo test -p qsonaut-gui
cargo test -p qsonaut-log
cargo test -p qsonaut-core
```

The normal workspace test/lint workflow remains the release gate. These
focused commands are a quick fixture smoke test, not a substitute for the
full CI matrix.

## Remaining evidence gaps

The deterministic software scope for issue #83 is now represented above. The
following evidence remains intentionally separate and should be added only
with source or generation metadata:

- checked-in physical weak-signal/noise recordings for FT8, FT4, CW, and SSTV;
- physical radio/audio disconnect timing and reconnect evidence;
- additional external-logger ADIF samples and interoperability reports;
- live-provider AI failure reports, including model loading and resource
  exhaustion behavior.

Physical recordings and hardware results belong in the validation notes for
the relevant radio/mode issue. They should include the station, device/sample
rate, generation or capture provenance, expected result, observed result, and
sanitized diagnostics.
