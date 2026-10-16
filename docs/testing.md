# Writing and running tests

```bash
aa start --reverse 10101          # a session must be running (the runner never starts one)
aa test examples/video-focus.yaml
aa test tests/ --report junit --report json --artifacts-dir ./test-results
```

`aa test` takes files or directories (searched recursively for `.yaml` / `.yml`, in sorted order).
Every file is parsed and validated **before** the session is touched, so a typo exits with code 2 and writes
nothing.

Progress is printed immediately by default, including suite setup, test names, each step starting,
and its result and duration. Output is flushed when redirected to a terminal or CI log. With `--json`,
progress goes to stderr and stdout contains only the final JSON report.

To watch the projected head unit while tests run:

```bash
aa test e2e/production.yaml --show-head-unit
```

`--show-head-unit` opens AutoLab's GUI connected to that test session, including a suite's private
session. Install it with `make install-gui`. The native backend is required. The window and its decoder
close when the run ends or is interrupted; closing the window yourself does not stop the tests.
The phone emulator can remain headless: `environment.headless` controls the phone's window, while
this flag shows the head unit. `--validate` opens no window. Viewer diagnostics go to
`head-unit-viewer.log` in the artifacts directory.

## Format

```yaml
name: Video focus recovery
backend: dhu                 # optional; the test fails if the session uses another backend
requirements:                # optional; checked against the backend's capabilities before any step runs
  - videoFocus               # allowed: videoFocus, screenshots
  - screenshots
steps:
  - action: focus.video
    value: off               # on | off
  - action: wait
    duration: 1s             # units are required: 500ms, 1s, 2m. Maximum 10m.
  - action: focus.video
    value: on
  - action: screenshot
    path: screenshots/restored.png
  - action: tap            # native backend only; needs `requirements: [touchInput]`
    x: 764                 # pixels of a screenshot (800x480), origin top left
    y: 348
  - action: swipe
    from: [300, 200]
    to: [100, 200]
    duration: 400ms        # optional, 50ms to 10s, default 300ms
```

The first example above is the PRD's, and it parses verbatim. Unknown keys, unknown actions, unknown
requirements, an empty `steps` list, durations without a unit or above 10 minutes, and empty expectations are
all rejected with exit code 2. YAML files are read with `serde_norway`, a maintained fork of the deprecated
`serde_yaml`. It accepts `off` as a word, a boolean or a quoted string.

| Action | Keys | Passes when |
|---|---|---|
| `focus.video` | `value: on\|off` | the backend accepted the command and its readback does not contradict it |
| `wait` | `duration` | the time has passed |
| `screenshot` | `path` | the backend wrote a PNG. The path is relative to the artifacts directory, must end in `.png`, may not contain `..` or whitespace |
| `assert.status` | `expect` | every field in `expect` matches the current status |
| `waitFor` | `condition`, `timeout` | `condition` matches before `timeout`. Polled every 250 ms, always bounded |

`expect` and `condition` take any of:

```yaml
state: running               # the DHU finished its handshake with the phone
backend: dhu
videoFocus:
  requested: off             # what AutoLab last asked for
  backendReported: off       # the DHU's own readback
  observed: on               # the phone's own word: native backend only (see below)
```

## What a pass does and does not mean

- With the DHU, `focus.video` passing means the command was **accepted** and the DHU's readback agrees. It does not
  mean the phone confirmed anything. The step's detail says so every time ("not confirmed by the phone"). With the
  native backend (`backend: native`) a pass means the phone itself sent the matching video start or stop message.
- `unknown` never satisfies an expectation, and `unknown` cannot be written in a test. Phone-side confirmation is
  unavailable with the DHU, so there `observed` is always unknown and an assertion on it fails, by design, rather
  than passing on nothing. The native backend does report it ([native-backend.md](native-backend.md)).
- Screenshots alone do not establish recovery. Suite tests can use `ui.expect` to assert visible projected
  app controls after restoring focus; backend state needs separate assertions.
- The DHU refuses screenshots while video focus is off, so take screenshots with focus on.

## Isolation

After every test, including failed ones, the runner restores video focus to `on` if the test may have left it
off, and records that as a `cleanup.focus.video` step. If the restore fails, the test fails. A test does not
assume the next one starts clean in any other way: nothing on the phone is reset.

**Ctrl+C and SIGTERM.** The first one asks the run to stop cleanly. Waits end within about 100 ms, the step in
progress is reported as interrupted, video focus is restored if the test had left it off, the artifacts for what
ran are written (`interrupted: true` in `results.json`), tests that had not started are not run, and the exit code
is 130. A second Ctrl+C exits immediately, without cleanup, in case a backend call is stuck. In that case run
`aa focus video on` yourself. The session itself always survives; only `aa test` stops.

## Results and exit codes

A run stops at the first step that does not pass; later steps are reported as skipped. A test is `passed`,
`failed` (an assertion or wait did not hold), `error` (something else went wrong, such as the session being
gone, a rejected command or a failed cleanup) or `skipped`.

| Code | Meaning in `aa test` |
|---|---|
| 0 | every test passed (skipped tests do not fail a run) |
| 2 | invalid test file or arguments, or a test written for another backend |
| 3 | no running session, or the backend is unavailable |
| 5 | a required capability is unsupported (use `--skip-unsupported` to skip such tests instead) |
| 6 | a `waitFor` timed out |
| 7 | an assertion failed |
| 130 | interrupted by Ctrl+C or SIGTERM (outranks every other code) |
| 1 | any other error |

`aa start` can also return 4 (a device or tunnel problem). `aa test` cannot: it never starts a session, and a
DHU that has died shows up as 3.

With several tests, one code is chosen in the fixed order 2, 3, 4, 5, 6, 7, 8, 1, so the result does not depend
on test order.

## Artifacts

```text
test-results/
  results.json        # with --report json (the default)
  results.xml         # with --report junit (validated with xmllint)
  metadata.json       # always
  logs/01-<test>.log  # one per test
  screenshots/...     # whatever the tests captured
```

`results.json` and `results.xml` are the only files cleaned up: if you request one format, an old report in the other
format is deleted so CI cannot read it by mistake. `logs/` and `screenshots/` accumulate across runs, so use a fresh
`--artifacts-dir` per run if you need them to match one run exactly. `--json` prints the report to stdout instead of
the human output.

`metadata.json` holds only values that were observed: AutoLab version, start time, duration, and the backend's
name, version, build and device serial. `deviceModel`, `androidVersion`, `androidAutoVersion` and `display`
are `null` because the backend interface cannot read them today.

The artifacts directory must not contain whitespace if any test takes a screenshot, because the path is passed
to the DHU console, whose handling of quoting is unverified.

## Not implemented

Retry policies, image assertions, key steps, event assertions and a Kotlin DSL.

A `tap` or `swipe` step passing means the touch was **sent**. The phone does not acknowledge touch, and the step
says so. To check that something happened, follow it with a `screenshot` and compare the pictures outside the test.

## Headless Android Auto suites

A suite owns an isolated native TCP session and discovers ordinary test YAML files. Validate it
without an emulator, APK, or running daemon:

```bash
aa test --validate e2e/suite.yaml
aa test e2e/suite.yaml --artifacts-dir test-results/run-001
```

```yaml
version: 1
environment:
  emulator: medium_phone
  headless: true
  apk: ../car-storybook/build/outputs/apk/debug/car-storybook-debug.apk
  package: ai.wayve.robotui.storybook
  appLabel: RobotUI Storybook
tests:
  - tests/*.yaml
```

APK and test paths resolve relative to the suite file. The emulator must be a dedicated, provisioned
Google Play phone AVD with Android Auto installed, developer mode enabled, and initial projection
consent accepted. `adb`, `ffmpeg`, and `tesseract` with English data must be on PATH. Set `ANDROID_HOME`
or `ANDROID_SDK_ROOT` to locate the Android SDK emulator, or put `emulator` on PATH.

AutoLab reuses a matching running AVD or starts the SDK emulator with `-no-window` and
`-no-snapshot`. A per-AVD lock rejects simultaneous suites. It installs the APK, starts Android
Auto's head-unit server through its Settings UI if needed, allocates a device-scoped ADB forward,
and starts a private daemon. This does not replace an existing physical-phone session. Cleanup
removes the forward and private daemon; it stops only an emulator that the suite itself started.
The AVD's user data is preserved: this is not a fresh device image or a production backend test.

Each test force-stops the configured application to reset its navigation stack. It does not erase
app data or grant permissions. Provisioning is a prerequisite, including English host/device UI.

New projected UI actions:

```yaml
name: Storybook catalog
backend: native
requirements: [touchInput, screenshots]
steps:
  - action: app.open
  - action: ui.waitFor
    text: RobotUI Storybook
    timeout: 30s
  - action: ui.tap
    text: Navigation / Active route
    until: Exit map
    timeout: 30s
  - action: ui.expect
    text: Exit map
    timeout: 30s
```

`ui.waitFor` and `ui.expect` pass only when text is observed in a screenshot of the projected
surface. Text matching is case-insensitive and normalizes whitespace. `ui.tap` locates observed
text and sends a real projected touch; follow it with an assertion of the result. Optional `until`
requires that result text to become visible and retries the source tap while its text remains visible.
Use this for navigation where the host can display a cached template during cold startup. All timeouts are
bounded and interruptible. There is no automatic scrolling: use an explicit swipe for off-screen
items. `app.open` uses the host launcher and a truncated label prefix; this first implementation
supports the native 800x480 display profile, which suites explicitly select. OCR requires visible,
legible text and should not be used to assert map pixels, clocks, or animation timing.

Each suite restarts Android Auto on its dedicated AVD before starting the head-unit server, to
discard projection threads restored by Quick Boot.

Suites default to fresh `test-results/run-*` directories and both JSON and JUnit reports. They also
save `suite.json`, `logcat.txt`, `head-unit.log`, and `ui/` screenshots, scaled images, OCR TSVs and
OCR diagnostics. The private daemon authentication file is removed rather than archived. Inspect
these artifacts for setup and assertion failures; screenshots alone do not make a test pass.

## Production-app journeys with local services

A suite can own local fixture services and prepare its emulator's network and runtime permissions:

```yaml
version: 1
environment:
  emulator: medium_phone
  headless: true
  apk: ../app/build/outputs/apk/dev/debug/app-dev-debug.apk
  package: example.app.dev.debug
  appLabel: Example App
  permissions: [android.permission.ACCESS_COARSE_LOCATION, android.permission.ACCESS_FINE_LOCATION]
  launchActivity: example.app.MainActivity
  reversePorts: [{device: 10101, host: 19101}]
services:
  - name: backend
    command: [node, backend/start.mjs]
    readyPort: 56551
    timeout: 60s
    env: {FIXTURE_MODE: '1'}
tests: [production/*.yaml]
```

Commands are argument arrays, run relative to the suite directory; optional `cwd` is also relative
to that directory. Service stdout/stderr goes to `<name>.log`. Services receive
`AUTOLAB_ARTIFACTS_DIR` so fixture code can create run-specific state there. AutoLab waits for each
loopback `readyPort`, fails if the port is occupied or leased by another suite, and stops owned
process groups on completion, startup failure or interruption. Service readiness should be exposed
only after dependencies are initialized; tests still assert actual backend responses.

The suite installs the APK, grants the declared runtime permissions, acquires device-to-host ADB
reverse mappings without replacing existing ones, and optionally opens `launchActivity` once before
projection. It removes its own reverse mappings during cleanup. These operations affect the dedicated
test AVD only. They do not clear application data.

Backend steps use a loopback fixture API, while the app uses its normal production network clients:

```yaml
- action: http.request
  method: POST
  url: http://127.0.0.1:56551/reset
  expect: {status: 200, json: {reset: true}}
  timeout: 10s
- action: app.open
- action: ui.tap
  text: VSO
  until: Search
  timeout: 30s
- action: ui.tap
  text: Search VSOs
  timeout: 15s
- action: keyboard.type
  text: aut
  timeout: 30s
- action: ui.tap
  text: AutoLab Driver
  timeout: 15s
- action: http.request
  method: GET
  url: http://127.0.0.1:56551/assertions/driver-selected
  expect: {status: 200, json: {completed: true}}
  timeout: 15s
- action: app.restart
- action: ui.expect
  text: AutoLab Driver
  timeout: 30s
```

`http.request` requires GET or POST, a plain HTTP URL with an explicit loopback IP, an expected status,
and a bounded timeout. Optional `json` is a POST body. `expect.json` matches object fields recursively;
arrays and scalars match exactly. GET polls mismatches; POST sends once and never retries a mutation.
Redirects, proxies, authentication headers and remote URLs are unsupported. Request/response evidence
is retained in `http/`; use synthetic fixture data. A failed backend assertion fails the test.

`keyboard.type` types through projected touchscreen keys. It currently supports only the native
800×480 English lowercase QWERTY layout: 1–128 lowercase ASCII letters, digits and spaces. It waits
for keyboard glyphs at their expected positions, including distinguishing `y`/`z` anchors, before
every touch, and fails without
further input if the layout disappears or the timeout expires. Focus the projected search field first;
assert on the resulting text or results afterward. Frames and OCR are retained under `keyboard/`.

`device.inputText` uses ADB to inject text into the focused phone input on the suite emulator. Its
behavior depends on the phone IME and display focus; use `keyboard.type` for the supported projected
Android Auto keyboard. Focus the phone field first. Input must be nonempty, at most 256 safe ASCII
characters (letters, digits, spaces, `_`, `@`, `.`, `-`); shell syntax is rejected before device access.
`app.restart` force-stops only the suite package and reopens it through the projected launcher, without
erasing data or preparing backend state. Combine backend request assertions with projected UI checks:
calling a fixture preparation endpoint alone does not prove the application used the backend.
