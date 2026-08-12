# Third-party notices

QSONaut would not have its current digital-mode capabilities without the work
of upstream open-source radio projects.

## mfsk-core

- Project: [`jl1nie/mfsk-core`](https://github.com/jl1nie/mfsk-core)
- Contributors: [`mfsk-core` contributor history](https://github.com/jl1nie/mfsk-core/graphs/contributors)
- License: [`GPL-3.0-or-later`](https://github.com/jl1nie/mfsk-core/blob/main/LICENSE)
- Release version: `v0.9.1`
- Release source revision: `b70ad5ceecb45ddbdd05ba6f3bd4ef090c010bf7`

QSONaut directly links `mfsk-core` and uses its pure-Rust implementations of
WSJT-family decoding, encoding, synthesis, DSP, synchronization, message
codecs, and error-correction machinery. In QSONaut today, that work underpins
FT8, FT4, FST4, WSPR, JT9, JT65, Q65, and MSK144 integration.

The QSONaut release workflow pins the revision above and packages the upstream
`mfsk-core` GPL license alongside desktop binaries. The complete corresponding
upstream source for that revision is available at:

<https://github.com/jl1nie/mfsk-core/tree/b70ad5ceecb45ddbdd05ba6f3bd4ef090c010bf7>

`mfsk-core` documents that its algorithms are Rust reimplementations of
WSJT-X, the reference implementation by Joe Taylor K1JT and collaborators.
Its upstream README and source tree contain the detailed algorithm-level
attributions and notices.
