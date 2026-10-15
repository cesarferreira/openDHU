# Writing and running tests

```bash
autolab start --reverse 10101          # a session must be running (the runner never starts one)
autolab test examples/video-focus.yaml
autolab test tests/ --report junit --report json --artifacts-dir ./test-results
```

`autolab test` takes files or directories (searched recursively for `.yaml` / `.yml`, in sorted order).
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
  observed: on               # confirmed by the phone (never available yet, see below)
```

## What a pass does and does not mean

- `focus.video` passing means the command was **accepted** and the DHU's readback agrees. It does not mean the
  phone confirmed anything. The step's detail says so every time ("not confirmed by the phone").
- `unknown` never satisfies an expectation, and `unknown` cannot be written in a test. Phone-side confirmation is
  not implemented yet, so `observed` is always unknown today. An assertion on `observed` fails, by design, rather
  than passing on nothing.
- The PRD's recovery example checks commands and a screenshot. It does not check that an app recovered.
  That needs an app-level assertion, which does not exist yet.
- The DHU refuses screenshots while video focus is off, so take screenshots with focus on.

## Isolation

After every test, including failed ones, the runner restores video focus to `on` if the test may have left it
off, and records that as a `cleanup.focus.video` step. If the restore fails, the test fails. A test does not
assume the next one starts clean in any other way: nothing on the phone is reset.

If you press Ctrl+C, only `autolab test` dies. The session survives, **video focus may be left off**, and no
artifacts are written for the interrupted run. Run `autolab focus video on` to put it back. Clean handling
needs a signal-handling dependency that has not been added.

## Results and exit codes

A run stops at the first step that does not pass; later steps are reported as skipped. A test is `passed`,
`failed` (an assertion or wait did not hold), `error` (something else went wrong, such as the session being
gone, a rejected command or a failed cleanup) or `skipped`.

| Code | Meaning in `autolab test` |
|---|---|
| 0 | every test passed (skipped tests do not fail a run) |
| 2 | invalid test file or arguments, or a test written for another backend |
| 3 | no running session, or the backend is unavailable |
| 5 | a required capability is unsupported (use `--skip-unsupported` to skip such tests instead) |
| 6 | a `waitFor` timed out |
| 7 | an assertion failed |
| 1 | any other error |

`autolab start` can also return 4 (a device or tunnel problem). `autolab test` cannot: it never starts a session, and a
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

Retry policies, image assertions, tap/swipe/key steps, event assertions, a Kotlin DSL and clean Ctrl+C handling.
