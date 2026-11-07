# Configuration

`opendhu.yaml` holds a project's named test suites and head-unit settings. The CLI reads the nearest
file in the current directory or its ancestors. Relative paths are resolved from the file's directory.
The daemon receives explicit, absolute values; it does not read the file itself.

Set `OPENDHU_CONFIG=/path/to/other.yaml` to use a specific file instead. That file must exist.
Without a file, openDHU uses its built-in defaults. `aa doctor` shows the file it loaded.

## Vehicle profile and suites

```yaml
suites:
  robotaxi: e2e/robotaxi.yaml
  drivers: e2e/production.yaml

session:
  backend: dhu                # native is the default
  serial: 1A141FDF600AF2     # optional
  reverse: [10101]           # optional ADB reverse ports

dhu:
  ini: head-unit/nissan_leaf.ini
  # path: /path/to/desktop-head-unit
  # headless: false

test:
  artifacts_dir: build/test-results
  report: [json, junit, html]
  skip_unsupported: false

daemon:
  port: 8765                 # only for `aa daemon`
```

`aa session start` from this directory or a child uses the Nissan Leaf DHU profile. Select a
different INI for one session with `aa session start --dhu --config head-unit/nissan_arya.ini`.
Google's DHU owns the INI crop, DPI and sensor options; the native backend uses its own resolution
setting described in [Native backend](native-backend.md).

## Precedence

Highest first: command-line flag, environment variable, `opendhu.yaml`, built-in default.

- `--dhu-path` overrides `OPENDHU_DHU_PATH`, which overrides `dhu.path`.
- `--dhu` or `--backend native|dhu` overrides `session.backend`. The two flags cannot be combined.
- A list flag (`--reverse`, `--report`) replaces the file's list.
- A switch flag (`--headless`, `--skip-unsupported`) turns a setting on. To turn off a setting
  that the file enabled, edit the file.

Every key is optional. Unknown sections and keys produce an error naming the file. `aa session start`,
`test`, `daemon` and `doctor` read it. `session status`, `session stop`, `focus` and `screenshot`
do not, so a broken file never prevents stopping a session.
