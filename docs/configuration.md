# Configuration

AutoLab works without a config file. Add one to stop repeating flags.

The file is TOML. Only the CLI reads it. The daemon never does: the CLI sends the daemon explicit,
absolute values in each request.

## Where the file lives

1. `$AUTOLAB_CONFIG`, if set. This file must exist, or the command fails with exit code 2.
2. `$XDG_CONFIG_HOME/autolab/config.toml`, if `XDG_CONFIG_HOME` is an absolute path.
3. `~/.config/autolab/config.toml`.

A missing file at the default location is fine. `autolab doctor` shows which file was loaded, or that none was.

## Precedence

Highest first: command-line flag, environment variable, config file, built-in default.

- `AUTOLAB_DHU_PATH` outranks `dhu.path`. `--dhu-path` outranks both.
- A list flag (`--reverse`, `--report`) **replaces** the file's list. It does not add to it.
- A switch flag (`--headless`, `--skip-unsupported`) can turn a setting on. It cannot turn off a setting
  that the file turned on. Edit the file for that.
- A relative path inside the file is relative to the directory of the file, not to where you run `autolab`.

## Example

```toml
[dhu]
path = "/Users/me/Library/Android/sdk/extras/google/auto/desktop-head-unit"
ini = "dhu.ini"          # the DHU's own .ini file (same as `autolab start --config`)
headless = false

[session]
serial = "1A141FDF600AF2"
reverse = [10101]        # same as `autolab start --reverse 10101`

[test]
artifacts_dir = "test-results"
report = ["json", "junit"]
skip_unsupported = false

[daemon]
port = 8765              # only for `autolab daemon`; a daemon started by `autolab start` picks its own
```

Every key is optional. An unknown section or key is an error that names the file, so a typo does not
pass silently. `autolab start`, `test`, `daemon` and `doctor` read the file. `status`, `stop`, `focus` and
`screenshot` do not, so a broken file never stops you from stopping a session.
