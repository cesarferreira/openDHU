# openDHU

Previously AutoLab. The command remains `aa`; helper binaries are `opendhu-native-hu` and
`opendhu-gui`. Environment variables now use the `OPENDHU_` prefix, and project suite aliases
live in `opendhu.yaml`. The default config and credentials live under `~/.config/opendhu/`;
existing `~/.config/autolab/config.toml` and `~/.config/autolab/credentials/` are read as fallbacks
when their new counterparts are absent.

Programmable control of Android Auto, for testing apps that run in a car. openDHU starts a head unit session with a
real phone over USB, then lets you press things from a terminal, a YAML test, an HTTP API or a desktop window: take
or release video focus, touch the screen, take screenshots, watch the display live, and record the sound.

It can drive **Google's Desktop Head Unit (DHU)**, or act as the head unit **itself** (the *native* backend), so the
DHU is not needed.

## What it does

| Feature | What it does | How to use it | DHU | Native |
|---|---|---|:-:|:-:|
| **Session** | Starts a head unit session with the phone, shows its state, stops it | `aa start`, `status`, `stop` | yes | yes |
| **Video focus** | Takes or releases the video focus, like a car switching to its own screen | `aa focus video on\|off` | accepted by the DHU | **confirmed by the phone** |
| **Screenshots** | Saves a PNG of the projected display | `aa screenshot out.png` | yes | yes (needs `ffmpeg`) |
| **Live view** | The head unit's display, live, in a window you can click and drag | `aa gui` | DHU has its own window | yes (needs `ffmpeg`) |
| **Touch** | Taps and swipes at a pixel of a screenshot (800×480) | `aa tap X Y`, `aa swipe X1 Y1 X2 Y2` | no | yes |
| **Audio** | Records what the phone plays (media, navigation prompts, assistant) to a WAV, or plays it | `aa audio out.wav`; window checkbox | DHU plays its own | yes |
| **Car buttons** | Presses Home, Back, arrows, Enter, media keys | `aa key home` | no | experimental, no visible effect yet |
| **Tunnels** | Puts back `adb reverse` tunnels that the phone drops when the session starts or mid-session | `aa start --reverse 10101` | yes | yes |
| **YAML tests** | Runs scripted tests with assertions, HTML, JSON and JUnit reports, Ctrl+C-safe cleanup | `aa test file.yaml` | yes | yes |
| **HTTP API** | Everything above over a local, authenticated API, with a live event stream | `aa daemon`, [docs/api.md](docs/api.md) | yes | yes |
| **Desktop window** | Session, live view, focus, screenshots, events, audio | `aa gui` | partly | yes |
| **Config file** | Defaults for backend, serial, ports, paths and reports | `~/.config/opendhu/config.toml` | yes | yes |
| **Doctor** | Read-only check of your setup (DHU, adb, ffmpeg, device) | `aa doctor` | yes | yes |

**DHU or native?** The DHU takes a command and tells you it took it; it never hears back from the phone. The native
backend is openDHU's own head unit: it sends the same messages, but it also reads the phone's reply, so
`Phone confirmed: off` means the phone itself stopped projecting. The native backend is newer and less tested.

| | DHU backend | Native backend |
|---|---|---|
| Needs | Google's DHU from the Android SDK | `opendhu-native-hu`, `ffmpeg` |
| `observed` (phone-side confirmation) | always `unknown` | the phone's own word |
| Screenshot | written by the DHU | decoded from the phone's video |
| Window | the DHU's own | openDHU's live view |
| Touch, audio, live video | no | yes |

`aa start` uses native by default. Use `aa start --dhu` for Google's DHU. The existing
`--backend native` and `--backend dhu` options remain supported; use either `--backend` or `--dhu`.
A configured `session.backend` overrides the built-in default, and command-line choices override it.

Details: [docs/native-backend.md](docs/native-backend.md).

## Requirements

- macOS (Linux is untested), Rust 1.87+
- A phone with Android Auto, connected by USB, unlocked, and authorized for adb
- **DHU backend:** the DHU from the Android SDK Manager (`extras;google;auto`). openDHU never bundles it.
- **Native backend:** `ffmpeg` (for screenshots, the live view and audio playback; `brew install ffmpeg`)

## Install

```bash
make install        # aa, opendhu-native-hu and opendhu-gui into ~/.cargo/bin
aa doctor      # checks the DHU, adb, ffmpeg, the native head unit and your phone
```

`make help` lists every target. Without installing, `make build` puts the binaries in `~/.cargo/target/debug/`.

## Quick start

```bash
aa start                       # native head unit by default; add --reverse 10101 for an adb tunnel
# Or: aa start --dhu           # Google's Desktop Head Unit
aa gui                             # the window: live view, buttons, events
aa focus video off             # the phone stops projecting
aa focus video on
aa screenshot out.png          # needs video focus on
aa tap 400 240                 # native: touch a pixel of a screenshot
aa swipe 300 200 100 200 --duration-ms 400
aa audio out.wav --seconds 10  # native: record what the phone plays
aa test examples/native-phone-confirms.yaml  # head-unit window and terminal spinners by default
aa test examples/native-phone-confirms.yaml --headless --no-interactive
aa test --validate e2e/suite.yaml  # validate a headless emulator suite before device setup
aa status --json
aa stop
```

Every command takes `--json` for machine-readable output.

### Try the included Android app

The [Android sample](samples/android/README.md) includes a companion phone screen, an Android Auto
places browser, and three E2E journeys covering navigation, favorite persistence and reset. It runs
offline without API keys or backend services.

```bash
make sample-build     # Build the sample APK (Android SDK + JDK 17+)
make sample-check     # Android unit tests, lint and E2E YAML validation
make sample-test      # Build and run on the provisioned medium_phone AVD
aa test sample        # Rerun without rebuilding; head unit and spinners enabled by default
aa test sample --usb  # Use the single authorized USB phone instead of the configured AVD
```

See the sample guide for one-time Android Auto provisioning on an emulator or USB phone. Add `--headless --no-interactive`
to `aa test sample` to run without the head-unit window or spinners. Reports go to
`samples/android/build/test-results/run-*/`; open `results.html` for results, steps, logs and screenshots.

### Custom native head-unit certificate

The native backend automatically loads your PEM client certificate and matching private key from:

```text
~/.config/opendhu/credentials/head-unit-cert.pem
~/.config/opendhu/credentials/head-unit-key.pem
```

Keep the private key outside the repository and restrict its permissions:

```bash
chmod 600 ~/.config/opendhu/credentials/head-unit-key.pem
aa start
```

With custom credentials, openDHU also advertises the developer DHU identity so debug Android Auto apps can
appear. Set `OPENDHU_NATIVE_IDENTITY=opendhu` to opt out.

If neither file exists, openDHU uses its bundled certificate. If only one exists or either is unreadable or
invalid, startup fails. To override the standard paths, set both `OPENDHU_NATIVE_CERT` and `OPENDHU_NATIVE_KEY`
before starting the daemon. Restart the session after changing files; restart the daemon after changing its
environment. `aa status` shows `Certificate: custom` or `Certificate: built-in` for the native backend;
`--json` includes `certificateSource`. Older head units report `unknown` (`null` in JSON).
See [Native backend](docs/native-backend.md) for details.

### Higher resolution

The native backend projects at 800×480 (480p) by default. Set `OPENDHU_NATIVE_RESOLUTION=720p` or `=1080p`
before starting the daemon to ask the phone for a bigger screen instead; restart the session after changing
it. `aa status` shows the active size (`Screen: 1280x720`); `--json` includes `screenWidth`/`screenHeight`.
Touches, screenshots and the live view all follow whatever the running session reports.

## Commands

| Command | What it does |
|---|---|
| `aa start [--dhu] [--serial S] [--reverse PORT]… [--dhu-path P] [--config F] [--headless]` | Start a native session; `--dhu` selects Google's DHU (starts the daemon if none runs) |
| `aa stop` | Stop the session |
| `aa gui` | Open the desktop window |
| `aa status` | Show the session: backend, phone, focus (requested, head unit, phone), tunnels |
| `aa focus video on\|off` | Take or release video focus |
| `aa screenshot PATH` | Save the display as a PNG (absolute or relative; must end in `.png`) |
| `aa tap X Y` | Touch a pixel (native) |
| `aa swipe X1 Y1 X2 Y2 [--duration-ms N]` | Drag, 50 to 10000 ms (native) |
| `aa key NAME [--long]` | Press a car button: `home back up down left right enter search play_pause next previous call end_call` (experimental) |
| `aa audio OUT.wav [--channel media\|system\|speech] [--seconds N]` | Record phone audio and print how much arrived and how loud (native) |
| `aa test FILES… [--headless] [--no-interactive] [--report json\|junit\|html] [--artifacts-dir D] [--skip-unsupported]` | Run YAML tests; native head-unit window and terminal spinners are enabled by default |
| `aa doctor` | Check the environment without touching a session |
| `aa daemon [--port N]` | Run the API daemon in the foreground (it stays up) |

Exit codes: 0 success, 1 general error, 2 invalid arguments, 3 backend unavailable, 4 device disconnected,
5 unsupported, 6 timeout, 7 assertion failure, 8 session conflict, 130 interrupted.

## YAML tests

`aa test` opens the native head-unit window and animates terminal progress by default. Use `--headless`
to hide the window, `--no-interactive` to disable spinners, or both for CI. Redirected logs stay plain;
`--json` keeps machine-readable output on stdout. `--validate` starts no session or window.
The suite's `environment.headless` setting separately controls the phone emulator's window.

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
`build/test-results/` as HTML and JSON, with optional JUnit. A passing `tap`, `swipe` or `key` step means it was **sent**: the phone
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
wireless Android Auto, and the DHU's `--adb` transport. The native backend's car buttons are sent and bound by the
phone, but had no visible effect on a Pixel 6, so they are marked experimental.

## Known limitations

- **openDHU can only control a session that it started.** A DHU launched from another terminal takes commands only on
  its own stdin.
- Starting a session re-enumerates the phone over USB, which drops `adb reverse` tunnels. `--reverse PORT` restores the
  ones you ask for, and openDHU re-applies them if the phone drops them later.
- Unplugging the phone ends the session (observed once). Plug it back in and run `aa start` again.
- Native sessions support USB or forwarded loopback TCP. One head unit can hold a phone at a time: stop any DHU before using the native backend, and the other
  way round. With several phones attached, the native backend refuses to guess.
- The native backend is an unofficial implementation of an undocumented protocol. An Android Auto update on the phone
  can break it; run `make hw-native` after updating.
- Tested with DHU 2.0 (build 2022-03-30) and Android Auto 18.0.164204 on a Pixel 6.

## Tests

```bash
make                  # lint, vendor check, build, all tests, parse every example (no hardware)
make hw-dhu           # real phone + Google DHU
make hw-native        # real phone + openDHU's own head unit
```

The hardware targets refuse to run if a DHU or daemon is already running, so they never touch someone else's session.

Real-phone tests (`make hw-*`) and Android sample Gradle/emulator tests (`make sample-check`,
`make sample-test`) remain separate. The sample's Android Auto journeys need a provisioned Google
Play AVD or an authorized, provisioned USB phone (`aa test sample --usb`); headless mode alone does not supply one. See [the sample guide](samples/android/README.md).
## Repository

| Crate | Job |
|---|---|
| `opendhu-core` | The backend trait, shared types, exit codes |
| `opendhu-dhu` | The DHU backend |
| `opendhu-native` | The native backend (drives `opendhu-native-hu`) |
| `opendhu-native-hu` | openDHU's own head unit, run as a child process |
| `opendhu-daemon` | The HTTP API, session state, events |
| `opendhu-test` | The YAML runner and reports |
| `opendhu-cli` | The `aa` command |
| `opendhu-gui` | The `opendhu-gui` window |
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
| [docs/vision.md](docs/vision.md) | Why openDHU exists, and what it's for |

## Licence

Apache-2.0. `vendor/android-auto` is LGPL-3.0-or-later and `vendor/rustls-webpki` is ISC; see the `OPENDHU-PATCHES.md`
in each. openDHU is a test tool, not a certified car head unit.
