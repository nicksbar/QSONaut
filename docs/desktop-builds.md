# Desktop builds

QSONaut supports native desktop builds for these release targets:

| Operating system | Architecture | Rust target | Distribution |
| --- | --- | --- | --- |
| Linux | x86_64 | `x86_64-unknown-linux-gnu` | `.tar.gz` |
| Linux | ARM64 | `aarch64-unknown-linux-gnu` | `.tar.gz` |
| Windows | x86_64 | `x86_64-pc-windows-msvc` | `.zip` |
| Windows | ARM64 | `aarch64-pc-windows-msvc` | `.zip` |

The release matrix is in `.github/workflows/release-builds.yml`. It runs for
version tags and can also be started manually. When a `v*` tag is pushed, the
workflow now also publishes the generated archives to a GitHub Release.
ARM builds run on native ARM runners so audio, USB/serial, and desktop
dependencies do not need a fragile cross-compilation setup.

## Native dependencies

Windows audio and serial access use the operating-system APIs and require no
separate runtime package.

Linux builders need the ALSA, udev, Wayland, and keyboard development packages:

```bash
sudo apt-get install libasound2-dev libudev-dev libwayland-dev libxkbcommon-dev
```

The packaged Linux executable dynamically links to the normal desktop audio and
windowing libraries. A later packaging pass can add AppImage, Flatpak, and a
Debian package after the binary matrix is stable.

## Device selection

Open **Devices** in the application header to choose the audio input, audio
output, and radio USB/serial endpoint reported by the current operating system.
Selections are stored in the OS user configuration directory (`%APPDATA%\QSONaut`
on Windows and `$XDG_CONFIG_HOME/qsonaut` or `~/.config/qsonaut` on Linux).
Audio output changes apply to the next transmission; input and radio changes
apply after restarting QSONaut. The pre-rename `.rigforge_profile.toml` file is
read as a one-way migration fallback.

Startup and runtime diagnostics are written to `logs/qsonaut.log` below that
same platform app directory. For bug reports, especially startup failures,
attach the log file captured from the affected machine.

For graphics backend debugging, QSONaut accepts `QSONAUT_RENDERER=wgpu` or
`QSONAUT_RENDERER=glow`. The chosen renderer is logged during startup and on
the first rendered frame.

UI scale presets use a rebased baseline where the previous physical 75% size
is now represented as 100%.

`mfsk-core` is resolved from its upstream Git dependency during builds; no
local checkout is required. The release archive includes QSONaut's
`THIRD_PARTY_NOTICES.md`, which identifies the upstream GPL-3.0-or-later
license and source location.
