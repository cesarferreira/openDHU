# openDHU Android demo

A small, offline Kotlin app with a phone screen and a real Android Auto app. Browse three places,
read their details, save a favorite, and clear it. The phone and car share ordinary app preferences,
so favorites survive process restarts. No server, account, location permission, API key, or test-only
app interface is needed.

The sample lives in its own Gradle project. It does not depend on openDHU's Rust crates or another
Android repository. Normal Rust builds and tests do not need the Android SDK.

## Build

Install JDK 17 or newer and the Android SDK. Set `ANDROID_HOME` to your SDK directory, or configure
`sdk.dir` in an ignored `local.properties` inside this directory. Install SDK Platform 36 and Build
Tools 36.0.0, and accept the SDK licenses. The checked-in Gradle wrapper downloads its pinned Gradle
version and verifies the distribution checksum.

From the repository root:

```bash
make sample-build
make sample-check
```

Or open this directory in Android Studio, sync, and run `app` on a phone/emulator. The APK is
`app/build/outputs/apk/debug/app-debug.apk`; its package is `dev.opendhu.demo` and launcher label is
**openDHU Demo**. The phone screen lets you choose the same favorite as the car screen.

## Try Android Auto

Install openDHU and its native head unit/GUI with `make install` from the repository root. Install
`adb`, `ffmpeg`, and `tesseract` with English OCR data. Configure openDHU's development head-unit
[credentials](../../docs/native-backend.md#debugging-knobs) outside the repository.

Use a dedicated Google Play phone AVD named **medium_phone**, with a system image matching your
host architecture. Set up Android Auto once: install/update it, complete its consent/permission
screens, and enable its developer settings and development app testing. Use an English phone/host
locale for the included text selectors. openDHU restarts the head-unit server during suite setup.
A plain Android emulator without Android Auto provisioning cannot run these E2E tests.

To provision the AVD, boot it visibly:

```bash
"$ANDROID_HOME/emulator/emulator" -avd medium_phone
```

After provisioning, close the emulator or leave it running. The suite reuses a matching running AVD;
otherwise it boots it without a phone window and stops only the emulator it started. If you use a
different AVD name, change `environment.emulator` in `e2e/suite.yaml`. The suite installs and stops
only this sample package and preserves its data.

For a physical phone, enable USB debugging, connect it by USB, unlock it, and accept the computer's
authorization prompt. Complete the same Android Auto development setup and configure the development
head-unit credentials above. Then run from the repository root:

```bash
make sample-test SAMPLE_TEST_ARGS="--usb"   # Build, install on the USB phone, and run the suite
aa test sample --usb                      # Rerun the already-built APK
```

`--usb` overrides `environment.emulator` for this run. Exactly one USB phone must be connected and
authorized; emulators and wireless ADB devices are ignored. The suite installs the APK with existing
app data preserved, applies its permissions and services, projects through native USB, and writes the
same reports. It cleans up its own session, logs and tunnels; it never boots or shuts down an emulator.
The phone's initial Android Auto consent screens may need to be accepted manually.

To explore manually instead, install the APK with `adb -s SERIAL install -r PATH_TO_APK`, then use
`aa start --serial SERIAL` and `aa gui`. Open **openDHU Demo** from Android Auto's app launcher.

## E2E tests

From the repository root:

```bash
make sample-validate                         # Parse all three tests; no SDK/device needed
make sample-test                             # Build, show the head unit, run with terminal spinners
aa test sample                               # Rerun the already-built APK
aa test sample --filter favorite-persistence  # Run just the restart journey
aa test sample --headless --no-interactive    # Hide the head unit and disable spinners
```

`OPENDHU_BIN="$HOME/.cargo/bin/aa" make sample-test` selects openDHU explicitly when Apple's
`/usr/bin/aa` wins on PATH. Pass test options through `SAMPLE_TEST_ARGS`, for example:

```bash
make sample-test SAMPLE_TEST_ARGS="--headless --no-interactive"
```

The suite covers:

- **Browse places:** open a destination, verify its description, return to the list, and open another.
- **Favorite persistence:** save a favorite through Android Auto, restart the app, and reopen the
  saved destination from the restored home screen.
- **Reset:** save a different favorite, clear it, restart, and verify that it remains cleared.

Tests drive actual projected touches and assert screenshot/OCR results. They do not inject test state.
The persistence journey also cross-checks that result two other ways: `car.expect` reads the template the
app reports through the debug-only `CarStateProvider`, and `phone.expect` reads the phone screen. Each case opens its own app session and establishes the state it
needs through normal controls. Assertions follow one-shot navigation, save and reset actions; a sent touch alone is
not success. These are local UI/persistence journeys, not network integration tests.

Reports, screenshots, OCR and host/logcat logs go under `android-sample/build/test-results/` (replaced on each run)
(relative to the repository root), covered by the Android project's existing build ignore rule.
Open `results.html` in the run folder for a summary, expandable steps, test logs and screenshots.
JSON and JUnit reports are also generated. Override the location with `--artifacts-dir PATH`.
`environment.headless: true` hides the phone emulator; `aa test --headless` independently hides the
head-unit viewer. Initial Android Auto consent and host eligibility remain setup prerequisites.

## Code

- `app/src/main/java/dev/opendhu/demo/MainActivity.kt`: companion phone screen.
- `DemoCarAppService.kt`: Android Auto entry point and host validation.
- `CarScreens.kt`: home, place list and destination screens using Car App Library templates.
- `FavoriteStore.kt`: shared catalog and persisted favorite.
- `app/src/test/`: preference persistence and cross-screen notification regressions.
- `e2e/tests/`: black-box Android Auto journeys.

The app follows Android's [Car App Library setup](https://developer.android.com/training/cars/apps/library/set-up-project)
and declares the point-of-interest category. It pins [Car App Library 1.7.0](https://developer.android.com/jetpack/androidx/releases/car-app)
and [Android Gradle Plugin 9.1.1](https://developer.android.com/build/releases/agp-9-1-0-release-notes)
with Gradle 9.3.1. Debug builds accept development hosts; non-debug builds validate the standard
AndroidX host allowlist. There is no separate E2E flavour or exported testing endpoint.
