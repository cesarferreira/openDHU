# Release workflow

Cut a release with an auto-generated changelog entry. This bumps every crate together, commits, tags `vX.Y.Z`, and pushes. The tag push opens a GitHub Release whose notes are that changelog entry. Nothing is published to crates.io.

```bash
make tag                      # asks patch, minor, or major; Enter keeps minor
make release                  # minor bump, no question
make release LEVEL=patch      # patch bump
make release LEVEL=major      # major bump
```

During `cargo release`'s pre-release hook, [git-cliff](https://git-cliff.org/) regenerates `CHANGELOG.md` from the git history, treating the commits since the latest `v*` tag as the version being released. Configuration lives in [`cliff.toml`](../cliff.toml).

## What gets generated

git-cliff groups commits by their [conventional-commit](https://www.conventionalcommits.org) type:

| Commit prefix | Section |
|---|---|
| `feat:` | `🚀 Features` |
| `fix:` | `🐛 Bug Fixes` |
| `refactor:` | `🚜 Refactor` |
| `perf:` | `⚡ Performance` |
| `docs:` | `📚 Documentation` |
| `style:` | `🎨 Styling` |
| `test:` | `🧪 Testing` |
| `chore:` / `ci:` | `⚙️ Miscellaneous Tasks` |
| `revert:` | `◀️ Revert` |
| anything else | `💼 Other` |

PR references like `(#123)` become links to this repo. The automated `chore: Release openDHU version …` commits are skipped. Sentence-style subjects are kept under Other, so a change is not dropped because it lacks a conventional prefix.

## Notes

- `make tag` asks for `patch`, `minor`, or `major` when a terminal is attached. Press Enter for minor. `make tag LEVEL=patch` skips the question, and so does a run with no terminal.
- `make release` takes the same levels and does not ask. It defaults to minor.
- For a dry run, invoke `cargo release <level> --no-confirm` directly without `--execute`. cargo-release skips the version bump, tag, and push. git-cliff still rewrites `CHANGELOG.md` in place; run `git checkout CHANGELOG.md` to discard it.
- Preview the upcoming entry without touching the file: `git-cliff --unreleased --tag v<next-version>`.
- `make release` installs `cargo-release` via `cargo install` if it is missing, and installs or reinstalls `git-cliff` at the version pinned by `GIT_CLIFF_VERSION` in the Makefile when it is missing or a different version. Override the pin for one run with `make release GIT_CLIFF_VERSION=x.y.z`.
