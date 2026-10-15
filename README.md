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
autolab start                   # launches the DHU over USB and keeps it under AutoLab's control
autolab focus video off
autolab focus video on
autolab status --json
autolab stop
```

`autolab focus video off` prints `Video focus off command accepted`. That means the DHU took the
command. It does not mean the phone acknowledged it. The output shows the DHU's own readback separately,
and `Phone confirmed` stays `unknown` because no phone-side check is implemented yet.

Exit codes: 0 success, 1 general error, 2 invalid arguments, 3 backend unavailable, 5 unsupported,
6 timeout, 8 session conflict.

## Known limitations

- **AutoLab can only control a DHU that it started.** A DHU launched from another terminal or script
  takes commands only on its own stdin, and nothing else can reach it.
- Starting a session re-enumerates the phone over USB, which drops `adb reverse`/`forward` tunnels. Re-apply any
  tunnel your app needs after `autolab start`.
- USB transport only. Only one DHU can hold a phone at a time, so stop other DHUs first.
- Only video focus is implemented. Night mode, driving restrictions, audio and nav focus, screenshots,
  the HTTP daemon and the test runner are not.
- Tested with DHU 2.0 (build 2022-03-30) and Android Auto 18.0.164204 on a Pixel 6.

## Tests

```bash
cargo test --workspace          # no hardware needed (uses a scripted fake DHU)
AUTOLAB_HW=1 cargo test -p autolab-cli --test hardware -- --ignored --nocapture   # real phone + DHU
```
