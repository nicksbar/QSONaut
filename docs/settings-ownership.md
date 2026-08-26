# QSONaut Settings Ownership

This document defines where a setting belongs, who owns it at runtime, and
where it is persisted. The important distinction is that a setting can be
stored in a profile file for compatibility while still being application-wide
at runtime.

## The three scopes

### Application-wide

There is one value for the running QSONaut process. Switching radio tabs must
not silently replace it.

Examples:

- station identity: callsign, grid, QTH, station notes, antenna, and rig notes
- UI scale and compute-backend preference
- session-only graphics power and GPU preference
- application logging and other desktop presentation policy

These values are edited from the Station or Settings areas. They are not
radio-tab state, even when older profile files contain a copy of them.

### Radio-tab / operator profile

Each open radio tab has its own independent `OperatorProfile` and worker
configuration. Switching tabs restores that tab's state without reconnecting
or reloading an already-running session.

Examples:

- serial/backend/model/endpoint and radio enablement
- capture and monitor audio devices, sample rate, channels, monitor volume
- digital timing, contest operation, decode policy, and mode assignments
- waterfall theme and profile-specific radio-scope preferences
- server association and tab-specific activity state

These values are managed from the profile drawer and persisted to the selected
operator-profile file.

### Global reusable radio definitions

Reusable control snapshots are not owned by an operator profile. They are a
shared library that can be assigned independently by each radio tab and mode.

The library contains named values such as mode, data mode, filter, AF/RF gain,
power, AGC, and supported vendor controls. The selected tab stores only its
mapping, for example `FT8 -> Contest`, while the named `Contest` definition is
shared globally.

Radio-definition creation, editing, deletion, and mode assignment belong in
**Radio Tuning**. Digital Timing must not create or save radio profiles.

## Persistence map

| Data | Runtime owner | Persistence |
| --- | --- | --- |
| Station identity and station notes | Application session | Active profile file for current compatibility and restart persistence |
| UI scale and compute preference | Application session | Active profile file; never reapplied during radio-tab switching |
| Graphics power and GPU preference | Application session | Not persisted; applied by a GUI process restart |
| Radio connection and audio devices | Radio tab | `profile.toml` or `profiles/<name>.toml` |
| Digital timing and contest state | Radio tab | Selected operator-profile file |
| Waterfall theme and profile scope settings | Radio tab | Selected operator-profile file |
| Per-mode radio-definition assignments | Radio tab | Selected operator-profile file |
| Named reusable radio definitions | Application-wide library | `radio-profiles.toml` |
| Active profile name | Application | `active-profile` |
| Runtime diagnostics | Application | Platform log directory, `qsonaut.log` |

The configuration directory is supplied by `qsonaut_log::app_config_dir()`.
On Linux this is normally `~/.config/qsonaut`; Windows and macOS use their
platform application-data locations.

## Switching and saving rules

1. Opening a tab loads its operator profile and starts only that tab's workers.
2. Switching tabs changes the active UI/session context; it does not stop a
   healthy inactive tab or overwrite application-wide station settings.
3. A tab-specific edit marks that tab dirty and saves its operator profile.
4. Editing a named radio definition saves the global radio library and also
   saves the current tab's mode assignment if it changed.
5. Closing a tab stops only that tab's workers. Deleting a profile removes its
   profile file and tab; it must not remove global radio definitions that other
   tabs may use.
6. Missing hardware disables only the affected tab's workers. It must not
   prevent the GUI from opening or prevent another tab from operating.

## Compatibility and migration

Older operator profiles may contain embedded `radio_profiles` arrays. Those
arrays are read once when no global library exists, merged by name across the
existing profiles, and written to `radio-profiles.toml`. New profile saves do
not write reusable definitions back into operator-profile files.

When adding a setting, decide its scope before adding a field:

1. Does it describe the station or desktop? Make it application-wide.
2. Does it describe one radio's hardware, workers, or activity? Put it in the
   radio-tab profile.
3. Is it a named reusable radio-control snapshot? Put it in the global
   library, with only the per-tab/per-mode selection in the profile.

Do not duplicate a value in multiple runtime owners. If compatibility requires
an old serialized field, document it as a migration field and ensure it cannot
override the authoritative value during tab switching.
