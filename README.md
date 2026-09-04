<p align="center">
  <img src="assets/branding/qsonaut-icon.png" width="180" alt="QSONaut astronaut radio icon">
</p>

# QSONaut

[![CI](https://github.com/nicksbar/QSONaut/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/nicksbar/QSONaut/actions/workflows/ci.yml)
[![Coverage gate](https://github.com/nicksbar/QSONaut/actions/workflows/coverage.yml/badge.svg?branch=main)](https://github.com/nicksbar/QSONaut/actions/workflows/coverage.yml)
[![Line coverage 59.01%](https://img.shields.io/badge/line%20coverage-59.01%25-yellow)](#coverage-area-snapshot)
[![GUI core 54.16%](https://img.shields.io/badge/GUI%20core-54.16%25-yellow)](#coverage-area-snapshot)
[![GUI workers 61.70%](https://img.shields.io/badge/GUI%20workers-61.70%25-yellow)](#coverage-area-snapshot)
[![GUI modes 53.44%](https://img.shields.io/badge/GUI%20modes-53.44%25-yellow)](#coverage-area-snapshot)
[![GUI panels 70.37%](https://img.shields.io/badge/GUI%20panels-70.37%25-yellow)](#coverage-area-snapshot)
[![Audio 36.28%](https://img.shields.io/badge/audio-36.28%25-yellow)](#coverage-area-snapshot)
[![Core 87.21%](https://img.shields.io/badge/core-87.21%25-brightgreen)](#coverage-area-snapshot)
[![Server client 84.25%](https://img.shields.io/badge/server%20client-84.25%25-brightgreen)](#coverage-area-snapshot)
[![PSK Reporter 81.85%](https://img.shields.io/badge/PSK%20Reporter-81.85%25-brightgreen)](#coverage-area-snapshot)
[![Logging 83.65%](https://img.shields.io/badge/logging-83.65%25-brightgreen)](#coverage-area-snapshot)
[![Release builds](https://github.com/nicksbar/QSONaut/actions/workflows/release-builds.yml/badge.svg)](https://github.com/nicksbar/QSONaut/actions/workflows/release-builds.yml)
[![Latest release](https://img.shields.io/github/v/release/nicksbar/QSONaut?display_name=tag&sort=semver)](https://github.com/nicksbar/QSONaut/releases)

**An enthusiast-built amateur-radio mission control console.**

QSONaut combines radio control, live audio and spectrum views, WSJT-family
digital modes, contact logging, and early operator-assist scaffolding in one
native Rust desktop app. Radios can be local or connected through the optional
[QSONaut-HostBridge](https://github.com/nicksbar/QSONaut-HostBridge) station
service.

> [!IMPORTANT]
> QSONaut is alpha software and is still evolving. Hardware support and
> on-air behavior vary by radio, platform, and mode; IC-7300 control is the
> current hardware-validated path. Review frequency, power, audio, band plan,
> and transmit state yourself. QSONaut is not an unattended-operation system
> or a safety interlock.

<p align="center">
  <img src="assets/main-screen.png" alt="QSONaut radio-console interface with station identity redacted">
  <br>
  <em>QSONaut radio console on Linux/WSL. The station identity is redacted and visible callsign suffixes are blurred.</em>
</p>

<p align="center">
  <img src="assets/activities-panel.png" width="48%" alt="QSONaut operating activity panel">
  <img src="assets/swr-meter.png" width="48%" alt="QSONaut SWR meter and sweep panel">
</p>

<p align="center">
  <em>Operating activities and the live SWR meter/sweep workflow.</em>
</p>

## Capabilities

The current application brings the station workflow into one native Rust desktop
console. This is an honest capability snapshot, not a compatibility promise:

| Area | Current maturity |
| --- | --- |
| Digital modes | FT8 and FT4 provide native decode, activity, conversation, TX history, sequencing, logging, and explicit global TX disarm. FST4, JT9, JT65, and all currently exposed Q65 submodes have experimental receive/scheduled-TX paths; WSPR and MSK144 are receive-only integrations. |
| SSTV and local images | Single-channel auto-targeting finds shifted VIS headers across the audio baseband, and Auto VIS receive or manual receive filtering decodes 13 Martin/Scottie/Robot/PD modes. Waterfall clicking overrides targeting; acquisition, ranked-candidate, progress, and failure diagnostics use the filterable Application Log. The pinned adapter also provides selectable experimental TX. Existing images can be browsed or generated through local Ollama/Lemonade models, and TX remains explicitly armed. |
| CW | Software audio CW through [cw-dit](https://github.com/nicksbar/cw-dit), with selected-channel streaming decode, adaptive timing, noise-floor slicing, and generated subband TX. Paddle/keyed-carrier input, prosigns, punctuation, and auto-sequencing are not implemented yet. |
| Radio control | Rigwright profiles cover Icom CI-V, modern and classic Yaesu CAT, and Kenwood PC control, with generic and model-specific profiles. Capability-gated power, AF/RF gain, squelch, RF power, preamp/attenuator, NB, NR, IP+, notch, AGC, tuner, normalized meters, and SWR controls are exposed where supported. IC-7300 is hardware-validated; other serial drivers remain experimental. |
| Spectrum and audio | Equal-height radio/audio waterfalls, narrow/active-band views, VBW controls, click-to-tune audio/radio scopes, upper-banner scope details, audio-device selection, decoder-channel monitoring, and RX monitor volume. Radio scope and vendor controls remain capability-gated. |
| Remote stations | [`QSONaut-HostBridge`](https://github.com/nicksbar/QSONaut-HostBridge) is the optional background service for remote radio operation. QSONaut authenticates, enumerates host-owned radios and audio endpoints, selects the driver/model and devices, and consumes bidirectional PCM, meters, controls, state, and optional CI-V scope frames over its documented WebSocket protocol. HostBridge owns private device paths, leases, and hardware safety; one remote session is allowed per QSONaut process. |
| SWR and tuner | Normalized live SWR display plus an experimental stepped active-band sweep with configurable range/step/interval, low-power carrier pipeline, tuner safety, stop/disarm handling, charting, and application-log diagnostics. |
| Station workflow | Contact log with ADIF import/export, operator profiles, QSO history, PSK Reporter (optional and off by default), and a live in-app application log with filtering, highlighting, copy, and bottom-follow. |
| QSONaut Server | Optional WSS event/catalog sync, station presence, radio metadata, idempotent QSO publication, shared channels, and manual diagnostics. Each outbound data category is independently opt-in. |
| Automation and compute | Permission-gated automation foundations and compute-backend detection exist; Discord/IRC connectors and GPU/NPU decoder kernels are not validated yet. |

See the detailed [QSONaut feature matrix](docs/feature-matrix.md) for the
implementation-level status of radio controls, normalized meters, SWR/tuner
workflows, digital modes, SSTV, station tools, server integration, automation,
and deliberate gaps.

The [project roadmap](docs/project-roadmap.md) is the current v0.4.0 through
v1 planning source across QSONaut and its sibling repositories.

For the settings split between application-wide station state, independent
radio tabs, and shared radio-tuning definitions, see
[Settings ownership](docs/settings-ownership.md).

The primary development environment is Linux/WSL with USB audio and Icom CI-V.
Release builds currently cover Linux x86_64, Linux ARM64 (including supported
Raspberry Pi environments), and Windows x86_64/ARM64. A green build is not the
same as hardware validation.

QSONaut is developed with AI-assisted tooling alongside human review, tests,
and hardware validation. That development history is part of the project, but
the practical maturity boundary is the alpha status and the hardware-specific
validation above.

## Build and run

QSONaut uses pinned Git revisions of the shared
[`qsonaut-modems`](https://github.com/nicksbar/qsonaut-modems) and
[`qsonaut-third-party`](https://github.com/nicksbar/qsonaut-third-party)
components. No local shared-modem checkout is required:

Remote operation additionally requires a running
[`QSONaut-HostBridge`](https://github.com/nicksbar/QSONaut-HostBridge) service
on the station computer. HostBridge is configured independently, then QSONaut
connects to its WebSocket endpoint from the radio profile. See
[`HostBridge client integration`](docs/hostbridge-client.md) for the protocol,
selection flow, media format, capabilities, reconnect, and safety rules.

```bash
git clone https://github.com/nicksbar/QSONaut.git
cd QSONaut
cargo run --release -p qsonaut -- --gui
```

Cargo resolves the pinned revisions and records those sources in `Cargo.lock`
for reproducible builds.

Use a release build for maximum live-decoding performance. The workspace
optimizes the lower-level modem DSP crates in dev builds by default, so the
GUI remains debuggable without making FST4/Q65 appear frozen. Use
`RUST_LOG=debug` (or a narrower module filter) for runtime diagnostics. When
stepping through unoptimized modem code, use Cargo's `modem-debug` profile
instead of the normal dev profile.

## Tests and coverage

Run the complete workspace test suite with:

```bash
cargo test --locked --workspace --all-targets
```

QSONaut uses LLVM source-based coverage through `cargo-llvm-cov`. Install the
tool once, then generate the terminal baseline with:

```bash
rustup component add llvm-tools-preview
cargo install cargo-llvm-cov --locked
cargo llvm-cov --locked --all-features --workspace \
  --ignore-filename-regex 'crates/qsonaut-gui/src/panels/(devices|profile_server|radio_ui)\.rs|crates/qsonaut-gui/src/modes/(sstv|ft8|ft4|voice|cw|jt9|jt65|q65|wspr|fst4)\.rs' \
  --summary-only
```

To generate the browsable report, use `--html`; it is written beneath
`target/llvm-cov/html`. Pull requests and pushes to `main` run the same
coverage workflow, enforce a 60% executable-contract line-coverage gate,
enforce changed-file coverage on pull requests, and upload the HTML report as
an artifact. The report intentionally excludes rendering-heavy UI and callback paths that
are unsuitable for deterministic unit coverage (`panels/devices.rs`,
`panels/profile_server.rs`, `panels/radio_ui.rs`, and the listed mode renderers);
`radio_ui.rs` contains only egui callback glue; its CAT/session transactions
remain in the measured Rigwright contract. Hardware-only Rigwright implementation code is
covered by Rigwright's own workflow. The per-file report remains the source of
truth for included areas.

### Coverage area snapshot

The current release-candidate measurement was generated on 2026-09-03 with
the workspace tests available in the validation environment. The grouped
executable-contract report is 62.18% (18,248 / 29,345 lines), above the CI
gate of 60%. Rigwright integration is 80.47% (2,983 / 3,707 lines), above its
80% target. Physical-radio behavior still requires the documented
hardware validation runs.
The high-coverage `qsonaut-sstv` crate is
now maintained behind the pinned `qsonaut-third-party` boundary and is covered
by that repository's workflow rather than this workspace. These are grouped
line-coverage figures from the same LLVM report; the downloadable HTML artifact
remains the detailed, per-file source of truth.

| Area | Covered / executable lines | Line coverage |
| --- | ---: | ---: |
| qsonaut-automation | 217 / 264 | 82.20% |
| qsonaut-log | 788 / 942 | 83.65% |
| qsonaut-accelerate | 281 / 318 | 88.36% |
| qsonaut-pskreporter | 275 / 336 | 81.85% |
| qsonaut-server-client | 733 / 870 | 84.25% |
| qsonaut-audio | 316 / 871 | 36.28% |
| qsonaut-core | 409 / 469 | 87.21% |
| HostBridge client | 161 / 413 | 38.98% |
| GUI core | 7,682 / 13,910 | 55.23% |
| GUI workers | 1,648 / 2,671 | 61.70% |
| GUI modes | 1,470 / 2,474 | 59.42% |
| GUI panels | 1,090 / 1,549 | 70.37% |
| Rigwright integration | 2,983 / 3,707 | 80.47% |
| Application entry point | 195 / 551 | 35.39% |

The workspace gate requires 60% overall coverage, and the Rigwright integration
target requires 80%; both are enforced by CI. Changes to these areas should add
deterministic seams or focused tests rather than weakening the contract gates.

Changed-file coverage is enforced with `scripts/check-changed-coverage.sh`.
The small set of pre-existing zero-coverage files is explicitly tracked in
`.coverage-baseline`; new zero-coverage production files fail the pull request
check.

Rigwright driver implementation coverage is intentionally separate: it is an
external dependency and is measured by Rigwright's own driver coverage
workflow. The `Rigwright integration` area above covers QSONaut's device and
radio-worker interaction with the HAL. Those tests must verify capability
discovery, readback, control dispatch, meter updates, and failure handling
without duplicating vendor protocol tests.

On Ubuntu, native build dependencies include:

```bash
sudo apt-get install libasound2-dev libudev-dev libwayland-dev libxkbcommon-dev
```

WSL also needs the ALSA PulseAudio bridge so CPAL can reach WSLg audio:

```bash
sudo apt-get install libasound2-plugins pulseaudio-utils
```

See [Audio monitoring](docs/audio-monitoring.md) for native OS setup, WSL
verification, device selection, and troubleshooting.

On WSLg systems with `/dev/dxg`, QSONaut supplies the Mesa D3D12 and WGPU GL
defaults automatically while preserving explicit environment overrides. To
force a particular Windows GPU through Mesa, use:

```bash
GALLIUM_DRIVER=d3d12 \
MESA_D3D12_DEFAULT_ADAPTER_NAME=AMD \
cargo run --release -p qsonaut -- --gui
```

The active rendering adapter and session-only power/GPU preferences are shown
under **Settings > Graphics**. The default is **Low power / Auto**. Applying a
change restarts only the GUI process; it does not write the choice to an
operator profile.

QSONaut also offers hardware discovery and lower-level radio commands. Audio
devices are refreshed and selected in **Settings > Devices**:

```bash
cargo run -p qsonaut -- --help
cargo run -p qsonaut -- --list-radio
```

## QSONaut Server Integration

QSONaut Server is the independent coordination service for QSONaut operators,
clubs, and group events. QSONaut can optionally use it for event/catalog
synchronization, station presence, selected radio metadata, idempotent QSO
publication, shared channels, and manually requested diagnostics.

Server connectivity and each sharing category are independently opt-in. The
server project owns deployment, accounts, enrollment, device tokens, and its
management UI; this repository documents only the QSONaut client boundary.
See [QSONaut-Server](https://github.com/nicksbar/QSONaut-Server) for server
setup and [`docs/server-integration.md`](docs/server-integration.md) for
client-side configuration and privacy behavior.

## Configuration and privacy

Copy `.env.example` for optional environment overrides or pass
`--config qsonaut.toml.example`. Local `.env`, operator profile, QSO log, and
recorded WAV files are ignored by Git.

The SSTV workspace can use an Ollama or Lemonade Server image model running on
the same computer. This integration is local-only by construction: QSONaut
accepts only `http://localhost`, `127.0.0.0/8`, or IPv6 loopback endpoints and
disables proxy discovery for these requests. No prompt, station detail, or
generated image is sent to QSONaut Server. See
[`docs/sstv-local-images.md`](docs/sstv-local-images.md).

Runtime logs are written to `qsonaut.log` under the platform app directory:

- Windows: `%APPDATA%\QSONaut\logs\qsonaut.log`
- Linux: `$XDG_CONFIG_HOME/qsonaut/logs/qsonaut.log` (or `~/.config/qsonaut/logs/qsonaut.log`)
- macOS: `~/Library/Application Support/QSONaut/logs/qsonaut.log`

The newest 256 KiB can also be viewed from the in-app **APP LOG** tab with live
tailing, bottom-follow, text/level filters, highlighting, and copy support.
Manual server diagnostic snapshots can optionally include a separately enabled,
redacted 24 KiB tail; logs are never uploaded continuously.

If QSONaut fails to open a window or crashes during startup, attach that log
file to the issue report.

QSONaut renders through eframe's `wgpu` backend on every platform. The selected
renderer is recorded in `qsonaut.log`.

UI scale now uses a rebased baseline: the physical size that previously looked
like 75% is now the 100% preset.

QSONaut respects the OS-provided DPI scale. Windows presets use the physical
baseline that previously appeared as 75% on this desktop, so that baseline is
labelled 100% on Windows; Linux keeps the existing preset labels. For other
platform-specific display quirks, the final application zoom can be tuned with
a multiplier from `0.75` to `1.50` (default `1.0`): `QSONAUT_WINDOWS_DPI_ADJUSTMENT`,
`QSONAUT_LINUX_DPI_ADJUSTMENT`, or `QSONAUT_MACOS_DPI_ADJUSTMENT`. Startup logs
record the selected OS, adjustment, raw DPI, application zoom, and effective
scale. All platforms provide the shared `50, 60, 75, 85, 100, 110, 125, 150,
175%` scale choices.

Reception data stays local unless you explicitly enable PSK Reporter. External
automation source declarations reference environment-variable names rather
than embedding Discord or IRC credentials.

QSONaut Server connectivity and each sharing category are also disabled by
default. See [`docs/server-integration.md`](docs/server-integration.md) for
device enrollment and proxy-friendly WSS configuration.

## Repository map

- `apps/qsonaut` — CLI and desktop entry point
- `crates/qsonaut-gui` — operator console and timed mode workflows
- [`rigwright`](https://github.com/nicksbar/rigwright) — sibling radio HAL and native Icom CI-V implementation
- [`QSONaut-HostBridge`](https://github.com/nicksbar/QSONaut-HostBridge) — optional remote station service and host-device provider

QSONaut resolves `rigwright` and the shared modem/third-party components from
immutable Git revisions recorded in the manifests and `Cargo.lock`. This keeps
CI and release builds reproducible while allowing local sibling checkouts to be
used temporarily for development validation. CI does not require any sibling
repository checkout.
- `crates/qsonaut-audio` — real-time audio and high-quality 48→12 kHz decimation
- [`qsonaut-modems`](https://github.com/nicksbar/qsonaut-modems) — shared consumer-neutral modem contracts
- [`qsonaut-third-party`](https://github.com/nicksbar/qsonaut-third-party) — pinned third-party modem adapters, including SSTV, CW, and WSJT-family modes
- `mfsk-core` and [`cw-dit`](https://github.com/swilcox/cw-dit) — transitive implementations owned and maintained behind `qsonaut-third-party`
- `crates/qsonaut-log`, `qsonaut-pskreporter` — local logging and opt-in reporting
- `crates/qsonaut-server-client` — optional authenticated WSS synchronization
- `crates/qsonaut-accelerate` — measured compute-backend selection
- `crates/qsonaut-automation` — sandboxed component and external-source foundation
- `docs` — current implementation notes; historical scratch research was intentionally removed before publication

## Versioning and release process

QSONaut uses SemVer tags (`vMAJOR.MINOR.PATCH`) and a curated
[`CHANGELOG.md`](CHANGELOG.md). Release assets and release notes are generated
from those tags and changelog entries.

See [`docs/versioning-and-releases.md`](docs/versioning-and-releases.md) for
the concrete policy and release checklist.

For the current consolidation baseline, see the [v0.4 maturity and feature
matrix](docs/feature-matrix.md) and the [deterministic regression fixture
catalog](docs/regression-fixtures.md). These documents distinguish software
test evidence from physical-station validation.

## Before transmitting

At minimum:

1. Confirm the selected audio input/output and CI-V serial device.
2. Verify dial frequency, mode, filter, data mode, RF power, and TX audio level.
3. Start into a dummy load or minimum safe power where practical.
4. Keep the global **STOP + DISARM ALL TX** control visible and tested.

You are responsible for lawful operation and for every transmission made with
this software.

## Contributing

Bug reports with platform, radio, audio device, logs, and exact reproduction
steps are especially useful. Tests and captured protocol fixtures are preferred
over claims of compatibility. Please do not commit credentials, personal QSO
logs, recordings, or proprietary manuals.

### About credits

The About panel credits the original author, plus optional contributors and
testers. Release builds read the repository Actions variables
`QSONAUT_CONTRIBUTORS` and `QSONAUT_TESTERS`; unset or empty variables display
`None listed`. The preferred value is a JSON array, which also supplies
identities to the null radio simulator:

```json
[
  {
    "name": "Nick",
    "callsign": "N7UF",
    "grid": "CN87",
    "power_dbm": 30,
    "role": "maintainer",
    "modes": ["ft8", "ft4", "sstv", "wspr"]
  },
  {
    "name": "Example Tester",
    "callsign": "W1AW",
    "grid": "FN31",
    "power_dbm": 30,
    "role": "tester",
    "modes": ["ft8", "sstv"],
    "enabled": true
  }
]
```

Only `callsign` is required for simulated exchanges. `grid` and `power_dbm`
are used by WSPR, while `name`, `role`, and `modes` are available for credit
and future simulator presentation. Set the variables under the repository's
**Settings → Secrets and variables → Actions → Variables** page so identities
can be updated without changing source code. Plain text remains accepted for
the About panel, but it cannot provide structured simulator metadata.

## Acknowledgements

QSONaut's WSJT-family decode and synthesis engine is powered by
**[`mfsk-core`](https://github.com/jl1nie/mfsk-core)**, developed and maintained
by the [`mfsk-core` contributors](https://github.com/jl1nie/mfsk-core/graphs/contributors).
It provides the protocol, DSP, message-codec, and waveform machinery behind
QSONaut's FT8, FT4, FST4, WSPR, JT9, JT65, Q65, and MSK144 integrations.

We are grateful to that project for making a serious pure-Rust WSJT-family
engine available to the amateur-radio community. `mfsk-core` is licensed
**GPL-3.0-or-later** and carries its own attribution to WSJT-X, Joe Taylor
K1JT, and collaborators. See [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md)
for the exact source revision and license details used by QSONaut releases.

## License

QSONaut's original source code is offered under the MIT license; see
[`LICENSE`](LICENSE). QSONaut currently links with GPL-3.0-or-later
`mfsk-core`, so distributed combined binaries are also subject to the
applicable GPL terms. See [`THIRD_PARTY_NOTICES.md`](THIRD_PARTY_NOTICES.md).
