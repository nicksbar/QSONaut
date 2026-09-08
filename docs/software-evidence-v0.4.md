# v0.4 software evidence

This record separates evidence that can be reproduced without a physical radio
from the final IC-7300 acceptance run.

## Reproducible checks

Run from the QSONaut workspace:

- `cargo fmt --all -- --check`
- `cargo test --offline -p qsonaut-core -p qsonaut-automation -p qsonaut-gui --lib`
- `cargo clippy --offline -p qsonaut-core -p qsonaut-automation -p qsonaut-gui --all-targets --all-features -- -D warnings`
- `git diff --check`

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

## Release boundary

Software evidence may close the deterministic implementation work. The v0.4
release remains hardware-pending until the IC-7300 matrix is rerun against the
release candidate and the report is retained with the release artifacts.
