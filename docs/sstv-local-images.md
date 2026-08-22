# SSTV and local image generation

QSONaut 0.3.7 adds a native Martin M1 SSTV workspace and local image generation
for activity artwork. These are deliberately connected: an operator can receive
an SSTV frame, load an existing image, or generate a new image locally, inspect
the exact 320×256 transmit frame, arm TX, and send it as Martin M1.

## Current SSTV scope

The native modem currently supports **Martin M1 only**:

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

SSTV does not use a movable narrow audio channel. QSONaut decodes the fixed
1100–2300 Hz SSTV tone range inside the radio's approximately 3 kHz USB
passband. In SSTV mode, the audio waterfall shows that fixed range and disables
RX/TX cursor selection. Tune the radio so the received sync tone is at 1200 Hz.
Reception must begin before the roughly 0.9-second VIS header; joining an image
mid-transmission cannot recover its mode or missing lines.

The implementation is software-tested with encode/decode round trips and
streaming acquisition. It is **not yet on-air validated**. Slant correction,
automatic clock calibration, Scottie, Robot, PD modes, and FSK ID are not part
of this release. A parity-valid unsupported VIS header is shown by code and
mode name so an audible SSTV signal does not look like decoder silence.

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

Generated and loaded transmit frames are resized/cropped to Martin M1's 320×256
canvas and saved as PNG files under the platform QSONaut configuration directory
in `sstv-images/`. Local server settings are stored in `local-image.json` there.

## TX safety workflow

Image generation never keys the radio. Transmission requires both controls in
the SSTV workspace:

1. **ARM SSTV TX**
2. **TRANSMIT MARTIN M1**

The global **STOP + DISARM ALL TX** control includes SSTV arming, active audio,
abort state, and PTT release. **STOP SSTV TX** aborts the current audio and drops
PTT. Always verify this behavior with a dummy load or monitor receiver before
on-air use.
