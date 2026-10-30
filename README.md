# openDHU

**Espresso for the car screen.** Drive and test Android Auto apps from your terminal, a desktop window or CI.

openDHU is a scriptable Android Auto head unit. It connects to a phone or an emulator, shows the projected
screen, and lets a script or a YAML test tap it, read it, film it and assert on it, exactly as a driver would
see it.

```text
$ aa test sample --tui --record
Suite: android-sample/e2e/suite.yaml (3 tests)
Test: Favorite selected in Android Auto survives an app restart [2/3] · ~1m left
  PASS 1. launch the app on the car screen (8.4s)
  PASS 2. clear any saved favorite (3.2s)
  PASS 8. save Harbour Cafe as favorite (1.3s)
  PASS 10. restart the app (8.2s)
  PASS 11. favorite survived the restart (2.9s)
  Video: build/test-results/videos/02-favorite-selected-in-android-auto-survives-an-app-restart.mp4
PASS  3 tests · 1 suite
```

## Install

You need Rust 1.87+, `adb` and `ffmpeg`. Tests that read the screen also need `tesseract` with English data.

```bash
git clone https://github.com/cesarferreira/openDHU.git
cd openDHU
make install  # aa, opendhu-native-hu, opendhu-gui and opendhu-tui into ~/.cargo/bin
aa doctor     # checks the environment, including Android Auto on each device
```

For a real phone, enable USB debugging, authorize the computer and finish Android Auto setup on it first.
Development apps need [head-unit credentials](docs/native-backend.md#debugging-knobs).

## Write a test

A test is a YAML file. Each step can carry an `as:` label, and the run prints that instead of the mechanics:

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

`ui.tap`, `ui.expect` and `ui.waitFor` find text by OCR on the projected frame, so a test only sees what a
driver would see. Other actions cover swipes, car keys, video focus, HTTP calls to local fixtures and status
assertions. See [Testing](docs/testing.md) for all of them.

## Run tests

A **suite** (see `opendhu.yaml`) builds and installs your APK, boots the emulator, starts your local services
and ADB tunnels, runs the tests and cleans up. Open `results.html` in the artifacts folder afterwards for steps,
screenshots, logs and videos. JSON and JUnit reports are written too.

### Basics

```bash
aa test                            # every suite registered in opendhu.yaml
aa test sample                     # one suite, on its configured emulator
aa test sample --filter persist    # only tests whose name or file matches
aa test sample --list              # list tests; `aa test --list` lists suites
aa test --validate                 # parse everything without touching a device
aa test examples/video-focus.yaml  # one test, against a session you started with `aa session start`
```

### With recording

```bash
aa test sample --record          # film each test, keep the clips of the ones that failed
aa test sample --record all      # keep every clip
aa record clip.mp4 --seconds 10  # film a session you started yourself
```

Clips are MP4s in `videos/` next to the report, and `results.html` plays them under their test. They follow the
real pace of the run, since the phone only sends a frame when the picture changes. Needs ffmpeg with libx264
and the native backend.

### Headless

```bash
aa test sample --headless  # no head-unit window
```

Without `--headless`, the head-unit window opens so you can watch. Use it on servers and in CI.

### On a USB phone

```bash
aa test sample --usb  # the one authorized USB phone instead of the emulator
aa test sample --usb --headless
```

Connect the phone, enable USB debugging, authorize the computer and finish Android Auto setup on it first.

`--usb` is never auto-detected: without it the emulator is used, even if a phone is plugged in. With it, exactly
one authorized USB phone is required (emulators and wireless ADB devices are ignored), and a missing, unauthorized
or second phone is an error. `aa test sample --validate --usb` checks the files without a phone.

After an app restart, a phone's Android Auto sometimes keeps showing its own "Waiting..." or "has encountered an
unexpected error" card instead of the app. When that card stays up for 10 seconds after `app.open` or
`app.restart`, a USB suite restarts the head-unit session once, restores its ADB tunnels and opens the app again;
the step detail says when this happened.

### With the TUI

```bash
aa test sample --tui               # head unit and test progress together, in your terminal
aa test sample --tui --record all  # both, keeping every clip
```

Draws the head unit with kitty graphics, so it needs kitty, Ghostty or WezTerm and `opendhu-tui`
(`make install`). It replaces the head-unit window, so it can't be combined with `--headless`.

### Without the animated progress

```bash
aa test sample --no-interactive  # plain line-by-line progress, for logs
aa test sample --json            # machine-readable output
```

## Android Auto on the emulator

Managed suites need Android Auto (Gearhead) on the device, and stop early with a hint if it is missing. In
order of preference:

1. **Create the AVD from a Google Play system image.** Android Auto is usually already there, and the Play
   Store can update it.
2. **Install an APK you trust:** `aa setup gearhead --apk auto.apk`. It also takes a directory of split APKs
   or an `.xapk`/`.apks` bundle.
3. **Let apkeep fetch a pinned build:**

   ```bash
   aa setup gearhead --download --version 17.9.664044 \
     --sha256 6415e1ed2a4caa8c8ec751e5896b896a5ba81d6a46d75e94a5381be14769fc94
   ```

   It needs [apkeep](https://github.com/EFForg/apkeep) and downloads from APKPure, so the checksum is what you
   trust: openDHU refuses to install a file that does not match. List versions with
   `apkeep -a com.google.android.projection.gearhead -d apk-pure -l`.

Nothing is downloaded or installed unless you run the command, and it does nothing if Android Auto is already
there (`--force` reinstalls).

## In CI

```yaml
- uses: actions/checkout@v4
- uses: cesarferreira/openDHU@main
  with:
    suite: sample
    record: failures
```

See [CI](docs/ci.md) for the inputs. Emulator runs are slow and use a lot of runner minutes, so run them on
demand or on a schedule, not on every push.

## Try the sample

The [Android sample](android-sample/README.md) is a small app with three Android Auto journeys:

```bash
make sample-build && aa test sample  # needs the Android SDK and JDK 17+
```

## Drive it by hand

```bash
aa session start  # native head unit over USB (--serial SERIAL to pick a device)
aa gui            # live display with mouse input and session controls
aa tui            # the same in your terminal
aa tap 400 240
aa swipe 300 200 100 200 --duration-ms 400
aa key home
aa focus video off
aa screenshot screen.png
aa audio recording.wav --seconds 10
aa session status --json
aa session restart  # stop and start again with the same settings
aa session stop
```

Touch, live view, audio and phone-confirmed video focus need the native backend. Use `aa session start --dhu` for
Google's DHU (install `extras;google;auto` with the Android SDK). `aa session start --reverse PORT` restores an ADB
tunnel after a USB reconnect. `aa --help` lists everything.

## Documentation

| | |
|---|---|
| [Testing](docs/testing.md) | YAML actions, suites, services, reports, video, and what a pass does and does not prove |
| [CI](docs/ci.md) | the GitHub Action: emulator, Android Auto, reports and videos as artifacts |
| [Docker](docs/docker.md) | a headless emulator and `aa` in a container on a Linux host with KVM |
| [Native backend](docs/native-backend.md) | credentials, resolution, transports and limits |
| [Desktop GUI](docs/gui.md) / [Terminal viewer](docs/tui.md) | the two viewers |
| [HTTP API](docs/api.md) | endpoints and live events |
| [Configuration](docs/configuration.md) | `config.toml` and environment variables |
| [Architecture](docs/architecture.md) | how the crates fit together |
| [Vision](docs/vision.md) / [Feasibility](docs/feasibility.md) | why it exists, and what was proven first |

## Limits

Native USB sessions support one connected phone at a time. Car-button input is experimental, and wireless
Android Auto is unsupported. CI currently runs on Linux only; macOS is supported but not covered by CI.

## Development

```bash
make            # format check, clippy, build, tests and example validation; no hardware needed
make help       # every target
make hw-native  # hardware checks against a real phone
```

## Licence

Apache-2.0. Vendored dependencies keep their licences:
[android-auto](vendor/android-auto/OPENDHU-PATCHES.md) (LGPL-3.0-or-later) and
[rustls-webpki](vendor/rustls-webpki/OPENDHU-PATCHES.md) (ISC).
