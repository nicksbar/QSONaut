# External Modem Backends (time-saver shortlist)

This project is intentionally moving toward **integration-first** modem strategy: use maintained external backends where possible, keep QSONaut focused on UX + radio orchestration.

## Integrated now

- [`mfsk-core`](https://github.com/jl1nie/mfsk-core) (GPL-3.0-or-later,
  developed by its upstream contributors; local path sibling clone)
  - FT8/FT4/FST4/WSPR/JT9/JT65/Q65/MSK144 support
  - QSONaut wires mode-specific receive adapters for each of those families.
  - FT8, FT4, FST4-60, JT9, JT65, and Q65-30A also have scheduled transmit
    synthesis. WSPR and MSK144 are receive-only in the current UI.
- [`komitoto-sstv`](https://github.com/IRendy/komitoto/tree/c98945f7c89f714b3182457a86b15a0c43cb6de6/crates/komitoto-sstv)
  (MIT, pinned Git revision)
  - Complete-frame codecs and timing definitions for 13 Martin, Scottie,
    Robot, and PD modes.
  - Wrapped by `qsonaut-sstv`; QSONaut keeps the live streaming receiver,
    audio-window alignment, VIS/AFC diagnostics, and TX safety boundary.
  - The UI exposes all 13 modes for experimental TX and receive. RX can select
    the codec automatically from VIS or filter for an explicitly selected mode.

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
3. License compatibility and complete attribution in QSONaut distributions.
4. Proven real-world decode behavior or strong test corpus.
5. Low integration complexity (streaming audio + callback model).

## Planned architecture

- Keep QSONaut GUI/workspaces mode-oriented.
- Route each mode to a backend adapter (e.g., `MfskBackend`, `CwBackend`, `FldigiBridge`).
- Avoid mode-specific DSP logic in GUI crate.
- Contribute bugfixes/perf improvements upstream where practical.
