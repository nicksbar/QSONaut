# Core hardening plan (feat/core-modes-hardening)

Status: active
Owner: QSONaut core
Date: 2026-08-12

## Scope locked from operator input

- Milestone strategy: **one bigger milestone PR**
- First milestone theme: **core mode reliability + test matrix**
- Production-ready target modes for this cycle:
  - **FT8**
  - **FT4**
  - **JT9**
- Contest workflow focus (all in scope):
  - Simple split workflow (safe/fake split orchestration)
  - Fox/Hound guidance + state-machine UX
  - Dupe checking + worked status
  - Run/S&P helper states and macros
  - Serial/CQ number workflow
- Logging/interoperability focus:
  - ADIF field completeness + validation
  - ADIF import
  - Export slices/filters
  - LoTW upload prep (file-level)
  - Contest exchange-specific fields

---

## Current-state gaps (code-verified)

1. Mode UX is broader than mode completeness
   - `WorkspaceMode` includes many modes, but only FT8/FT4 have richer operating workflows.
   - `CW` and `FLDIGI` currently show explicit placeholder messaging in GUI.
2. Contest operations are not first-class
   - No explicit contest profile/model (Run/S&P, exchange templates, serial lifecycle).
   - Split control exists in radio HAL, but no guided contest UX around it.
3. Logging is usable but not fully baked for contest + interoperability
   - QSO log + ADIF export exist.
   - Missing robust import/validation pipeline and contest-oriented fields/workflows.

### Upstream implementation references (WSJT-X)

To avoid cargo-culting and keep behavior explainable, align hardening decisions with WSJT-X decode flow checkpoints:

- FT8 decode staging and early/full pass behavior:
  - `lib/ft8_decode.f90`
  - candidate search + decode core: `lib/ft8/sync8.f90`, `lib/ft8/bpdecode174_91.f90`
- FT4 multi-pass/AP logic and candidate handling:
  - `lib/ft4_decode.f90`
  - candidate/sync internals: `lib/ft4/getcandidates4.f90`, `lib/ft4/sync4d.f90`
- JT9 decode entry and soft-symbol/fano path:
  - `lib/jt9_decode.f90`
  - sync core: `lib/jt9sync.f90`
- Global decoder orchestration and per-mode callback accounting:
  - `lib/decoder.f90` (`multimode_decoder`)

Reference clone used for this milestone work:

- SourceForge mirror head: `b4f9a43`
- Remote: `https://git.code.sf.net/p/wsjt/wsjtx`

---

## Milestone checklist (make-it-solid list)

## A) Core mode reliability + test matrix (FT8/FT4/JT9)

- [x] Define explicit reliability SLOs for FT8/FT4/JT9
  - Decode cadence, slot timing tolerance, max missed-slot rate, and TX trigger correctness.
- [x] Build mode matrix document with expected behavior per mode
  - RX only vs RX/TX, timing model, decode path, status messages, and fallbacks.
  - Initial artifact: `docs/core-mode-reliability-matrix.md`
- [ ] Expand test coverage with scenario-focused tests
  - Boundary timing tests, startup mid-slot behavior, TX-slot skip behavior, deferred decode behavior.
- [ ] Add deterministic regression fixtures
  - Golden vectors for each targeted mode, including weak-signal and near-edge timing cases.
- [ ] Add CI matrix targets for core mode tests
  - Keep strict lint + tests, and add mode-focused test grouping to prevent regressions.

Acceptance criteria:
- FT8/FT4/JT9 pass deterministic decode tests and timing tests in CI.
- No known flaky tests in core mode pipeline.
- Mode statuses and guardrails are operator-comprehensible.

## B) Contest + DX workflow layer

- [ ] Introduce `ContestProfile` model (operator-visible)
  - Run/S&P mode, exchange policy, serial scheme, split policy, and safety options.
- [ ] Implement simple split workflow with safe orchestration
  - “Fake split” support where radio support is incomplete, with clear status and guardrails.
- [ ] Add Fox/Hound guidance/state machine UX
  - Guided states, constraints, and contextual actions (instead of hidden assumptions).
- [ ] Add dupe and worked-status engine
  - Real-time call matching against log, per-band/per-mode views.
  - Status: pre-TX duplicate guard is now enforced in GUI TX queue paths (FT8 + native digital) using `call+band+mode` matching when contest dupe-check is enabled.
- [ ] Add serial/CQ number workflow
  - Increment/decrement rules, resend behavior, and persistence across restart.
- [ ] Add macro-friendly Run/S&P helper actions
  - Operator controls should reduce manual branch decisions during pileups.

Acceptance criteria:
- Operator can run an end-to-end contest contact workflow without manual state juggling.
- Split/Fox-Hound behavior is explicit and predictable.
- Dupe/wrked status is visible and correct in pre-TX decision points.

## C) Logging + interoperability hardening

- [ ] Expand ADIF writer to complete required/common contest fields where available.
- [x] Add ADIF import with validation and conflict policy
  - duplicate strategy, normalization rules, and bad-record reporting.
  - Status: `qsonaut-log` now imports ADIF with record normalization, duplicate suppression (`call+date+time+band+mode`), and import summary counters (imported/duplicates/invalid); GUI log panel includes `Import ADIF` action.
- [ ] Add filtered export slices
  - by date range, mode, band, and contest profile.
- [ ] Add LoTW-prep export path
  - file-level prep, metadata checks, and operator instructions.
- [ ] Add contest exchange-specific fields in log model
  - preserve exchange details and serials without free-text loss.

Acceptance criteria:
- Import/export round-trip tests pass for representative ADIF samples.
- Contest QSOs preserve exchange fidelity in both internal log and ADIF export.

## D) Automation hooks (cross-cutting)

- [x] Publish automation events for contest/profile state transitions
  - `contest_state` and `operator_profile` event streams should be emitted with stable fields.
  - Status: event kinds and app events added; GUI now emits hook events on profile/contest/radio-state changes.
- [x] Connect GUI event stream to runtime automation host
  - GUI owns `AutomationHost`, normalizes app events, and dispatches rule components.
  - Status: runtime dispatch active for contest/profile plus structured `callsign_hit` and `qso_logged` events.
- [x] Map approved automation actions to existing safety gates
  - TX/radio-control actions must respect global disarm/armed guardrails.
  - Status: GUI now executes approved `radio_command` and `request_transmit` actions through TX-active/disarmed guard checks; default grants still keep high-risk actions off unless explicitly enabled.
- [x] Publish external adapter receive path into the same event stream (`external_message`)
  - Status: GUI local ingress simulator publishes typed `ExternalMessageReceived` app events (`source`, `author`, `message`, `channel`) into the automation pipeline; live Discord/IRC adapter transport remains follow-up.
- [x] Add sample automation recipes for contest workflow assists
  - Examples: serial nudge notifications, dupe warnings, Run↔S&P context prompts.
  - Status: `automation.example.toml` includes a contest-state activation notification rule.

---

## Delivery order (single milestone PR with internal phases)

1. **Phase 1**: Core mode reliability/test matrix (FT8/FT4/JT9)
2. **Phase 2**: Contest profile + split + Fox/Hound foundations
3. **Phase 3**: Dupe/worked + serial workflow + macros
4. **Phase 4**: ADIF import/export hardening + LoTW prep
5. **Phase 5**: Polish + docs + operator walkthrough

---

## Change control rules for this branch

- Keep PR scope focused on hardening and operator workflow clarity.
- Preserve radio-capability abstractions; avoid vendor lock-in in UX/state models.
- Every feature addition must include tests and operator-visible status language.
- No silent behavior changes for TX-related logic.

---

## Immediate next implementation steps

- [x] Create `ContestProfile` data model + persistence stubs
  - Added in `crates/qsonaut-core/src/config.rs` with defaults, env overrides, and tests.
- [x] Add mode reliability test harness scaffolding for FT8/FT4/JT9
  - Added/extended tests in `crates/qsonaut-gui/src/lib.rs`:
    - `phase1_target_modes_have_slot_and_decoder_support`
    - `jt9_workspace_adapter_decodes_generated_audio`
- [x] Publish initial core reliability matrix + SLO draft
  - `docs/core-mode-reliability-matrix.md`
- [ ] Add backlog labels/checklist into milestone PR description
