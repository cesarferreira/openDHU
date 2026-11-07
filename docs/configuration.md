# Configuration

openDHU works without a config file. Add one to stop repeating flags.

The file is TOML. Only the CLI reads it. The daemon never does: the CLI sends the daemon explicit,
absolute values in each request.

## Where files live

By default, openDHU loads the optional user file at `$XDG_CONFIG_HOME/opendhu/config.toml` (when
`XDG_CONFIG_HOME` is absolute), or `~/.config/opendhu/config.toml`. It then finds the nearest
`opendhu.toml` in the current directory or its ancestors and applies its settings over the user
file. Missing files are fine. A relative path in either file is resolved from that file's directory.

Set `$OPENDHU_CONFIG` to use one specific file instead. It must exist; openDHU then skips both
automatic locations. `aa doctor` shows every file it loaded, or that none was loaded.

## Precedence

Highest first: command-line flag, environment variable, project `opendhu.toml`, user
`config.toml`, built-in default. A project file overrides only the keys it sets.

- `OPENDHU_DHU_PATH` outranks `dhu.path`. `--dhu-path` outranks both.
- `aa session start` defaults to native. `--dhu` selects Google's DHU; `--backend native|dhu` is also supported.
  Either command-line choice overrides `session.backend`; `--dhu` and `--backend` cannot be combined.
- A list flag (`--reverse`, `--report`) **replaces** the file's list. It does not add to it.
- A switch flag (`--headless`, `--skip-unsupported`) can turn a setting on. It cannot turn off a setting
  that the file turned on. Edit the file for that.
- A project file can set `headless = false` or an empty list to override a user setting.

## Vehicle profile in a project

Put this next to a `head-unit/` directory containing a Google DHU `.ini` file:

```toml
[session]
backend = "dhu"

[dhu]
ini = "head-unit/nissan_leaf.ini"
```

Running `aa session start` from that directory or any child uses that INI. You can select another
profile for one session with `aa session start --dhu --config head-unit/nissan_arya.ini`.
The native backend uses its own resolution setting; DHU INI crop, DPI and sensor options belong to
Google's DHU.

## Example

```toml
[dhu]
path = "/Users/me/Library/Android/sdk/extras/google/auto/desktop-head-unit"
ini = "dhu.ini"          # the DHU's own .ini file (same as `aa session start --dhu --config`)
headless = false

[session]
backend = "native"       # the default; or "dhu"; see native-backend.md. The `dhu.*` keys are ignored for native.
serial = "1A141FDF600AF2"
reverse = [10101]        # same as `aa session start --reverse 10101`

[test]
artifacts_dir = "build/test-results"
report = ["json", "junit", "html"]
skip_unsupported = false

[daemon]
port = 8765              # only for `aa daemon`; a daemon started by `aa session start` picks its own
```

Every key is optional. An unknown section or key is an error that names the file, so a typo does not
pass silently. `aa session start`, `test`, `daemon` and `doctor` read the files. `session status`, `session stop`,
`focus` and `screenshot` do not, so a broken file never stops you from stopping a session.
