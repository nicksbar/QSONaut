# Contesting review and implementation results

2026-09-10. This is an initial implementation increment, not completion of the
full attached architecture. See [the phased plan](contesting-implementation-plan.md).

## Files changed by repository

### QSONaut

- `CHANGELOG.md`
- `README.md`
- `crates/qsonaut-contests/src/lib.rs`
- `crates/qsonaut-gui/src/activity.rs`
- `crates/qsonaut-gui/src/chat.rs`
- `crates/qsonaut-gui/src/contest.rs`
- `crates/qsonaut-gui/src/lib.rs`
- `crates/qsonaut-gui/src/modes/cw.rs`
- `crates/qsonaut-gui/src/modes/ft4_runtime.rs`
- `crates/qsonaut-gui/src/modes/ft8_runtime.rs`
- `crates/qsonaut-gui/src/modes/voice.rs`
- `crates/qsonaut-gui/src/panels/profile_server.rs`
- `crates/qsonaut-gui/src/profile.rs`
- `crates/qsonaut-gui/src/profile_manager.rs`
- `crates/qsonaut-gui/src/rendering/activity.rs`
- `crates/qsonaut-gui/src/rendering/mod.rs`
- `crates/qsonaut-gui/src/rendering/workspace.rs`
- `crates/qsonaut-gui/src/reporting/qso.rs`
- `crates/qsonaut-gui/src/runtime/constructor.rs`
- `crates/qsonaut-gui/src/server_integration.rs`
- `crates/qsonaut-log/src/lib.rs`
- `docs/feature-matrix.md`
- `crates/qsonaut-contests/src/rules.rs`
- `crates/qsonaut-gui/src/rendering/contest_session.rs`
- `docs/contesting-implementation-plan.md`
- `docs/contesting.md`
- `docs/contesting-review-results.md` (this report)

### QSONaut-Server

- `crates/qsonaut-api/src/error.rs`
- `crates/qsonaut-api/src/realtime.rs`
- `docs/club-operations.md`
- `docs/qsonaut-client-sync.md`
- `CHANGELOG.md`
- `crates/qsonaut-store/tests/event_authorization.rs`
- `docs/contesting-migration.md`
- `migrations/0019_event_log_authorization.sql`

The small chat conditional cleanup fixes an existing Clippy error discovered
by the required workspace gate; it does not change LAN behavior.

## Data model and migration

Local QSO records add backward-compatible operator/station callsign, contest
template/session, server-event and club fields. Profile records add operating
activity and a persistent local contest occurrence ID. ADIF round trips preserve
these values; old records load empty values without inferring an entitlement.

Server migration 0019 installs an event submission trigger without rewriting
historical data. It locks event/membership rows, checks active operating roles,
rejects draft/cancelled events, and checks a half-open event time window.
Completed events allow in-window delayed uploads. Already accepted idempotent
retries still return the original record. No migration has been applied to a
live or test PostgreSQL instance during this run.

## Protocol

The v1 protocol version is unchanged. Desktop uploads use the event saved on
the contact and include captured identity/template/club fields in exchange
metadata. Those client fields are not authorization. The server surfaces its
intentional event-policy messages; unexpected database errors remain generic.
Typed/versioned contest declarations and scoring receipts remain outstanding.

## Operator workflow

Choose Local Contest or Field Day; choose a definition in Station Settings.
Use the shared setup/exchange editor in Voice, CW, FT8, FT4 and the other mode
workspaces. Local TX validates setup and band/mode; Dupe check follows catalog
rules within the saved contest occurrence. New local session resets duplicate
context and serials. Serial advancement and clearing received exchange values
are shared after logging. Context changes disarm application TX. Historical
uploads retain the original event, even after selecting another event.

Server contest TX is explicitly unavailable until synchronized event rules
and station authorization exist. Exchange fields do not imply support for
additional FT8/FT4 on-air message encodings. No scores are calculated yet.

## Authorization decisions

Authenticated server users remain the actors. Selecting a club never replaces
the operator with a club/special callsign. Current desktop identity capture uses
the configured personal call for both separately stored identity fields.
Membership/event checks run in the database and therefore cover legacy clients.
A managed callsign registry and explicit participant/station assignments are
still required; membership alone is not such an assignment.

## Verification

- Both repositories: `cargo fmt --all`,
  `cargo test --locked --workspace --all-targets`,
  `cargo clippy --locked --workspace --all-targets -- -D warnings`, and
  `git diff --check` pass. Desktop tests: 463 passed, 2 ignored.
- Both repositories: `cargo llvm-cov --locked --all-features --workspace --summary-only`
  completes. Desktop unfiltered coverage: 52.02%; server: 15.22%.
- Desktop report with the unchanged CI filename exclusions: 60.50% workspace,
  82.31% Rigwright integration, 81% CW. All existing numeric gates pass.
- Changed-file coverage check passes against HEAD. The new rules file has 100%
  line coverage; the new shared editor has 79.73%. Existing baseline exclusions
  are unchanged. No remote CI was run.
- Server web: `npm ci`, `npm run check`, and `npm run build` pass.
- PostgreSQL contract tests compile but skip execution when
  `QSONAUT_TEST_DATABASE_URL` is absent, as it is here. This includes the new
  authorization trigger test. This is not database validation.
- Shared editor tests exercise Voice/CW/FT8/FT4 headlessly. No rendered application
  screenshot or physical radio validation was performed.

## Remaining architecture

Phase 1 still needs logging-time exchange enforcement with durable recovery for
automatic QSOs that fail validation. The unified session model, POTA/SOTA role
integration, managed callsign lifecycle/audit, participant/station assignments,
server-owned versioned definitions/configuration, metadata expiry/revocation,
per-installation idempotency, transactional duplicate/multiplier calculations,
scoring strategies/receipts, and management controls remain unimplemented.
The server still stores client-submitted points; do not present them as final
contest scores. The plan is intentionally marked incomplete.
