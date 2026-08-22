# SSTV and local image generation

QSONaut 0.3.7 adds an SSTV workspace and local image generation for activity
artwork. An operator can receive a frame, browse for an existing image, or
generate one locally, inspect the transmit preview, select a TX mode, arm TX,
and send it through the shared safety path.

## Current SSTV scope

Live image reception supports 13 Martin, Scottie, Robot, and PD modes. In
**Auto (VIS)**, a valid header selects the matching decoder and its frame
duration automatically. An operator can instead choose a receive mode; that
acts as a filter and visibly ignores a different mode's VIS header.

**Auto Target** is separate from Auto (VIS). It scans for the complete shifted
VIS signature from -900 through +700 Hz, keeping the full 1100–2300 Hz tone
plan inside the normal approximately 0–3000 Hz radio passband. It locks the first
valid signal, and shows `AUTO SCAN` or `AUTO LOCK` on the audio waterfall.
Clicking a signal center immediately switches to a manual target; re-enable
Auto Target to resume scanning. This is deliberately single-channel reception.

Experimental transmit codecs are selectable for 13 modes: Martin M1/M2,
Scottie S1/S2, Robot 36/72, and PD 50/90/120/160/180/240/290. The selector
shows each mode's native dimensions and approximate duration.

Martin M1 live RX uses:

- VIS code 44 (`0x2c`)
- 320×256 RGB, green/blue/red channel order
- 1200 Hz synchronization
- 1500 Hz black through 2300 Hz white
- approximately 114 seconds per image
- mono 12 kHz RX/TX audio

The receiver searches continuously for a valid VIS header, shows capture
progress, and publishes a completed image to the workspace. The transmitter
generates phase-continuous audio and uses QSONaut's existing PTT acknowledgement,
abort, tail, and unconditional PTT-release handling.

SSTV uses a 1200 Hz-wide tone plan inside the radio's approximately 3 kHz USB
passband. The decoder starts at 1100–2300 Hz. Clicking the received signal's
center in the audio waterfall moves the complete decoder window; VIS detection,
pixel tones, and residual AFC move with it. Tune or align so the received sync
tone falls on the displayed sync marker.
Reception must begin before the roughly 0.9-second VIS header; joining an image
mid-transmission cannot recover its mode or missing lines.

The SSTV workspace keeps the newest 80 acquisition events and displays the
newest eight. The local application log also records target offset, VIS/mode,
input level, RF frequency, 25/50/75-percent progress, completion dimensions and
elapsed time, no-header audio diagnostics, and failures. Repeated no-header
audio diagnostics are rate-limited to one every ten seconds.

The implementation is software-tested with encode/decode round trips and
streaming acquisition. It is **not yet on-air validated**. Slant correction,
automatic clock calibration and FSK ID are not part of this release. A
parity-valid VIS header is shown by code and mode name so an audible SSTV signal
does not look like decoder silence.

## Frequency presets

The workspace provides these US ARRL band-plan SSTV centers:

| Band | Frequency |
| --- | ---: |
| 80 m | 3.845 MHz |
| 40 m | 7.171 MHz |
| 20 m | 14.230 MHz |
| 15 m | 21.340 MHz |
| 10 m | 28.680 MHz |

These are voluntary operating centers, not authorization to transmit. Confirm
the band plan for your country, license privileges, image content, occupied
bandwidth, power, and that the channel is clear. QSONaut selects USB and the
wide voice filter for SSTV.

Reference: [ARRL Band Plan](https://www.arrl.org/band-plan).

## Local image servers

Open **SSTV → Local Image Lab**, select a server, enter its loopback URL, and
choose **Find models**.

### Ollama

Default URL: `http://127.0.0.1:11434`

QSONaut lists installed models with `GET /api/tags` and generates with Ollama's
experimental `POST /api/generate` image support. Select an image-capable model,
for example `x/z-image-turbo` or a supported FLUX image model. Ollama exposes
all installed models in its tag response, so selecting a text-only model results
in a clear generation error rather than silent fallback.

References: [Ollama image generation](https://ollama.com/blog/image-generation),
[Ollama API](https://github.com/ollama/ollama/blob/main/docs/api.md).

### Lemonade Server

Default URL: `http://127.0.0.1:13305`

QSONaut lists downloaded models with `GET /v1/models`, shows only entries labeled
`image`, and generates with `POST /v1/images/generations`. Model defaults vary;
the workspace exposes output width, height, and step count.

Reference: [Lemonade OpenAI-compatible API](https://lemonade-server.ai/docs/api/openai/).

## Privacy boundary

Local means loopback, not merely a server that happens to be on the home LAN.
QSONaut enforces all of the following:

- only `http://` URLs are accepted;
- the host must be `localhost` or an IPv4/IPv6 loopback address;
- URLs containing credentials are rejected;
- system HTTP proxies are disabled for image-server requests;
- HTTP redirects are rejected so a loopback server cannot forward a prompt;
- responses larger than 96 MiB are rejected;
- prompts and images are never sent to QSONaut Server.

Generated and loaded source frames are kept as a 320×256 preview, then the
selected codec resizes/crops them to its native transmit dimensions. Preview
PNGs are saved under the platform QSONaut configuration directory in
`sstv-images/`. Local server settings are stored in `local-image.json` there.

## TX safety workflow

Image generation never keys the radio. Transmission requires both controls in
the SSTV workspace:

1. **ARM SSTV TX**
2. **TRANSMIT &lt;SELECTED MODE&gt;**

The global **STOP + DISARM ALL TX** control includes SSTV arming, active audio,
abort state, and PTT release. **STOP SSTV TX** aborts the current audio and drops
PTT. Always verify this behavior with a dummy load or monitor receiver before
on-air use.
