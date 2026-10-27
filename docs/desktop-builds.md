# Desktop builds

QSONaut supports native desktop builds for these release targets:

| Operating system | Architecture | Rust target | Distribution |
| --- | --- | --- | --- |
| Linux | x86_64 | `x86_64-unknown-linux-gnu` | `.tar.gz` |
| Linux | ARM64 | `aarch64-unknown-linux-gnu` | `.tar.gz` |
| Windows | x86_64 | `x86_64-pc-windows-msvc` | `.zip` |
| Windows | ARM64 | `aarch64-pc-windows-msvc` | `.zip` |

The release matrix is in `.github/workflows/release-builds.yml`. It runs for
`v*` version-tag pushes, verifies the SemVer tag against the workspace version,
and publishes the generated archives to a GitHub Release. Windows application
builds use MSVC on native Windows x86_64 and ARM64 runners; Linux builds use
native Ubuntu runners for each architecture.

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

QSONaut uses eframe's `wgpu` renderer. The selected renderer is logged during
startup and on the first rendered frame; `QSONAUT_RENDERER` is no longer used.

UI scale presets use a rebased baseline where the previous physical 75% size
is now represented as 100%.

`mfsk-core` is resolved transitively through the pinned `qsonaut-third-party`
adapter at immutable revision `c21458c90bcb853f20181e9e7cac3c2f9b4bad34`
(`0.14.0` plus unreleased upstream corrections); no local checkout is required.
The release archive includes QSONaut's `THIRD_PARTY_NOTICES.md` and the
`mfsk-core-LICENSE` copied from that locked source. The dependency is
`GPL-3.0-only`, and combined binaries must comply with GPL version 3.

`cw-dit` is likewise resolved from its pinned upstream Git dependency; no
local checkout is required for CW builds.
