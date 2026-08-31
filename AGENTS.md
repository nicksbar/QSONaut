# QSONaut implementation controls

These rules are part of the application contract and must be preserved by
future agent changes:

- Keep radio, audio, GUI, and server boundaries explicit. Generic behavior
  belongs in the shared layer; vendor/model-specific behavior belongs in the
  driver or profile that owns the capability.
- Prefer logical, responsibility-based module separation. When a source file
  starts becoming difficult to navigate, test, or fit into review context,
  split it into focused modules with a small facade; do not allow a large
  file to become a catch-all simply because the code is related.
- Run `cargo fmt --all`, `cargo test --locked --workspace --all-targets`,
  `cargo clippy --locked --workspace --all-targets -- -D warnings`, and
  `git diff --check` before declaring a change complete.
- Coverage is part of the public documentation contract. After adding or
  changing production code or tests, run:
  `cargo llvm-cov --locked --all-features --workspace --summary-only`.
  Keep the workspace gate in `scripts/check-coverage.sh` passing and update
  the area snapshot in `README.md` from the same report. Do not leave the
  documented baseline or area values stale.
- Preserve the coverage workflow in `.github/workflows/coverage.yml`: it must
  run on pull requests and pushes to `main`, enforce the workspace baseline,
  enforce changed-file coverage on pull requests, publish the job summary, and
  upload the HTML report. Changes to the test or build matrix must keep
  coverage execution representative of the workspace.
- The README coverage table is a current line-coverage snapshot, not a claim
  of hardware validation. Keep area names stable so trends remain comparable;
  add a new area only when its ownership boundary is clear.
- A new production Rust file should ship with a test or an explicit follow-up
  in the coverage plan. Aggregate coverage must not be used to disguise a
  completely untested new path. Existing zero-coverage exceptions are tracked
  only in `.coverage-baseline`; do not add new exceptions casually, and remove
  each entry when its focused tests land.
- Keep release/version documentation synchronized with `Cargo.toml`, the
  current release branch/tag, and `CHANGELOG.md`.
- Treat README status badges as maintained release documentation. When adding,
  renaming, or removing CI, coverage, release, or dependency workflows, verify
  that every README badge points to a real workflow or authoritative status
  source, uses the correct branch/tag, and still describes the checked-in
  gates. Add useful coverage badges only when their source is stable and can be
  kept current; do not add decorative or stale badges.

Coverage areas are grouped as follows: application entry point; GUI core,
modes, panels, workers, and the QSONaut-to-Rigwright integration boundary; and
each non-GUI workspace crate. If code moves between areas, recalculate the
table rather than copying old percentages. Rigwright's driver implementation
itself is an external dependency and must be covered by Rigwright's workflow;
QSONaut must cover the integration contract separately.
