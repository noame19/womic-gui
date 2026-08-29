# WO Mic GUI

A tiny Slint-based GUI wrapper for the [official WO Mic Linux CLI client](https://wolicheng.com/womic/wo_mic_linux.html).

Spins up `micclient-x86_64.AppImage` in the background, surfaces its stdout/stderr in a log panel, and detects "Connected / Disconnected" lines to drive a status indicator. Settings (micclient path, auto-load `snd-aloop`) persist to `~/.config/womic-gui/config.json`.

## Features

- WiFi / Bluetooth mode switch
- Auto-detects missing `snd-aloop` kernel module (prompts via `pkexec` / `sudo`)
- Live log of micclient output
- Clean SIGINT → SIGKILL disconnect
- ~3 MB static-ish binary, ~15 MB RAM

## Install

1. Download `womic-gui-x86_64.AppImage` from the [latest workflow run](../../actions/workflows/build.yml) (artifact named `womic-gui-x86_64.AppImage`).
2. Download `micclient-x86_64.AppImage` from <https://wolicheng.com/womic/softwares/micclient-x86_64.AppImage> and `chmod +x` it.
3. Run:
   ```bash
   chmod +x womic-gui-x86_64.AppImage
   ./womic-gui-x86_64.AppImage
   ```
4. In the GUI's Settings section, point **micclient path** at your `micclient-x86_64.AppImage`.
5. Click **Connect**.

If `snd-aloop` is not loaded, either:
- Tick **Auto-load snd-aloop** and re-connect (you'll get a `pkexec` password prompt), or
- Run `sudo modprobe snd-aloop` once manually.

Audio will appear on the `Loopback` ALSA device — verify with `arecord -c 1 -r 48000 -f S16_LE -D "hw:CARD=Loopback,DEV=1,SUBDEV=0" test.wav`.

## Build

All builds run on GitHub Actions. Trigger by pushing to `main` or manually via the Actions tab. Artifacts are uploaded as `womic-gui-x86_64.AppImage`.

To rebuild locally you need a Slint-capable Rust toolchain (`cargo install slint` is not a thing — see <https://slint.dev/docs/rust/slint/>). Most users should just download the artifact.

## Architecture

```
ui/app.slint      ─┐
                   ├─ Slint compiles app.slint into Rust (build-time)
src/main.rs       ─┘  ├─ AppWindow struct (generated)
                       ├─ spawns micclient subprocess
                       ├─ reader threads → slint::invoke_from_event_loop
                       └─ ~/.config/womic-gui/config.json (serde_json)
```

The AppImage wraps `usr/bin/womic-gui` + an `AppRun` launcher + a `.desktop` entry + an SVG icon. Slint runtime dependencies (libwayland, libxkbcommon, libfontconfig, libgcc_s, libstdc++, etc.) are bundled by `linuxdeploy`.

## Why no upstream WO Mic GUI?

The WO Mic Linux client is command-line only (no Qt/GTK in its ELF NEEDED list). The official [FAQ](https://wolicheng.com/womic/wo_mic_linux.html) is just the wget-and-run instructions. This project is a 600-line wrapper that adds a panel without changing the underlying micclient at all.

## License

MIT.
