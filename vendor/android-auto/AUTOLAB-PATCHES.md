# Vendored copy: what is changed

This directory is a copy of [uglyoldbob/android-auto](https://github.com/uglyoldbob/android-auto) 0.3.8 at commit
`06c68d5ea8ac4f0ef0ab3b97dc60b9665f7074cb` (2026-09-28), licensed LGPL-3.0-or-later (`COPYING.LESSER`,
`COPYING`). AutoLab links it into `autolab-native-hu` only. Anyone who gets that binary can rebuild it against
a modified copy of this directory, which satisfies the LGPL.

Changes against upstream:

1. **Head-unit-initiated video focus** (`src/lib.rs`). Upstream can only answer the phone's own focus requests.
   Added `SendableChannelType::Video` and `AndroidAutoMessage::VideoFocus`, which sends a
   `VideoFocusIndication` on the video channel. This is how AutoLab releases and takes back video focus.
2. **Phone-side video events** (`src/lib.rs`, `src/video.rs`). Two new default methods on
   `AndroidAutoVideoChannelTrait`, `phone_video_started` and `phone_video_stopped`, are called when the phone
   sends a video `StartIndication` or `StopIndication`. Log lines (`PHONE video ...`) show the same messages,
   plus the phone's `FocusRequest`. This is what lets AutoLab report what the phone did, not only what it sent.
3. **Audio acknowledgements** (`src/mediaaudio.rs`, `src/sysaudio.rs`, `src/speechaudio.rs`). Upstream never
   acknowledges the audio buffers it receives, although the video channel does. The phone sends at most ten
   unacknowledged buffers (`max_unacked`), so audio would stall after about ten. Each handler now remembers the
   phone's session from its start message and acknowledges every buffer with it.
4. **`Cargo.toml`.** Removed the example app's dev-dependencies and the `bindgen` feature of `aws-lc-rs`,
   dropped the git `[patch]` (the workspace root carries it, pointing at `vendor/rustls-webpki`), and pinned
   `rustls` to `=0.23.37`. The `examples/` directory is not copied.

5. **ADB-forwarded TCP transport** (`src/lib.rs`). Added `AndroidAutoMainTrait::run_tcp` to connect
   to an existing head-unit server tunnel with a bounded connection timeout. It reuses the USB
   framing/TLS/discovery handlers and never enumerates or claims USB devices.

`src/cert.rs` holds the head-unit client certificate and its private key that upstream ships. They are the
public, long-known credentials from the old HeadUnit project, not secrets of ours. They are in the repository on
purpose: a secret scanner will flag them, and that finding can be allow-listed.

Known weakness: the crate panics (`todo!()`, `unimplemented!()`, `.expect()`) on messages it does not handle.
That is why AutoLab runs it in a separate child process and never inside the daemon.
