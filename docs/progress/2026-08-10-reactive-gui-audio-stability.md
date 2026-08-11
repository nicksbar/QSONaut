# Progress update — 2026-08-10

## What improved
- Replaced the waterfall worker's timer-driven wakeups with a more reactive path driven by real radio data and user actions.
- Stabilized the audio-spectrum worker to avoid state flapping and reduce unnecessary CPU churn.
- Reduced texture upload churn for the waterfall and lowered the GUI repaint cadence to keep the UI smoother.
- Removed dead radio helpers and added regression tests for waterfall and audio-spectrum behavior.

## Current state
- Radio waterfall and control state are now reactive and smoother.
- Audio spectrum stays stable instead of flashing between states.
- The codebase is cleaner and test coverage is stronger for the GUI and radio crates.

## Next milestone
- Move from visualization tuning to actual signal decoding work.
- Start wiring decoded demodulated data paths and validating them against live radio input.
