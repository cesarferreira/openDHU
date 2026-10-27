# openDHU

**Drive and test Android Auto from your terminal.**

openDHU is a scriptable Android Auto head unit. It connects to a phone or an emulator, shows the projected
screen, and lets you tap, swipe, change video focus and take screenshots from a CLI, a desktop window, your
terminal, an HTTP API, or YAML tests. It is Espresso for the car screen.

```text
$ aa test sample
Suite: android-sample/e2e/suite.yaml (3 tests)
Test: Favorite selected in Android Auto survives an app restart [2/3] · ~1m left
  PASS 1. launch the app on the car screen (8.4s)
  PASS 2. clear any saved favorite (3.2s)
  PASS 8. save Harbour Cafe as favorite (1.3s)
  PASS 10. restart the app (8.2s)
  PASS 11. favorite survived the restart (2.9s)
  ...
PASS  3 tests · 1 suite
```

- **Interact:** tap, swipe, press car buttons, toggle video focus, and watch the display live.
- **Capture:** screenshots, and audio the phone plays through the car.
- **Test:** find text on the projected screen with OCR, tap it, assert what happened, and get HTML, JSON and
  JUnit reports.
- **Automate:** managed suites install the app, start local services and ADB tunnels, and clean up after
  themselves.
- **Integrate:** every command has `--json`, and there is an authenticated local HTTP API with live events.

openDHU runs its own native head unit by default, and can also drive Google's Desktop Head Unit (DHU).

## Install

You need Rust 1.87+, `adb` and `ffmpeg`. Tests that read the screen also need `tesseract` with English
data. It is tested on macOS and Linux.

```bash
git clone https://github.com/cesarferreira/openDHU.git
cd openDHU
make install     # aa, opendhu-native-hu, opendhu-gui and opendhu-tui into ~/.cargo/bin
aa doctor        # checks the environment without touching a session
```

`make install` builds optimized release binaries; `make install-debug` is for development. For a real phone,
enable USB debugging, authorize the computer and complete Android Auto setup first. Development apps need
[head-unit credentials](docs/native-backend.md#debugging-knobs).

## Use it

```bash
aa start                          # native head unit over USB (--serial SERIAL to pick a device)
aa gui                            # live display with mouse input and session controls
aa tui                            # the same in your terminal (kitty graphics: kitty, Ghostty, WezTerm)
aa tap 400 240
aa swipe 300 200 100 200 --duration-ms 400
aa key home
aa focus video off
aa screenshot screen.png
aa audio recording.wav --seconds 10
aa status --json
aa stop
```

Touch, live view, audio capture and phone-confirmed video focus need the native backend. Use `aa start --dhu`
for Google's DHU (install `extras;google;auto` with the Android SDK). `aa start --reverse PORT` restores an ADB
tunnel after a USB reconnect. `aa --help` lists everything.

## Test Android Auto apps

A test is a YAML file. Each step can carry an `as:` label saying what it is for, and the run prints that
instead of the mechanics:

```yaml
name: Favorite survives an app restart
backend: native
requirements: [touchInput, screenshots]
steps:
  - action: app.open
    as: launch the app on the car screen
  - action: ui.tap
    as: open the places list
    text: Browse places
    timeout: 20s
  - action: ui.expect
    as: places list is showing
    text: Riverside Park
    timeout: 20s
  - action: screenshot
    as: capture the list
    path: screenshots/places.png
```

`ui.tap`, `ui.expect` and `ui.waitFor` find text by OCR on the real projected frame, so tests only see what
a driver would see. Other actions cover swipes, keys, video focus, HTTP calls to local fixtures and status
assertions. See [Testing](docs/testing.md) for every action.

**Run one test** against a session you started with `aa start`:

```bash
aa test examples/video-focus.yaml
aa test examples/video-focus.yaml --tui    # watch the head unit and the progress together
```

**Run managed suites**, which pick a device, build and install the APK, start services and tear it all down:

```bash
aa test                          # every suite registered in opendhu.yaml
aa test sample                   # one suite, on the configured emulator
aa test sample --usb             # on one authorized USB phone instead
aa test sample --filter persist  # only matching tests
aa test sample --list            # list tests; `aa test --list` lists suites
aa test --validate               # parse everything without touching a device
```

Test and suite headers are numbered (`[6/14]`, `[suite 2/4]`), and once a run has passed, test headers show an
estimate of the time left. Open `results.html` in the artifacts folder for steps, screenshots and logs.

The [Android sample](android-sample/README.md) is a small app with three Android Auto journeys. Try it with
`make sample-build && aa test sample` (needs the Android SDK and JDK 17+). Provision Android Auto on the
emulator or phone first; the sample README walks through it.

## Documentation

| | |
|---|---|
| [Testing](docs/testing.md) | YAML actions, suites, services, reports, and what a pass does and does not prove |
| [Native backend](docs/native-backend.md) | credentials, resolution, transports and limits |
| [Desktop GUI](docs/gui.md) / [Terminal viewer](docs/tui.md) | the two viewers |
| [HTTP API](docs/api.md) | endpoints and live events |
| [Configuration](docs/configuration.md) | `config.toml` and environment variables |
| [Architecture](docs/architecture.md) | how the crates fit together |
| [Vision](docs/vision.md) / [Feasibility](docs/feasibility.md) | why it exists, and what was proven first |

## Development

```bash
make              # format check, clippy, build, tests and example validation; no hardware needed
make help         # every target
make hw-native    # hardware checks against a real phone
```

Native USB sessions support one connected phone at a time. Car-button input is experimental, and wireless
Android Auto is unsupported.

## Licence

Apache-2.0. Vendored dependencies keep their licences:
[android-auto](vendor/android-auto/OPENDHU-PATCHES.md) (LGPL-3.0-or-later) and
[rustls-webpki](vendor/rustls-webpki/OPENDHU-PATCHES.md) (ISC).
