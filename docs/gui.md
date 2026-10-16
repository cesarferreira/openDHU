# Desktop window (`autolab-gui`)

`autolab-gui` is a native window for the AutoLab daemon. It is a client of the HTTP API in [api.md](api.md),
like the CLI. It adds no new power and needs no security change: it reads `daemon.json` for the port and the
token, and sends no `Origin` header.

```bash
cargo run -p autolab-gui          # from a checkout
cargo install --path crates/autolab-gui --locked
```

The window works with or without a session. If no daemon runs, it says so and offers **Start session**, with a
choice of head unit (Google DHU or AutoLab native). That button runs `autolab start`, so the daemon start-up and your [config file](configuration.md) behave exactly as in
the terminal. `autolab` must sit next to `autolab-gui` or be on `PATH`.

## What it shows

- **Session**: backend and version, phone serial, transport, process ids, and each `--reverse` tunnel with its live
  state. **Stop session** ends it.
- **Live view** (native backend): the head unit's display, live: the projected Android Auto screen, as a car's screen would show it (not the phone's own screen). It replaces the DHU's window. The window reads `GET /v1/video` and decodes it with
  `ffmpeg`, so `ffmpeg` must be installed. **Click** the picture to tap, **drag** to swipe; the drag takes as long as
  you took (50 ms to 2 s). Touches go to the phone as a car's touch screen would send them. Untick "Show the head unit display" to stop the
  stream. The DHU draws its own window, so there is no live view for it.
- **Audio**: tick **Play the phone's audio** (off by default) to hear what the phone plays, through `ffplay`. A line under
  the picture shows how many seconds of sound have arrived on each channel.
- **Video focus**: what AutoLab requested, what the head unit reports, and the phone-side value. With the DHU that
  is always `unknown`. With the native backend it is the phone's own word. **Focus on** and **Focus off** send the command.
- **Screenshot**: saves a PNG to the path you type (absolute, ends in `.png`, no spaces) and previews it. On the
  native backend, **clicking the preview taps the phone at that spot**. It is a real touch on the real phone.
- **Events**: the daemon's event stream, live. Events from before the window connected are not shown, because the
  API has no replay.

Buttons are disabled while a command runs and when the session cannot take them (for example Focus before the
session is running). Errors from the daemon appear in the bar at the bottom, in the daemon's words.

## Limits

- Clicking in the real window was checked against the daemon's request log: Start, Take screenshot, Focus on and off,
  and Stop all produced the matching requests. **Not confirmed in the real window:** a click on the preview (tap) never
  reached the daemon in the runs I watched, and I could not tell from the log whether the sessions were native or
  the DHU. Both are covered by headless tests that drive the window with synthetic clicks, which is weaker evidence.
  Taps themselves were verified on the phone from the CLI.
- macOS only so far (that is where it was built and run). It uses `eframe` with the OpenGL backend.
- A screenshot preview shows whatever is on the phone's screen. Keep saved screenshots out of version control.
- The GUI adds about 110 crates to the workspace build. The `autolab` binary does not depend on it.

## Checking how it looks

Set `AUTOLAB_GUI_SNAPSHOT=/path/to/out.png` and the window saves an image of itself after about 2.5 s and quits.
It captures only the AutoLab window, never the rest of the desktop.
