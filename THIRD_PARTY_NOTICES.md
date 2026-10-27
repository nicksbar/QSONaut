# Third-party notices

QSONaut would not have its current digital-mode capabilities without the work
of upstream open-source radio projects.

## Shared modem boundary

- `qsonaut-modems` and `qsonaut-js8`: `nicksbar/qsonaut-modems`, revision
	`a0ef913c4684e8aa4711d13bbbe6775baa436db0` (`0.1.1`). The contracts crate
	is `MIT`; the JS8 implementation is `GPL-3.0-or-later`.
- `qsonaut-third-party` and `qsonaut-n3fjp`: `nicksbar/qsonaut-third-party`,
	revision `3ef1fbdc037d54eab6f71babfa25b5ff335c178b` (`0.3.0`). The modem
	adapter is `GPL-3.0-only`; the independent N3FJP crate is `GPL-3.0-or-later`.

These are immutable Git sources recorded in `Cargo.lock`, not floating branch
heads or local sibling checkouts. The adapter owns the transitive sources below.
QSONaut's original source remains MIT licensed; combined binaries containing
the GPL-3.0-only modem engine must comply with GPL version 3.

## mfsk-core

- Project: [`jl1nie/mfsk-core`](https://github.com/jl1nie/mfsk-core)
- Contributors: [`mfsk-core` contributor history](https://github.com/jl1nie/mfsk-core/graphs/contributors)
- License: [`GPL-3.0-only`](https://github.com/jl1nie/mfsk-core/blob/c21458c90bcb853f20181e9e7cac3c2f9b4bad34/LICENSE)
- Pinned revision: `c21458c90bcb853f20181e9e7cac3c2f9b4bad34`
- Source version: `0.14.0` plus unreleased upstream corrections (522 commits
	after the adapter's previous `0.11.1` source, `ef3fc8eaa4ec3936b76bbb21e8befd91728cb8e8`)

QSONaut links `mfsk-core` through `qsonaut-third-party` and uses its pure-Rust
implementations of WSJT-family decoding, encoding, synthesis, DSP, synchronization, message
codecs, and error-correction machinery. In QSONaut today, that work underpins
FT8, FT4, FST4, WSPR, JT9, JT65, Q65, and MSK144 integration.

The QSONaut release workflow packages the GPL license from the same locked
`mfsk-core` checkout used to build the desktop binary as `mfsk-core-LICENSE`.
The corresponding dependency source is available at:

<https://github.com/jl1nie/mfsk-core/tree/c21458c90bcb853f20181e9e7cac3c2f9b4bad34>

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

- Project: [`swilcox/cw-dit`](https://github.com/swilcox/cw-dit)
- Components used: `cwdit-dsp`, `cwdit-morse`
- License: `MIT OR Apache-2.0`
- Pinned revision: `153fc247ce6e4934c94e0cd2dcbf7887e368ec29`

QSONaut uses the reusable cw-dit DSP and Morse crates through the pinned adapter
for selected-channel CW receive decoding. QSONaut's own PCM synthesizer is used for CW transmit audio.
The current integration supports letters A-Z, digits 0-9, and spaces.

cw-dit is Copyright its respective author and contributors and is distributed
under the MIT OR Apache-2.0 license. The full upstream source and license are available at:

<https://github.com/swilcox/cw-dit/tree/153fc247ce6e4934c94e0cd2dcbf7887e368ec29>

QSONaut uses only the reusable crates, not the cw-dit CLI, server, or web UI.

## komitoto-sstv

- Project: [`IRendy/komitoto`](https://github.com/IRendy/komitoto)
- Component used: `komitoto-sstv`
- License: `MIT`
- Pinned revision: `c98945f7c89f714b3182457a86b15a0c43cb6de6`

QSONaut uses the reusable multi-mode SSTV encoder/decoder as a codec beneath
its own radio-facing adapter. The upstream component supplies Martin, Scottie,
Robot, and PD mode definitions and complete-frame codecs. QSONaut retains its
streaming capture, VIS handling, tuning/AFC, UI, and transmit safety layers.

The full upstream source and license are available at:

<https://github.com/IRendy/komitoto/tree/c98945f7c89f714b3182457a86b15a0c43cb6de6/crates/komitoto-sstv>
