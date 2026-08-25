# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- Added a lower-right radio-themed About panel with version, copyright,
  author, contributor/tester credits, GitHub, issue-reporting, and qsonaut.com
  links. Release credits can be supplied through repository Actions variables.
- Added a WGPU-native AI tab icon painted with egui primitives, plus compact
  colored icons across the signal-panel tabs without relying on emoji glyph
  coverage.
- Added a full multi-radio operating model: every profile tab owns independent
  radio, audio, decoder, monitor, waterfall, and control state, and remains
  live while another tab is active. Tabs can be started, stopped, and switched
  independently without reconnecting or reloading the other radios; a profile
  always has a tab, and deleting the profile removes its tab.
- Added profile-scoped radio/audio device assignment, live device refresh, and
  clear in-use labels so multiple radios can be configured without silently
  sharing hardware, including independent monitoring and failure recovery per
  profile.
- Added a global reusable radio-tuning library with per-profile, per-mode
  assignments and a profile-management drawer with direct Radio, Tuning,
  Digital Timing, Monitoring, and Waterfall tabs.

### Changed
- Migrated the desktop renderer to WGPU-only operation with explicit Vulkan,
  DX12, Metal, GL, and WGSL backend features; removed the Glow path, added
  logged hardware-aware backend selection and WSL fallback, and documented the
  cross-platform renderer requirements.
- Consolidated the radio, mode, status, volume, monitor, and scope controls
  into a responsive top banner with compact aligned sections and adaptive
  wrapping for smaller windows.
- Reduced multi-radio rendering overhead by coalescing worker repaint requests
  across profiles instead of redrawing independently for every audio chunk.
- Refined AI, SSTV, automation, server, radio, and application-log UI labels
  to avoid unsupported Unicode glyphs while retaining compact visual cues.

### Fixed
- Initialized persistent logging before WSL graphics preparation so platform
  setup, WGPU startup, panic, and eframe launch failures are recorded in
  `qsonaut.log`.
- Added WSL GPU re-exec diagnostics and clarified that an X11/WSLg clipboard
  connection reset can terminate the native event loop; the full failure
  chain is now retained in the diagnostic log.
- Restored radio-aware Band filtering: known native Rigwright profiles now
  expose only their supported HF/6m or VHF/UHF bands, while unknown external
  connections remain unfiltered.
- Made audio reads cancellation-aware so closing multiple live radio profiles
  no longer accumulates the full hardware read timeout for every worker; added
  shutdown-duration logging for regression diagnosis.

## [0.3.8] - 2026-08-23

### Added
- Added a capability-gated upper-right radio power button. Native modern
  Icom CI-V, Yaesu CAT, and Kenwood PC-control drivers now expose documented
  radio on/off commands; unsupported backends remain disabled.
- Added capability-gated radio controls for normalized AF/RF gain, squelch,
  RF power, preamp/attenuator, noise blanker, noise reduction, IP+, notch, and
  AGC where the selected Rigwright profile supports them.
- Added normalized SWR presentation and an experimental stepped active-band
  SWR sweep with low-power carrier control, tuner safety, stop/disarm handling,
  charting, and application-log diagnostics.

### Changed
- Updated the native radio integration to the published Rigwright `0.1.10`
  release, including capability discovery, normalized meters, tuner controls,
  Icom scope support, and tolerant CI-V echo-back handling.
- Redesigned the combined radio/audio waterfall deck: both waterfalls share
  equal vertical space, scope details live in the upper banner, and redundant
  waterfall captions and resize text are removed.

### Fixed
- Added lifecycle diagnostics for radio scope configuration and stream
  enable/disable transitions, plus deduplicated audio input failure and
  recovery logging.

## [0.3.7] - 2026-08-22

### Added
- Added a real Application Log clear action that truncates the active log file,
  plus structured logging for device lifecycle, PSK Reporter, automation,
  server ingress, and core rendering failures.
- Added a contact-editor delete action that removes the selected QSO from the
  persistent contact log.
- Added a dedicated Voice Logger workspace with callsign-first QSO entry,
  reports, grid/state, contest exchanges and serials, notes, and explicit
  logging through the existing `QsoRecord` and HamDB enrichment flow.
- Added blur-triggered HamDB callsign lookup for active Voice contacts, with
  cached results, operator preview, and blank-field population.
- Added an initial top-bar operating activity framework with icon-based
  selectors for General, POTA, SOTA, Contest, Field Day, DX, Satellite, and
  EMCOMM activities.
- Added shared core band scopes and activity mode preferences; General remains
  unrestricted, while hard band/mode limits are reserved for active local or
  server contest constraints.
- Added native Martin M1 SSTV receive and transmit at 12 kHz, including VIS 44
  detection, live receive progress, 320×256 image preview, standard HF calling
  frequencies, and one-shot TX through the global disarm/PTT safety path.
- Added strictly local image generation for SSTV activity artwork through
  Ollama and Lemonade Server, with server/model selection, activity-aware
  prompts, local image persistence, and hard loopback-only URL enforcement.
- Added the reusable `qsonaut-sstv` modem crate with encode/decode and streaming
  receiver tests.
- Added a revision-pinned `komitoto-sstv` adapter for 13 Martin, Scottie, Robot,
  and PD codecs, with VIS mapping and cross-backend Martin M2 round-trip tests.
- Added an explicit RX `Auto (VIS)` state with the detected mode shown in the
  SSTV header, plus a 13-mode TX selector with native resolution and duration.
- Connected automatic VIS selection and the manual RX-mode filter to live image
  reconstruction for all 13 pinned Martin, Scottie, Robot, and PD codecs.
- Added single-channel SSTV auto-target acquisition across shifted audio-baseband
  VIS headers, with visible scan/lock/manual states and waterfall click override.
- Added structured Application Log events for SSTV acquisition, ranked leader
  diagnostics, no-header audio, progress, completion, and decode failure, plus
  an SSTV filter shortcut instead of a duplicate mode-local log.

### Changed
- Kept Voice focused on contact logging; PTT and radio controls remain in the
  global radio operations bar.
- Made Voice band presets follow conventional sidebands: LSB on 160/80/40 m,
  USB on higher HF bands, and FM on 2 m/70 cm.
- Extended the prominent global TX safety control to cover armed, queued, and
  active SSTV transmissions.
- Documented local image-server setup, SSTV operating frequencies, format
  boundaries, and the distinction between software validation and on-air
  validation.
- Persisted the selected workspace mode so QSONaut restores the last-used mode
  instead of always starting in FT8.
- Expanded operator profiles with rig, antenna, station notes, and reusable LLM
  prompt guidance, including SSTV image requirements and model notes.
- Moved station and image-generation profile fields below the HamDB controls and
  made them fill the available profile-panel width.

### Fixed
- Corrected CI-V Voice transitions so the waterfall stays in centered-span
  mode, and the top-bar mode label reflects the radio's actual data-mode flag.
- Made the 1200 Hz-wide SSTV decoder window movable on the audio waterfall;
  clicking a received signal now shifts VIS detection, pixel decoding, and the
  displayed tone plan together while residual AFC handles fine alignment.
- Restored the SSTV decode/model layout after the image-path row could overflow
  its column, and made existing PNG/JPEG loading a prominent full-width block
  with an in-app cross-platform file browser.
- Fitted the SSTV frame and local-model panels to the available central
  viewport like FT8/FT4; image loading stays at the top while the model lab
  scrolls within its own column.
- Added explicit SSTV RX diagnostics for audio without a complete header,
  unsupported parity-valid VIS modes, configured sample-rate incompatibility,
  and the requirement to begin capture before the VIS header.

## [0.3.6] - 2026-08-21

### Added
- Added selectable native serial, Hamlib `rigctld`, and DX Lab Suite Commander
  radio backends through Rigwright `v0.1.7`.
- Kept completed digital transmit history visible in FT4 and FT8 conversation
  panels after the active QSO target is cleared.
- Added a live, filterable APP LOG tab with bottom-following, severity
  highlighting, and copy support.
- Added an opt-in redacted application-log tail to manual server diagnostics;
  logs are never uploaded continuously.
- Resolved the cw-dit DSP and Morse crates from a pinned upstream Git revision
  so builds no longer depend on a local sibling checkout.

### Changed
- Refocused the README on current station, digital-mode, CW, radio-control,
  audio, waterfall, logging, and server-integration capabilities while keeping
  alpha maturity and AI-assisted development disclosures clear and concise.
- Replaced the project screenshot with a fresh source image and blurred only
  callsign fields before publication.
- Kept QSONaut Server documentation at the client integration and privacy
  boundary; server deployment and account setup remain in the independent
  QSONaut-Server project.

### Fixed
- Updated the committed Rigwright dependency and lockfile to the published
  `v0.1.7` release, including the new model catalog and profile-driven native
  radio drivers.
- Kept CI and release builds aligned with the tagged Rigwright `v0.1.7`
  dependency instead of relying on the ignored local sibling checkout.
- Preserved FT8 tones when calling CQ instead of resetting the operator's
  selected RX/TX tones.
- Persisted the selected radio waterfall scope view across profile reloads.
- Exposed generic Rigwright radio profiles in the native radio selector.
- Removed duplicate PTT lead and tail controls from the FT4 and FT8 workspaces;
  the shared radio timing settings remain authoritative.
- Removed the obsolete RX test-tone player and its packaging/documentation
  references.
- Simplified radio status and tuning controls to reduce duplicate or redundant
  information in the device panel.
- Resolved `mfsk-core` from a pinned upstream Git revision instead of requiring
  a local sibling checkout; aligned Cargo metadata, CI, release packaging, and
  desktop-build documentation with that source model.

## [0.3.5] - 2026-08-21

### Added
- Added endpoint configuration and in-app radio reconnect support.
- Added dedicated native-mode workspaces for FST4, JT9, JT65, and Q65, with
  per-mode layouts and shared digital conversation handling.
- Added FST4 submode selection for the slow-mode variants exposed by
  `mfsk-core`.
- Added a WSPR Type-1 beacon workflow with callsign, four-character locator,
  dBm power, timing, and backend-drift status controls.
- Added per-mode native digital transmit sequencing with shared timing logic,
  isolated sequencing state, late-slot protection, and complete TX lifecycle
  handling.
- Replaced the previous CW implementation with `cw-dit` streaming DSP and
  Morse components, including signal tracking, selected-channel decoding, and
  generated audio transmission.
- Added explicit CW and digital waterfall channel selection behavior, including
  bandwidth indicators and mode-specific cursor placement.

### Fixed
- Fixed audio monitor and added a volume control
- Removed TX audio monitor, unsupported idea
- Persisted radio backend and endpoint selections in operator profiles.
- Made native serial control the default and migrated legacy `none` settings.
- Added detailed radio initialization diagnostics.
- Improved native digital TX timing so late or canceled transmissions cannot
  leave stale automation state that blocks subsequent cycles.
- Kept native-mode decode decks inside the available layout bounds and matched
  native-mode sizing with the FT8 workspace.
- Shared the digital conversation component across native digital workspaces,
  reducing duplicated UI state and behavior.
- Improved light-theme text contrast and clarified native digital mode
  operation in the UI.
- Smoothed radio waterfall repainting and reduced unnecessary redraw work.
- Preserved radio scope VBW preferences across sessions.
- Improved CW waterfall signal tracking and separated CW cursor behavior from
  digital channel selection.

## [0.3.4] - 2026-08-18

### Added
- Current release state includes dedicated WSPR, FST4, JT9, JT65, and Q65 workspaces;
  FST4 supports the five submodes exposed by `mfsk-core`, while WSPR remains a Type-1,
  one-shot 120-second beacon workflow.
- Added an initial WSPR Type-1 transmit path using callsign, 4-character locator, and dBm
  power entered as `CALL GRID POWER_DBM`; the waveform uses the native 120-second slot.
- Added late-slot protection and timing telemetry to native digital TX, preventing FT4/FST4/JT9/
  JT65/Q65 frames from launching after their valid transmit window.
- Grouped digital workspace modes into HF/primary and other/experimental sections; MSK144
  is visibly disabled for stations without a UHF radio while WSPR and the other HF modes
  remain directly accessible.
- Added an Operator Profile action to load callsign, grid, and QTH details from the
  callsign's HamDB license record.
- Added asynchronous POTA activator-spot lookup with a short-lived cache. CQ POTA decodes
  are marked with a tree icon and show a matched park reference and name when available.
- Added a persisted hidden automation-control unlock. The unattended-CQ control appears only
  after ten logo clicks within ten seconds, with a brief logo-spin unlock indication.

### Fixed
- Restored the parent radio/filter bandwidth as the full CW waterfall display range. Only the
  selected tone is constrained to cw-dit's supported audio range, so the right side remains
  visible and clickable instead of being silently discarded.
- Limited the CW audio waterfall to the cw-dit-supported audio tone range so the full
  visible view maps to valid decoder tones; CW selection is no longer compressed into the left
  portion of a wider USB-D filter view.
- Corrected audio waterfall cursor semantics to use the selected QSONaut workspace mode:
  CW keeps RX/TX centered and linked, while digital modes show their distinct channel widths
  and edge-oriented selection behavior.
- Audio waterfall selection now shows the active channel bandwidth and marks the selected
  edge for digital modes; CW keeps a centered tone marker because its decoder is tone-centered.
- Removed an audio-waterfall rendering regression where every repaint copied and cropped the
  entire rolling row buffer; the selected bandwidth is now sampled directly while building the
  texture, reducing allocation and UI work.
- Throttled radio waterfall repaint requests to the same approximately 15 FPS cadence as
  audio waterfall updates, preventing high-rate scope sweeps from making the display chunky.
- Added RX monitor diagnostics: a live monitor-volume control and a short 700 Hz test tone
  make it possible to verify the selected output device independently of radio input audio.
- Clears stale FT4 session state after a failed or canceled transmission so automation can start
  a fresh cycle without requiring a manual response or application restart.
- Reworked the Contact Log into a resizable two-column layout with a scrollable contact
  list, a selected-contact editor, and an explicit Close action for the editor.
- Removed the misleading resize cursor and inactive resize boundary between the radio
  controls and the waterfall deck; the waterfall deck's lower edge remains resizable.
- Made RX audio monitoring resample captured audio to the selected output device's native
  rate, and report monitor startup failures instead of silently disabling playback.
- Calling CQ now clears the previously selected FT8 decode/contact and returns the RX/TX
  audio channel to the normal operating frequency unless HOLD TX FREQ is enabled.
- Pauses the RX audio monitor while transmitting and restores it after TX, preventing
  Windows audio-device contention from blocking unattended or manually armed CQ output.
- Prevents a stale canceled-TX state from permanently blocking subsequent FT8 transmissions,
  and adds diagnostic logging for TX requests and gate conditions.
- Normalizes six-character station locators such as `CN84JU` to the required four-character
  FT8 locator (`CN84`) when composing CQ and exchange messages.
- Aligns the audio waterfall's visible spectrum with the RX/TX cursor positions when using
  a narrower radio filter view.
- Preserves the active FT8 conversation after stopping TX and keeps directed responses
  addressed to the operator visible when CQ-only filtering is enabled.
- Always starts FT8/FT4 transmit automation disarmed, and leaves unattended-CQ answering
  unchecked until explicitly enabled during the current run.

## [0.3.3] - 2026-08-18

### Changed
- Removed the PTT button from the top radio-control banner. PTT is now controlled only by
  software (TX automation) or at the radio itself; the TX deck's PTT/stop controls remain.
- Added explanatory hover tooltips to the "Answer unattended CQs" checkbox in the FT8 and
  FT4 workspaces so it is clear what enabling it does.
- Removed the "lit up the receiver" operator-call banner from the FT8 and FT4 workspaces.
  Operator calls are still highlighted inline in the decode log, which is sufficient.
- PSK Reporter status now appears in the status bar (off / waiting / queued+sent / error),
  while the Reporting panel keeps the enable toggle and gains submission-rule controls:
  batch interval, re-report cache timeout, and max pending spots. These follow PSK
  Reporter's IPFIX/UDP guidance and mirror WSJT-X's internal knobs.
- Reworked the FT8 and FT4 workspace layout so global live decodes remain in a vertical,
  scrollable panel on the left while the active contact conversation appears beside it on
  the right. The decode deck is vertically resizable and both panels fill its available
  height.
- Promoted the Contact Log to a resizable global panel above the compact bottom status bar,
  removed the duplicate Log side-panel tab, and restored connection, server, compute,
  reporting, and attention indicators to the bottom status line.
- Moved PTT lead and tail timing controls into Settings and removed the redundant manual
  PTT and stop/disarm controls from the TX deck while retaining programmatic TX safety
  controls.
- Added a local SQLite HamDB cache with a 30-day TTL. HamDB lookups run in the background,
  retain the complete returned callsign record, and never overwrite explicit QSO grid or
  state values.
- Expanded the Contact Log with callsign, operator name, grid, state, band, and mode columns,
  complete HamDB detail display, and explicit per-contact HamDB refresh through the button or
  F5. Refreshed HamDB details are also attached to existing matching log records and included
  in ADIF comments.
- Expanded the Achievement Hunter with acknowledgement-based hiding, reversible show/hide
  acknowledged controls, alert enablement settings, and additional band, grid, mode, time-of-day,
  contest, weak-signal, and QSO milestone achievements.
- Added opt-in RX audio monitoring with a bounded cpal output stream, selectable monitor output,
  and compact 🎧/🔊 indicators in the top radio-details panel. TX audio continues to use the
  configured TX output path, and monitor controls are persisted with the audio configuration.
- Added a dedicated Radio Tuning tab with named profile creation, editing, deletion, apply actions,
  complete stored control visibility, and FT8/FT4/CW/Other default-profile assignments. The top
  radio-details area now shows the active tuning profile and live AF/RF/power values reported by
  the connected radio.

### Fixed
- Eliminated the Windows startup window flashing. Restored geometry is applied through the
  viewport builder before the window is created, and the maximized state is applied after
  the first painted frame, so winit no longer performs `SW_MAXIMIZE`/`SW_HIDE` round trips
  on an unpainted window.
- Suppressed the console window that flashed during GPU detection by spawning `nvidia-smi`
  with `CREATE_NO_WINDOW` on Windows.
- Corrected a Windows-only test failure where a temporary log path embedded a thread name
  containing `::`, which is not a legal path character.

### Changed
- Switched the GUI to eframe's `glow` (OpenGL) backend on every platform and removed the
  `wgpu` renderer, dropping `wgpu`, `wgpu-hal`, and `naga` from the dependency tree. This
  also stops the 2D console from requesting a high-performance discrete GPU adapter.
- Moved audio/serial device enumeration and compute-backend probing off the UI thread,
  reducing time-to-first-frame from roughly 1.8 s to under 10 ms of app construction.
- Capped waterfall-driven UI repaints at roughly 15 Hz instead of repainting on every audio
  chunk, substantially lowering GPU and CPU load during receive.
- QSONaut now persists native window geometry itself in `window.json` under the platform
  app directory, validating stored values so a stale or corrupt entry cannot open the window
  off-screen or at an unusable size.

## [0.3.2] - 2026-08-16

### Fixed
- Explicitly install the Rustls Ring crypto provider before starting QSONaut Server WSS
  connections, preventing HTTPS/WSS startup panics in builds where provider inference is
  unavailable.

## [0.3.1] - 2026-08-16

### Added
- Named operator profiles with create, select, save, reload, active-profile persistence,
  safe profile names, and legacy profile fallback.
- Dedicated Contest, Reporting, Waterfall, Settings, Server, Log, and Achievements views.
- Built-in achievement catalog with locked/unlocked states, threshold progress, recent
  activity, and custom achievement management.
- Rolling live CW receive decoding from the same audio stream used by the waterfall, with
  one-second updates over an eight-second context and overlap-aware transcript updates.
- Selected-tone CW signal gating that rejects broadband noise and steady carriers before
  invoking cw-dit.
- Explicit QSONaut Server diagnostic delivery feedback for queued, accepted, rejected, and
  unavailable-client states.

### Changed
- Consolidated mode, mode-aware band, filter, tuning, PTT, and AF controls beneath the
  frequency/mode header; either mode or band selection now applies the complete decoder
  radio preset and exact mode-specific center frequency.
- Reserved the bottom deck for connection state and nonredundant runtime health rather than
  duplicating operational controls and status cards.
- Reorganized waterfall settings into shared display, radio-scope-only, and audio-only
  groups while keeping the live waterfall deck focused on signal display.
- Moved mode-specific operating frequencies into their mode implementations; the shared
  band-plan module now dispatches those definitions and retains common band lookup.
- Synchronized physical radio frequency, mode, and filter state on a low-latency cadence,
  with immediate repaint and separately throttled AF/RF/power polling.
- Updated Rigwright to `v0.1.3` for correct IC-7300 extended mode/filter status parsing.

### Fixed
- Restored accurate FIL1/FIL2/FIL3 banner state after filter changes instead of displaying
  an unknown or stale filter.
- Removed the nonfunctional radio mode-cycle button and made direct mode selectors retune
  the active band to the selected decoder's operating frequency.
- Prevented CI-V scope reads from monopolizing serial access and delaying radio commands or
  external VFO updates.
- Prevented live CW decode from producing rapid nonsense text when no keyed CW signal is
  present.
- Surfaced diagnostic server acknowledgements instead of leaving the client indefinitely
  at a misleading queued state.

### Safety
- Profile names are constrained to safe, human-readable filenames.
- Diagnostic snapshots remain manual and opt-in; tokens, audio samples, and configured
  device names are not included.
- Existing global transmit disarm and PTT guardrails remain in force after the UI move.

## [0.2.3] - 2026-08-14

### Added
- Optional QSONaut Server integration over authenticated, proxy-friendly WebSockets.
- Server synchronization for event and contest-catalog snapshots, station presence,
  idempotent QSO log publication, diagnostics, and shared channel messages.
- Server connection settings, operator presence/privacy controls, diagnostic snapshot
  controls, connection status, reconnect handling, and heartbeat traffic.
- Automation events and permission-gated actions for server synchronization and channel
  publishing.

### Changed
- Added the `qsonaut-server-client` crate for the stable `v1` server contract.
- Extended the operator profile/configuration model with server integration settings.
- Preserved standalone desktop operation: server connectivity remains optional and disabled
  by default.

### Safety
- Radio control, transmit, and external-send automation executors remain disabled by default.
- Presence sharing and diagnostic sharing are independently opt-in.
- Device tokens are supplied through local configuration and are not embedded in source or
  automation manifests.

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
