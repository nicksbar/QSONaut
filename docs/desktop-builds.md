# Desktop builds

QSONaut supports native desktop builds for these release targets:

| Operating system | Architecture | Rust target | Distribution |
| --- | --- | --- | --- |
| Linux | x86_64 | `x86_64-unknown-linux-gnu` | `.tar.gz` |
| Linux | ARM64 | `aarch64-unknown-linux-gnu` | `.tar.gz` |
| Windows | x86_64 | `x86_64-pc-windows-msvc` | `.zip` |
| Windows | ARM64 | `aarch64-pc-windows-msvc` | `.zip` |

The release matrix is in `.github/workflows/release-builds.yml`. It runs for
version tags and can also be started manually. ARM builds run on native ARM
runners so audio, USB/serial, and desktop dependencies do not need a fragile
cross-compilation setup.

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

`mfsk-core` remains a sibling source dependency during active modem development.
The workflow checks it out alongside QSONaut. Pin its checkout to a release tag
before publishing production binaries so builds remain reproducible.
