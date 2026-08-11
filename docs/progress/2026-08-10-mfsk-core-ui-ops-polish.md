# 2026-08-10 — mfsk-core integration + operator UX polish

## Summary

RigForge UI transitioned to an integration-first modem strategy for active digital decode:

- FT8 live decode worker now uses `mfsk-core` instead of the previous in-app custom decode chain.
- Workspace tabs expanded to represent staged modem surfaces: FT8/FT4/FST4/WSPR/JT9/JT65/Q65/MSK144 plus CW/FLDIGI integration targets.
- Decode log usability improved for live operating sessions.

## UX updates implemented

- Decode log auto-follow toggle (`Follow`) for live scroll behavior.
- Configurable decode-log retention (`Keep N rows`) to trim older entries.
- Visual readability improvements:
  - column headers
  - separator lines between UTC groups
- Global operator profile area added (callsign, grid, QTH) and reflected in mode workflows.

## Follow-on ops improvements

- Added profile persistence file: `.rigforge_profile.toml`.
  - Auto-load at startup.
  - Auto-bootstrap defaults when missing.
  - In-app `Save Profile` / `Reload Profile` controls.
  - Stores callsign/grid/QTH plus FT8 operator preferences (follow-log, retention, decode depth).
- Reduced live FT8 decode lag risk:
  - Decode worker now enforces a single in-flight decode pass (no pileup/backlog threads).
  - If a decode pass is still running at the next period boundary, the stale trigger is skipped.
  - FT8 UI now exposes `DECODE: FAST/DEEP` toggle (FAST default for lower latency, DEEP for harder-copy conditions).

## Maintenance direction

- Keep GUI focused on orchestration and operator ergonomics.
- Keep modem/DSP decode behavior in external, maintained backends where practical.
- Prefer contribution upstream to modem backends over forking signal-processing logic into RigForge UI.

## Notes

- Current in-app operator profile edits are runtime/session-level UX state.
- Persisting profile edits back to `.env`/TOML can be added as a separate configuration-writing task.
