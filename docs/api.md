# HTTP API (v1)

The AutoLab daemon owns the head unit session and serves this API on `127.0.0.1`. The CLI uses exactly this
API, and so should anything else (a GUI, a CI script). There is no other way to reach the session.

This is a pre-1.0 API. The `/v1` prefix reserves room for a breaking change later; until then, small changes
can still happen. There is no changelog yet, so check the commit history.

## Running the daemon

```bash
autolab start            # starts a daemon if none is running, then a session; the daemon exits when the session ends
autolab daemon           # foreground daemon that stays up; Ctrl+C or SIGTERM stops the session and exits
autolab daemon --port 9000
```

The default port is **8765**. If it is busy and no `--port` was given, the daemon uses any free port. Clients
should not assume 8765: read `daemon.json`.

## Finding and authenticating to the daemon

The daemon writes `daemon.json` into the runtime directory, `$AUTOLAB_RUNTIME_DIR` if set, otherwise
`$TMPDIR/autolab-$USER`. The directory must be a real directory with mode 0700, and the file has mode 0600:

```json
{ "port": 8765, "pid": 41022, "token": "3f…(64 hex chars)", "version": "0.1.0" }
```

Every request must carry `Authorization: Bearer <token>` and a `Host` of `127.0.0.1:<port>` or
`localhost:<port>`. A request with an `Origin` header is refused. A fresh token is generated on every daemon
start and is never logged.

```bash
DIR="${TMPDIR:-/tmp}/autolab-$USER"; DIR="${DIR%/}"
PORT=$(python3 -c "import json;print(json.load(open('$DIR/daemon.json'))['port'])")
TOKEN=$(python3 -c "import json;print(json.load(open('$DIR/daemon.json'))['token'])")
curl -s -H "Authorization: Bearer $TOKEN" http://127.0.0.1:$PORT/v1/status
```

A leftover `daemon.json` from a crashed daemon is not trusted: clients check that the daemon answers, and a
daemon that does not answer is treated as absent.

### Trust model

The listener is bound to loopback only; there is no option to bind elsewhere. Loopback is reachable by every
local user and by browsers, so the token, the `Host` check and the `Origin` check are what keep other users and
web pages out, and the 0700 directory is what keeps the token private.

**Anyone who can read `daemon.json` can run code as you through this API.** `POST /v1/session/start` launches an
executable (`dhuPath`), and `POST /v1/screenshot` writes a PNG to a path of the caller's choosing. That is the same
power as running `autolab` yourself. Do not copy the token somewhere less private than the runtime directory.

## Errors

Every error response, including unknown routes, wrong methods and malformed JSON, has this shape:

```json
{ "error": { "code": "unsupported", "message": "audio focus is not implemented yet", "exitCode": 5 } }
```

`message` is the bare message. `exitCode` is the CLI exit code the error maps to, so a client can reproduce it
exactly.

| `code` | HTTP | `exitCode` | Meaning |
|---|---|---|---|
| `invalid_args` | 400 | 2 | malformed JSON, unknown or missing field, bad value |
| `unauthorized` | 401 | 1 | missing or wrong token |
| `forbidden` | 403 | 1 | unexpected `Host`, or an `Origin` header |
| `not_found` | 404 | 1 | no such endpoint |
| `method_not_allowed` | 405 | 1 | the endpoint exists but not for that method |
| `session_conflict` | 409 | 8 | a session is already running, starting or stopping |
| `error` | 500 | 1 | anything else |
| `unsupported` | 501 | 5 | the endpoint exists in the PRD but is not implemented |
| `backend_unavailable` | 503 | 3 | no session, session not ready, DHU or adb missing |
| `device_disconnected` | 503 | 4 | no device, or a tunnel could not be set up |
| `timeout` | 504 | 6 | a bounded wait expired |

Authentication is checked before routing, so an unauthenticated client cannot probe which routes exist.

## Endpoints

Request bodies are JSON and reject unknown fields. Keys are camelCase.

| Method and path | Purpose |
|---|---|
| `GET /v1/status` | daemon info and the session, if any |
| `GET /v1/capabilities` | what the backend supports (needs a running session) |
| `GET /v1/devices` | adb devices: serial, state, model |
| `POST /v1/session/start` | start a session; returns `201` with the status |
| `POST /v1/session/stop` | stop the session; returns `{"stopped": true}` |
| `POST /v1/focus/video` | `{"enabled": true\|false}` |
| `POST /v1/screenshot` | `{"path": "/abs/file.png"}` |
| `GET /v1/events` | server-sent events |
| `GET /v1/audio/{channel}` | the phone's audio on `media`, `system` or `speech` as raw PCM (native backend only) |
| `GET /v1/video` | the phone's live video as raw H.264 (native backend only) |
| `POST /v1/daemon/shutdown` | stop the session and exit the daemon |
| `POST /v1/input/tap` | `{"x": 400, "y": 240}`: touch a pixel of the screenshot (800×480, origin top left) |
| `POST /v1/input/swipe` | `{"x1":..,"y1":..,"x2":..,"y2":..,"durationMs": 300}`: drag; 50 to 10000 ms, default 300 |
| `POST /v1/focus/audio`, `/v1/focus/navigation`, `/v1/display/configure` | **not implemented**: always `501 unsupported` |

### `GET /v1/status`

```json
{
  "daemon": { "version": "0.1.0", "pid": 41022, "port": 8765, "exitWhenIdle": true },
  "session": {
    "sessionId": "790845",
    "phase": "running",
    "backend": "dhu",
    "state": "running",
    "serial": "1A141FDF600AF2",
    "backendVersion": "2.0-mac-arm64",
    "backendBuild": "2022-03-30-438482292",
    "videoFocus": { "requested": "on", "backendReported": "on", "observed": "unknown" },
    "transport": "usb",
    "daemonPid": 41022,
    "dhuPid": 41030,
    "tunnels": [ { "port": 10101, "state": "active", "reapplied": 0 } ]
  }
}
```

`session` is `null` when there is no session. While the session is starting or stopping it is only
`{"sessionId": "...", "phase": "starting"}`, with none of the detail fields: status answers immediately even
during a slow start. `phase` is the daemon's lifecycle (`starting`, `running`, `stopping`). `state` is the
backend link: `running` means the DHU finished its handshake with the phone, not that projection is visible.

`videoFocus.requested` is what AutoLab last asked for. `backendReported` is the DHU's own readback.
`observed` is a phone-side confirmation. With the DHU it is **always `unknown`**: the DHU gives no such signal. With the
native backend it is what the phone itself last reported (`on` after its video start message, `off` after its stop
message). Unknown values are explicit and are never guessed.

Each entry in `tunnels` is checked live against `adb reverse --list` (`active`, `missing`, or `unknown` if adb
did not answer), and `reapplied` counts how often AutoLab had to put it back.

### `POST /v1/session/start`

```json
{ "backend": "dhu", "serial": "1A141FDF600AF2", "dhuPath": "/path/to/desktop-head-unit",
  "config": "/path/to/dhu.ini", "headless": true, "reverse": [10101] }
```

Every field is optional. `backend` is `dhu` (Google's DHU, the default) or `native` (AutoLab's own head unit, see
[native-backend.md](native-backend.md)). `dhuPath`, `config` and `headless` belong to the DHU; sending them with
`native` is a `400 invalid_args`. Without `serial`, the only authorized adb device is used (waiting up to 10 s for it to
appear), and several devices is an error rather than a guess. Without `dhuPath`, the DHU is found through
`AUTOLAB_DHU_PATH` and the Android SDK; an explicit path that does not exist is an error, never a silent fallback.
`reverse` lists ports to keep reachable on the phone with `adb reverse tcp:PORT tcp:PORT`. A tunnel counts only
once `adb reverse --list` shows it; otherwise the whole start fails and the DHU is stopped. Relative paths are
resolved against the daemon's working directory, so send absolute ones (the CLI does).

The call returns when the session is running, which takes about 5 s, and up to roughly 85 s in the worst case (10 s to find the device, 45 s for the DHU
handshake, 30 s for the phone to return to adb before tunnels).
A second `start` while a session is `starting`, `running` or `stopping` gets `409 session_conflict`.

### `POST /v1/focus/video`

```json
{ "operationId": "op-1", "backend": "dhu", "status": "accepted", "requested": "off",
  "backendReported": "off", "observed": "unknown", "evidence": [] }
```

With the DHU, `status: "accepted"` means the DHU took the command. It does **not** mean the phone acknowledged it,
and `observed` stays `unknown`. With `backend: "native"` the phone itself reports: `status: "observed"` means the
phone sent the matching video start or stop message (`observed` is then `on` or `off`), and `status: "timed_out"`
(HTTP 200, CLI exit code 6) means it did not within 4 s. A rejected command comes back with `status: "failed"` and the DHU's own words in
`evidence`. Operation ids (`op-1`, `op-2`, …) are unique within a daemon and are shared by focus and screenshot.

(The PRD sketches `requestedState` and `observedState`. This API uses `requested`, `backendReported` and
`observed` to match the CLI's JSON and to keep the DHU's readback visible.)

### `POST /v1/screenshot`

```json
{ "operationId": "op-3", "path": "/tmp/out.png", "width": 800, "height": 480 }
```

`path` must be absolute, end in `.png`, contain no whitespace (the DHU console cannot take it safely), and sit in
an existing directory. Any older file is removed first and the response describes the file actually written. The
DHU refuses screenshots while video focus is off, which comes back as an error carrying the DHU's message.

### `GET /v1/capabilities`

```json
{ "backend": "dhu", "videoFocus": "supported", "screenshots": "supported",
  "audioFocus": "unverified", "navFocus": "unverified", "nightMode": "unverified",
  "touchInput": "unverified", "screenRecording": "unsupported" }  // DHU; native reports touchInput "supported"
```

`supported` was verified on real hardware. `unverified` means the backend may offer it (the DHU console has the
command) but AutoLab has not exercised it, so the matching endpoint returns `501`. `unsupported` means the
backend has no way to do it. Requires a running session, because it asks the backend.

### Live video: `GET /v1/video`

A never-ending response (`video/h264`) of Annex-B H.264, exactly as the phone sends it: 800×480, about 45 frames per
second while the screen moves. The first bytes are the parameter sets (SPS, PPS) and the last keyframe, so a viewer
can start decoding at once; after that come the live frames. It needs the native backend and a running session:
`503` with no session, `501 unsupported` with the DHU (which draws its own window).

```bash
curl -N -H "Authorization: Bearer $TOKEN" http://127.0.0.1:$PORT/v1/video | ffplay -f h264 -i -
```

Several viewers may watch at once. The head unit streams only while at least one does. A still screen sends no
frames, so the daemon sends a filler NAL unit (type 12, which decoders skip) about once a second, which lets a
client tell "quiet" from "dead". A viewer that falls too far behind is disconnected rather than sent a corrupt
picture, and can reconnect. The stream ends when the session ends.

### Audio: `GET /v1/audio/{channel}`

Raw 16-bit little-endian PCM from the phone, one channel per request: `media` (music, 48 kHz stereo), `system`
(navigation prompts and notification sounds, 16 kHz mono) or `speech` (the assistant, 16 kHz mono). The format is also
in the `x-audio-format` header (`s16le;rate=48000;channels=2`). The phone sends nothing while a channel is silent, so
the response may sit quiet for a long time. Native backend only: `501` with the DHU (which plays the sound itself),
`503` with no session, `400` for an unknown channel. The head unit streams a channel only while someone listens.

```bash
curl -N -H "Authorization: Bearer $TOKEN" http://127.0.0.1:$PORT/v1/audio/media | ffplay -f s16le -ar 48000 -ac 2 -nodisp -i -
autolab audio out.wav --seconds 10        # record a channel to a WAV and print how loud it was
```

### Touch: `POST /v1/input/tap` and `/v1/input/swipe`

```json
{ "operationId": "op-4", "backend": "native", "status": "accepted",
  "evidence": ["sent tap at (764, 348)", "25 video frames arrived during and after it (context only: ...)"] }
```

Only the native backend can touch; the DHU answers `501 unsupported`. Coordinates are pixels of a screenshot, and a
point outside 800×480 is `400`. Touch is refused (`500`-class `error`, CLI exit code 1) while the phone is not
projecting. The best status is `accepted`: **the phone never acknowledges touch**, so whether anything on the
screen reacted is for the caller to check, for example by comparing screenshots. The frame count in `evidence` is
context, not proof. Events: `input.requested`, then `input.accepted` or `input.failed` (`kind` is `tap` or `swipe`).

## Events

`GET /v1/events` is a server-sent event stream. The first event is `stream.opened`; once you have it, the
subscription is live and nothing emitted afterwards is missed. Each later event looks like:

```text
id: 7
event: focus.video.accepted
data: {"id":7,"type":"focus.video.accepted","timestamp":"2026-10-15T15:56:06Z","sessionId":"790845","data":{"operationId":"op-1","requested":"off","backendReported":"off","evidence":[]}}
```

`id` increases within a daemon. There is no replay: a new subscriber sees only events from now on, and a
keepalive comment is sent every 15 s. A subscriber that falls too far behind gets `stream.lagged`
(`{"missed": n}`) instead of slowing the daemon down.

| Type | Meaning |
|---|---|
| `session.starting` / `session.started` / `session.start_failed` | session lifecycle; failure carries `message` |
| `session.stopping` / `session.stopped` | a requested stop |
| `session.ended` | the DHU died on its own; `reason` says how |
| `focus.video.requested` | **AutoLab issued the command** (`operationId`, `enabled`) |
| `focus.video.accepted` / `focus.video.failed` | **what the backend answered** (`requested`, `backendReported`, `observed`, `evidence`). `accepted` includes a native-backend change the phone confirmed; a `timed_out` one is `failed` |
| `screenshot.requested` / `screenshot.saved` / `screenshot.failed` | same split for screenshots |
| `input.requested` / `input.accepted` / `input.failed` | touch, same split (`operationId`, `kind`, `at`, `evidence`) |
| `tunnel.reapplied` | a dropped `adb reverse` tunnel was put back (`port`) |

`requested` events are commands AutoLab issued; `accepted` and `failed` events report the backend's answer. No
event is named `confirmed` or `observed`: the `observed` field inside an event carries what the phone said, and is
`unknown` with the DHU.

When the daemon shuts down, every event already emitted is delivered and then the stream ends, so a subscriber
sees the final `session.stopped`.

## Concurrency and lifecycle

- **One session**, one backend. Commands are serialized on the DHU console, so concurrent clients cannot interleave
  on it. Twenty parallel focus requests all succeed and leave AutoLab's idea of the state equal to the DHU's.
- **Two clients starting at once:** exactly one gets `201`, the other `409`.
- **A daemon started by `autolab start`** exits when its session ends (`stop`, a failed start, or the DHU dying).
  **A daemon you started with `autolab daemon`** stays up and can host another session.
  A daemon started by `autolab start` also exits after 30 s with no session at all, for example when the CLI that
  spawned it died before sending `start`, so it cannot linger.
- **No orphans:** the daemon owns the DHU's stdin, and the DHU quits when it closes. If the daemon is killed with
  `SIGKILL`, the DHU still exits. SIGTERM and Ctrl+C stop the session and remove `daemon.json`.
- **Logs:** one JSON object per line on stdout (`daemon.log` for a daemon started by `autolab start`). They
  include one `http.request` line per request, with method, path, status and duration, and never the token or
  the `Authorization` header.

## Not in this API

HTTP is loopback only, with no TLS and no remote access. There is no event replay, no pagination, and no
per-client sessions. Audio and navigation focus, driving restrictions, night mode, multi-touch and key input, display
configuration and screen recording are not implemented.
