# QSONaut Radio HAL Architecture (Rust)

## Goals

- Keep a **stable app-facing API** while adding radios/protocols over time.
- Support **typed common controls** (freq/mode/PTT/gain/filter/split/etc.).
- Support **full vendor escape hatches** for advanced CI-V options.
- Avoid protocol-specific leakage into app/TUI/business logic.

## Layers

1. **App Layer** (`apps/qsonaut`)
   - Uses a radio-agnostic HAL interface only.
2. **HAL Layer** ([Rigwright](https://github.com/nicksbar/rigwright))
   - Common traits, capabilities, control IDs, typed values.
3. **Protocol Drivers**
   - Rigwright CI-V, modern Yaesu CAT, legacy Yaesu CAT, and Kenwood CAT drivers.
4. **Transport Layer**
   - Serial (USB/TTY), TCP (future), mock transport for tests.

## HAL surface

- `Radio` (minimal common trait)
- `RadioHal` (extensible typed trait)
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

## Direction

1. Keep typed core controls first (freq/mode/PTT + common knobs).
2. Add CI-V command registry for advanced feature parity.
3. Grow and validate Rigwright's per-radio capability profiles (IC-7300 first).
4. Add integration tests with captured byte frames.
5. Add optional rigctl parity checks as diagnostics, not primary driver path.

## Testing Rules

- Decoder tests use **real captured frames**.
- Integration tests verify: command frame -> response frame -> typed state.
- Do not claim support for a CI-V control until a frame-level test exists.

## Current status

- Deterministic CI-V frequency decode implemented (BCD little-endian bytes).
- The live probe reports frequency and mode from compatible Icom radios.
- HAL primitives live in the independent `rigwright` crate to support multi-radio growth.
- `IcomCiVRadio` now implements both `Radio` and `RadioHal` for live operations.
- QSONaut can select Rigwright profiles for popular Icom, Yaesu, and Kenwood radios.
- Common frequency, mode, and PTT operations route through the selected protocol driver.
- Native window geometry and expanded/collapsed section state persist between launches.
- Implemented live CI-V write paths for:
   - set frequency (`0x05` + BCD Hz)
   - set mode (`0x06`)
   - set PTT (`0x1C 0x00`)
- Added registry-backed control dispatch (first batch scaffolded):
   - Preamp, Attenuator, NB, NR, AGC, Split
- Added `protocol_write_read()` escape hatch for full/raw CI-V frames.

## Known gaps

1. IC-7300 is the only radio used regularly during development.
2. Captured-frame coverage is incomplete for the wider control registry.
3. Unsupported controls still need stronger per-radio capability gating.
4. USB reconnect and radio reboot behavior needs broader hardware testing.
5. Non-IC-7300 profiles and the Yaesu/Kenwood serial drivers are experimental until tested against physical radios.
6. Icom-only spectrum and advanced CI-V controls stay disabled for radios that do not expose them.
