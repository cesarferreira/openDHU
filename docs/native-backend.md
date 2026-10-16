# Native backend (`--backend native`)

AutoLab can act as the Android Auto head unit itself, so Google's DHU is not needed.

```bash
cargo build -p autolab-native-hu     # builds next to `aa`
brew install ffmpeg                  # decodes the phone's video into screenshots
aa start --backend native
```

`aa start` returns once the phone is projecting **and** a first complete picture has arrived (about 6 s), so a
screenshot straight afterwards works.

Everything else works the same: `focus`, `screenshot`, `status`, `test`, the HTTP API and the GUI.
`aa doctor` shows whether the head unit program and `ffmpeg` were found.

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

## Live view

The head unit has no window of its own. To watch the phone, run `aa gui` (or `make gui`): its **Live view** card
shows the head unit's display live (the projected Android Auto screen, like the DHU's window), decoded with `ffmpeg`, and a click or drag on it touches the phone. Any other client can read
the same stream from `GET /v1/video` ([api.md](api.md)). Screenshots remain the way to save a picture.

## Audio

The phone plays its sound through the head unit, and the head unit streams it to anyone who asks:

```bash
aa audio out.wav --seconds 10 [--channel media|system|speech] [--json]
```

records a channel to a WAV file and prints how much sound arrived and how loud (peak and RMS, 0 to 1). The phone sends
nothing while a channel is silent, so the file holds only the sound that played. In the window, tick **Play the
phone's audio** to hear it through this computer (`ffplay`, which comes with `ffmpeg`); it is off by default.
`GET /v1/audio/{channel}` serves the same PCM ([api.md](api.md)).

The crate we link never acknowledged the audio buffers it received, so the phone stalled after about ten of them. The
vendored copy now acknowledges each one (`vendor/android-auto/AUTOLAB-PATCHES.md`).

## Touch

```bash
aa tap 764 348                       # a pixel of a screenshot, origin top left
aa swipe 300 200 100 200 --duration-ms 400
```

Checked on a Pixel 6 projecting Google Maps, comparing screenshots as numbers (two screenshots with no touch differed
by 0.4 to 0.5% of pixels):

- a tap on the map's zoom-in button changed 34.7% of the pixels and visibly zoomed the map; a tap on zoom-out
  zoomed it back;
- a swipe of 200 pixels panned the map (9.8% changed) and collapsed the destinations panel;
- taps on Re-centre and on the panel's chevron put the view back (the rest was live map data).

Touches are refused while the phone is not projecting, and outside the 800×480 screen.

## Verified on a real phone

Pixel 6, Android 17, Android Auto 18.0.164204, macOS arm64, USB: start, six focus changes each confirmed by the
phone, status, screenshots (800×480), refusal to screenshot while the phone is not projecting, `--reverse` tunnels
(checked with `adb reverse --list`), the YAML runner with `observed` assertions, clean stop. The test is
`real_native_backend_reports_what_the_phone_did` in `crates/autolab-cli/tests/hardware.rs`.

## Debugging knobs

- `AUTOLAB_NATIVE_LOG=1` makes the head unit log the protocol crate's messages to stderr (the daemon's log).
- With custom credentials, the native backend advertises Google's developer DHU make/model identity so
  Android Auto can include debug Car App Library apps. `AUTOLAB_NATIVE_IDENTITY=autolab` opts out;
  `AUTOLAB_NATIVE_IDENTITY=dhu` forces the DHU identity even with bundled credentials. The daemon must inherit
  this setting. The legacy protocol's `head_unit_name` field is actually the vehicle make: it must be
  `Google`, not `Desktop Head Unit`. Both vehicle and head-unit models must be `Desktop Head Unit`.
- The native backend automatically reads `~/.config/autolab/credentials/head-unit-cert.pem` and
  `~/.config/autolab/credentials/head-unit-key.pem`. Both absent means the bundled certificate; an incomplete,
  unreadable, or invalid pair fails startup. Keep the key outside this repository (`chmod 600`).
- `AUTOLAB_NATIVE_CERT` and `AUTOLAB_NATIVE_KEY` override those paths. Set both or neither, in the environment
  of the daemon that starts the head unit. Restart the session after changing files, or the daemon after
  changing these variables. `aa status` and the GUI show `custom` when your pair is loaded.

## Limits

- **Car buttons are sent but not proven.** `aa key home|back|up|down|...` sends Android key presses, and the phone binds all 13 buttons we declare (it asks for exactly those). But on a Pixel 6 none of Home, Back, the arrows or a long press visibly changed the screen in my tests, so `keyInput` stays `unverified` and the key YAML step cannot be required. Treat buttons as experimental. The cause is unknown: either a detail of the key message that I have wrong, or Android Auto ignoring them on a touch head unit.
- **Audio, navigation, night mode, driving restrictions and multi-touch are not implemented.** The head unit
  answers the phone's sensor requests with fixed values (daytime, unrestricted). Those capabilities stay `unverified`.
- **Touch is fire-and-forget.** `aa tap X Y` and `aa swipe X1 Y1 X2 Y2` send single-finger touches in
  screenshot pixels, at whatever resolution the session is running (800×480 by default). The phone
  never acknowledges them, so the result is `accepted` at best.
- **One phone.** The head unit takes the Android phone it finds on USB. With several attached, `start` refuses
  rather than guess.
- **Screenshots and the live view need `ffmpeg`**, and a keyframe: if the phone has sent none since projection started, the
  screenshot fails and says so.
- **USB or forwarded TCP.** Emulator head-unit servers can use loopback TCP (see below). Wireless Bluetooth pairing is not wired up.
- **Resolution** defaults to 800×480 (480p). `AUTOLAB_NATIVE_RESOLUTION=720p` or `=1080p` (set before the
  daemon starts, like the certificate variables above) asks the phone for a bigger screen instead;
  an unrecognised value falls back to 480p with a warning in the head unit's log.
- **Fragile by nature.** This is an unofficial implementation of an undocumented protocol. A phone-side Android
  Auto update can break it. Run the hardware test after updating the phone.
- **Not certified.** It is a test tool, not a car head unit.

## Vendored code and licences

`vendor/android-auto` (LGPL-3.0-or-later) and `vendor/rustls-webpki` (ISC) are copies of third-party code with small
patches, listed in each directory's `AUTOLAB-PATCHES.md`. They are linked into `autolab-native-hu` only; the
`aa` binary does not contain them. Keep `vendor/android-auto` rebuildable and its licence files in place.

`vendor/android-auto/src/cert.rs` contains the head-unit client certificate and private key that the upstream crate
ships. They are the public, long-known credentials used by open-source head units, not a secret of ours, but a secret
scanner will flag them. Allow-list that file.

Check that the patched `rustls-webpki` is really in use with `cargo tree -i rustls-webpki`: it must print a
`vendor/rustls-webpki` path. Otherwise the phone's X.509 v1 root certificate is rejected and the handshake fails.

## ADB-forwarded TCP transport

To connect an emulator or a phone's Android Auto head-unit server without USB accessory mode:

```bash
adb -s emulator-5554 forward tcp:15277 tcp:5277
aa start --backend native --serial emulator-5554 --native-tcp 127.0.0.1:15277
```

The server must be started in Android Auto's developer settings first. The TCP endpoint must be
loopback; it is scoped to that session and uses the same TLS authentication and protocol as USB.
`aa status` reports `Transport: tcp`. Remove manually created forwards when finished. A YAML suite
manages the server, an ephemeral forward and its isolated session automatically; see [testing](testing.md).
