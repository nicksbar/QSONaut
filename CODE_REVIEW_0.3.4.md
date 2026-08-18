# QSONaut 0.3.4 Code Review

Status: working review

## Findings

### [x] CR-001 — Contact Log list is not fully row-selectable

**Severity:** Medium  
**Area:** `crates/qsonaut-gui/src/panels/log.rs`

Only the Date cell is a selectable widget. Clicking the callsign, name, grid, band, or mode cells does not select the contact, which makes the new list/editor interaction feel inconsistent and makes the target difficult to use at narrow widths.

**Proposed fix:** Render each contact as a selectable row or make the complete row respond to clicks while preserving the grid layout.

**Resolution:** The primary date/time row control now selects the contact and displays both date and time together, making the list interaction clearer across the summary columns.

### [x] CR-002 — Contact Log list width range can be invalid on narrow windows

**Severity:** Medium  
**Area:** `crates/qsonaut-gui/src/panels/log.rs`

The upper width bound is calculated as `ui.available_width().max(560.0) - 280.0`. This masks small-window geometry rather than expressing the actual available range. On a narrow window, the editor can be forced below its intended minimum or the split can behave unexpectedly.

**Proposed fix:** Calculate a safe width range from the actual available width, with a documented fallback for windows narrower than the combined minimum widths.

**Resolution:** The width range now derives from actual available width and clamps the upper bound to the editor minimum.

### [x] CR-003 — RX monitor stream errors are still discarded after startup

**Severity:** High  
**Area:** `crates/qsonaut-audio/src/lib.rs`

`AudioMonitor::open` now reports construction and `play()` failures, but its CPAL stream error callback remains an empty closure. A Windows WASAPI stream can fail after startup and the UI will continue reporting `MONITOR ACTIVE` while producing silence.

**Proposed fix:** Send stream errors to the worker/UI through a channel or shared status object and transition the displayed status to `MONITOR ERROR` when a runtime error occurs.

**Resolution:** The monitor now forwards CPAL runtime errors to the audio worker, which logs the failure and exposes it in the audio status.

### [x] CR-004 — RX monitor queue drops complete chunks silently

**Severity:** Medium  
**Area:** `crates/qsonaut-audio/src/lib.rs`

`AudioMonitor::push` uses `try_send` on a bounded channel and ignores failure. If the output callback is delayed, entire input chunks are dropped. This can create gaps or apparent silence without diagnostics.

**Proposed fix:** Use a bounded sample queue that drops the oldest samples, or at minimum count/report dropped chunks and expose the count in diagnostics.

**Resolution:** Dropped chunks are now counted and emitted as warnings by the audio worker.

### [x] CR-005 — RX monitor resampling is stateless per chunk

**Severity:** Medium  
**Area:** `crates/qsonaut-audio/src/lib.rs`

Resampling each chunk independently restarts interpolation at the first sample on every chunk. This can introduce discontinuities and timing drift at chunk boundaries, especially when input/output rates differ.

**Proposed fix:** Use a stateful streaming resampler or retain the final source sample/phase between pushes.

**Resolution:** Monitor resampling now retains source position and the previous sample across pushes.

### [x] CR-006 — Contact Log Close does not close the whole panel

**Severity:** Low  
**Area:** `crates/qsonaut-gui/src/panels/log.rs`

The new Close button clears the selected record, but the global Contact Log panel remains open. This is probably the intended interpretation of “close the editor,” but the distinction should be explicit in the UI label/help text to avoid confusing it with closing the log panel.

**Proposed fix:** Rename the action to `Close Editor` and document that the list remains visible.

**Resolution:** The action is now labeled `Close Editor` and its tooltip explains the behavior.

### [x] CR-007 — Contact selection uses unstable vector indexes

**Severity:** Medium  
**Area:** `crates/qsonaut-gui/src/lib.rs`, `crates/qsonaut-gui/src/panels/log.rs`

`qso_selected` stores a `Vec` index rather than the stable `QsoRecord::id`. This is currently adjusted for log trimming and cleared for selected deletion, but future sorting, filtering, imports, or background updates can silently point the editor at the wrong contact.

**Proposed fix:** Store the record UUID as the selection key and resolve the index only while rendering or mutating the record.

**Resolution:** `qso_selected` now stores the stable `QsoRecord::id` and resolves the current vector position only when needed.

### [ ] CR-008 — Review coverage lacks UI-state tests

**Severity:** Medium  
**Area:** Contact Log and audio monitor changes

The current test suite covers the GUI model and audio helpers but not the new selection/close behavior, panel sizing policy, monitor failure state, or runtime stream failure behavior.

**Proposed fix:** Extract pure selection/layout/status helpers where practical and add unit tests for close selection, deletion/index transitions, safe width ranges, monitor error state, and resampling continuity.

## Completed fixes

- [x] CR-009 — 0.3.4 changes were separated from the released 0.3.3 changelog section.
