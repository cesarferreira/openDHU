# Writing and running tests

```bash
aa start --dhu --reverse 10101    # this example targets Google's DHU; ordinary test files need a running session
aa test examples/video-focus.yaml
aa test tests/ --report junit --report json --report html --artifacts-dir ./build/test-results
```

`aa test` takes files or directories (searched recursively for `.yaml` / `.yml`, in sorted order).
Every file is parsed and validated **before** the session is touched, so a typo exits with code 2 and writes
nothing.

Progress is printed immediately by default, including suite setup, test names, each step starting,
and its result and duration. Output is flushed when redirected to a terminal or CI log. With `--json`,
progress goes to stderr and stdout contains only the final JSON report.

Native tests open openDHU's head-unit window by default:

```bash
aa test e2e/production.yaml
aa test e2e/production.yaml --headless  # Run without the head-unit window
```

The viewer connects to the test session, including a suite's private session. Install it with
`make install-gui`. Google DHU sessions use their own existing window instead. The openDHU window
and its decoder close when the run ends or is interrupted; closing the window yourself does not
stop the tests. `--show-head-unit` remains accepted for existing commands.
The phone emulator can remain headless: `environment.headless` controls the phone's window, while
`aa test --headless` hides the head-unit viewer. `--validate` opens no window. Viewer diagnostics go
to `head-unit-viewer.log` in the artifacts directory.

### Watch a run in the terminal (`--tui`)

Instead of a window, `--tui` shows the head unit and the test progress in the terminal you ran `aa test`
in, which also works over SSH. The terminal must support the kitty graphics protocol (kitty, Ghostty,
WezTerm). Install the viewer with `make install-tui`.

```bash
aa test e2e/production.yaml --tui
aa test sample --tui          # a managed suite: the viewer follows each test's own session
```

The live picture is on top and the progress lines (`RUN`, `PASS`, `FAIL`) are underneath. Click and drag
on the picture to touch the phone, as in `aa tui`. When the run ends the viewer waits for `q`, then the
whole run is printed into your scrollback and `aa` exits with the run's status. Closing the viewer early
does not stop the tests; their output continues in the shell. `--tui` cannot be combined with
`--headless`, `--show-head-unit`, `--interactive` or `--json`, and `--list` and `--validate` do not open
it. See [tui.md](tui.md) for keys, SSH behaviour and troubleshooting.

## Terminal progress and named suites

Terminal output colors current work cyan, passes green, failures/errors red and skips yellow.
`NO_COLOR` disables colors; redirected output and `TERM=dumb` stay plain.

```bash
aa test e2e/production.yaml                              # Window and animated terminal progress
aa test e2e/production.yaml --no-interactive             # Window and ordinary live progress
aa test e2e/production.yaml --headless --no-interactive  # No window or animated spinners
```

Interactive progress is enabled by default in a terminal: the current setup stage or step animates
on one line with elapsed time, and is replaced by its result when finished. `--no-interactive`
disables the animation while retaining live logs and terminal colors. `--interactive` remains
accepted for existing commands. Opposite flags cannot be combined.
Redirected output falls back to ordinary live logs. `--json` disables animation and styling,
keeping the final JSON on stdout and plain progress on stderr. Use `--headless` for CI runners
without a display or installed GUI; this is independent of progress formatting.

Progress is numbered so you can tell how far along a run is. Each test header ends with its place in
the whole run (`Test: Robotaxi ride ... [6/14]`, counted across every suite of `aa test all`), each
suite header with its place among the suites (`[suite 2/4]`), and each step with its place in the test
(`PASS 8/19. save Harbour Cafe as favorite`).

Register convenient names in a project `opendhu.yaml` without duplicating suite setup:

```yaml
suites:
  robotaxi: e2e/production.yaml
  storybook: e2e/suite.yaml
```

```bash
aa test --list                           # List registered suite names and paths
aa test robotaxi --list                   # List tests without starting a device
aa test robotaxi
aa test robotaxi --filter persistence --interactive
aa test storybook --filter route --validate
aa test                                  # Run every registered suite in name order
aa test all                              # Explicit form of the same command
aa test all --usb                        # Use one USB phone for each managed suite
aa test all --filter persistence --validate --json
```

A name resolves in the nearest `opendhu.yaml` in the current directory or its ancestors. Target paths
are relative to that file. Existing file/directory arguments take precedence over names. YAML still
defines each suite's environment, services and test files. If no registry is found, pass a YAML path
directly or create `opendhu.yaml` with the mapping above.

`--list` never starts devices, services, or a viewer. Without targets it prints suite names and paths;
with a target (including `all`) it prints test names and files, applying `--filter` when provided.
`aa test --list --filter TEXT` lists matching tests across all registered suites. `--json` returns
the same information as one JSON object.

With no target, `aa test` runs all suites from the nearest `opendhu.yaml`, just like `aa test all`.
It validates every selected suite's YAML before setup, then runs suites sequentially in name order.
Each required AVD is reused across suites and stays locked for the run. Emulators started by the
runner stop after all suites finish or the run is interrupted; pre-existing emulators stay running.
Backend services, app setup and projection sessions are still created and cleaned up per suite.
With `--filter`, suites without matching tests are skipped; no matches anywhere is an argument error.
Each suite gets its own directory under `build/test-results/`, or under the directory given by
`--artifacts-dir`, and one consolidated `results.html` (plus `results.json`) at the top lists every
suite and test with links to the per-suite reports and logs. Without `--artifacts-dir` the folder holds
only the latest run: it is cleared of earlier reports, logs, suite folders and old `run-*`/`all-*`
folders first. An explicit `--artifacts-dir` is never cleared. A failed suite does not prevent later suites from running; the command
returns the first nonzero exit code. Ctrl+C cleans up the active suite, skips the rest, and exits 130.
With `--json`, progress stays on stderr and stdout contains one summary of suite outcomes and report
locations. `all --validate` validates the selected YAML without requiring APKs or devices.
`--filter` selects test names or filenames by case-insensitive substring. All YAML is validated first;
a filter matching nothing exits with an argument error before starting services, devices or a viewer.

## Diagnose a suite before running it

```bash
aa doctor robotaxi                   # Registered name, or pass a suite YAML path
aa doctor robotaxi --usb             # Check a USB phone instead of the configured AVD
aa doctor robotaxi --json            # Same checks as a JSON array
```

Suite diagnostics use the same configuration resolver and test validation as `aa test`.
They check the APK, required native head unit and tools, Tesseract English data, the
configured AVD (which may be stopped), service executables/directories and readiness ports.
USB diagnostics select exactly one authorized USB phone and skip the emulator check.
The command never executes service commands, installs APKs, boots an emulator or starts projection.
It returns 3 for failed checks and 2 for invalid suite configuration. Ports are checked only
at that moment; running the suite still acquires its normal resource locks.
Android Auto provisioning, head-unit credentials, app permissions and service health need a real run.
Plain `aa doctor` retains its general environment checks.

## Share suite setup

A suite may extend one local YAML file. Put common environment and services in a setup file:

```yaml
# e2e/shared.yaml
version: 1
environment:
  emulator: medium_phone
  apk: ../app/build/outputs/apk/dev/debug/app-dev-debug.apk
  package: example.app.dev.debug
  appLabel: Example App
services:
  - name: backend
    command: [node, backend/start.mjs]
    readyPort: 56551
```

Each journey then supplies its tests and any differences:

```yaml
# e2e/robotaxi.yaml
extends: shared.yaml
environment:
  appLabel: Example Robotaxi
tests: [robotaxi/*.yaml]
```

Environment fields override individually. Every list, including `services`, `tests`,
`permissions` and `reversePorts`, replaces the inherited list completely; `services: []`
explicitly disables inherited services. Other top-level fields replace their inherited value.
There is no implicit service merging or list concatenation.

Paths resolve relative to the file that declares them: the parent path, APK, test globs,
`artifactsDir` and service `cwd`. A service without `cwd` runs in the directory containing
its declaration; relative command paths and arguments are interpreted there as usual.
A shared file can omit `tests`, but the resolved suite must include tests and all required
configuration. Parent chains are supported up to 32 files; cycles, missing files and unknown
fields are errors. Names, `all`, `--list`, `--validate` and diagnostics all use this resolver.

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

### Step labels

Any step may carry an optional `as:` label saying what the step is for, like the GIVEN/WHEN/THEN
name of a unit test:

```yaml
  - action: ui.tap
    as: save Harbour Cafe as favorite
    text: Save favorite
    timeout: 10s
```

The run prints the label instead of the mechanics, so a passing step reads
`PASS 8. save Harbour Cafe as favorite (0.4s)`. Without `as:`, openDHU derives a description from the
step's arguments (`tap "Save favorite"`, `GET /health`, `tap at (159, 383)`). A step that does not pass
also prints its detail (status codes, poll counts, artifact paths); `--verbose` prints that detail for
passing steps too. Labels appear in `logs/`, JUnit and JSON reports. `as:` must be a non-empty string.

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
  requested: off             # what openDHU last asked for
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

After cleanup, terminal output ends with one summary and the report and artifact paths:

```text
PASS  3 tests · 1 suite

Report:     build/test-results/results.html
Artifacts:  build/test-results
```

Failures, errors and skipped tests appear in the summary only when nonzero. Failed tests include
their names and log paths. A run of several suites gets one report link; a single suite links its own report. HTML is preferred, falling back to JSON or
JUnit when HTML was not requested. Paths within the current directory are shown relative to it.
Use `aa test --verbose` (or `-v`) to list report files, metadata and per-test logs, and repeat full failure details.
Terminal status labels use green for passes, red for failures and yellow for skips or interruption,
with cyan path labels and muted paths. `NO_COLOR`, `TERM=dumb` and redirected output disable colors.

```text
build/test-results/
  results.html        # browser report (default), or --report html
  results.json        # with --report json (also default)
  results.xml         # with --report junit (validated with xmllint)
  metadata.json       # always
  logs/01-<test>.log  # one per test
  screenshots/...     # whatever the tests captured
```

`results.html`, `results.json` and `results.xml` are the only files cleaned up: reports in unrequested
formats are deleted so CI cannot read it by mistake. `logs/` and `screenshots/` accumulate across runs, so use a fresh
`--artifacts-dir` per run if you need them to match one run exactly. `--json` prints the report to stdout instead of
the human output.

Open `results.html` in a browser for status counts, durations, expandable test steps, log links and a
screenshot gallery. It works offline without scripts or external resources. Keep the artifact folder
together when sharing it so relative evidence links work. `--report` is repeatable and selects the exact
formats to write, for example `--report html --report junit`.

`metadata.json` holds only values that were observed: openDHU version, start time, duration, and the backend's
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
aa test e2e/suite.yaml --artifacts-dir build/test-results/run-001
```

```yaml
version: 1
artifactsDir: ../build/test-results  # optional; relative to this YAML, replaced on each run
environment:
  emulator: medium_phone
  headless: true
  build:                                # optional; runs before every install
    command: [./gradlew, ":car-storybook:assembleDebug"]
    cwd: ..                             # optional; relative to this YAML, defaults to its directory
  apk: ../car-storybook/build/outputs/apk/debug/car-storybook-debug.apk
  package: ai.wayve.robotui.storybook
  appLabel: RobotUI Storybook
tests:
  - tests/*.yaml
```

APK and test paths resolve relative to the suite file. `aa test` installs whatever APK is on disk, so after
switching branches it can install a stale build. Set `environment.build` to rebuild first: the command runs
before the APK check and install, output goes to `build.log` in the artifacts folder, and a nonzero exit
stops the run before any device is touched. Gradle's incremental build keeps repeat runs cheap. The install itself is skipped when the device
already runs this exact APK (same size and modification time as the last suite install, recorded in
`/data/local/tmp/opendhu-apk-<package>`); the app is force-stopped instead, and any doubt means a normal
install. The emulator must be a dedicated, provisioned
Google Play phone AVD with Android Auto installed, developer mode enabled, and initial projection
consent accepted. `adb`, `ffmpeg`, and `tesseract` with English data must be on PATH. Set `ANDROID_HOME`
or `ANDROID_SDK_ROOT` to locate the Android SDK emulator, or put `emulator` on PATH.

Optional `environment.dnsServers` lists up to four DNS server IP addresses passed to the SDK
emulator at startup, for example `[8.8.8.8]` when host DNS auto-detection fails with a local or VPN
resolver. Omitting it preserves SDK auto-detection. An already-running AVD keeps its current DNS.

openDHU reuses a matching running AVD or starts the SDK emulator with `-no-snapshot`,
`-no-audio` and `-no-boot-anim`; headless suites also pass `-no-window -gpu swiftshader_indirect`.
Once an emulator it booted reports boot complete, it sets the window, transition and animator
animation scales to 0. An already-running AVD is left untouched. Tune this per suite, which matters
on hosts without a GPU or with nested virtualization:

```yaml
environment:
  gpu: host                 # -gpu mode; default swiftshader_indirect when headless, else the AVD's
  animations: true          # keep Android animations on (default false)
  emulatorArgs: [-cores, "6", -memory, "3072"]  # extra emulator arguments, one per entry
```

A per-AVD lock rejects simultaneous suites. It installs the APK, starts Android
Auto's head-unit server through its Settings UI if needed, allocates a device-scoped ADB forward,
and starts a private daemon. This does not replace an existing physical-phone session. Cleanup
removes the forward and private daemon. A standalone suite stops the emulator it started; an
all-suites run retains that emulator until the entire run ends. Pre-existing emulators stay running.
The AVD's user data is preserved: this is not a fresh device image or a production backend test.

Pass `--usb` to use a physical phone instead of `environment.emulator`:

```bash
aa test e2e/suite.yaml --usb
aa test sample --usb
```

Connect exactly one USB phone, enable USB debugging, authorize the computer, and provision Android
Auto development mode and the native head-unit credentials. The runner ignores emulators and
wireless ADB devices; missing, unauthorized, offline or multiple USB phones produce an error.
The same APK installation, permissions, services, tests and reports apply. USB projection bypasses
the emulator's head-unit server restart and TCP forward. The runner waits for ADB to return after
USB accessory negotiation, then sets up reverse tunnels and captures logcat. Cleanup removes owned
tunnels before stopping projection, and never starts or shuts down an emulator. `--validate --usb`
only validates the files and needs no connected phone.


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

Suites default to `build/test-results`, replaced on each run, with HTML, JSON and JUnit reports.
Set top-level `artifactsDir: ../build/test-results` in a suite YAML to choose its base directory
relative to that YAML file. Each run replaces the contents there. An explicit
`--artifacts-dir` (or configured `test.artifacts_dir`) overrides this with the exact directory supplied.
The included Android sample uses this setting to keep reports in `android-sample/build/test-results/`.

Suites also save `suite.json`, `logcat.txt`, `head-unit.log`, and `ui/` screenshots, scaled images, OCR TSVs and
OCR diagnostics. The private daemon authentication file is removed rather than archived. Inspect
these artifacts for setup and assertion failures; screenshots alone do not make a test pass.

Managed suites monitor the selected package (including its subprocesses) for fatal Java/Kotlin
exceptions, native crashes, and ANRs. An unexpected crash fails the affected test with exit code 7,
even if the app recovers and its UI assertions succeed. Reports include an `app.health` failure
with the timestamp and logcat excerpt. Losing logcat access or the collector produces an error,
never a passing health check. Each test finishes with a logcat checkpoint before its result is
finalized, so pending records are included.

The force-stop before each test and explicit `app.restart` actions are allowed; ordinary error
logs and crashes in unrelated apps are ignored. Crash detection remains active during relaunch.
This monitoring applies to managed suites; individual YAML files run against an existing session
have no suite package or device context and do not receive this guarantee.

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
  preferences:
    file: APPLICATION_SHARED_PREFERENCES
    booleans: {EnableSomeDeveloperFlag: true}
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
`OPENDHU_ARTIFACTS_DIR` so fixture code can create run-specific state there. openDHU waits for each
loopback `readyPort`, fails if the port is occupied or leased by another suite, and stops owned
process groups on completion, startup failure or interruption. Service readiness should be exposed
only after dependencies are initialized; tests still assert actual backend responses.

The suite installs the APK, grants the declared runtime permissions, acquires device-to-host ADB
reverse mappings without replacing existing ones, and optionally opens `launchActivity` once before
projection. It removes its own reverse mappings during cleanup. These operations affect the dedicated
test AVD only. They do not clear application data.

`preferences` sets boolean entries in one of the app's SharedPreferences files (`file` is the name
without `.xml`) after install and before launch, so a journey can turn on a developer flag instead of
relying on a manually provisioned AVD. The app is stopped first, other entries in the file are kept,
and the file is written through `run-as`, so the app must be a debuggable build. Like other
environment fields, a child suite's `preferences` replaces the inherited one completely; a shared
base can set a flag to `false` so every suite starts from a known state.

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
  text: openDHU Driver
  timeout: 15s
- action: http.request
  method: GET
  url: http://127.0.0.1:56551/assertions/driver-selected
  expect: {status: 200, json: {completed: true}}
  timeout: 15s
- action: app.restart
- action: ui.expect
  text: openDHU Driver
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
