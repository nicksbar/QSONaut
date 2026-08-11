# External Modem Backends (time-saver shortlist)

This project is intentionally moving toward **integration-first** modem strategy: use maintained external backends where possible, keep RigForge focused on UX + radio orchestration.

## Integrated now

- `mfsk-core` (local path sibling clone)
  - FT8/FT4/FST4/WSPR/JT9/JT65/Q65/MSK144 support
  - RigForge currently wires FT8 decode path through this backend.

## Next targets

### CW / Morse

Candidate Rust backends observed:

- `swilcox/cw-dit`
  - Cross-platform CW/Morse decoder app/project.
  - Promising as DSP/decoder reference for live CW detection workflows.
- `burumdev/morse-codec`
  - Library-style Morse encoder/decoder API (embedded-friendly orientation).
- `qsantos/ripmors`
  - Fast Morse encoder/decoder crate, useful for text-domain CW handling.

### FLDIGI ecosystem integration

Candidate Rust bridge:

- `elitegreg/ham-radio-digital-interfacing-rs`
  - Focuses on interfacing with digital apps (WSJT-X/FLDIGI class integration patterns).
  - Useful for control/interop bridge rather than full DSP modem implementation.

## Selection criteria before adoption

1. Active maintenance cadence (recent commits/issues).
2. Clear API surface for library embedding (not app-only).
3. License compatibility with RigForge distribution strategy.
4. Proven real-world decode behavior or strong test corpus.
5. Low integration complexity (streaming audio + callback model).

## Planned architecture

- Keep RigForge GUI/workspaces mode-oriented.
- Route each mode to a backend adapter (e.g., `MfskBackend`, `CwBackend`, `FldigiBridge`).
- Avoid mode-specific DSP logic in GUI crate.
- Contribute bugfixes/perf improvements upstream where practical.
