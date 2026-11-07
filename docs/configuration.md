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
  backend: native             # native is the default; required for TUI/web video
  resolution: 720p            # interactive native session: 480p, 720p or 1080p
  serial: 1A141FDF600AF2     # optional
  reverse: [10101]           # optional ADB reverse ports

dhu:
  ini: head-unit/nissan_leaf.ini
  # path: /path/to/desktop-head-unit
  # headless: false

test:
  resolution: 480p            # managed suites use this, independently of session.resolution
  artifacts_dir: build/test-results
  report: [json, junit, html]
  skip_unsupported: false

daemon:
  port: 8765                 # only for `aa daemon`
```

`aa session start` from this directory or a child uses the native 720p profile, which `aa tui`,
`aa gui` and `aa web` can display. `aa test` starts managed suites at 480p regardless of the
interactive setting. Pin `test.resolution` in the project's YAML for reproducible E2E runs;
tests with fixed coordinates must be calibrated for that resolution. Direct test files run against
an already running session and do not start or resize it.

Start a Nissan Leaf DHU session explicitly with `aa session start --dhu`; the `dhu.ini` path is
used then. Select a different INI for one session with
`aa session start --dhu --config head-unit/nissan_arya.ini`. Google's DHU owns the INI crop, DPI
and sensor options and opens its own window; TUI and web video require the native backend.

## Precedence

Highest first: command-line flag, environment variable, `opendhu.yaml`, built-in default.

- `--dhu-path` overrides `OPENDHU_DHU_PATH`, which overrides `dhu.path`.
- `--dhu` or `--backend native|dhu` overrides `session.backend`. The two flags cannot be combined.
- `aa session start --resolution 480p|720p|1080p` overrides `session.resolution` for a native session.
- `OPENDHU_NATIVE_RESOLUTION` overrides `session.resolution`. Managed suites use `test.resolution`
  (480p by default) and ignore interactive resolution settings.
- A list flag (`--reverse`, `--report`) replaces the file's list.
- A switch flag (`--headless`, `--skip-unsupported`) turns a setting on. To turn off a setting
  that the file enabled, edit the file.

Every key is optional. Unknown sections and keys produce an error naming the file. `aa session start`,
`test`, `daemon` and `doctor` read it. `session status`, `session stop`, `focus` and `screenshot`
do not, so a broken file never prevents stopping a session.
