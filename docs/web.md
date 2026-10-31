# Browser viewer (`aa web`)

`opendhu-web` shows the head unit in a browser tab. It serves one page on `127.0.0.1` with the live picture,
touch, the car buttons and, under `aa test --web`, the test progress. Like the desktop window ([gui.md](gui.md))
and the terminal viewer ([tui.md](tui.md)) it is a client of the HTTP API in [api.md](api.md): the browser
talks to `opendhu-web`, never to the daemon, and never sees its token.

```bash
aa session start
aa web                       # opens http://127.0.0.1:8766/ in the default browser
aa web --no-open --port 9000 # just print the address
cargo run -p opendhu-web     # from a checkout
aa test sample --web         # with the test progress beside the picture; see testing.md
```

`aa web` finds `opendhu-web` next to the CLI or on `PATH` (`make install-web`). Port 8766 is tried first, then
any free port; the address is printed either way. The video is decoded with `ffmpeg` into an MJPEG stream,
once for all open tabs, and only while a tab is watching.

## Controls

| Input | Action |
| --- | --- |
| Click | Tap |
| Drag | Swipe, taking as long as you held the button |
| Wheel | Scroll |
| `Esc` or `b` | Back |
| `h` | Home |
| `[`, `]` | Turn the rotary dial: focus the previous or next item |
| Arrows | Nudge focus to the next area of the screen in that direction (it does not step between items) |
| `Enter` | Press the focused item |
| `s` | Search |
| Space | Play/pause |
| `n`, `p` | Next, previous track |
| `c`, `e` | Call, end call |
| `f` | Toggle video focus |
| `S` | Download a screenshot as `aa-screenshot-<time>.png` |

The buttons under the picture do the same.

## With a test run

`aa test --web` starts the viewer for the run and feeds it the progress lines; the page follows each test's own
session, so it is empty while a suite boots its emulator. A tab opened late gets the whole run so far. When
the run ends the page says so and keeps the log, and `opendhu-web` exits a moment later.

## Security

The page can touch the phone, and loopback is reachable by every page your browser has open, so
`opendhu-web` only binds to `127.0.0.1`, refuses any `Host` other than its own address (which blocks DNS
rebinding) and any `Origin` other than its own page (which blocks other sites). Touch and key requests must be
JSON, which a plain cross-site form cannot send. Other local users can still reach the port, as with any
loopback server; do not run it on a shared machine you do not trust.

## Troubleshooting

- **"no session":** start one with `aa session start`.
- **"waiting for video" stays up:** check `ffmpeg` is installed; the page shows the decoder's error when
  there is one. Video needs the native backend.
- **The browser did not open:** go to the printed address, or use `--no-open` and open it yourself.
