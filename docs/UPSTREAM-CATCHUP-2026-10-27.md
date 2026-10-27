# Downstream modem maintenance evidence — 2026-10-27

## Status and scope

**Resolved: all local software gates pass.** The null-audio compatibility root
cause is fixed: the shared synthetic fallback
pool now uses standard-message-compatible `0Z` demo identifiers. The original
failure filter passes (4 passed), and two new identity regressions pass.
Full workspace and coverage validation is recorded below. No assertions were
weakened and no tests were suppressed.

This update is based on QSONaut `main` at
`0b264a46ade7050fda36ea4c5c9fc3a07f8a789d`, on maintenance branch
`maintenance/upstream-catchup-2026-10-27`. The desktop version is prepared as
`0.4.3` (unreleased; no tag or binary release yet).
Changes include dependency pins, the lockfile, notices/documentation,
dependency-license packaging, and the synthetic fallback pool in production
Rust plus focused tests. Consumer TX, arm/disarm, audio-processing, clock, and
protocol-adoption policies, coverage exclusions, and gates are unchanged.

## Immutable sources

| Packages | Revision | Version / license |
| --- | --- | --- |
| `qsonaut-modems` | `a0ef913c4684e8aa4711d13bbbe6775baa436db0` | `0.1.1`, MIT |
| `qsonaut-js8` | `a0ef913c4684e8aa4711d13bbbe6775baa436db0` | `0.1.1`, GPL-3.0-or-later |
| `qsonaut-third-party` | `3ef1fbdc037d54eab6f71babfa25b5ff335c178b` | `0.3.0`, GPL-3.0-only |
| `qsonaut-n3fjp` | `3ef1fbdc037d54eab6f71babfa25b5ff335c178b` | `0.3.0`, GPL-3.0-or-later |
| transitive `mfsk-core` | `c21458c90bcb853f20181e9e7cac3c2f9b4bad34` | `0.14.0` plus unreleased upstream corrections, GPL-3.0-only |

The initial maintenance lock update changed only the versions and Git sources
of these five packages. The subsequent `0.4.3` metadata-only release preparation
changed only the 11 workspace package versions from `0.4.2` to `0.4.3`, with no
registry version or dependency-list changes at that step. The later Dependabot
follow-up below updates Rustls and shared Clap/thiserror patches separately.
QSONaut's old lock
used `mfsk-core` `0.11.0` at `5cba86d5924d1b8736914b5511350eaf1bff3fbc`;
the adapter's separate 522-commit catch-up comparison starts from its previous
`0.11.1` source, `ef3fc8eaa4ec3936b76bbb21e8befd91728cb8e8`.

Unchanged transitive sources are `cwdit-dsp` / `cwdit-morse` at
`153fc247ce6e4934c94e0cd2dcbf7887e368ec29` from `swilcox/cw-dit`, and
`komitoto-sstv` at `c98945f7c89f714b3182457a86b15a0c43cb6de6` from
`IRendy/komitoto`. HostBridge protocol remains locked to
`493673e2387bed69476ee8dd1bee8e05ca5ba335` (`v0.1.5`).

The ignored project `.cargo/config.toml` is absent. Read-only inspection found
no ancestor/Cargo-home source replacements or path patches, and no Cargo
environment overrides. Full locked metadata confirms all nine resolved Git
packages use Cargo's Git-cache checkouts, not sibling repositories. Workspace
crates intentionally remain local path dependencies. No temporary overrides
are used or committed.

## Validation

Logs are under `/tmp/qsonaut-downstream-catchup-2026-10-27/`, not the user's
untracked `coverage-summary.txt` or `summary` reports.

| Gate | Result |
| --- | --- |
| `cargo fmt --all -- --check` | Pass |
| `cargo test --locked --workspace --all-targets` | Pass: 481 passed, 0 failed, 2 ignored |
| New `demo_pool_` tests | Pass: 2 passed, 0 failed |
| Original `null_audio_generator_` failure filter | Pass: 4 passed, 0 failed |
| `cargo test --release --locked -p qsonaut-gui --lib -- --ignored --nocapture` | Pass: both slow generated-fixture tests, 2 passed, 0 failed |
| `cargo clippy --locked --workspace --all-targets -- -D warnings` | Pass |
| `cargo metadata --format-version 1 --locked --no-deps` | Pass, 11 workspace packages |
| Full locked metadata / Git source audit | Pass |
| `cargo llvm-cov --locked --all-features --workspace --summary-only` | Pass: 23,544 / 44,570 lines, 52.82% unfiltered |
| Same complete coverage run with workflow ignore regex | Pass: 23,109 / 37,777 lines, 61.17% |
| `scripts/check-coverage.sh` workflow report `60 80` | Pass: workspace 61.17%; Rigwright integration 3,365 / 4,099, 82.09% |
| `scripts/check-cw-coverage.sh` workflow report `80` | Pass: 440 / 539 lines, 81.63% (script displays integer 81%) |
| `git diff --check` | Pass |
| Workflow YAML parsing | Pass |
| Locked `mfsk-core` license-copy probe | Pass, byte-for-byte GPL v3 license copy |

The enabled generated-audio FT8/FT4/JT9 and JS8 waterfall fixture tests pass,
including the existing level/timing/noise variations and steady-carrier
rejection test. Both ignored slow native round-trips also pass when explicitly
run in release mode: `tx_audio::tests::native_tx_waveforms_decode_back_through_the_adapter`
and `workers::decode::tests::native_generated_signals_decode_through_the_adapter`.
See `resolved-slow-tests.log`. These two passes are separate from the
workspace suite's 481 passes and are not double-counted there. The pre-fix
workspace result was 477 passed / 2 failed / 2 ignored; fixing those failures
and adding two new regressions gives the current 481 passes.

Production Rust and tests changed, so coverage was mandatory and was measured,
not carried forward. The README badges and every area were checked against
`resolved-coverage-workflow.txt`, grouped using the existing script. JS8 is
598 / 809 lines (73.92%). The workflow regex, `.coverage-baseline`, workspace
60%, Rigwright 80%, CW 80%, and changed-file gate rules are unchanged. The two
coverage commands are separate complete runs; the documented area table uses
only the workflow-filtered run, not mixed per-file maxima. No changed-file PR
base comparison is claimed locally.

Coverage stdout is saved as `resolved-coverage-unfiltered.txt` and
`resolved-coverage-workflow.txt`; stderr/build/test output is in their matching
`.log` files. `resolved-coverage-gates.log` records both gate scripts.
`resolved-workspace-tests.log`, `resolved-clippy.log`, `resolved-metadata.json`,
and `resolved-gate-status.log` record the remaining gates. Cargo build jobs
were limited to 2 unless explicitly configured by the user; both first coverage
attempts passed without compiler memory/disk failure or profile mitigation.

No local software blocker remains. PowerShell is unavailable locally: the
Windows packaging path is statically reviewed, not executed. The desktop-build
documentation now matches the workflow's tag-only trigger and native Windows
MSVC runners. No desktop release archive, cross-platform build, hardware or
on-air validation is claimed. The validated adapter update was published as
`3ef1fbdc037d54eab6f71babfa25b5ff335c178b`, and the desktop integration as
`09f70e20dc28f8dfa174777d21672bb2d51647ac`. Both maintenance branches were
fast-forwarded and pushed to their respective `main` branches. No release
tags or binary releases were created.

### Release version synchronization

The subsequent metadata-only preparation aligns the workspace packages,
lockfile, changelog, and current release documentation at `0.4.3`, with a fresh
empty `[Unreleased]` section. It preserves the publication history above and
does not change production Rust, tests, dependency pins, or coverage policy.
The 481 workspace passes, 2 separate slow-test passes, and local coverage
figures above are the measured maintenance evidence; coverage is not rerun for
this metadata-only bump. GitHub coverage checks are not claimed green, and no
new hardware or on-air evidence is claimed. Release-preparation gate logs are
kept separately under `/tmp/qsonaut-release-0.4.3/`. The maintenance branch is
prepared for review only, not merged or tagged by this release preparation.

### Dependabot follow-up — PR #105

On the existing QSONaut PR #106 maintenance branch, QSONaut PR #105's body and
complete diff were compared against the active lock before running
`cargo update -p rustls --precise 0.23.46`. Cargo reproduced both proposed hunks.
The cross-repository assessment also identified shared updates proposed by
`nicksbar/QSONaut-HostBridge` PR #12 (Clap) and PR #14 (thiserror); these are
HostBridge PR numbers, not QSONaut PR numbers. Their bodies were reviewed and
the same exact versions applied with only
`cargo update -p clap@4.6.6 --precise 4.6.7` and
`cargo update -p thiserror@2.0.20 --precise 2.0.21`. Each command also updated
its companion packages to the intended versions; no global update was used.

The complete dependency change set relative to this branch's committed lock is:

| Package | Before | After |
| --- | --- | --- |
| `rustls` | `0.23.44` | `0.23.46` |
| `clap` / `clap_builder` | `4.6.6` | `4.6.7` |
| `clap_derive` | `4.6.4` | `4.6.7` |
| `thiserror` / `thiserror-impl` (2.x) | `2.0.20` | `2.0.21` |
| `tempfile` | `3.27.0`, edge to `getrandom 0.4.3` | `3.27.0`, edge to `getrandom 0.3.4` |

Only these six package versions/checksums, dependent references to
`thiserror 2.0.21`, and the pre-existing `tempfile` edge change differ.
Unrelated Windows dependency-edge rewrites produced during targeted resolution
were restored to the starting lock; full locked metadata accepts the minimal
result. Required transitive `thiserror` and `thiserror-impl` `1.0.69` are
unchanged. `serialport` `4.10.1` and `uuid` `1.26.1` were already current and
were not updated. `tempfile` permits
`getrandom >=0.3.0, <0.5`; both getrandom versions were already locked and
remain present, with their package records unchanged. No companion version
updates beyond the listed Clap/thiserror packages were needed. All nine Git
package names/versions/sources/revisions and all 11 workspace versions (`0.4.3`)
remain unchanged; Git packages' thiserror dependency references follow the
updated registry version. The manifest still uses
Rustls `0.23`, `default-features = false`, and `ring`/`std`; both clients retain
explicit Ring provider installation. Host and all-target inverse trees resolve
one Rustls version, `0.23.46`, through the server/HostBridge clients and
Reqwest/Tokio-Tungstenite. Resolved Rustls features remain `ring`, `std`, and
transitive `tls12`.

Shared HostBridge patch assessment:

- `nicksbar/QSONaut-HostBridge` PR #12: Clap `4.6.7` adds the derive attribute
  `#[command(defer = <bool>)]` for opt-in lazy subcommand initialization.
  QSONaut does not opt in, so no CLI behavior change is requested or assumed.
- `nicksbar/QSONaut-HostBridge` PR #14: thiserror `2.0.21` fixes parsing of
  generic unit variants in display expressions. Its minimum Rust version rises
  from `1.71` to `1.77`; the validated local compiler is `rustc 1.93.1` and
  satisfies that requirement. No production Rust or test edits are needed.

Upstream patch assessment, based on the
[0.23.45 release](https://github.com/rustls/rustls/releases/tag/v/0.23.45),
[0.23.46 release](https://github.com/rustls/rustls/releases/tag/v/0.23.46),
and [exact comparison](https://github.com/rustls/rustls/compare/v/0.23.44...v/0.23.46):

- `0.23.45` fixes TLS 1.3 handshake encryption-level boundary acceptance
  ([GHSA-2mjx-qc3c-rqvc](https://github.com/rustls/rustls/security/advisories/GHSA-2mjx-qc3c-rqvc)),
  affecting `0.23.13`–`0.23.44`. Upstream says transcript authentication still
  prevents a network attacker from altering or completing the handshake.
- `0.23.46` includes session-secret Debug redaction
  ([GHSA-c8m5-c952-6cm4](https://github.com/rustls/rustls/security/advisories/GHSA-c8m5-c952-6cm4)),
  authenticated KX-hint storage
  ([GHSA-w44r-93j7-qv28](https://github.com/rustls/rustls/security/advisories/GHSA-w44r-93j7-qv28)),
  Brotli allocation limits
  ([GHSA-h2v8-394m-p7vv](https://github.com/rustls/rustls/security/advisories/GHSA-h2v8-394m-p7vv)),
  and rejection before sending buffered ECH application data
  ([GHSA-vp88-798v-h67f](https://github.com/rustls/rustls/security/advisories/GHSA-vp88-798v-h67f)).
  The ECH advisory's patched-version field currently says "None"; the release
  notes and comparison identify the included fix. No CVE identifier is claimed.
- Exposure is configuration-dependent: Brotli is not enabled in the resolved
  Rustls features; QSONaut does not configure ECH or custom session stores that
  Debug-log session secrets. Ring does not provide the hybrid key exchange
  involved in the KX advisory's post-quantum downgrade scenario. This is not a
  claim that every upstream advisory was exploitable in QSONaut.
- Additional patches cover timely TLS 1.3 key updates, TLS 1.2 record limits,
  HPKE sequence handling, stream backpressure, short writes, and error/flush
  behavior. Existing consumer APIs compile unchanged; no production Rust or
  test edits are needed.

Fresh local validation of the complete shared dependency set: formatting,
481 workspace/all-target tests (0 failed, 2 ignored), strict
workspace/all-target Clippy, full locked metadata (11 workspace packages and
nine Git sources), exact lockfile-diff audit, and `git diff --check` all pass.
The earlier Rustls-only dependency trees remain separate evidence.
The server-client suite's existing
`installs_a_process_level_tls_crypto_provider` test passes. Coverage and the
two ignored slow tests are not rerun for this lockfile/docs-only follow-up;
the earlier measured coverage and separate slow-test evidence remain historical.
No live TLS endpoint, hardware, cross-platform build, or remote CI pass is claimed.

Evidence is separate under `/tmp/qsonaut-dependabot-catchup-2026-10-27/`:
`pr-105-view.json`, `pr-105-body.txt`, `pr-105.diff`, `before-Cargo.lock`,
`cargo-update-rustls.log`, `lock.diff`, `lock-audit.log`, `fmt.log`,
`workspace-tests.log`, `clippy.log`, `metadata.json`, `metadata.stderr.log`,
`rustls-tree.log`, `rustls-features.log`, `rustls-tree-all-targets.log`,
`gate-status.log`, `diff-check.log`, and `validation-audit.log`.
The complete shared update adds `shared-cargo-update.log`,
`shared-lock-audit.log`, `shared-lock.diff`, `shared-fmt.log`,
`shared-workspace-tests.log`, `shared-test-counts.log`, `shared-clippy.log`,
`shared-metadata.json`, `shared-metadata.log`, `shared-metadata-audit.log`,
`shared-gate-status.log`, `shared-diff-check.log`, and `shared-final-audit.log`;
repository-qualified PR bodies are saved as `shared-QSONaut-pr-105.json`,
`shared-QSONaut-pr-106.json`, `shared-QSONaut-HostBridge-pr-12.json`, and
`shared-QSONaut-HostBridge-pr-14.json`.
The user's untracked `coverage-summary.txt` and `summary` are preserved and
unstaged. This follow-up is carried by existing QSONaut PR #106; no main merge,
tag, or release publication. The local finishing step does not commit, push,
merge, or close any PR.

## Reproduced simulator regression and root-cause fix

Before the fallback fix, new pins failed these unchanged GUI tests:

- `workers::audio::tests::null_audio_generator_builds_waveforms_for_supported_demo_modes`:
  `null source did not encode Ft8` at `workers/audio.rs:2178`.
- `workers::audio::tests::null_audio_generator_uses_mode_slots_and_cycles_exchange_frames`:
  zero FT8 waveforms instead of six at `workers/audio.rs:2235`.

The same `null_audio_generator_` filter against an extracted, unchanged `main`
tree using its old published locked dependencies passes: **4 passed, 0 failed**.
See `workspace-tests.log`, `workspace-tests-no-fail-fast.log`, and
`main-baseline-tests.log`.

The new `mfsk-core` standard-message packer explicitly rejects Q-prefixed
callsigns (`msg/wsjt77.rs`, `type12_standard_call` and `callok`). QSONaut's
shared default demo pool deliberately uses fictional `QZ0NA` through `QZ5NF`;
all six FT8 simulation messages fail synthesis. The generator discards packing
errors, leaving no waveforms. The tests expose that runtime compatibility gap.

### Identity decision and exact evidence

Replace only the shared synthetic fallback identities and missing-call defaults
with `0Z0NA`, `0Z1NB`, `0Z2NC`, `0Z3ND`, `0Z4NE`, and `0Z5NF`. They are demo
identifiers, not real operator identities or authority to transmit. Grids,
names, power, contributor/tester variable parsing, enabled/mode filtering, and
the priority of supplied identities are unchanged. Invalid supplied calls
remain subject to codec rejection, never silently replaced or bypassed.

Read-only inspection of upstream `mfsk-core/mfsk-core/src/msg/wsjt77.rs`
confirmed `callok` rejects leading `Q`, requires the last digit at position
2 or 3, at least one prefix letter, and 1–3 suffix letters. `pack28` accepts
the corresponding standard shape. The `0Z` prefix, digit, and two-letter suffix
meet both rules; four-letter `DEMO` suffixes do not.

Before adoption, a standalone probe linked QSONaut's actual locked `c21458c`
Cargo-cache library, not the sibling checkout. `pack28` tokens were:

| Identifier | 28-bit token |
| --- | ---: |
| `0Z0NA` | 20243059 |
| `0Z1NB` | 20262769 |
| `0Z2NC` | 20282479 |
| `0Z3ND` | 20302189 |
| `0Z4NE` | 20321899 |
| `0Z5NF` | 20341609 |

For each station and its cyclic neighbor, all six CQ/report/R-report/RR73/73/CQ
exchange forms passed `pack77`, asserted Type 1 bits, and returned byte-for-byte
original text through `unpack77`: **36 exact standard-message round-trips**.
Controls rejected `pack77("CQ", "QZ0NA", "CN87")` and `pack28("0Z0DEMO")`.
See `identity-probe.log`. This proves standard encoding without hashing,
nonstandard fallback, truncation, or a codec exception.

Fresh pre-edit `identity-before-tests.log` reproduced **2 passed / 2 failed**
under `null_audio_generator_`. After the fix, `resolved-null-tests.log` reports
**4 passed / 0 failed**, retaining the existing assertion of six FT8 exchange
frames. `resolved-demo-tests.log` reports **2 passed / 0 failed**: encoder-only
checks cover all six identities and all six exchange messages plus invalid-call
controls, and one full FT8 waveform decode asserts the exact directed message
`0Z0NA 0Z1NB -10` with both identities intact. Six expensive full decode loops
are unnecessary. Independent hardcoded JS8 `QZ` fixtures are unchanged.

No additional protocols/message types or radio/TX policies were adopted. There
are no consumer `sync_scale` / `WsjtSyncScale` references or exhaustive matches
needing a `SyncFraction` arm.