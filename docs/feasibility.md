# Milestone 0 — DHU feasibility

Status: **viable.** openDHU can control video focus on a real Android Auto session through a persistent
DHU console. The one hard constraint is that openDHU must start (and own) the DHU. It cannot attach to a
DHU that something else started.

Everything below is labelled **Observed** (seen on real hardware in this investigation), **Inferred**
(a conclusion drawn from observations) or **Not tested**. Nothing is assumed from online examples.

## 1. Environment tested

| Item | Value |
|---|---|
| Host | macOS 26.7.1, arm64 |
| DHU | `~/Library/Android/sdk/extras/google/auto/desktop-head-unit`, Mach-O arm64 |
| DHU version | `Version: 2.0-mac-arm64`, `Build: 2022-03-30-438482292` (SDK package `extras;google;auto`, revision 2.0) |
| Phone | Pixel 6, Android 17 (SDK 37), build type `user` |
| Android Auto | `com.google.android.projection.gearhead` 18.0.164204-release.daily |
| adb | 1.0.41, platform-tools 37.0.1 |
| Transport | USB (Android Open Accessory v2) |

The DHU build is from 2022 and the phone runs a recent Android Auto daily build. The handshake still
succeeds (protocol 1.7, TLS 1.2). Compatibility with other version pairs is **not tested**.

## 2. How the DHU is launched

```
desktop-head-unit [--usb=SERIAL | --adb[=HOSTPORT]] [--config=FILE.ini] [--headless] [-i touch|rotary|hybrid]
```

- **Observed:** `--usb=<serial>` works. The DHU switches the phone into accessory mode (USB product id
  `4ee7` → `2d01`), attaches, and completes the TLS handshake in roughly 5 seconds. The phone's adb
  connection drops and re-enumerates during the switch (transport id changes), so anything that watches
  the phone over adb must start after the DHU has attached.
- **Observed:** `--headless` works over USB. It attaches, answers console commands and writes screenshots.
- **Observed (ADB transport only):** started with the default ADB transport while nothing listened on the
  phone's port 5277, the DHU printed `Failed to start Google Automotive Link.` and exited 0. Exit status alone
  is therefore not a failure signal, and openDHU reads the output instead. A `--usb` run with no phone
  attached was **not tested**.
- **Not tested:** `--adb` transport. It needs the phone's "Head Unit Server" started by hand in Android Auto
  developer settings plus `adb forward tcp:5277 tcp:5277`. Port 5277 was not listening on the phone during
  this investigation. openDHU does not script taps on the phone, so this path is left alone.
- **Not tested:** `-I/--bridge` (wireless bridge), `-A/--aoa_audio_mode`.

## 3. The console

**Observed.** The DHU takes commands on stdin and prints a `> ` prompt (no trailing newline) when it is
ready for the next one.

- **Plain pipes work.** No pty is needed. Verified with piped stdin, with the DHU in windowed and in
  `--headless` mode.
- **Stream split matters.** The prompt and the banner go to **stdout**. Replies and log lines
  (`[I]:`, `[E]:`, `Verify returned: ok`) go to **stderr**. Reading them from two pipes loses their order,
  and the prompt can be read before the reply. openDHU merges both into one pipe so they arrive in write
  order. This was a real bug in the first implementation. See section 9.
- **Replies are synchronous and fast.** Every reply arrived within the same instant as the command
  (timestamps `+0.000s`), including repeated same-state commands.
- **A silent command prints only the prompt.** `focus video on|off` and `screenshot` print nothing on success.
- **Errors** look like `[E]: Invalid state 'bogus'` and `[E]: Command 'focus' was not found.`
- **Startup is idle.** Nothing is printed between handshake and the first command.

### Authoritative command list (`help`, verbatim)

```
Android Auto - Desktop Head Unit
  Build: 2022-03-30-438482292
  Version: 2.0-mac-arm64

The available commands are:
	Controller:
		"dpad up" "dpad down" "dpad left" "dpad right" "dpad ur" "dpad dl" "dpad ul" "dpad dr" "dpad click" "dpad back" "dpad soft left" "dpad soft right" "dpad 0" "dpad 1" "dpad 2" "dpad 3" "dpad 4" "dpad 5" "dpad 6" "dpad 7" "dpad 8" "dpad 9" "dpad *" "dpad #" "dpad +" "dpad rotate left" "dpad flick left" "dpad rotate right" "dpad flick right"
	Day/Night Sensor:
		"night" "day" "daynight" "nightday"
	Driving Restrictions:
		"restrict none" "restrict all"
	Focus:
		"focus audio" "focus nav" "focus video" "focus video toggle"
	Fuel:
		"fuel" "range" "lowfuel"
	Keycode:
		"keycode home" "keycode back" "keycode call" "keycode endcall" "keycode search" "keycode media_play_pause" "keycode media_next" "keycode media_previous" "keycode media_play" "keycode media_pause" "keycode media" "keycode navigation" "keycode tel"
	Microphone:
		"mic begin" "mic play" "mic repeat" "mic reject"
	Miscellaneous Sensors:
		"accel" "compass" "gyro" "location" "odometer" "speed"
	System:
		"help" "quit" "exit" "licenses" "sleep" "screenshot"
	Toll Card Sensor:
		"tollcard insert" "tollcard remove"
	Touch:
		"tap"
```

`help <command>` text, as returned by this build:

| Command | Help text |
|---|---|
| `focus video` | `focus video [lock\|off\|off-transient\|on\|unlock\|toggle]` — "Gets or sets video focus." |
| `focus audio` | `focus audio [lock\|lock-guidance-only\|lock-media-only\|off\|off-transient\|off-transient-duck\|on\|on-media\|on-transient\|on-transient-guidance\|unlock\|toggle]` — "Gets or sets audio focus." |
| `focus nav` | `focus nav [lock\|off\|on\|unlock\|toggle]` — "Gets or sets nav focus." |
| `screenshot` | `screenshot filename.png` — "Save screenshot to filename in PNG format." |
| `night` | "Force night mode." |
| `quit` | "Quit the desktop head unit." |

`help focus` and `help restrict` fail with `Command '...' was not found.` Only full command names have help.

## 4. Video focus

**Observed** on real hardware, three independent sessions plus the openDHU CLI:

```
focus video            -> [I]: Has video focus: true
focus video off        -> (prompt only)
focus video            -> [I]: Has video focus: false
focus video on         -> (prompt only)
focus video            -> [I]: Has video focus: true
focus video on         -> (prompt only)      # repeat of the current state is accepted
focus video bogus      -> [E]: Invalid state 'bogus'
```

A 12-call sequence that mixed on, off and repeated states ran against the real DHU through the openDHU
CLI, and the DHU readback matched the requested state every time.

### What is "accepted" and what is "observed"

| Signal | Source | What it proves |
|---|---|---|
| Command returns a prompt, no `[E]:` | DHU console | The DHU took the command. **Accepted.** |
| `focus video` → `Has video focus: …` | DHU console | The DHU's own idea of the state. |
| `logcat` lines on the phone | Phone | The phone changed state. **Observed.** |

**Inferred:** the `focus video` getter reports DHU-side state, not a phone acknowledgement. The DHU answers
instantly, but the phone reacts about 1.5 seconds later (below). A getter that waited for the phone would
not answer in the same instant.

**Observed** phone-side effect, from `adb logcat` started after the DHU attached, over two sessions:

| Command sent | Phone log, about 1.4–1.5 s later |
|---|---|
| `focus video off` | `CAR.GAL.VIDEO.LITE: VideoFocus lost unsolicited=true transient=false`, `CAR.VIDEO: Video focus lost. transient=false`, `CAR.SETTING: … has_video_focus`, display `com.google.android.projection.gearhead/…` → `OFF` |
| `focus video on` | `CAR.GAL.VIDEO.LITE: VideoFocus gained unsolicited=true`, `CAR.VIDEO: Video focus gained for display type: 0.`, a `c2.exynos.h264.encoder` video encoder starts, `GH.ConnLoggerV2 … CAR_SERVICE_VIDEO_FOCUS_GAINED` |

Measured delay between command and log line: 1.43 s and 1.43 s in the first session, 1.49 s and 1.51 s in the
second (the second figure includes the CLI's own startup).

These are internal Android Auto log tags from version 18.0.164204. They are **not a stable API**. They can
change with any Android Auto update, and they need adb. They are still the only phone-side signal found.
openDHU reports `observed: unknown` until a phone-side check is built on them. Do not treat the DHU
readback as proof that the phone changed state.

## 5. Screenshots

- **Observed:** `screenshot /absolute/path.png` writes a PNG and prints nothing. With `default_720p.ini`
  the file is 1280×720, 8-bit RGBA. It worked in windowed and in `--headless` mode.
- **Observed:** the windowed capture showed only the projected content, with no window chrome.
- **Not tested:** that the headless capture looks right. Its format and size were checked with `file`, and
  it was not opened. Resolution with other configs (for example a cropped 1920×1080 layout) is not tested.
- **Observed:** with video focus **off** the DHU refuses: `[E]: Don't have video focus - nothing to screenshot.`
  No file is written. A screenshot step must therefore run while video focus is on.
- **Observed:** the resolution follows the DHU config: 1280×720 with `default_720p.ini`, 800×480 with the
  built-in default.
- **Not known:** how the DHU parses a path that contains spaces. openDHU rejects such paths instead of guessing.
- Implemented as `aa screenshot PATH` (see section 10). The DHU replies before the file is complete, so
  openDHU waits for the file to appear and stop growing, then reads its size from the PNG header.

## 6. Process lifecycle

All **observed** on a live USB session:

| Event | Result |
|---|---|
| `quit` on the console | exit 0 in about 1.2 s |
| `SIGTERM` | exit 0 in about 1.1 s |
| `SIGINT` | exit 0 in about 1.1 s |
| stdin closed (EOF) | exit 0 in about 1.3 s |
| Parent process killed with `SIGKILL` | the DHU exits too (its stdin pipe closes). No orphan. |

**Inferred, not tested:** a second DHU cannot attach to the same phone while the first holds its USB
accessory link. `aa doctor` reports a `desktop-head-unit` that openDHU did not start, because that is
the likeliest cause of a failed `aa start`.

Because a closed stdin ends the DHU, the daemon that owns the pipe cannot leave an orphan behind, even if
the daemon is killed. **Tested on the phone:** unplugging the cable ends the session (see 7b). The
exit status 0 on every path means the exit code cannot tell a crash from a clean exit.

## 7. The "existing session" constraint

**Observed.** A DHU the user had started from their own terminal had
stdin, stdout and stderr attached to the user's terminal (`/dev/ttys051`). It opened no listening port and
no control socket.

A DHU has exactly one command channel: its own stdin. openDHU cannot write to another process's stdin.
The only way in would be pushing keystrokes into the user's terminal, which is fragile, depends on the
terminal, and is global desktop-style input injection that the PRD forbids.

**Consequence.** "Existing session" means a session openDHU started. `aa start` launches (if none is
running) the openDHU daemon, which owns the DHU's pipes and serves the loopback HTTP API described in
[api.md](api.md). `aa focus video on|off`, `status`, `stop` and every other command are clients of that
API. (The first version used a Unix socket owned by a supervisor process; the PRD requires the CLI and GUI to
share one API, so that socket is gone.)

Options for teams that already launch the DHU from their own script (not implemented, not tested):

1. Run the DHU through openDHU. This is what the PoC does.
2. Change the launcher to read the DHU's stdin from a named pipe that both the user's terminal and openDHU
   can write to. The existing script passes the terminal's stdin through with `<&0`.

## 7a. Side effect: USB re-enumeration and adb tunnels

- **Observed:** the first logcat capture shows the phone failing to reach `127.0.0.1:10101`
  (`TcpMetricsLogger: connect … dst=[127.0.0.1]:10101 … err=111`, connection refused), and the projected
  app showed a retry state. After the openDHU sessions, `adb reverse --list` was empty.
- **Inferred:** the accessory-mode switch re-enumerates the phone over USB, which drops `adb reverse` and
  `adb forward` tunnels. The user's own launch script says the same in a comment and re-applies
  its `adb reverse` tunnel after the switch.
- **Handled:** `aa start --reverse PORT` (repeatable) runs `adb reverse tcp:PORT tcp:PORT` once the DHU is
  attached, because the switch is what drops them. A tunnel counts only after `adb reverse --list` shows it;
  otherwise `start` fails and the DHU is stopped, so a session never looks healthy while the app cannot reach its
  backend. `status` re-checks each tunnel against `adb reverse --list` (`active`, `missing` or `unknown`). Only the
  ports asked for are added, and nothing is ever removed.
- **Verified on the phone:** after `aa start --reverse 10101`, `adb reverse --list` independently showed
  `UsbFfs tcp:10101 tcp:10101`.
- **Handled mid-session too:** a daemon thread checks `adb reverse --list` every 2 s and re-applies only the
  requested tunnels that are missing. It acts only when adb answers, so a phone that is mid-re-enumeration is left
  alone and never hammered. `status` shows how many times each tunnel was re-applied.
- **Verified on the phone:** with `--reverse 47123`, removing that tunnel with `adb reverse --remove` was repaired
  within 2 s (`status`: `active`, `reapplied: 1`; an independent `adb reverse --list` agreed). That drop was
  *simulated*. A real unplug is covered in 7b.

## 7b. Physical unplug and replug (observed on the phone)

One run, Pixel 6, session started with `--reverse 47123`, cable pulled, then plugged back in about 10 s later
(the user did it by hand, so the times are approximate):

- adb lost the device at 10:28:37 (1 s polling). The DHU exited with status 0 at 10:28:49 and the daemon
  emitted `session.ended` (`the DHU exited (exit status: 0)`). Whether the DHU noticed the unplug or the replug
  was not separated.
- The ephemeral daemon then exited and removed `daemon.json`. `aa status` returned exit code 3. No
  processes were left.
- After the replug, a fresh `aa start --reverse 47123` worked, and an independent `adb reverse --list` showed the
  tunnel.

**So a real unplug does not give the tunnel keeper a chance to act: the session is gone, not paused.** The keeper
only repairs tunnels dropped while the DHU is still alive. Nothing resumes a session automatically; start a new one.
One run only; a different cable, phone or timing could behave differently.

## 8. Capabilities (DHU 2.0 + Android Auto 18.0.164204, USB)

| Capability | Status |
|---|---|
| Video focus on/off, with DHU readback | **Verified.** Implemented in the CLI. |
| Phone-side confirmation of video focus | Signal exists (logcat), **fragile**, not implemented. |
| Screenshot to PNG | **Verified** (windowed and headless). Implemented in the CLI. Needs video focus on. |
| Persistent console, no restart per command | **Verified.** |
| Headless operation | **Verified** over USB. Still needs a real phone. |
| Day / night mode (`day`, `night`, `daynight`) | In `help`. **Not exercised.** |
| Driving restrictions | In `help` as `restrict none\|all`. **Not exercised.** |
| Audio focus, nav focus | In `help` with a wider state list than video. **Not exercised.** |
| Touch (`tap`), `dpad`, `keycode` | In `help`. **Not exercised.** Syntax of `tap` was not queried. |
| Display resolution/DPI at runtime | Set by `--config` at launch. No console command exists. |
| Screen recording | No console command exists. |
| Connection state | Partly observable (see below). |
| Device disconnect detection | **Not tested.** |
| `focus video toggle|lock|unlock|off-transient` | In `help`. **Not exercised.** |

**Connection state.** The line `Verify returned: ok` (TLS finished) proves the link to the phone is up. It does
not prove that projection is visible. adb visibility proves even less. openDHU uses the handshake line as its
"running" signal and says no more.

### Where the PRD's names do not match this DHU

- Driving state is `restrict none|all`, not `parked|moving`. FR-005 needs a mapping decision.
- There is no `restart` console command. A restart means stopping and starting the process.
- `focus` has no `help focus` entry. Each of `focus video|audio|nav` has its own help.
- Display configuration is launch-time only (`--config`). FR-011 cannot change it on a live session.

## 9. Bugs found while building the PoC

Found by running against real hardware. The first two have regression tests. For the third, only device
selection is unit-tested; the wait itself was checked by hand, not by an automated test:

1. **Discovery fell through an explicit path.** `--dhu-path /missing` quietly launched the SDK's DHU. An
   explicit path or `OPENDHU_DHU_PATH` that is unusable is now an error.
2. **Replies lost to the prompt.** Reading stdout and stderr from separate pipes dropped the DHU's reply
   intermittently, which showed up as `backend_reported: unknown` on the real device. Fixed by using
   one shared pipe. The regression test (300 consecutive readbacks) failed on the old code in 3 of 3 runs
   and passes on the fix.

3. **Start right after stop could not find the phone.** After a session ends the phone leaves accessory mode
   and is briefly invisible to adb, so `aa start` without `--serial` failed with "no device".
   Auto-selection now waits up to 10 s for exactly one authorized device. With several devices it still
   refuses to guess.

## 10. What is implemented

`aa start [--reverse PORT]… | stop | status | focus video on|off | screenshot | test | doctor`, with `--json` (camelCase keys), deterministic
exit codes (PRD 8.2) and the `HeadUnitBackend` trait in `opendhu-core`. USB transport only. `doctor` is
read-only and warns about a DHU that openDHU does not own.

`aa screenshot PATH|-o PATH` validates the path (must end in `.png`, no whitespace, existing directory),
removes any stale file, and reports the path and size of the file that was actually written.

`aa test PATH…` runs YAML tests through the backend trait (`opendhu-test` depends only on `opendhu-core`);
see [testing.md](testing.md). Verified on the phone with `examples/video-focus.yaml`, `focus-status.yaml` and
`screenshots.yaml`, and the generated JUnit passes `xmllint`. Ctrl+C and SIGTERM stop a run cleanly (focus restored, artifacts
written, exit 130); this was verified with real signals against the fake DHU and on the phone.

The daemon serves a versioned loopback HTTP API with an event stream ([api.md](api.md)), checked with real
HTTP clients (including plain `curl` against the real phone). The CLI is a client of it.

The native backend (own head unit, no DHU) is described in [native-backend.md](native-backend.md).
Not implemented: night mode, driving restrictions, audio and nav focus (their endpoints answer `501`),
`--adb` transport, device selection beyond `--serial`.

## 11. Tests and how to run them

- `cargo test --workspace` needs no hardware. It covers console parsing (using strings captured from the real
  DHU), discovery, adb parsing, exit codes, and the process and session lifecycle against a scripted fake DHU
  (`crates/opendhu-dhu/tests/fixtures/fake-dhu.sh`). The fake reproduces the real stream split.
- `OPENDHU_HW=1 cargo test -p opendhu-cli --test hardware -- --ignored --nocapture` runs the real DHU against a
  connected phone. It needs the phone unlocked with Android Auto set up and nothing else holding the phone's
  USB accessory link (stop any DHU first).

The fake DHU proves openDHU's handling of the console contract. It does not prove the real DHU. Only the
hardware test and the observations above do.

## 12. Not done in Milestone 0

- **Native receiver research** (PRD 6.4 and 15.4). Out of scope for this pass by instruction. Nothing in
  this document says anything about OpenAuto, aasdk, or protocol compatibility.
- Multiple attached devices, wireless Android Auto.
- Behaviour across other DHU or Android Auto versions.

## 13. Acceptance check

| Criterion | Result |
|---|---|
| At least one focus operation demonstrated | Yes, video focus on and off on a real Pixel 6, with phone-side evidence. |
| Backend control mechanism understood | Yes: persistent stdin console, owned by openDHU. |
| Supported capabilities recorded | Yes, section 8. |
| No unsupported functionality assumed | Unverified items are marked as such. |

**Decision:** continue to Milestone 1 on this control approach. The ownership constraint in section 7 should
be stated in the README.
