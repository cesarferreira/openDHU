# AutoLab

Programmable control of Google's Android Auto Desktop Head Unit (DHU). Early proof of concept: it
toggles video focus on a real DHU session from the command line.

Status and verified behaviour: [docs/feasibility.md](docs/feasibility.md).

## Requirements

- macOS (Linux is untested), Rust 1.87+
- The DHU from the Android SDK Manager (`extras;google;auto`). AutoLab never bundles it.
- A phone with Android Auto, connected by USB and authorized for adb.

## Quick start

```bash
cargo install --path crates/autolab-cli
autolab doctor                  # read-only environment check
autolab start                   # starts the AutoLab daemon if needed, then a DHU session over USB
                                # add --reverse 10101 to re-apply an adb reverse tunnel (repeatable)
autolab focus video off
autolab focus video on
autolab screenshot out.png   # needs video focus on
autolab test examples/video-focus.yaml   # YAML tests against the running session; see docs/testing.md
autolab status --json
autolab stop
```

The CLI talks to a local daemon over an HTTP API on `127.0.0.1`, and so can any other client; see
[docs/api.md](docs/api.md) (`autolab daemon` runs one in the foreground). A bearer token in a 0700 directory guards
it, and anyone who can read that token can run code as you through the API.

`autolab focus video off` prints `Video focus off command accepted`. That means the DHU took the
command. It does not mean the phone acknowledged it. The output shows the DHU's own readback separately,
and `Phone confirmed` stays `unknown` because no phone-side check is implemented yet.

Exit codes: 0 success, 1 general error, 2 invalid arguments, 3 backend unavailable, 4 device disconnected,
5 unsupported, 6 timeout, 7 assertion failure, 8 session conflict, 130 interrupted.

## Known limitations

- **AutoLab can only control a DHU that it started.** A DHU launched from another terminal or script
  takes commands only on its own stdin, and nothing else can reach it.
- Starting a session re-enumerates the phone over USB, which drops `adb reverse`/`forward` tunnels. Pass
  `--reverse PORT` to `autolab start` to restore the ones you need; AutoLab also re-applies them if the phone drops
  them later in the session. Only `adb reverse` is supported.
- USB transport only. Only one DHU can hold a phone at a time, so stop other DHUs first.
- Video focus, screenshots, the YAML test runner and the HTTP API are implemented. Night mode, driving restrictions,
  audio and nav focus are not; their API endpoints answer `501`.
- A passing `focus.video` step means the command was accepted, not that the phone confirmed it. Ctrl+C during
  `autolab test` stops cleanly (focus restored, artifacts written, exit 130); a second Ctrl+C exits at once without
  cleanup (details in docs/testing.md).
- Tested with DHU 2.0 (build 2022-03-30) and Android Auto 18.0.164204 on a Pixel 6.

## Tests

```bash
cargo test --workspace          # no hardware needed (uses a scripted fake DHU)
AUTOLAB_HW=1 cargo test -p autolab-cli --test hardware -- --ignored --nocapture   # real phone + DHU
```
