# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.2] - 2026-08-13

### Added
- Model-aware radio selection for supported Icom, Yaesu, and Kenwood families, including experimental FTDX101 and FT-857D profiles.
- Side-by-side radio and audio waterfall monitoring with narrow passband and active-band overview views.
- IC-7300 scope controls for sweep speed, VBW, hold, reference level, contrast, and color palette.
- Live CI-V scope diagnostics reporting sweep rate, division rate, and discarded incomplete sweeps.

### Changed
- Radio control now uses Rigwright `v0.1.2`, pinned to its GitHub release for reproducible builds.
- Window geometry, expanded sections, selected radio, and waterfall layout preferences persist between sessions.
- IC-7300 scope rendering preserves complete native 475-bin sweeps and useful USB/LSB passband placement.

### Fixed
- CI-V scope divisions interleaved with ordinary CAT replies are retained instead of discarded.
- Waterfall resizing now follows egui panel state rather than snapping back to content-derived dimensions.
- Removed serial response stalls and stale timeout behavior that caused visibly bursty scope updates.

## [0.2.1] - 2026-08-12

### Fixed
- Windows release builds now downscale the embedded app icon before generating the ICO, unblocking the ARM64 and x86_64 release artifacts.

## [0.2.0] - 2026-08-12

### Added
- Contest workflow profile with split policy, Fox/Hound role guidance, serial cursor controls, and operator-visible exchange previews.
- Fake-split transmit offset handling for contest operation when the radio itself does not expose split control.
- Achievement Hunter persistence with built-in unlocks and custom operator-defined achievements.
- ADIF export filtering for the current mode/band view.

### Changed
- Contest QSOs now preserve serial/exchange metadata through the log model and ADIF import/export.
- GUI contest status now advertises role-aware guidance and serial progress in the TX compose deck.
- Workspace release version advanced to `0.2.0` for the new hardening milestone.

### Fixed
- Release workflow now has the contest/logging changes needed to build the full desktop matrix from the tagged release.

## [0.1.1] - 2026-08-12

### Added
- Platform log file output (`qsonaut.log`) for startup/runtime diagnostics that users can attach to issue reports.

### Changed
- Windows release builds now launch as native GUI applications instead of console-first binaries.
- GUI state, operator profile, and QSO logs now resolve through one central platform app directory path.
- UI scaling baseline was rebased so the previous physical 75% size now corresponds to 100% in the scale selector.
- Renderer selection is now explicit and logged (`wgpu` default on Windows, `glow` on non-Windows, override via `QSONAUT_RENDERER`).

### Fixed
- Fatal GUI startup failures now preserve diagnostics in the log file and surface a user-visible error path instead of silently exiting.

## [0.1.0] - 2026-08-12

### Added
- Multi-platform release build workflow for Linux (x86_64/ARM64) and Windows (x86_64/ARM64).
- Automated CI workflow for formatting, workspace build, and tests on pull requests and main.
- GitHub release publishing from `vMAJOR.MINOR.PATCH` tags with attached build artifacts.
- README badges for CI, release builds, and latest release.
- Repository governance baseline with `CODEOWNERS`.

### Changed
- Release archives now include `CHANGELOG.md` alongside licenses and notices.
- README language now explicitly states AI/LLM functionality is currently placeholder scaffolding.
- `.env.example` now marks AI-related environment variables as reserved and currently inactive.

### Fixed
- Workspace-wide strict clippy issues across `qsonaut-*` crates and app entrypoint.
- Lint clean under `cargo clippy --workspace --all-targets -- -D warnings`.
- Validation clean under `cargo test --workspace --all-targets`.
