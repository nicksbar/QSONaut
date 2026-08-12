# Versioning and releases

QSONaut uses **Semantic Versioning** and `vMAJOR.MINOR.PATCH` git tags.

## Versioning policy

- `MAJOR`: breaking API/behavior changes.
- `MINOR`: backward-compatible features.
- `PATCH`: backward-compatible fixes.

Until `1.0.0`, breaking changes may still happen; bump at least `MINOR` when they do.

### Practical pre-1.0 progression (`0.x`)

Use this rule-of-thumb while the project is pre-1.0:

- `0.1.z`: bug fixes and small hardening on the same operator workflow.
- `0.2.0`, `0.3.0`, ...: any meaningful behavior shift, workflow redesign,
  protocol-surface expansion, or compatibility-affecting change.
- `0.x.y`: keep for low-risk fixes after a `0.x.0` cut.

Suggested pattern:

1. Cut `0.N.0` when a new capability set lands.
2. Stabilize with `0.N.1`, `0.N.2`, ... as on-air and platform fixes come in.
3. Move to `0.(N+1).0` when behavior or expectations change enough that users
   should treat it as a new train.

### First `1.0.0` decision criteria

Declare `1.0.0` only when all of the following are true:

- Core workflows are intentional and documented (not experimental by default).
- Safety controls are validated on supported radios/platforms.
- Release matrix is consistently green for declared targets.
- Upgrade/config behavior is predictable and migration notes are documented.
- Known high-risk caveats are narrowed to acceptable operational risk.

`1.0.0` is a stability contract, not a feature-count milestone.

## Release checklist

1. Ensure CI is green on `main`.
2. Update `CHANGELOG.md`:
   - Move meaningful entries from `[Unreleased]` into a new section:
     - `## [X.Y.Z] - YYYY-MM-DD`
3. Bump workspace/package versions as needed.
4. Create and push tag `vX.Y.Z`.
5. Verify GitHub release assets and notes were generated from changelog.

## First `v1.0.0` execution checklist

When you are ready for the first stable release:

1. Freeze risky feature work; accept only release blockers.
2. Run full validation on supported hardware/platform targets.
3. Promote `CHANGELOG.md` entries into `## [1.0.0] - YYYY-MM-DD`.
4. Set crate/workspace versions to `1.0.0`.
5. Tag and push `v1.0.0`.
6. Confirm release artifacts publish successfully and smoke-test each package.
7. Open `## [Unreleased]` again for post-1.0 development.

## Why no mandatory PR/issue templates right now

The project intentionally keeps contribution flow lightweight to reduce friction
for operator-led and AI-assisted iteration. Governance is currently enforced via:

- required CI checks,
- changelog-gated releases,
- CODEOWNERS review routing.
