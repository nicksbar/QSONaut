# QSONaut project roadmap

Status: reconciliation baseline  
Date: 2026-09-01

This is the current planning source for the QSONaut family after radio HAL and
modem extraction. The older [core hardening plan](core-hardening-plan.md) is
kept as historical design context; its unchecked items are not automatically
current backlog items.

## Product progression

### v0.4.0 — consolidation and dependable core workflows

The v0.4 release should make the current architecture honest and usable, not
expand every possible radio or modem surface.

- [x] Consume driver-owned Rigwright sessions and profile-gated controls.
- [x] Keep modem ownership behind `qsonaut-modems` and
  `qsonaut-third-party` with pinned revisions.
- [x] Establish deterministic FT8, FT4, and JT9 test paths, plus null-radio
  and null-audio safety coverage.
- [x] Establish the reference-station evidence boundary: Linux/WSL and Icom
  IC-7300, with hardware claims separated from software tests.
- [x] Add contest profile persistence, duplicate protection, serial state,
  exchange fields, and global TX disarm behavior.
- [x] Add ADIF import/export, filtered views, application diagnostics, and
  opt-in server/automation boundaries.
- [x] Reach the current 60% overall QSONaut executable-line coverage baseline;
  preserve the Rigwright integration gate.
- [ ] Raise QSONaut's Rigwright integration coverage from the current 74.33%
  snapshot to the enforced 80% gate.
- [ ] Finish the deterministic regression-fixture catalog and promote the
  remaining high-value cases into CI.
- [ ] Complete a documented IC-7300 acceptance run for the v0.4 release
  candidate, including safe settings restoration and sanitized logs.
- [ ] Reconcile release metadata, dependency revisions, README badges, and
  the GitHub v0.4 milestone before tagging.

Explicit v0.4 non-goals: universal hardware validation, unattended operation,
new modem implementations, broad new radio-family expansion, complete CW
paddle operation, and production Discord/IRC connectors.

### v0.4.x — validation and hardening train

Patch releases are for fixes and evidence, not silent scope expansion.

- Hardware validation reports for additional radios and model profiles.
- Weak-signal, noise, disconnect, cancellation, and failed-TX fixtures.
- Remaining ADIF/LoTW interoperability and contest workflow gaps.
- CI and coverage repairs that preserve the documented gates.
- Consumer integration fixes that do not move application policy into shared
  crates.

### v0.5.0 — broader station and ecosystem capability

Cut this only after v0.4 has a stable reference workflow. Candidate scope:

- More complete RIT/XIT, memory, antenna, repeater, and VFO workflows where
  the HAL and profile evidence support them.
- Expanded validated radio matrix and operator-facing capability diagnostics.
- QSONaut-HostBridge physical-device provider work and remote station
  selection, reservations, and exclusive radio sessions.
- More complete server coordination and automation adapters, with explicit
  privacy and TX authority boundaries.

### v1.0.0 — stability contract

Do not schedule v1 by date or feature count. Tag v1 only when the following
are true:

- Core workflows and their limitations are intentional and documented.
- Supported-radio safety behavior has physical evidence, not only profile or
  unit-test evidence.
- Release, coverage, platform, and dependency checks are consistently green.
- Configuration migration and upgrade behavior are predictable.
- High-risk caveats are narrow enough to publish as an operational contract.

## Cross-repository ownership

| Area | Owning repository | QSONaut responsibility |
|---|---|---|
| Radio HAL, vendor transports, model profiles, probes | `rigwright` | Consume the published API; do not duplicate CAT protocol logic. |
| Consumer-neutral audio/modem contracts | `qsonaut-modems` | Pin and validate the contract revision. |
| GPL/third-party modem adapters | `qsonaut-third-party` | Pin and validate adapters; keep application workflow in QSONaut. |
| Desktop operator workflow | `QSONaut` | Own GUI, scheduling, TX safety, logging, privacy, and platform policy. |
| Remote host device catalog and leases | `QSONaut-HostBridge` | Own host paths, device exclusivity, and client-visible stable metadata. |
| Coordination service and hosted governance | `QSONaut-Server` | Own accounts, enrollment, server policy, persistence, and deployment. |
| Android station application | `QSONoid` | Own Android UI/USB/audio lifecycle while reusing Rigwright/domain contracts. |

### Current coordinated issue set

The QSONaut roadmap retains consumer acceptance and release-parent issues;
implementation-specific follow-up now lives with the owning repository:

| Repository | Issue | Coordinates with |
|---|---|---|
| `rigwright` | [#24](https://github.com/nicksbar/rigwright/issues/24) capability and validation contract | QSONaut #41, #69 |
| `qsonaut-modems` | [#2](https://github.com/nicksbar/qsonaut-modems/issues/2) deterministic modem fixture contract | QSONaut #83 |
| `qsonaut-third-party` | [#5](https://github.com/nicksbar/qsonaut-third-party/issues/5) adapter fixtures and failure contracts | QSONaut #64, #83 |
| `QSONaut-HostBridge` | [#4](https://github.com/nicksbar/QSONaut-HostBridge/issues/4) physical radio provider contract | QSONaut #81 |
| `QSONaut-Server` | [#5](https://github.com/nicksbar/QSONaut-Server/issues/5) hosted collaboration boundary | QSONaut #36 |
| `QSONoid` | [#3](https://github.com/nicksbar/QSONoid/issues/3) Android Rigwright integration validation | QSONaut #69, #81 |

These are child coordination issues, not copies of the QSONaut acceptance
issues. Defects discovered during validation should be filed in the owning
repository and linked back to the relevant QSONaut parent.

## GitHub project reconciliation

The live GitHub project is now `QSONaut Release & Ecosystem Roadmap`.
Its existing 30 QSONaut issues are retained for history, with the newer
v0.4 issues #77–#85 restored to the board. Milestones now distinguish
v0.4.0 Definition, Stabilize, Validate, and Release Candidate work from
v0.4.x hardening, v0.5 ecosystem work, v1.0 stability, and post-v1 research.
The board also has Owning repo, Evidence, and Dependency fields; Phase and
Release assignments were refreshed to match the new progression. The board
URL is <https://github.com/users/nicksbar/projects/1>.

The project uses the existing Status, Priority, Area, Phase, Release, and
Size fields for compatibility with its current views. The recommendations
below describe the desired semantics; where GitHub's current field options
are narrower, the milestone and Evidence fields carry the missing detail.

When GitHub access is available, the project should be rebuilt around these
milestones and labels rather than one historical epic:

| Milestone | Purpose | Candidate issue source |
|---|---|---|
| `v0.4.0 Consolidation` | Release blockers for the dependable reference workflow | Existing QSONaut #83, #84, radio validation #41, and remaining release work |
| `v0.4.x Validation` | Hardware evidence and bounded interoperability fixes | Model-validation issues and fixture follow-ups |
| `v0.5.0 Ecosystem` | HostBridge, broader radio workflows, and service integration | Cross-repo issues after ownership review |
| `v1.0.0 Stability` | Stability contract and release qualification only | New issues created from explicit acceptance failures |

Recommended labels:

- `area:qsonaut`, `area:rigwright`, `area:modems`, `area:third-party`,
  `area:hostbridge`, `area:server`, `area:qsonoid`
- `type:feature`, `type:bug`, `type:validation`, `type:docs`,
  `type:release`, `type:cross-repo`
- `evidence:software-tested`, `evidence:hardware-tested`,
  `evidence:needs-hardware`
- `priority:release-blocker`, `priority:next`, `priority:later`, `status:blocked`

Recommended project fields and views:

| Field | Values or type | Use |
|---|---|---|
| Status | `Backlog`, `Ready`, `In progress`, `Blocked`, `In review`, `Done` | Workflow state; do not encode this in issue titles. |
| Owning repo | Single-select by repository | Makes cross-repo ownership visible without duplicating issues. |
| Release | `v0.4.0`, `v0.4.x`, `v0.5.0`, `v1.0.0`, `Unscheduled` | Release progression and filtering. |
| Evidence | `None`, `Software`, `Hardware`, `Both`, `Needs hardware` | Separates implementation from validation. |
| Priority | `Release blocker`, `Next`, `Later` | Keeps v0.4 work visible without ranking every idea. |
| Dependency | Text or linked issue | Records the external repo/PR/revision that must move with it. |

Create these views:

- **v0.4 release board**: Release = `v0.4.0`, grouped by Status, filtered to
  `Release blocker` and `Next`.
- **Hardware validation queue**: Evidence = `Needs hardware` or `Hardware`,
  grouped by Owning repo and filtered to model/radio issues.
- **Cross-repo dependencies**: type = `cross-repo`, grouped by Owning repo,
  showing Dependency and Release fields.
- **v1 readiness**: Release = `v1.0.0`, grouped by Status; this is a
  qualification list, not a feature wishlist.

Project automation should move newly added items to `Backlog`, move issues to
`In review` when a linked pull request opens, and move them to `Done` only when
the pull request is merged or the issue is explicitly closed. A closed issue
without an implementation or validation link should be reviewed before it is
treated as completed.

Issue handling rules:

1. Close or update completed items with links to the implementing commits and
   tests; do not leave historical checkboxes as active work.
2. Move an issue to the repository that owns the code. Add a short trace note
   in the original issue when GitHub supports a transfer; otherwise create a
   linked target issue and close the duplicate.
3. Keep cross-repository work as a parent issue in the consumer or integration
   owner, with linked implementation issues in each owning repository.
4. Every hardware-validation issue must include model, firmware/manual source,
   transport settings, reversible operations, expected result, observed result,
   and a sanitized probe log.
5. Every release issue must identify the exact dependency revisions, CI gates,
   changelog entry, tag, and rollback/repair path.

## Immediate next queue

1. Raise and stabilize the Rigwright integration gate before cutting v0.4.
2. Finish v0.4 fixture and reference-station acceptance evidence, using the
   shared-repository child issues above for implementation gaps.
3. Audit QSONaut, Rigwright, and shared-crate release/version metadata as one
   dependency graph.
4. Begin v0.4 implementation work only after the release blockers are visible
   and separated from v0.5/v1 ideas.
