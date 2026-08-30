# v0.4 deterministic regression fixtures

Status: initial catalog

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
| FT4 | `early_ft4_capture_contains_a_deliberately_late_decodable_waveform` | The early capture path handles a deliberately late-decodable waveform. | Timing behavior on a real device clock or RF path. |
| FT4 | `ft4_workspace_adapter_decodes_generated_audio` | The workspace adapter reaches the decoder with generated audio. | Broad weak-signal and interference performance. |
| JT9 | `jt9_workspace_adapter_decodes_generated_audio` | The JT9 workspace adapter decodes its generated test signal. | A complete JT9 operating workflow or hardware result. |
| Slot guards | `digital_slot_gate_requires_a_complete_period_after_startup`, `digital_slot_gate_reset_requires_new_boundary_again` | Startup and reset cannot trigger an unsafe mid-period decode. | Long-running clock drift and device scheduling behavior. |
| FT8 scheduling | `reply_deadline_boundary_is_inclusive_then_rolls_forward`, `next_tx_period_rejects_candidate_once_ptt_window_opens`, `retry_guard_is_saturating_and_never_wraps_period_math` | Boundary, PTT-window, and retry arithmetic behavior is deterministic. | Actual PTT latency on a radio. |
| Configuration | Isolated temporary-file fixtures in `qsonaut-core/src/config.rs` | Invalid and valid configuration loading is testable without the user profile. | Every historical migration format. |
| ADIF | Representative records in `qsonaut-log/src/lib.rs` tests | Import normalization, duplicate handling, and export fields are stable for covered records. | Interoperability with every external logger. |

The names above are intentionally linked to source tests rather than copied
audio files. This keeps the first CI catalog reviewable and avoids silently
claiming provenance for recordings that are not checked into the repository.

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

## Next fixture additions

The following gaps remain open for issue #83 and should be added with source
or generation metadata before being treated as release evidence:

- checked-in or reproducibly generated weak-signal, noise, and steady-carrier
  vectors for FT8, FT4, CW, and SSTV;
- invalid headers, late-slot, cancellation, and failed-TX recovery cases;
- malformed profile/configuration and local-AI provider/model failure cases;
- representative ADIF records covering contest, POTA, grid, and malformed
  fields;
- mockable radio/audio disconnect cases with expected safe-state diagnostics.

Physical recordings and hardware results belong in the validation notes for
the relevant radio/mode issue. They should include the station, device/sample
rate, generation or capture provenance, expected result, observed result, and
sanitized diagnostics.
