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
aa doctor sample  # Check a registered suite before running it
```

Installs `aa`, `opendhu-native-hu`, and `opendhu-gui` into `~/.cargo/bin`; add it to your `PATH`.
`make install` builds debug binaries; use `make install-release` for optimized binaries.
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
aa test                                 # Run every registered suite
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

## Performance

Local CPU and memory measurements on **19 October 2026**, using an Apple M4 Pro (14 cores, 24 GiB RAM),
Google DHU 2.1 for ARM64, and openDHU at commit `9ec7d69`. Both used the same Android emulator over
ADB TCP at **800×480, 30 FPS requested, and 111 DPI**, with no media playing.

Each GUI result averages two 30-second samples: idle on the app launcher, then switching between
the launcher and dashboard every two seconds. The dashboard showed Maps' location-permission
placeholder, so this measures UI transitions rather than active navigation.

| Build | Idle CPU | Interacting CPU | Interacting RAM |
|---|---:|---:|---:|
| Google DHU 2.1 | 1.6% | 4.5% | 92 MiB |
| openDHU debug | 1.8% | 38.3% | 175 MiB |
| openDHU release | 0.7% | 11.1% | 172 MiB |

**100% CPU means one logical core.** CPU is measured from interval CPU-time deltas. RAM is summed
resident set size (RSS); openDHU totals include the daemon, native head unit, GUI, and FFmpeg decoder.
The shared emulator, ADB, WindowServer, benchmark controller, startup, and screenshot capture are excluded.

Release compilation reduced openDHU's interaction CPU use by **71%** compared with its debug build.
In this workload, Google's GUI still used less CPU and memory than optimized openDHU.

With no viewer attached, one additional 30-second idle sample per backend gave:

| Headless backend | CPU | RAM |
|---|---:|---:|
| Google DHU 2.1 | 1.33% | 79 MiB |
| openDHU release | 0.03% | 17 MiB |

Both headless sessions retained video focus and projected the launcher; Google's headless video focus
was explicitly enabled. Default desktop window sizes were retained for GUI tests. These are short local
measurements, not cross-platform guarantees. RSS can count shared pages more than once and does not
capture compressed memory or GPU allocations. Delivered FPS, frame drops, latency, energy use, and
GPU utilization were not measured.

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
