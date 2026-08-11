# RigForge

RigForge is a modern radio operating platform with a strict architectural rule:

**DSP and radio control must never depend on the LLM.**

AI is advisory and tool-driven, not part of the realtime signal/control path.

## Current status

This repository is initialized to **M0 (Bootstrap)** as a Rust workspace with:

- App shell (`apps/rigforge`)
- Core configuration + event bus (`crates/rigforge-core`)
- Structured logging (`crates/rigforge-log`)
- Native GUI shell (`crates/rigforge-gui`)
- Device/audio abstraction (`crates/rigforge-audio`)
- Radio abstraction (`crates/rigforge-radio`)
- DSP and modes placeholders (`crates/rigforge-dsp`, `crates/rigforge-modes`)
- AI provider abstraction (`crates/rigforge-ai`)

Recent architecture update:

- FT8 live decode path in GUI is now integrated with external modem backend `mfsk-core` (local path dependency during active development), replacing custom in-app decoder logic for the active mode.
- Workspace mode tabs now cover the modem families being staged (`FT8/FT4/FST4/WSPR/JT9/JT65/Q65/MSK144`) plus `CW` and `FLDIGI` integration surfaces.
- Decode log UX includes auto-follow toggle, configurable trimming, and clearer row grouping.
- Operator profile inputs (callsign/grid/QTH) are now editable in-app and shared across modem workspaces.
- FT8 automatic operation supports standard QSO sequencing and deterministic caller selection by first decoded, strongest, weakest, or closest RX tone.
- FT8 TX now confirms CI-V PTT before audio and applies configurable lead/tail timing with visible failure reporting.
- Completed FT8 contacts are saved to an editable local QSO log with ADIF export for future service integrations.

## Architecture boundaries

- **Realtime path:** device I/O, DSP, decoders, timing, radio control.
- **Advisory path:** station intelligence, recommendations, operator-assist workflows.

In practical terms: with AI disabled, RigForge remains fully functional.

## Repository context docs

The `docs/` directory contains curated research imported from an adjacent project and should be treated as foundational context for protocol and radio-control implementation planning.

See also:

- `docs/external-modem-backends.md` for backend selection and integration strategy.
- `docs/progress/2026-08-10-reactive-gui-audio-stability.md` for stability milestones.

## Run

```text
cargo run -p rigforge -- --help
```

Launch the native operator console (recommended):

```text
cargo run -p rigforge -- --gui
```

## Environment and hardware config

RigForge now supports a `.env` file and hardware toggles so you can keep the app usable even when radios are unplugged.

Copy the example file:

```text
cp .env.example .env
```

Useful variables:

```text
RIGFORGE_AUDIO_ENABLED=true
RIGFORGE_AUDIO_INPUT_DEVICE="USB Audio CODEC"
RIGFORGE_AUDIO_OUTPUT_DEVICE="USB Audio CODEC"
RIGFORGE_AUDIO_SAMPLE_RATE_HZ=48000
RIGFORGE_AUDIO_CHANNELS=1

RIGFORGE_RADIO_ENABLED=true
RIGFORGE_RADIO_BACKEND="none"
RIGFORGE_RADIO_SERIAL_PORT="/dev/ttyUSB0"
RIGFORGE_RADIO_BAUD_RATE=115200
```

With `RIGFORGE_AUDIO_ENABLED=false` or `RIGFORGE_RADIO_ENABLED=false`, RigForge will stay functional without requiring those devices.

Radio control defaults to `115200` baud unless overridden by `--radio-baud` or `RIGFORGE_RADIO_BAUD_RATE`.

## Stage 1 (current) quick checks

Discover audio capture devices:

```text
cargo run -p rigforge -- --list-audio
```

Discover serial endpoints (CI-V candidates):

```text
cargo run -p rigforge -- --list-radio
```

Capture a WAV sample (10s default):

```text
cargo run -p rigforge -- --record-wav recordings/ic7300-test.wav --duration-secs 10
```

## Milestones

- M0 — Bootstrap workspace + shell (current)
- M1 — Audio capture from radio interface
- M2 — Spectrum/waterfall + DSP metrics
- M3 — CI-V radio control abstraction + implementation
- M4+ — FT8 decode, station intelligence, and eventually safe AI assistant tooling
