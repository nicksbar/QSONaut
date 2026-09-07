# Achievement automation

Status: implemented on the `release/0.3.18` branch

QSONaut's Achievement Hunter is a read-only operator-progress system built on
the same structured event boundary as QSONaut automation. It turns normal
station activity into persistent, explainable goals without giving an
achievement definition permission to control the radio, transmit, or publish
operator data.

## What is included

The current catalog contains 22 achievements covering:

- first decode, directed calls, and QSO milestones;
- unique callsigns, bands, grids, and operating modes;
- Worked All States (50 distinct US states);
- FT8, FT4, and CW per-mode milestones;
- DX, early-morning, late-night, contest-exchange, and weak-signal contacts;
- duplicate-transmit protection; and
- long-running decode/audio activity.

The checked-in catalog is [`achievements.example.toml`](../achievements.example.toml).
It is compiled into the desktop application as a versioned, reviewable default
catalog. It is intentionally an example/catalog artifact rather than an
unrestricted runtime script: changing achievement semantics is a release-level
change, while operator-created threshold rules remain available in the GUI.

## Event flow

```text
radio/mode/log/contest activity
              |
              v
        qsonaut-core AppEvent
              |
              v
 normalize_app_event_for_automation()
              |
              v
       qsonaut-automation AutomationEvent
          |                    |
          |                    +--> ordinary permission-gated rules/actions
          v
   AchievementEvaluator
          |
          +--> progress and unlock decision
          +--> persisted AchievementKind unlock
          +--> Hunter alert feed
          +--> profile autosave
```

The application publishes events such as `CallsignHit`, `QsoLogged`, contest
state changes, and duplicate-TX blocks. The GUI adapter normalizes them into
`AutomationEvent` values with typed event kinds, fields, and tags. The
achievement evaluator observes those events; it does not consume application
log text.

## Catalog schema

Each `[[achievements]]` entry contains:

| Field | Meaning |
| --- | --- |
| `id` | Stable catalog identifier used for routing and migration. |
| `title` | Operator-facing achievement title. |
| `detail` | Explanation shown in the Hunter panel and alert feed. |
| `on` | Event kind that can advance the achievement. |
| `when` | Optional list of predicates; all predicates must match. |
| `metric` | `event_count` or `unique_field`. |
| `field` | Field used by `unique_field`, such as `call`, `mode`, `grid`, or `state`. |
| `target` | Saturating progress target; zero is invalid/no-op. |

Supported predicates are:

- `field_equals` and `field_not_equals`;
- `field_contains`;
- `field_less_than` and `field_greater_than` for numeric event fields; and
- `has_tag` for normalized semantic conditions.

`unique_field` values are trimmed and case-normalized before insertion. This
prevents `ft8`, `FT8`, and ` FT8 ` from becoming three separate modes. Progress
is capped at the target, and an achievement emits an unlock only once per
stable identifier.

## Structured achievement signals

QSO events carry metadata used by the catalog:

- `mode`, `band`, `grid`, `state`, and `country`;
- `time_on`, `report_received`, and `operation_mode`; and
- `contest_exchange_received`.

The normalizer derives semantic tags where the meaning is more useful than
repeating parsing logic in every definition:

- `early_bird` for contacts before 07:00 UTC;
- `night_owl` for contacts at or after 23:00 UTC;
- `dx` for a non-US country;
- `contest_operator` for a non-empty contest exchange; and
- `signal_survivor` for a report below -20 dB.

The duplicate guard publishes a `command` event tagged `dupe_blocked`, and
decode batches publish `decode` events. These paths let achievement rules
observe safety and decoder activity without coupling the shared automation
crate to GUI internals.

## Persistence and UI behavior

Unlock state is stored in the operator profile through the existing
`AchievementKind` representation. This preserves compatibility with profiles
created before the declarative catalog was introduced. The evaluator's
short-lived counters and unique-value sets are rebuilt from current runtime
activity; the durable source of truth for completed built-ins is the profile's
unlocked set.

The Achievement Hunter panel shows:

- unlocked and acknowledged achievements;
- progress bars for built-in goals;
- recent achievement alerts;
- the number of loaded catalog definitions; and
- per-mode QSO counts.

Alerts can be disabled without disabling evaluation or persistence.
Acknowledging an achievement hides it from the default list but does not
remove its unlocked state.

The existing custom-achievement UI remains separate. Custom rules currently
use the operator-selected session metrics (`unique_heard`, directed hits,
QSOs, duplicate blocks, and decode bursts), while the catalog uses structured
automation events. This separation keeps profile compatibility while the
catalog schema evolves.

## Why this design

### Structured events instead of log parsing

Logs are for diagnostics and evidence. They are text-oriented, may be filtered,
and can change wording. Events are typed, testable contracts with stable fields
and tags, so a change in log presentation cannot silently change achievement
semantics.

### Shared evaluator, GUI-owned presentation

`qsonaut-automation` owns definitions, predicates, counting, uniqueness, and
one-time unlock decisions. The GUI owns alerts, profile persistence, and
presentation. This keeps generic rule logic reusable while preserving the
explicit radio/audio/GUI/server boundaries required by QSONaut.

### Read-only achievement authority

Achievement rules observe activity only. They do not request
`RadioControl`, `Transmit`, `ExternalSend`, or server capabilities. Ordinary
automation actions remain subject to component manifests and operator grants,
and all radio/TX paths retain their existing safety gates.

### Stable IDs and compatibility

Catalog IDs are mapped to the existing persisted `AchievementKind` values.
That deliberate compatibility layer avoids invalidating existing profiles while
allowing future catalog entries to be added with explicit migration decisions.

## Verification

The implementation is covered by:

- catalog TOML parsing tests;
- event-count and unique-field evaluator tests;
- predicate tests;
- QSO metadata and semantic-tag normalization tests;
- duplicate-event routing tests; and
- existing GUI achievement rendering and profile tests.

Release validation uses the workspace format, test, clippy, diff, and coverage
gates described in [`AGENTS.md`](../AGENTS.md). Coverage demonstrates software
execution only; it is not a substitute for on-air or hardware acceptance.

## Current limitations

- The default catalog is compiled into the application; arbitrary external
  catalog loading is not yet a user-facing configuration feature.
- The evaluator's live counters are not serialized independently; completed
  unlocks persist, while progress is reconstructed from the current log/session
  state where the UI has the required data.
- The catalog-to-legacy `AchievementKind` mapping is explicit and must be
  updated when a new persisted built-in is introduced.
- Custom GUI rules and declarative catalog definitions do not yet share one
  schema. They are intentionally kept compatible but remain separate surfaces.
