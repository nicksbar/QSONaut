# Contest operating architecture review

Reviewed 2026-09-10 against the operating-activity, contest, club, and callsign brief.

## Current implementation

- The desktop packages 19 definitions and exposes setup fields, serials, run/search
  operation, and duplicate TX checks. Voice has structured exchange editors; CW
  and digital modes mostly use exchange strings.
- Activity, contest enablement, template selection, and server event selection are
  independent state. Selecting another activity does not clear the selected event.
- Duplicate checks compare callsign/band/mode across the entire log, ignoring the
  catalog's `none`, `band`, and `band-mode` distinctions and contest context.
- Voice builds exchange controls from setup fields, although those are different
  concepts (Field Day power is setup, not a received exchange).
- Server publishing reads the currently selected event, including when publishing
  an edited historical QSO. Local records do not preserve that event or station
  identity.
- The server owns templates and checks event configuration during management.
  Its QSO insertion path currently accepts event references and submitted points
  without event membership, schedule, exchange, or scoring validation.
- The server client counts templates from snapshots but does not retain the
  definitions for client validation. Cached event data has no expiry contract.

## Phased implementation

1. **Local contest correctness:** reusable catalog validation and duplicate rules;
   distinguish setup from exchange; preserve identity/template/event context in
   logs and ADIF; clear old activity context; expose validation beside operation.
2. **One operating session:** consolidate activity, setup, identity, and exchanges;
   use one editor in Voice, CW, FT8, and FT4; gate logging and application-controlled
   TX; persist defaults without persisting server permissions.
3. **Server authorization:** managed callsign registry with lifecycle/audit records,
   active membership and explicit participant/station assignments; enforce these
   transactionally for all log paths, including legacy clients and retries.
4. **Versioned synchronization:** shared typed protocol contracts, retained server
   templates, expiry/revocation, declaration, reconnect handling, immutable QSO
   context and per-installation idempotency.
5. **Scoring and operations:** versioned scoring strategies with explicit unsupported
   rules, transactional duplicate/multiplier handling, local provisional previews,
   authoritative server receipts, club/event management controls and migration docs.

Each phase must include regression tests and repository gates. Catalog point
defaults and descriptive formulas are insufficient to claim complete contest
scoring. No radio or real contest operation is validated by unit tests.

## Increment status

Implemented: shared setup/exchange UI, local occurrence IDs and profile activity
persistence, catalog band/mode/setup validation before application TX, local
duplicate rules, common serial advancement, captured identity/event fields and
ADIF round trips, context-change TX disarming, and a minimum server event
membership/schedule database guard. Server-event TX is explicitly unavailable
until server rules and station authorization can be resolved.

Not complete: the entire attached architecture. Phase 1 still needs exchange
validation at logging with a durable recovery workflow for rejected automatic
QSOs; phases 2–5 retain the model, registry, participation, synchronized catalog,
authorization-expiry, and scoring work above. The new database guard does not
replace explicit participant or callsign assignments. PostgreSQL execution and
visual/radio operation have not been validated in this environment.
