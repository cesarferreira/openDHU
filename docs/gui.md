# Desktop window (`aa gui`)

`opendhu-gui` is a native window for the openDHU daemon. Like the CLI, it uses the HTTP API in
[api.md](api.md): it reads `daemon.json` for the port and token, and sends no `Origin` header.

```bash
aa gui                          # after make install
cargo run -p opendhu-gui          # from a checkout
cargo install --path crates/opendhu-gui --locked
```

`aa gui` finds `opendhu-gui` next to the CLI or on `PATH`.

## Connect

The connection bar shows the selected phone, connection state, **Settings**, and **Start** / **Stop**.
The device picker discovers phones through adb even before a daemon is running. Unauthorized and offline
phones remain visible but cannot be selected. **Automatic device** lets `aa start` choose the only authorized
phone. A running session shows its actual phone; stop it before choosing another device.

With no session, the window shows a single connection panel. **Start session** runs `aa start --backend native`
(or `aa start --dhu` for Google DHU), so daemon startup and your [config file](configuration.md) behave as they
do in the terminal. `aa` must sit next to `opendhu-gui` or be on `PATH`.

**Settings** holds the head-unit choice, optional device serial, reverse ports, and Google DHU's headless option.
Session settings are disabled while a session is active. The screenshot folder can be changed at any time.

Selecting an SDK emulator with the native backend automatically restarts Android Auto's developer head-unit
server and creates an ephemeral adb TCP forward. The session owns that forward and removes it on Stop or
startup failure. The emulator must already have Android Auto developer mode and initial projection consent
provisioned, as it does for test suites. Physical phones continue to use USB.

## Live display and controls

The native backend's projected Android Auto display fills the main area. It fits both window dimensions,
keeps the negotiated aspect ratio, and stays above the controls without scrolling. It is the head unit's
screen, as a car would show it. **Click** to tap; **drag** to swipe. The swipe takes as long as the drag
(50 ms to 2 s), and coordinates are mapped back to the negotiated screen resolution.

The picture comes from `GET /v1/video` and is decoded by `ffmpeg`, which must be installed. Google DHU has
its own display window; the viewer explains this instead of showing a second projection.

The lower toolbar has:

- **Home** and **Back**, sent as car-button key presses to the phone.
- **Audio**, off by default, to play the phone's audio through `ffplay`. Audio can still be muted when video
  focus is off.
- **Screenshot**, which saves a uniquely named PNG in the chosen folder. **Open screenshot** appears in
  the status bar after a successful capture. The folder must exist, be absolute, and contain no spaces.
- **Fullscreen**; press **Escape** to return to the window.
- **More**, for pausing the display, long button presses, video focus, directional buttons, and media keys.

When the phone releases video focus, the old picture is dimmed, touch is disabled, and **Resume projection**
requests focus again. Stream failures also dim the old picture and disable touch. Car buttons are experimental:
the phone may ignore them.

The screenshot folder is remembered when Settings is closed, in `$XDG_CONFIG_HOME/opendhu/gui.json` or
`~/.config/opendhu/gui.json`. Screenshot filenames include a timestamp so repeated captures do not overwrite
one another.

## Diagnostics

**Diagnostics** opens a resizable drawer beneath the controls. It is closed by default and contains session
and backend details, certificate source, process IDs, reverse-tunnel state, video-focus observations, stream
and audio counters, the last screenshot, and live events. Events from before the window connected are not
shown because the API has no replay.

Errors and stream/tunnel problems appear in the status bar even when the drawer is closed. Open the drawer
or hover over a truncated message for details. Commands that change session state stay disabled while another
command runs; live touches do not block subsequent touches. Successful taps do not fill the status bar.

## Verification and limits

- Automated GUI tests cover startup options, fitted display geometry and scaled taps at multiple window sizes,
  stale-frame touch suppression, screenshot commands, collapsed diagnostics, tunnel failures after startup,
  and muting audio after losing video focus. Worker tests exercise a real daemon with a scripted DHU.
- The native window, Settings, diagnostics drawer, screenshot capture, Home/Back, touch, and fullscreen were
  checked with an isolated HTTP/video fixture. This validates the UI and command requests, not a physical
  phone's response. A physical-phone session was not available for this redesign.
- macOS is the supported desktop platform so far. The window uses `eframe` with the OpenGL backend.
- The GUI dependencies are separate from the `aa` binary.

## Checking how it looks

Set `OPENDHU_GUI_SNAPSHOT=/path/to/out.png` and the window saves an image of itself after about 2.5 s and quits.
It captures only the openDHU content area. `OPENDHU_GUI_SNAPSHOT_AFTER_MS` overrides the delay for a stream
that needs longer to fill.
