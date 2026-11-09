---
name: opendhu
description: Drive an Android Auto head unit with the `aa` CLI (openDHU). Use when asked to start or stop an Android Auto session, tap, swipe/scroll, press car keys (home, back, dial), take screenshots, record video or audio, or run Android Auto YAML test suites against a phone or emulator.
---

# openDHU (`aa`)

`aa` controls a real Android Auto head unit session. A daemon owns the session; every command talks to it.
The projected screen is **800x480 by default** (`--resolution 720p|1080p` at session start). Coordinates are
pixels of a screenshot, `0,0` top left.

## Before anything

```bash
command -v aa || echo "not installed: git clone https://github.com/cesarferreira/openDHU && cd openDHU && make install"
aa doctor            # checks adb, ffmpeg, devices, Android Auto; touches no session
aa session status --json
```

Needs a phone (USB debugging on, authorized, Android Auto set up, unlocked) or an emulator. Never start a
session if `aa session status` shows one already running: it is someone else's. Ask first.

## Loop: look, act, look

The phone never acknowledges touch, so **verify every action with a screenshot**.

```bash
aa session start                    # native backend; --serial SERIAL if several devices; --dhu for Google's DHU
aa screenshot /tmp/s0.png           # then READ the PNG to see the screen
aa tap 400 240                      # touch a pixel
aa swipe 600 300 200 300            # scroll right: drag from x1,y1 to x2,y2 (--duration-ms 50..10000, default 300)
aa key home                         # home back up down left right enter search play_pause next previous call end_call
aa key rotate_right                 # dial: focus next item; `aa key enter` presses it
aa key home --long                  # hold about one second
aa screenshot /tmp/s1.png           # confirm it worked
aa session stop                     # when finished
```

- **Scroll**: there is no scroll command. Swipe opposite to the direction content should move: to see lower
  items in a list, swipe up (e.g. `aa swipe 400 380 400 120`). Use 300-500 ms; a short fast swipe flings.
- **Find targets** by reading a screenshot and estimating pixel centres. Re-screenshot after each action since
  layout shifts. Prefer `aa key rotate_*` + `enter` when taps miss; it navigates by focus.
- `aa focus video on|off` takes or releases video focus.
- `aa record out.mp4 --seconds 10` films the screen (needs ffmpeg with libx264).
- `aa audio out.wav --seconds 5 [--channel media|system|speech]` records sound; the summary reports loudness,
  which is how to check audio without listening.
- Live viewers for a human: `aa gui`, `aa tui`, `aa web` (these are for people; agents should use screenshots).

## Output and errors

Add `--json` to any command for machine-readable output. Exit codes: 0 ok, 1 generic, 2 bad args,
3 no session or backend unavailable (run `aa session start`), 4 device disconnected, 5 unsupported (touch,
audio and record need the native backend, not `--dhu`), 6 timeout, 8 session conflict.

Touch is refused while the phone is not projecting: unlock the phone, check `aa session status`, and restart
with `aa session restart` if needed. Native key presses such as Home are also refused without video.

## Tests

```bash
aa test --list                      # registered suites in opendhu.yaml
aa test --validate path/to/tests    # parse YAML without devices
aa test sample --headless           # run a suite; reports land in ./build/test-results
```

Full docs: the repo's `docs/` (`api.md`, `testing.md`, `native-backend.md`).
