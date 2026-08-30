# QSONaut feature matrix

This is the detailed implementation inventory for the QSONaut application.
The README gives the short version; this page records what is actually wired
into the application, what is gated by the selected backend/radio profile, and
what remains experimental or planned.

## v0.4 maturity vocabulary

The v0.4 consolidation uses four release-planning classifications. The
implementation levels below remain useful detail, but they do not by
themselves imply release support.

| Maturity | Meaning for v0.4 |
|---|---|
| **Stable enough to depend on** | The workflow is implemented, has automated coverage where practical, and has either the reference-station evidence or a clearly bounded non-hardware contract. |
| **Experimental but usable** | An operator can use the workflow, but timing, hardware, external-service, or completeness limits remain. |
| **Concept only** | Models, permissions, placeholders, or partial scaffolding exist without a complete dependable operator workflow. |
| **Deferred** | Explicitly outside the v0.4 consolidation scope; do not use its absence as an implementation defect for this release. |

Maturity is intentionally conservative: a passing software test is not a
hardware-validation result, and a cataloged Rigwright model is not a validated
radio. The detailed tables below retain the more precise implementation level
and the limitation that explains each classification.

## Reference station and evidence boundary

The v0.4 reference station is Linux/WSL with an Icom IC-7300 using its USB
CI-V/control and USB audio paths. The repeatable setup guidance is in the
[IC-7300 guide](radios/models/ic-7300/guide.md) and the [audio monitoring
guide](audio-monitoring.md). The IC-7300 native control/scope path is the
current hardware-validated baseline; other radio profiles, serial families,
and alternate platforms remain experimental until their own evidence is
recorded.

Evidence in this document has two separate meanings:

- **Software-tested** means covered by unit/integration tests, generated
  waveforms, mock/offline backends, or CI checks. It proves deterministic
  application behavior within that test boundary.
- **Hardware-tested** means exercised on the stated physical radio/audio
  setup with the operating conditions and expected-versus-observed result
  recorded. It is not inferred from a build, a profile entry, or a software
  test.

The current baseline is intentionally not a claim that every listed mode,
radio, connector, or platform is hardware-tested. Open hardware evidence is
tracked by the radio validation program in issue #41 and its model issues.

## v0.4 classification summary

| Component or feature group | Maturity | Evidence and boundary |
|---|---|---|
| Reference IC-7300 native control and scope | Stable enough to depend on | Hardware-validated development path; model-specific controls and scope have software coverage as well. |
| FT8 and FT4 core workflows | Stable enough to depend on | Generated-audio and timing tests plus the established IC-7300 path; broader hardware campaign remains tracked separately. |
| QSO logging, ADIF import/export, and local persistence | Stable enough to depend on | Software-tested round trips and GUI workflow; backup/restore and broader acceptance evidence remain open. |
| Audio selection, canonical 48 kHz processing, and monitoring | Stable enough to depend on | Software-tested negotiation/resampling and documented Linux/WSL setup; device-matrix validation remains open. |
| SSTV receive, transmit preparation, and explicit TX safety | Experimental but usable | Software round-trip coverage and operator-visible abort/disarm paths; end-to-end hardware validation remains open. |
| JT9, JT65, Q65, FST4, and CW | Experimental but usable | Native or generated-audio paths exist, with uneven workflow completeness and validation. |
| WSPR and MSK144 | Experimental but usable | Receive-only integration; transmit workflows are intentionally unavailable. |
| Radio profiles other than the IC-7300 | Experimental but usable | Capability-gated implementations; physical validation is not implied by catalog presence. |
| Contest workflow and automation actions | Experimental but usable | Software state/persistence/guard tests exist; complete contest and external-adapter workflows remain unvalidated. |
| Local AI, PSK Reporter, and QSONaut Server integration | Experimental but usable | Optional paths with failure isolation and opt-in boundaries; external service and privacy validation remain open. |
| Discord/IRC connectors and unattended operation | Concept only | Scaffolding or policy exists without a complete validated production workflow. |
| New modem implementations, broad radio expansion, advanced contest scoring, and new distribution formats | Deferred | Explicit v0.4 non-goals from the consolidation epic. |

| Level | Meaning |
|---|---|
| **Validated** | Exercised end-to-end in the project’s normal hardware path. The current radio baseline is the Icom IC-7300. |
| **Integrated** | Wired into the application with tests and operator-visible state, but not hardware-validated on every supported device. |
| **Gated** | Integrated behind a capability/profile check; the UI exposes it only when Rigwright reports support. |
| **Experimental** | The workflow exists, but protocol, timing, hardware, or operator behavior still needs broader validation. |
| **RX-only** | Receive/diagnostic behavior exists; transmit behavior is intentionally unavailable. |
| **Scaffolded** | Architecture, models, or permissions exist, but the complete operator workflow is not finished. |
| **Not implemented** | Explicitly absent from the current application. |

## Radio integration

QSONaut currently has four selectable radio backends. The native backend covers
four Rigwright protocol families; the other three backends are integration,
diagnostic, or test backends. The backend determines which capability surface
can be advertised to the UI.

| Backend | Level | Radio features exposed in QSONaut |
|---|---|---|
| Native Rigwright | Integrated / Gated | Model-aware frequency, mode, PTT, power, typed controls, normalized meters, tuner, SWR, and Icom scope where the selected profile supports them. |
| Hamlib `rigctld` | Integrated / Experimental | External frequency/mode/PTT/power transport through rigctld. Native Rigwright vendor controls, normalized meters, tuner, and scope are not assumed or advertised through this backend. |
| DX Lab Commander | Integrated / Experimental | External Commander frequency/mode/PTT/power transport. Vendor-specific controls, normalized meters, tuner, and scope are not assumed. |
| Null/offline radio | Integrated / Test | In-memory frequency, mode, and PTT behavior for UI development and tests; no physical radio capabilities. |

| Feature | Level | Current implementation |
|---|---|---|
| Multi-vendor radio selection | Integrated / Gated | Model-aware Rigwright profiles for Icom CI-V, modern Yaesu CAT, classic Yaesu CAT, and Kenwood PC control. Generic profiles remain conservative. |
| Frequency, mode, data mode, filter, and PTT | Integrated / Gated | Native radio worker and workspace presets use Rigwright’s protocol-neutral HAL; unsupported operations are capability-gated. |
| Radio power | Gated | Upper-right graphical power control with pending/settling state and tooltip; power-read support varies by backend/vendor, and Icom power state is write-only at the protocol level. |
| AF/RF gain, squelch, and RF power | Gated | Compact banner controls use normalized `0..=255` values and convert display percentages at the UI boundary. RF power is also used by the low-power SWR workflow. |
| Preamp and attenuator | Gated | Profile-dependent compact controls; exact ranges remain radio-specific. |
| NB, NR, NR level, IP+, notch, manual notch, and AGC | Gated | Compact top-banner controls appear only when the loaded Rigwright profile advertises them. NR level is currently consumed for modern Yaesu profiles. |
| Split and tuner controls | Gated | Split follows profile support. Tuner enable/status and explicit tuner start are available where Rigwright supports them; tuner actions are blocked during an SWR sweep. |
| Normalized meter panel | Gated | QSONaut polls and displays every `MeterId` advertised by the selected profile. Values are normalized HAL deflection levels, not universal physical units. |
| Live SWR display | Integrated / Gated | SWR is shown with IC-7300-specific calibrated ratio anchors; other radios show normalized meter presentation unless a verified ratio mapping exists. |
| Stepped TX SWR sweep | Experimental / Gated | Active-band defaults, configurable start/stop/step/interval, low-power RTTY carrier pipeline, tuner disable/restore, per-point logging, stop control, charting, and state restoration. It requires a radio that exposes SWR telemetry. |
| Native Icom scope/waterfall | Validated / Gated | Profile-specific CI-V scope setup and ordered waveform assembly; IC-7300 is the hardware-validated path, with additional model geometries implemented but not broadly validated. |
| Radio capability discovery | Integrated | Native controls/meters are collected from Rigwright’s `supports_control` and `supports_meter` methods rather than assumed from vendor names; external backends expose only their own root capabilities. |
| RIT/XIT, antenna selection, memory/channel, main/sub workflow | Not implemented / Partial | HAL/manual surfaces may exist or be documented, but QSONaut has no complete operator workflow for these functions. |

### Current radio maturity

| Radio family/profile | Application level | Current QSONaut coverage |
|---|---|---|
| Icom CI-V generic | Gated / Experimental | Protocol-only profile. Core radio operations may work, but model-specific controls, meters, and scope are deliberately withheld. |
| Icom IC-7300 | Validated | Primary hardware path: power write, profile controls, IP+, notch, tuner, normalized SWR, SWR sweep, scope, and CI-V echo-back tolerance. |
| Icom IC-705 | Gated / Experimental | Profile-specific controls, tuner, normalized SWR, and scope geometry; no broad physical validation. |
| Icom IC-7610 | Gated / Experimental | Profile-specific controls, main/sub metadata, tuner, normalized SWR, and dual-receiver scope geometry; no broad physical validation. |
| Icom IC-9700 | Gated / Experimental | Profile-specific controls, external preamp, main/sub metadata, tuner, normalized SWR, and VHF/UHF scope ranges; no broad physical validation. |
| Modern Yaesu CAT generic | Gated / Experimental | Protocol-only profile; typed modern controls and meters require an exact model profile. |
| FT-710, FTDX10, FTDX101D, FTDX101MP | Gated / Experimental | Frequency, mode, readable PTT, power, profile split, AGC, NR, NR level, and normalized signal/power/SWR/ALC/compression/current/voltage meters. |
| FT-991A | Gated / Experimental | Same modern CAT meter/control family, with model-specific mode/range behavior and split not currently profiled as typed. |
| Classic Yaesu CAT generic | Gated / Experimental | Protocol-only five-byte CAT profile; no model-specific split until an exact classic model is selected. |
| FT-817ND, FT-818, FT-857D, FT-897D | Gated / Experimental | Frequency, mode, readable/writable PTT, status, and split through the legacy CAT family. Power, normalized meters, tuner, and modern controls are intentionally absent. |
| Kenwood PC control generic | Gated / Experimental | Protocol-only profile; model-specific power, split, meter selector, range, and PTT behavior require an exact model. |
| TS-590SG, TS-890S, TS-2000 | Gated / Experimental | Frequency, mode, power, split, model-specific PTT behavior, normalized signal/SWR, and interleaved Auto Information response handling. |

## Digital modes

| Mode | Level | Current implementation |
|---|---|---|
| FT8 | Integrated / Validated path | Slot-aligned decode, activity, compose/reply flow, sequencing, TX history, logging, duplicate guards, and global TX disarm. |
| FT4 | Integrated | Native decode and scheduled TX workflow with activity, conversation, sequencing, logging, and TX disarm. |
| FST4-60 | Experimental | Native decoder/workspace and scheduled-TX path; broader timing and hardware validation remains open. |
| JT9, JT65, Q65-30A | Experimental | Native receive/decode and manual or scheduled waveform TX paths; Q65 is currently configured around the A30 submode. |
| WSPR | RX-only | Receive/decode integration; no native automatic transmit workflow. |
| MSK144 | RX-only | Receive/decode integration; no complete native TX workflow. |
| CW | Experimental | Software audio decode, selected-channel monitoring, adaptive timing, and generated subband TX. Paddle input, prosigns, punctuation, and automatic sequencing are not implemented. |

## SSTV and imaging

| Feature | Level | Current implementation |
|---|---|---|
| SSTV receive | Integrated / Experimental | Auto VIS targeting, manual filtering, shifted-header diagnostics, progress/failure logging, and 13 Martin/Scottie/Robot/PD modes. |
| SSTV transmit | Experimental | Selectable mode and image/audio pipeline with explicit TX arming and global disarm. |
| Local image browser | Integrated | Browse existing images for SSTV transmission. |
| Local AI image tools | Experimental | Loopback-only Ollama/Lemonade discovery, generation, analysis, and reinterpretation workflows. |

## Station and operator workflow

| Feature | Level | Current implementation |
|---|---|---|
| QSO logging | Integrated | Local contact log, QSO history, ADIF import/export, operator profiles, and contest exchange fields. |
| Contest workflow | Integrated / Experimental | Run/S&P policy, serial persistence, exchange preview, duplicate guards, role-aware Fox/Hound guidance, and automation events. |
| PSK Reporter | Integrated / Opt-in | Optional reporting, disabled by default. |
| Application log | Integrated | Filterable live tail, severity highlighting, copy, bottom-follow, and mode-specific diagnostic messages. |
| Audio devices and monitoring | Integrated | Settings-based input/output selection, decoder-channel monitoring, monitor volume, and audio diagnostics. |
| Global TX stop/disarm | Integrated | Central stop/disarm path releases active PTT and clears mode-specific TX arming state. |

## Server, automation, and compute

| Feature | Level | Current implementation |
|---|---|---|
| QSONaut Server WSS integration | Integrated / Opt-in | Event/catalog sync, station presence, selected radio metadata, idempotent QSO publication, shared channels, and manual diagnostics. Each outbound category is independently enabled. |
| Automation event bus | Scaffolded / Integrated | Events cover decodes, callsign hits, logged QSOs, radio state, contest/profile changes, commands, external messages, server messages, and timers. |
| Automation action safety | Integrated / Experimental | Permission-gated radio commands and transmit requests enforce TX-active/disarmed checks; direct PTT control remains restricted. |
| External Discord/IRC connectors | Scaffolded | Component/source declarations exist, but connectors are not validated as a complete production workflow. |
| Compute backend discovery | Integrated | CPU/SIMD and local backend discovery are reported; GPU/NPU decode kernels are not validated. |

## Deliberate gaps

The following are intentionally not claimed as complete features: unattended
operation, universal radio compatibility, universal physical-unit meter
conversion, automatic tuner behavior, complete RIT/XIT/antenna/memory control,
CW paddle input, full MSK144/WSPR transmit workflows, and validated GPU/NPU
decoder acceleration.

When a feature changes, update this matrix and the README summary together.
For radio changes, also update the Rigwright capability matrix and add or
adjust tests before changing a maturity label.
