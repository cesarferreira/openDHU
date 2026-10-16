# Desktop window (`autolab-gui`)

`autolab-gui` is a native window for the AutoLab daemon. It is a client of the HTTP API in [api.md](api.md),
like the CLI. It adds no new power and needs no security change: it reads `daemon.json` for the port and the
token, and sends no `Origin` header.

```bash
cargo run -p autolab-gui          # from a checkout
cargo install --path crates/autolab-gui --locked
```

The window works with or without a session. If no daemon runs, it says so and offers **Start session**. That
button runs `autolab start`, so the daemon start-up and your [config file](configuration.md) behave exactly as in
the terminal. `autolab` must sit next to `autolab-gui` or be on `PATH`.

## What it shows

- **Session**: backend and version, phone serial, transport, process ids, and each `--reverse` tunnel with its live
  state. **Stop session** ends it.
- **Video focus**: what AutoLab requested, what the DHU reports, and the phone-side value, which is always
  `unknown` because no phone-side check exists. **Focus on** and **Focus off** send the command.
- **Screenshot**: saves a PNG to the path you type (absolute, ends in `.png`, no spaces) and previews it.
- **Events**: the daemon's event stream, live. Events from before the window connected are not shown, because the
  API has no replay.

Buttons are disabled while a command runs and when the session cannot take them (for example Focus before the
session is running). Errors from the daemon appear in the bar at the bottom, in the daemon's words.

## Limits

- Not tested by clicking: the window was checked by rendering it against a fake DHU and against the real phone
  (session, focus, tunnel and event views), and its commands are tested against a real in-process daemon. No button
  has been clicked, by hand or by automation. A click handler that only calls these commands is the untested part.
- macOS only so far (that is where it was built and run). It uses `eframe` with the OpenGL backend.
- A screenshot preview shows whatever is on the phone's screen. Keep saved screenshots out of version control.
- The GUI adds about 110 crates to the workspace build. The `autolab` binary does not depend on it.

## Checking how it looks

Set `AUTOLAB_GUI_SNAPSHOT=/path/to/out.png` and the window saves an image of itself after about 2.5 s and quits.
It captures only the AutoLab window, never the rest of the desktop.
