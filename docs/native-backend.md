# Native backend (`--backend native`)

AutoLab can act as the Android Auto head unit itself, so Google's DHU is not needed.

```bash
cargo build -p autolab-native-hu     # builds next to `autolab`
brew install ffmpeg                  # decodes the phone's video into screenshots
autolab start --backend native
```

Everything else works the same: `focus`, `screenshot`, `status`, `test`, the HTTP API and the GUI.
`autolab doctor` shows whether the head unit program and `ffmpeg` were found.

## What is different from the DHU

| | DHU | native |
|---|---|---|
| Video focus | the DHU takes the command (`accepted`) | the head unit tells the phone, then waits for the phone's own start or stop message (`observed`, or `timed_out` after 4 s) |
| `observed` | always `unknown` | the phone's own word: `on` after its video start message, `off` after its stop message |
| Screenshot | the DHU writes a PNG | the last picture is decoded from the phone's H.264 video with `ffmpeg` |
| Needs | Google's DHU | `autolab-native-hu`, `ffmpeg` |

Tests that need the phone's word use `backend: native` and assert `observed`
([examples/native-phone-confirms.yaml](../examples/native-phone-confirms.yaml)).

## How it works

`autolab-native` (a library) drives `autolab-native-hu` (a child process) with JSON lines on stdin and stdout.
The child speaks the Android Auto protocol over USB, using a vendored copy of the open-source
[`android-auto`](https://github.com/uglyoldbob/android-auto) crate. It runs as a separate process for the same
reasons the DHU does: the crate panics on messages it does not handle, and the child exits when the daemon closes
its stdin, so a killed daemon leaves no head unit behind.

Release of video focus is a `VideoFocusIndication` the head unit sends on its own initiative. Measured on a Pixel 6,
the phone answers a release with a stop message after about 0.5 s and frames stop; it answers a take-back with a
start message within milliseconds and frames resume.

## Verified on a real phone

Pixel 6, Android 17, Android Auto 18.0.164204, macOS arm64, USB: start, six focus changes each confirmed by the
phone, status, screenshots (800×480), refusal to screenshot while the phone is not projecting, `--reverse` tunnels
(checked with `adb reverse --list`), the YAML runner with `observed` assertions, clean stop. The test is
`real_native_backend_reports_what_the_phone_did` in `crates/autolab-cli/tests/hardware.rs`.

## Limits

- **Touch, audio, navigation, night mode and driving restrictions are not implemented.** The head unit answers the
  phone's sensor requests with fixed values (daytime, unrestricted) and declares a touch screen it never sends
  events from. Those capabilities stay `unverified`.
- **One phone.** The head unit takes the Android phone it finds on USB. With several attached, `start` refuses
  rather than guess.
- **Screenshots need `ffmpeg`**, and a keyframe: if the phone has sent none since projection started, the
  screenshot fails and says so.
- **USB only.** Wireless Android Auto is not wired up.
- **Resolution** is fixed at 800×480 (480p).
- **Fragile by nature.** This is an unofficial implementation of an undocumented protocol. A phone-side Android
  Auto update can break it. Run the hardware test after updating the phone.
- **Not certified.** It is a test tool, not a car head unit.

## Vendored code and licences

`vendor/android-auto` (LGPL-3.0-or-later) and `vendor/rustls-webpki` (ISC) are copies of third-party code with small
patches, listed in each directory's `AUTOLAB-PATCHES.md`. They are linked into `autolab-native-hu` only; the
`autolab` binary does not contain them. Keep `vendor/android-auto` rebuildable and its licence files in place.

`vendor/android-auto/src/cert.rs` contains the head-unit client certificate and private key that the upstream crate
ships. They are the public, long-known credentials used by open-source head units, not a secret of ours, but a secret
scanner will flag them. Allow-list that file.

Check that the patched `rustls-webpki` is really in use with `cargo tree -i rustls-webpki`: it must print a
`vendor/rustls-webpki` path. Otherwise the phone's X.509 v1 root certificate is rejected and the handshake fails.
