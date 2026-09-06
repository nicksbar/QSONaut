# External Modem Backends (time-saver shortlist)

This project is intentionally moving toward **integration-first** modem strategy: use maintained external backends where possible, keep QSONaut focused on UX + radio orchestration.

## Integrated now

- [`mfsk-core`](https://github.com/jl1nie/mfsk-core) (GPL-3.0-or-later,
  developed by its upstream contributors; pinned Git revision behind
  `qsonaut-third-party`)
  - FT8/FT4/FST4/WSPR/JT9/JT65/all wired Q65 submodes/MSK144 support
  - QSONaut wires mode-specific receive adapters for each of those families.
  - FT8, FT4, FST4-60, JT9, JT65, and all selectable Q65 submodes have
    scheduled transmit synthesis in the current QSONaut UI. WSPR and MSK144
    are receive-only in the current UI.
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

### RADE voice adapter

RADE V1 and V2 are now exposed as a first-class QSONaut `WorkspaceMode` and
share the Voice mode's band vocabulary. The GUI uses the adapter's stable
capability metadata and labels V2 as upstream development; it does not hide
V2 behind an experimental product path or create a second voice GUI.

The current consumer boundary is intentionally visible in the RADE panel:
the third-party adapter provides the V1/V2 modem/IQ and speech feature
surface, while QSONaut owns capture, buffering, resampling, status, and TX
safety. With `rade-speech` enabled, the audio worker now routes captured audio
through the RADE receiver and FARGAN speech decoder; synthesized speech is
validated at the boundary but is not yet routed to an operator playback path.
QSONaut does not claim live RADE TX yet.

When the native backend is intentionally installed, the optional
`qsonaut-gui/rade-speech` feature makes the null-audio source run deterministic
16 kHz speech frames through the third-party LPCNet/FARGAN bridge, aggregate
them into RADE V1/V2 modem frames, and resample the resulting 8 kHz waveform
into QSONaut's simulated audio stream. The lower-level `rade-c` feature remains
available for feature-vector fixtures. Neither feature enables live microphone
TX or requires the default build to link the native library.

### Future modem review

VarAC is intentionally not presented as a QSONaut roadmap target because its
proprietary protocol does not meet the project's adoption preference. Before
implementing any future backend, verify that its protocol is documented, open
source or otherwise legally usable, and suitable for a QSONaut-owned adapter.
Do not add a dependency or reverse-engineer a closed protocol as a shortcut.

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
