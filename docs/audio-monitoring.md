# Audio monitoring

QSONaut opens the selected RX device using the best format the hardware and
operating-system driver will actually start. It tries ranked fallback formats
when a driver advertises a configuration but rejects it at open time. Device
audio is converted at the boundary into QSONaut's canonical 48 kHz mono `f32`
stream using continuous band-limited resampling. Callback size, hardware channel
count, sample representation, and device rate therefore do not change the
waterfall or decoder contracts.

The RX monitor and generated/TX audio use the reverse boundary. QSONaut opens
the selected output at a supported native format, resamples from the canonical
stream, and copies mono audio to every hardware output channel. Monitor queues
remain bounded so a slow output cannot stall decoding. Because input and monitor
devices have independent physical clocks, the monitor starts after a short
prebuffer and continuously adjusts its conversion ratio within a narrow bounded
parts-per-million range. Buffer underruns trigger silence and automatic
rebuffering without affecting the canonical decoder stream or TX timing.

The **Audio output** device is used for transmitted/generated audio. The
optional **RX monitor output** can be different and falls back to Audio output
when no override is selected. On Linux, choose separate hardware for radio TX
and local monitoring when the ALSA device cannot be opened for both purposes.

Changes made in **Settings > Devices** are saved in the active operator profile.
Use **Restart audio now** to apply input or monitor-output changes. The compact
top-bar monitor controls apply enable, output, and volume changes immediately.
In the CW workspace, monitoring follows the same 240 Hz channel centered on the
selected CW tone that feeds the decoder; other workspaces monitor the full mono
RX stream.

## Native operating systems

QSONaut uses CPAL's normal platform backend:

- Windows: WASAPI
- macOS: CoreAudio
- Linux: ALSA

Select the radio's USB audio codec as **Audio input** and speakers or headphones
as **RX monitor output**. Start with monitor volume below `1.0x`; both that
control and the operating-system output volume apply.

If no devices appear, verify that the OS can record from and play to the device
first. On Windows, also allow desktop applications microphone access under
**Privacy & security > Microphone**. On Linux, the build/runtime packages are:

```bash
sudo apt-get install libasound2-dev
```

## WSLg

WSLg exposes the Windows default recording and playback devices through a
PulseAudio server. QSONaut's current Linux backend is ALSA, so the distro also
needs ALSA's PulseAudio plugin:

```bash
sudo apt-get update
sudo apt-get install libasound2-dev libasound2-plugins pulseaudio-utils
```

No `~/.asoundrc` should be necessary. When QSONaut detects WSL and
`PULSE_SERVER`, it automatically prefers the ALSA device named `pulse`. Restart
QSONaut after installing the package, then refresh **Settings > Devices**.

These checks isolate WSLg from QSONaut:

```bash
pactl info
aplay -L | grep '^pulse$'
timeout 3s parec --device=RDPSource --format=s16le --rate=48000 \
  --channels=1 --raw > /tmp/qsonaut-wsl-input.raw
stat -c '%s bytes captured' /tmp/qsonaut-wsl-input.raw
speaker-test -D pulse -c 2 -t sine -l 1
```

`pactl info` should name a server and default source/sink. `aplay -L` should
include `pulse`, the capture file should be non-empty, and the speaker test
should be audible. If PulseAudio is missing or stale, update and restart WSL
from PowerShell, then repeat the checks:

```powershell
wsl --update
wsl --shutdown
```

WSLg normally follows Windows' default microphone and speaker. A USB radio codec
that is not exposed through those endpoints may require selecting it as the
Windows default device or attaching it directly to WSL and using its ALSA device.

## Configuration overrides

The same settings can be supplied through environment variables:

```text
QSONAUT_AUDIO_ENABLED=true
QSONAUT_AUDIO_INPUT_DEVICE=<exact device name>
QSONAUT_AUDIO_OUTPUT_DEVICE=<exact device name>
QSONAUT_AUDIO_MONITOR_ENABLED=true
QSONAUT_AUDIO_MONITOR_OUTPUT_DEVICE=<exact device name>
QSONAUT_AUDIO_MONITOR_VOLUME=0.75
QSONAUT_AUDIO_SAMPLE_RATE_HZ=48000
QSONAUT_AUDIO_CHANNELS=1
```

The processing values are retained for configuration compatibility, but the GUI
runtime normalizes them to the canonical 48 kHz mono format. Multi-channel
hardware is accepted and downmixed before entering that stream.

## Validation boundary

Automated tests cover sample-format conversion, stereo downmixing, callback-
independent band-limited resampling, canonical stream sizing, and output-rate
conversion. Diagnostics record the negotiated input rate, channels, and sample
format; a total open failure lists every attempted configuration. A final live
check still needs real hardware: confirm waterfall activity, stable long-running
monitor buffer/ppm telemetry, audible RX without repeated underruns, correct
monitor volume, and unchanged decoding while monitoring is enabled.
