# Writing and running tests

```bash
aa start --reverse 10101          # a session must be running (the runner never starts one)
aa test examples/video-focus.yaml
aa test tests/ --report junit --report json --artifacts-dir ./test-results
```

`aa test` takes files or directories (searched recursively for `.yaml` / `.yml`, in sorted order).
Every file is parsed and validated **before** the session is touched, so a typo exits with code 2 and writes
nothing.

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
`-no-snapshot-save`. A per-AVD lock rejects simultaneous suites. It installs the APK, starts Android
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
