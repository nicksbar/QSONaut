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

## DitDah

- Project: [`yuvadm/ditdah`](https://github.com/yuvadm/ditdah)
- Author and contributors: [`DitDah` contributor history](https://github.com/yuvadm/ditdah/graphs/contributors)
- License: [`MIT`](https://github.com/yuvadm/ditdah/blob/main/LICENSE)
- Integrated version: local checkout of `0.2.0` from the `main` branch

QSONaut directly links DitDah for Morse/CW audio decoding and waveform
generation. DitDah provides automatic CW pitch and speed detection over
bounded receive windows and generates keyed audio for QSONaut's CW transmit
path. The current integration supports letters A-Z, digits 0-9, and spaces.

DitDah is Copyright its respective author and contributors and is distributed
under the MIT License. The full upstream source and license are available at:

<https://github.com/yuvadm/ditdah>

DitDah's upstream license notes that its concepts and approaches were inspired
by [`ggerganov/ggmorse`](https://github.com/ggerganov/ggmorse), also licensed
under the MIT License. QSONaut preserves that notice in the packaged DitDah
license.
