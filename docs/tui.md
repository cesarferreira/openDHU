# Terminal viewer (`aa tui`)

`opendhu-tui` shows the head unit in your terminal. It draws the live picture with the
[kitty graphics protocol](https://sw.kovidgoyal.net/kitty/graphics-protocol/), so it needs a terminal that
supports it: kitty, Ghostty or WezTerm. Like the desktop window ([gui.md](gui.md)) it uses the HTTP API in
[api.md](api.md), and the video is decoded with `ffmpeg`.

```bash
aa session start
aa tui                           # after make install-tui
cargo run -p opendhu-tui         # from a checkout
aa test e2e/production.yaml --tui  # with the test progress underneath; see testing.md
```

`aa tui` finds `opendhu-tui` next to the CLI or on `PATH`.

## Controls

| Input | Action |
| --- | --- |
| Click | Tap |
| Drag | Swipe, taking as long as you held the button |
| Wheel | Scroll |
| `Esc` or `b` | Back |
| `h` | Home |
| Arrows, `Enter` | D-pad |
| `s` | Search |
| Space | Play/pause |
| `n`, `p` | Next, previous track |
| `c`, `e` | Call, end call |
| `f` | Toggle video focus |
| `S` | Save a screenshot as `aa-screenshot-<time>.png` in the current directory |
| `q` or Ctrl-C | Quit |

The mouse is mapped to phone pixels exactly when the terminal reports pixel positions, and to the middle
of the clicked cell otherwise (the header says `pixel mouse` or `cell mouse`).

## Over SSH

Every picture crosses the network, so when `SSH_CONNECTION` or `SSH_TTY` is set the viewer sends frames at
most 640 pixels wide and 10 times a second. Locally it uses 960 pixels and about 30 a second. Frames are
always zlib-compressed, and a frame identical to the one on screen is not sent again. Set
`OPENDHU_TUI_REMOTE=1` or `0` to force the choice, for example inside a multiplexer that hides SSH.

## Troubleshooting

- **No picture, header works:** the terminal is not showing kitty graphics. Run `aa tui` directly in the
  terminal app, not inside tmux or another multiplexer, unless it passes graphics through.
- **Header says `no session`:** start one with `aa session start`. With `aa test --tui` the viewer follows the
  session of the test that is running, so it stays empty while a suite boots its emulator.
- **Picture lags:** use a release build (`make install-tui`); a debug build encodes frames slowly.
