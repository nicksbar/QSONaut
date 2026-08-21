# Third-party notices

QSONaut would not have its current digital-mode capabilities without the work
of upstream open-source radio projects.

## mfsk-core

- Project: [`jl1nie/mfsk-core`](https://github.com/jl1nie/mfsk-core)
- Contributors: [`mfsk-core` contributor history](https://github.com/jl1nie/mfsk-core/graphs/contributors)
- License: [`GPL-3.0-or-later`](https://github.com/jl1nie/mfsk-core/blob/main/LICENSE)
- Upstream channel: `main` branch (active development head)

QSONaut directly links `mfsk-core` and uses its pure-Rust implementations of
WSJT-family decoding, encoding, synthesis, DSP, synchronization, message
codecs, and error-correction machinery. In QSONaut today, that work underpins
FT8, FT4, FST4, WSPR, JT9, JT65, Q65, and MSK144 integration.

The QSONaut release workflow tracks upstream `mfsk-core` `main` and packages
the upstream `mfsk-core` GPL license alongside desktop binaries. The complete
corresponding upstream source is available at:

<https://github.com/jl1nie/mfsk-core/tree/main>

Recent upstream history includes QSONaut's FT8 coarse-sync bounds correction
([mfsk-core PR #279](https://github.com/jl1nie/mfsk-core/pull/279)) and the
maintainer's follow-up restoration of WSJT-X's fixed-window primary sync
channel ([PR #281](https://github.com/jl1nie/mfsk-core/pull/281)). QSONaut no
longer carries a downstream modification of `mfsk-core`.

`mfsk-core` documents that its algorithms are Rust reimplementations of
WSJT-X, the reference implementation by Joe Taylor K1JT and collaborators.
Its upstream README and source tree contain the detailed algorithm-level
attributions and notices.

## cw-dit

- Project: [`nicksbar/cw-dit`](https://github.com/nicksbar/cw-dit)
- Upstream project: [`swilcox/cw-dit`](https://github.com/swilcox/cw-dit)
- Components used: `cwdit-dsp`, `cwdit-morse`
- License: `MIT OR Apache-2.0`

QSONaut directly uses the reusable cw-dit DSP and Morse crates for selected-channel
CW receive decoding. QSONaut's own PCM synthesizer is used for CW transmit audio.
The current integration supports letters A-Z, digits 0-9, and spaces.

cw-dit is Copyright its respective author and contributors and is distributed
under the MIT OR Apache-2.0 license. The full upstream source and license are available at:

<https://github.com/nicksbar/cw-dit>

QSONaut uses only the reusable crates, not the cw-dit CLI, server, or web UI.
