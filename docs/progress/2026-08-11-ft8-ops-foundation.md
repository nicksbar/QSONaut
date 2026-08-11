# 2026-08-11 - FT8 operating foundation

## Completed

- Added a testable FT8 operating layer separate from egui rendering.
- Parsed standard message roles as destination, source, and exchange instead of guessing a reply callsign from token position.
- Added standard QSO progression for grid, report, R-report, RRR/RR73, and 73 messages.
- Added automatic caller selection policies: first decoded, strongest, weakest, and closest to the selected RX tone.
- Kept unsolicited CQ answering behind a separate explicit operator option.
- Preserved TX parity when the immediate reply slot is missed.
- Replaced simultaneous PTT/audio launch with an acknowledged TX transaction:
  - configurable PTT lead and tail;
  - CI-V PTT acknowledgement before audio;
  - late-start rejection;
  - unconditional PTT release attempt;
  - playback and radio errors returned to the operator.
- Added explicit FT8 decoder readiness, runtime, clock-delta, audio-level, and clipping diagnostics.
- Reset the FT8 decimator and slot buffer when returning from another workspace, preventing stale cross-mode audio from entering a decode.
- Added a separately configurable TX audio device through `RIGFORGE_AUDIO_OUTPUT_DEVICE`.
- Corrected stereo capture downmixing so equal full-scale channels do not lose half their level.

## Current operating scope

The automatic sequence engine currently covers standard FT8 QSOs. Contest exchanges, nonstandard callsign transmission, DXpedition roles, durable ADIF logging, and the WSJT-X UDP companion protocol remain explicit follow-on work. They should extend the operating layer rather than adding contest-specific decisions to the GUI or DSP backend.

`mfsk-core` remains responsible for protocol decoding, packing supported message forms, and waveform generation. RigForge owns operator policy, session state, radio/audio sequencing, diagnostics, and interoperability.
