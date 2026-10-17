# openDHU

Control and test **Android Auto** from your terminal, YAML tests, a desktop window, or an HTTP API.
openDHU runs its own native head unit or drives Google's Desktop Head Unit (DHU).

- **Interact:** tap, swipe, change video focus, and watch the projected display live.
- **Capture:** save screenshots and record media, navigation, or assistant audio.
- **Test:** find text with OCR, tap UI labels, assert outcomes, and generate HTML, JSON, and JUnit reports.
- **Automate:** manage app installation, local services, ADB tunnels, and test-session cleanup.
- **Integrate:** use JSON CLI output, an authenticated local HTTP API, and live events.

## Install

Requires Rust 1.87+, `adb`, and `ffmpeg`. Tested on macOS.
For a real phone, enable USB debugging, authorize your computer, and complete Android Auto setup.

```bash
git clone https://github.com/cesarferreira/openDHU.git
cd openDHU
make install
aa doctor
```

Installs `aa`, `opendhu-native-hu`, and `opendhu-gui` into `~/.cargo/bin`; add it to your `PATH`.
For development apps, see [native head-unit credentials](docs/native-backend.md#debugging-knobs).

## Use

```bash
aa start                       # Native USB head unit; add --serial SERIAL to select the ADB device
aa gui                         # Live display with mouse input and session controls
aa tap 400 240
aa swipe 300 200 100 200 --duration-ms 400
aa screenshot screen.png
aa audio recording.wav --seconds 10
aa focus video off
aa focus video on
aa status --json
aa stop
```

Use `aa start --dhu` for Google's DHU (install `extras;google;auto` through the Android SDK).
Touch input, live view, audio capture, and phone-confirmed video focus use the native backend.
Add `--reverse PORT` to `aa start` to restore an ADB tunnel after USB reconnections.
Every command supports `--json`; `aa --help` lists commands.

## Test Android Auto apps

Run individual YAML tests against a session you started with `aa start`:

```bash
aa test examples/native-phone-confirms.yaml
```

```yaml
name: Restore video focus
backend: native
requirements: [videoFocus, screenshots]
steps:
  - action: focus.video
    value: off
  - action: assert.status
    expect: {videoFocus: {observed: off}}
  - action: focus.video
    value: on
  - action: screenshot
    path: screenshots/restored.png
```

Managed suites handle device setup, APK installation, and cleanup. Try the included
[Android demo](samples/android/README.md), which tests browsing, favorite persistence, and reset:

```bash
make sample-build                       # Requires Android SDK and JDK 17+
aa test sample                          # Use the configured emulator
aa test sample --usb                    # Use one authorized USB phone instead
aa test sample --filter persistence     # Run matching tests
aa test sample --headless --no-interactive
aa test --list                          # List registered suites
aa test sample --list                   # List tests in a suite
aa test all --usb                       # Run every registered suite on the USB phone
```

OCR tests require Tesseract with English language data. Provision Android Auto on the phone or
emulator first; see the [sample setup](samples/android/README.md#try-android-auto).
Open `results.html` in the reported artifacts directory for steps, screenshots, and logs.
Use `--validate` to check YAML without starting devices or sessions.

## Development

```bash
make          # Format check, Clippy, build, tests, and example validation; no hardware needed
make hw-native
make hw-dhu
```

`make help` lists all targets. Native USB sessions support one connected phone at a time.
Car-button input is experimental; wireless Android Auto is unsupported.

## Documentation

- [Native backend](docs/native-backend.md): credentials, resolution, transports, and limitations
- [Testing](docs/testing.md): YAML actions, managed suites, services, and reports
- [Desktop GUI](docs/gui.md)
- [HTTP API](docs/api.md)
- [Configuration](docs/configuration.md)
- [Architecture](docs/architecture.md)

## Licence

Apache-2.0. Vendored dependencies retain their licences:
[android-auto](vendor/android-auto/OPENDHU-PATCHES.md) (LGPL-3.0-or-later) and
[rustls-webpki](vendor/rustls-webpki/OPENDHU-PATCHES.md) (ISC).
