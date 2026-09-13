# v0.4 software evidence

This record separates evidence that can be reproduced without a physical radio
from the live IC-7300 evidence recorded in
[`hardware-acceptance.md`](hardware-acceptance.md). Updated 2026-09-13 for the
`0.4.2` release line.

## Reproducible checks

Run from the QSONaut workspace:

- `cargo fmt --all --check`
- `cargo test --locked --workspace --all-targets`
- `cargo clippy --locked --workspace --all-targets -- -D warnings`
- `git diff --check`
- `cargo llvm-cov --locked --all-features --workspace` with the checked-in CI
	exclusions and coverage gates

The null-radio and null-audio fixtures cover lifecycle startup/failure/recovery,
command acceptance and terminal outcomes, duplicate command rejection, timeout
and generation cancellation, PTT gating, global disarm, shutdown ordering,
readback state ownership, deterministic audio formats, and mode decode paths.

The QSO and automation fixtures cover duplicate protection, persistence-gated
`QsoLogged` events, ADIF import/export behavior, permission-denied action
results, connector provenance normalization, and immutable event mapping.

## Explicit hardware-only evidence

The following cannot be promoted from a software test:

- CI-V/USB transport behavior and IC-7300 power-state behavior
- physical frequency/mode/filter/RF-power agreement
- actual PTT assertion and release into an approved load
- native scope/waterfall frames from the radio
- USB audio capture/playback and real decoder input
- unplug/reconnect recovery while the physical device is in use
- proof that shutdown or failure leaves the physical transmitter unkeyed

These rows belong in `docs/hardware-acceptance.md` and must include the commit
SHA, model/firmware, transport settings, sanitized diagnostics, observed
result, and final safe-state readback.

## Current release boundary

The current software evidence is green: workspace line coverage is 61.12%,
Rigwright integration coverage is 82.31%, and the CW coverage gate passes.
The IC-7300 has fresh non-transmitting status, reversible-probe, and native
spectrum-stream evidence on `/dev/ttyUSB0` at 115200 baud; the final readback
was 14.074 MHz USB-D with PTT off.

The release is not hardware-complete. PTT-SAFE process-stop evidence and any
additional transmit evidence remain blocked until an operator confirms an
approved load and performs the bounded test described in the hardware matrix.
