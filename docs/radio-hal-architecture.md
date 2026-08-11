# RigForge Radio HAL Architecture (Rust)

## Goals

- Keep a **stable app-facing API** while adding radios/protocols over time.
- Support **typed common controls** (freq/mode/PTT/gain/filter/split/etc.).
- Support **full vendor escape hatches** for advanced CI-V options.
- Avoid protocol-specific leakage into app/TUI/business logic.

## Layers

1. **App Layer** (`apps/rigforge`)
   - Uses a radio-agnostic HAL interface only.
2. **HAL Layer** (`crates/rigforge-radio`)
   - Common traits, capabilities, control IDs, typed values.
3. **Protocol Drivers**
   - `IcomCiVRadio` now, Yaesu/Kenwood later.
4. **Transport Layer**
   - Serial (USB/TTY), TCP (future), mock transport for tests.

## HAL Surface (current foundation)

- `Radio` (existing minimal trait)
- `RadioHal` (new extensible trait)
- `RadioCapabilities`
- `ControlId` and `ControlValue`

This allows app code to ask:
- “Can this radio set frequency?”
- “Does it expose AGC/filter controls?”
- “Can I send raw protocol payloads?”

## CI-V Coverage Strategy

### A) Standard controls (typed)

Map CI-V commands into `ControlId` entries:
- `AfGain`, `RfGain`, `Squelch`, `RfPower`
- `Preamp`, `Attenuator`, `NoiseBlanker`, `NoiseReduction`
- `Agc`, `Filter`, `DataMode`
- `Rit`, `Xit`, `Split`, `Tuner`

### B) Advanced controls (raw + registry)

For “pretty much all CI-V options,” add a **command registry**:
- key: logical control/action name
- value: `{ cmd, subcmd, encode, decode, rw }`

This supports many commands without hardcoding every one into the trait.

## Implementation Plan

1. Keep typed core controls first (freq/mode/PTT + common knobs).
2. Add CI-V command registry for advanced feature parity.
3. Introduce per-radio capability profiles (IC-7300 first).
4. Add integration tests with captured byte frames.
5. Add optional rigctl parity checks as diagnostics, not primary driver path.

## Testing Rules

- Decoder tests use **real captured frames**.
- Integration tests verify: command frame -> response frame -> typed state.
- Do not claim support for a CI-V control until a frame-level test exists.

## Current Status

- Deterministic CI-V frequency decode implemented (BCD little-endian bytes).
- Live probe now reports frequency and mode from your radio.
- HAL primitives added in `rigforge-radio` to support multi-radio growth.
- `IcomCiVRadio` now implements both `Radio` and `RadioHal` for live operations.
- Implemented live CI-V write paths for:
   - set frequency (`0x05` + BCD Hz)
   - set mode (`0x06`)
   - set PTT (`0x1C 0x00`)
- Added registry-backed control dispatch (first batch scaffolded):
   - Preamp, Attenuator, NB, NR, AGC, Split
- Added `protocol_write_read()` escape hatch for full/raw CI-V frames.

## Next Immediate Build Steps

1. Expose HAL control commands via CLI/TUI (safe subset first).
2. Add integration tests with captured CI-V set/ack frames.
3. Expand registry coverage to AF/RF/SQL/RF Power + mode filter/data-mode controls.
4. Add per-control capability gating so unsupported controls degrade cleanly.
5. Add explicit spectrum/waterfall stream bootstrap in driver lifecycle:
   - On connect, send CI-V scope bootstrap (`0x27 0x10 0x01`, `0x27 0x20 0x01`, then `0x27 0x00`).
   - On disconnect, optionally disable scope output (`0x27 0x20 0x00`, `0x27 0x10 0x00`).
   - After radio reboot/reconnect, always re-enable stream (state is not persistent).
   - Mark waterfall as "ready" only after receiving first `0x27 0x00 ...` waveform frame.
