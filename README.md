# AutoLab

Programmable control of Android Auto, for testing apps that run in a car. AutoLab starts a head unit session with a
real phone over USB, then lets you press things from a terminal, a YAML test, an HTTP API or a desktop window: take
or release video focus, touch the screen, take screenshots, watch the display live, and record the sound.

It can drive **Google's Desktop Head Unit (DHU)**, or act as the head unit **itself** (the *native* backend), so the
DHU is not needed.

## What it does

| Feature | What it does | How to use it | DHU | Native |
|---|---|---|:-:|:-:|
| **Session** | Starts a head unit session with the phone, shows its state, stops it | `autolab start`, `status`, `stop` | yes | yes |
| **Video focus** | Takes or releases the video focus, like a car switching to its own screen | `autolab focus video on\|off` | accepted by the DHU | **confirmed by the phone** |
| **Screenshots** | Saves a PNG of the projected display | `autolab screenshot out.png` | yes | yes (needs `ffmpeg`) |
| **Live view** | The head unit's display, live, in a window you can click and drag | `autolab-gui` | DHU has its own window | yes (needs `ffmpeg`) |
| **Touch** | Taps and swipes at a pixel of a screenshot (800×480) | `autolab tap X Y`, `autolab swipe X1 Y1 X2 Y2` | no | yes |
| **Audio** | Records what the phone plays (media, navigation prompts, assistant) to a WAV, or plays it | `autolab audio out.wav`; window checkbox | DHU plays its own | yes |
| **Car buttons** | Presses Home, Back, arrows, Enter, media keys | `autolab key home` | no | experimental, no visible effect yet |
| **Tunnels** | Puts back `adb reverse` tunnels that the phone drops when the session starts or mid-session | `autolab start --reverse 10101` | yes | yes |
| **YAML tests** | Runs scripted tests with assertions, JSON and JUnit reports, Ctrl+C-safe cleanup | `autolab test file.yaml` | yes | yes |
| **HTTP API** | Everything above over a local, authenticated API, with a live event stream | `autolab daemon`, [docs/api.md](docs/api.md) | yes | yes |
| **Desktop window** | Session, live view, focus, screenshots, events, audio | `autolab-gui` | partly | yes |
| **Config file** | Defaults for backend, serial, ports, paths and reports | `~/.config/autolab/config.toml` | yes | yes |
| **Doctor** | Read-only check of your setup (DHU, adb, ffmpeg, device) | `autolab doctor` | yes | yes |

**DHU or native?** The DHU takes a command and tells you it took it; it never hears back from the phone. The native
backend is AutoLab's own head unit: it sends the same messages, but it also reads the phone's reply, so
`Phone confirmed: off` means the phone itself stopped projecting. The native backend is newer and less tested.

| | DHU backend | Native backend |
|---|---|---|
| Needs | Google's DHU from the Android SDK | `autolab-native-hu`, `ffmpeg` |
| `observed` (phone-side confirmation) | always `unknown` | the phone's own word |
| Screenshot | written by the DHU | decoded from the phone's video |
| Window | the DHU's own | AutoLab's live view |
| Touch, audio, live video | no | yes |

Details: [docs/native-backend.md](docs/native-backend.md).

## Requirements

- macOS (Linux is untested), Rust 1.87+
- A phone with Android Auto, connected by USB, unlocked, and authorized for adb
- **DHU backend:** the DHU from the Android SDK Manager (`extras;google;auto`). AutoLab never bundles it.
- **Native backend:** `ffmpeg` (for screenshots, the live view and audio playback; `brew install ffmpeg`)

## Install

```bash
make install        # autolab, autolab-native-hu and autolab-gui into ~/.cargo/bin
autolab doctor      # checks the DHU, adb, ffmpeg, the native head unit and your phone
```

`make help` lists every target. Without installing, `make build` puts the binaries in `~/.cargo/target/debug/`.

## Quick start

```bash
autolab start --backend native      # or just `autolab start` for the DHU; add --reverse 10101 for an adb tunnel
autolab-gui                         # the window: live view, buttons, events
autolab focus video off             # the phone stops projecting
autolab focus video on
autolab screenshot out.png          # needs video focus on
autolab tap 400 240                 # native: touch a pixel of a screenshot
autolab swipe 300 200 100 200 --duration-ms 400
autolab audio out.wav --seconds 10  # native: record what the phone plays
autolab test examples/native-phone-confirms.yaml
autolab status --json
autolab stop
```

Every command takes `--json` for machine-readable output.

### Custom native head-unit certificate

The native backend can use your own PEM client certificate and matching private key. A recommended location is:

```text
~/.config/autolab/credentials/head-unit-cert.pem
~/.config/autolab/credentials/head-unit-key.pem
```

Keep the private key outside the repository and restrict its permissions:

```bash
chmod 600 ~/.config/autolab/credentials/head-unit-key.pem
export AUTOLAB_NATIVE_CERT="$HOME/.config/autolab/credentials/head-unit-cert.pem"
export AUTOLAB_NATIVE_KEY="$HOME/.config/autolab/credentials/head-unit-key.pem"
autolab start --backend native
```

Set both variables or neither. If neither is set, AutoLab uses the certificate shipped by the vendored protocol
crate. The variables must be exported before AutoLab starts its daemon; stop and restart an existing daemon so it
inherits them. `autolab status` shows `Certificate: custom` or `Certificate: built-in` for the native backend;
`--json` includes `certificateSource`. Older head units report `unknown` (`null` in JSON).
See [Native backend](docs/native-backend.md) for details.

## Commands

| Command | What it does |
|---|---|
| `autolab start [--backend dhu\|native] [--serial S] [--reverse PORT]… [--dhu-path P] [--config F] [--headless]` | Start a session (starts the daemon if none runs) |
| `autolab stop` | Stop the session |
| `autolab status` | Show the session: backend, phone, focus (requested, head unit, phone), tunnels |
| `autolab focus video on\|off` | Take or release video focus |
| `autolab screenshot PATH` | Save the display as a PNG (absolute or relative; must end in `.png`) |
| `autolab tap X Y` | Touch a pixel (native) |
| `autolab swipe X1 Y1 X2 Y2 [--duration-ms N]` | Drag, 50 to 10000 ms (native) |
| `autolab key NAME [--long]` | Press a car button: `home back up down left right enter search play_pause next previous call end_call` (experimental) |
| `autolab audio OUT.wav [--channel media\|system\|speech] [--seconds N]` | Record phone audio and print how much arrived and how loud (native) |
| `autolab test FILES… [--report json\|junit] [--artifacts-dir D] [--skip-unsupported]` | Run YAML tests against the running session |
| `autolab doctor` | Check the environment without touching a session |
| `autolab daemon [--port N]` | Run the API daemon in the foreground (it stays up) |

Exit codes: 0 success, 1 general error, 2 invalid arguments, 3 backend unavailable, 4 device disconnected,
5 unsupported, 6 timeout, 7 assertion failure, 8 session conflict, 130 interrupted.

## YAML tests

```yaml
name: The phone confirms video focus changes
backend: native                 # tests can pin the backend they were written for
requirements: [videoFocus, screenshots]
steps:
  - action: focus.video
    value: off
  - action: assert.status
    expect: { videoFocus: { requested: off, observed: off } }
  - action: focus.video
    value: on
  - action: screenshot
    path: screenshots/after.png
```

Steps: `focus.video`, `wait`, `waitFor`, `assert.status`, `screenshot`, `tap`, `swipe`, `key`. Reports are written to
`test-results/` as JSON and optionally JUnit. A passing `tap`, `swipe` or `key` step means it was **sent**: the phone
never acknowledges touch. More in [docs/testing.md](docs/testing.md) and `examples/`.

## HTTP API

The daemon serves `/v1` on `127.0.0.1` with a bearer token. The CLI and the window are clients of it, and so can be
anything else.

| Endpoint | What it does |
|---|---|
| `GET /v1/status`, `/capabilities`, `/devices` | State and what the backend supports |
| `POST /v1/session/start`, `/session/stop` | Session lifecycle |
| `POST /v1/focus/video` | Take or release video focus |
| `POST /v1/screenshot` | Save a PNG |
| `POST /v1/input/tap`, `/swipe`, `/key` | Touch and buttons (native) |
| `GET /v1/video` | The live display as raw H.264 (native) |
| `GET /v1/audio/{channel}` | The phone's audio as raw PCM (native) |
| `GET /v1/events` | Server-sent events: session, focus, input, screenshots, tunnels |
| `POST /v1/daemon/shutdown` | Stop the session and exit |

A token in a 0700 directory guards it, and anyone who can read that token can run code as you through the API. See
[docs/api.md](docs/api.md).

## Not implemented

Night mode, driving restrictions, audio and navigation focus control (their endpoints answer `501`), multi-touch,
wireless Android Auto, resolutions above 800×480, and the DHU's `--adb` transport. The native backend's car buttons are
sent and bound by the phone, but had no visible effect on a Pixel 6, so they are marked experimental.

## Known limitations

- **AutoLab can only control a session that it started.** A DHU launched from another terminal takes commands only on
  its own stdin.
- Starting a session re-enumerates the phone over USB, which drops `adb reverse` tunnels. `--reverse PORT` restores the
  ones you ask for, and AutoLab re-applies them if the phone drops them later.
- Unplugging the phone ends the session (observed once). Plug it back in and run `autolab start` again.
- USB only, and one head unit can hold a phone at a time: stop any DHU before using the native backend, and the other
  way round. With several phones attached, the native backend refuses to guess.
- The native backend is an unofficial implementation of an undocumented protocol. An Android Auto update on the phone
  can break it; run `make hw-native` after updating.
- Tested with DHU 2.0 (build 2022-03-30) and Android Auto 18.0.164204 on a Pixel 6.

## Tests

```bash
make                  # lint, vendor check, build, all tests, parse every example (no hardware)
make hw-dhu           # real phone + Google DHU
make hw-native        # real phone + AutoLab's own head unit
```

The hardware targets refuse to run if a DHU or daemon is already running, so they never touch someone else's session.

## Repository

| Crate | Job |
|---|---|
| `autolab-core` | The backend trait, shared types, exit codes |
| `autolab-dhu` | The DHU backend |
| `autolab-native` | The native backend (drives `autolab-native-hu`) |
| `autolab-native-hu` | AutoLab's own head unit, run as a child process |
| `autolab-daemon` | The HTTP API, session state, events |
| `autolab-test` | The YAML runner and reports |
| `autolab-cli` | The `autolab` command |
| `autolab-gui` | The `autolab-gui` window |
| `vendor/` | Patched copies of the `android-auto` crate (LGPL) and `rustls-webpki`, used by the native head unit only |

## Docs

| | |
|---|---|
| [docs/native-backend.md](docs/native-backend.md) | The native head unit: setup, touch, live view, audio, limits |
| [docs/gui.md](docs/gui.md) | The desktop window |
| [docs/api.md](docs/api.md) | The HTTP API |
| [docs/testing.md](docs/testing.md) | YAML tests and reports |
| [docs/configuration.md](docs/configuration.md) | The config file |
| [docs/architecture.md](docs/architecture.md) | How the parts fit together |
| [docs/feasibility.md](docs/feasibility.md) | What was verified on a real phone, with evidence |

## Licence

Apache-2.0. `vendor/android-auto` is LGPL-3.0-or-later and `vendor/rustls-webpki` is ISC; see the `AUTOLAB-PATCHES.md`
in each. AutoLab is a test tool, not a certified car head unit.
