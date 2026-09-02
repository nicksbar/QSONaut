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
  - The reusable streaming receiver, audio-window alignment, and VIS/AFC
    diagnostics now live in `qsonaut-third-party`; QSONaut owns the GUI and TX
    safety boundary.
  - The UI exposes all 13 modes for experimental TX and receive. RX can select
    the codec automatically from VIS or filter for an explicitly selected mode.
  - A QSONaut-owned single-channel acquisition layer scans shifted VIS headers
    across the audio baseband, retains waterfall click override, and publishes
    structured diagnostics to the shared local Application Log.

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

### Future modem review

The GUI keeps disabled roadmap tabs for VaraAC and DV RADE. They are not
`WorkspaceMode` variants and have no runtime, audio, or decoder integration.
Before implementation, verify that each project's protocol is documented,
open source or otherwise legally usable, and suitable for a QSONaut-owned
adapter. Do not add a dependency or reverse-engineer a closed protocol as a
shortcut.

## Selection criteria before adoption

1. Active maintenance cadence (recent commits/issues).
2. Clear API surface for library embedding (not app-only).
3. License compatibility and complete attribution in QSONaut distributions.
4. Proven real-world decode behavior or strong test corpus.
5. Low integration complexity (streaming audio + callback model).

## Planned architecture

- Keep QSONaut GUI/workspaces mode-oriented.
- Route each implemented mode to a backend adapter (e.g., `MfskBackend`, `CwBackend`).
- Avoid mode-specific DSP logic in GUI crate.
- Contribute bugfixes/perf improvements upstream where practical.
