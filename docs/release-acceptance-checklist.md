# Release acceptance checklist

This checklist is the operator-facing acceptance surface for issue
[#16](https://github.com/nicksbar/QSONaut/issues/16). It separates repeatable
software checks from evidence that requires the reference station. A passing
build or GUI smoke test must not be recorded as hardware validation.

Record the commit SHA, QSONaut version, platform, radio profile, audio devices,
date, operator, and `PASS`, `FAIL`, or `BLOCKED` for every applicable item.
Attach sanitized logs and note the observed result for every failure or block.

## Before starting

- [ ] Confirm the exact commit, dependency revisions, and release candidate
      version.
- [ ] Confirm the selected radio, CI-V/CAT settings, audio devices, band,
      frequency, mode, antenna or dummy load, and conservative RF power.
- [ ] Disable unattended automation and confirm the global TX stop/disarm path
      is available.
- [ ] Start with PTT off and record the initial radio state.

## Application and station startup

- [ ] Application starts with the documented command and reaches the main
      station view without an unhandled error.
- [ ] Configuration and operator profile load without losing valid settings;
      malformed persisted data is rejected safely.
- [ ] Radio and audio workers report their actual state, including unavailable
      or disabled devices, without claiming a connection that was not made.
- [ ] Selected radio profile exposes only the controls and meters advertised by
      its capabilities.

## Radio, audio, and waterfall

- [ ] Frequency and mode read back correctly after startup and after a change.
- [ ] Audio input/output selection is visible and the selected input reaches
      the decoder path at the expected sample rate.
- [ ] Waterfall or scope produces frames, follows the active frequency, and
      stops cleanly when disabled.
- [ ] Meter and diagnostic errors are bounded, visible, and do not freeze the
      operator interface.
- [ ] Use the [hardware acceptance matrix](hardware-acceptance.md) for physical
      radio commands, PTT safety, disconnect, reconnect, and recovery evidence.

## Modes and station workflow

- [ ] Each release-claimed receive mode decodes its deterministic fixture or
      documented reference sample.
- [ ] A representative live receive path shows activity and diagnostics
      without creating a false transmit request.
- [ ] TX-capable modes require explicit arming, use conservative settings, and
      release PTT on completion, cancellation, and failure.
- [ ] QSO logging creates the expected record, preserves operator/profile
      fields, and exports a readable ADIF record.
- [ ] Duplicate protection and contest exchange fields behave as documented.

## Recovery, persistence, and safety

- [ ] Global TX stop/disarm clears active, queued, and mode-specific transmit
      state and leaves PTT off.
- [ ] Stopping and restarting workers does not replay stale radio or transmit
      actions.
- [ ] A radio/audio disconnect reports a recoverable error; reconnect restores
      the session without requiring unrelated application state to be reset.
- [ ] Settings changed during the run persist only when expected and reload on
      the next startup.
- [ ] Shutdown leaves PTT off, stops owned workers, and closes optional service
      connections cleanly.

## Release evidence

- [ ] `cargo fmt --all -- --check` passes.
- [ ] Locked workspace tests and clippy pass with warnings treated as errors.
- [ ] Coverage and changed-file coverage pass, with README snapshots updated
      when production code changed.
- [ ] `git diff --check` passes and the changelog/version metadata is current.
- [ ] Release artifacts are built for the declared targets and the release
      notes link this checklist, the hardware report, and known limitations.

## Evidence boundary

Deterministic tests may satisfy software rows, but they do not satisfy rows
requiring a physical radio, USB audio path, waterfall frame, RF load, or
disconnect/reconnect. Store those observations in the hardware acceptance
report with the model, firmware, transport settings, expected result, observed
result, and sanitized diagnostics.