# Architecture

```
aa (CLI) ──HTTP, 127.0.0.1──▶ aa daemon ──stdin/stderr──▶ desktop-head-unit ──USB──▶ phone
 GUI / curl  ──────────────────────▲   (owns the session)               (Google's DHU)
```

One daemon owns one head unit session. Every client uses the same HTTP API, including the CLI. The
daemon is the only process that talks to the DHU.

## Crates

| Crate | Job |
|---|---|
| `opendhu-core` | The `HeadUnitBackend` trait, shared types (`HeadUnitStatus`, `OperationResult`, `Capabilities`), error type and exit codes. No I/O. |
| `opendhu-dhu` | The DHU backend: finds the executable, starts it, writes console commands, parses its output, wraps `adb`. |
| `opendhu-native` | The native backend: drives `opendhu-native-hu` over JSON lines. No Android Auto code. |
| `opendhu-native-hu` | openDHU's own head unit (a child process). Links the vendored `android-auto` crate. |
| `opendhu-test` | YAML test schema, runner, JSON and JUnit reports, artifacts layout. The runner uses only the backend trait, so tests use a mock backend. |
| `opendhu-daemon` | The HTTP server, auth, session state machine, event stream, tunnel keeper, and a small client. |
| `opendhu-gui` | The `opendhu-gui` window (egui). A client of the daemon API only; see [gui.md](gui.md). |
| `opendhu-tui` | The terminal viewer, drawn with kitty graphics. A client of the daemon API only; see [tui.md](tui.md). |
| `opendhu-web` | The browser viewer: a loopback page with MJPEG video and touch. A client of the daemon API only; see [web.md](web.md). |
| `opendhu-cli` | The `aa` binary: argument parsing, config file, daemon start-up, output. |

The dependency direction is `cli, gui, tui, web → daemon`, `cli → test`, `daemon, test → dhu → core`. The CLI does not depend on the GUI. The native backend is the second `HeadUnitBackend`. The daemon uses `opendhu-native`, which starts `opendhu-native-hu` as a child; the child links `opendhu-native` only for the shared protocol types. Nothing above the trait changed.

## A request, step by step

`aa focus video off`:

1. The CLI reads `daemon.json` from the runtime directory to get the port and the token.
2. It sends `POST /v1/focus/video` with the token. The daemon checks the token, then `Host`, then `Origin`.
3. The daemon takes the session lock, so only one command runs on the DHU console at a time.
4. The DHU backend writes `focus video off` to the DHU's stdin and reads the answer.
5. The daemon emits `focus.video.requested` and `focus.video.accepted` (or `failed`) events, and returns the result
   with an operation id.

`aa session start` first launches a daemon if none answers, then sends `POST /v1/session/start`.

## State and files

- **Runtime directory** (`$OPENDHU_RUNTIME_DIR`, or `$TMPDIR/opendhu-$USER`): mode 0700. It holds `daemon.json`
  (mode 0600, with the bearer token) and `daemon.log` for an auto-started daemon.
- **Session phases**: `idle → starting → running → stopping → idle`. `status` answers in every phase.
- **No orphans**: the DHU quits when the daemon closes its stdin, so even a `SIGKILL` of the daemon leaves no DHU.

## What is real and what is not

`supported` in `/v1/capabilities` means verified on real hardware (video focus, screenshots). `unverified` means
the DHU has the command but openDHU has not tried it. `observed` (phone-side confirmation) is always `unknown`.
See [feasibility.md](feasibility.md) for evidence and [api.md](api.md) for the wire format.
