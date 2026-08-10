# RigForge

RigForge is a modern radio operating platform with a strict architectural rule:

**DSP and radio control must never depend on the LLM.**

AI is advisory and tool-driven, not part of the realtime signal/control path.

## Current status

This repository is initialized to **M0 (Bootstrap)** as a Rust workspace with:

- App shell (`apps/rigforge`)
- Core configuration + event bus (`crates/rigforge-core`)
- Structured logging (`crates/rigforge-log`)
- TUI shell scaffolding (`crates/rigforge-tui`)
- Device/audio abstraction (`crates/rigforge-audio`)
- Radio abstraction (`crates/rigforge-radio`)
- DSP and modes placeholders (`crates/rigforge-dsp`, `crates/rigforge-modes`)
- AI provider abstraction (`crates/rigforge-ai`)

## Architecture boundaries

- **Realtime path:** device I/O, DSP, decoders, timing, radio control.
- **Advisory path:** station intelligence, recommendations, operator-assist workflows.

In practical terms: with AI disabled, RigForge remains fully functional.

## Repository context docs

The `docs/` directory contains curated research imported from an adjacent project and should be treated as foundational context for protocol and radio-control implementation planning.

## Run

```text
cargo run -p rigforge -- --help
```

## Milestones

- M0 — Bootstrap workspace + shell (current)
- M1 — Audio capture from radio interface
- M2 — Spectrum/waterfall + DSP metrics
- M3 — CI-V radio control abstraction + implementation
- M4+ — FT8 decode, station intelligence, and eventually safe AI assistant tooling
