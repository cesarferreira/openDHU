# Running openDHU in CI

The repository is a composite GitHub Action. On a Linux runner with KVM (`ubuntu-latest`) it:

1. enables KVM and installs ffmpeg and tesseract;
2. builds `aa` and `opendhu-native-hu` from the action's own checkout (cached);
3. creates an AVD from a Google Play system image and boots it headless;
4. installs Android Auto if you supply it (see below);
5. runs `aa test` with the video mode you choose;
6. writes a results table to the job summary and uploads the artifacts folder, even when tests fail;
7. stops the emulator.

```yaml
jobs:
  e2e:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-java@v4        # whatever your suite's build step needs
        with: { distribution: temurin, java-version: "17" }
      - uses: cesarferreira/openDHU@main
        with:
          suite: sample
          record: failures
```

Pin a tag or commit instead of `main` once you depend on it.

## Inputs

| Input | Default | |
|---|---|---|
| `suite` | all suites | Suite name or test path, as for `aa test` |
| `working-directory` | `.` | Where `opendhu.yaml` or the suite lives |
| `avd-name` | `medium_phone` | Must equal `environment.emulator` in the suite |
| `device` | `pixel_6` | `avdmanager` device definition |
| `api-level` | `35` | System image API level |
| `target` | `google_apis_playstore` | Use a Play image: it ships with Android Auto |
| `arch` | `x86_64` | The only fast choice on GitHub's runners |
| `gearhead-apk` | | Android Auto APK, split directory or `.xapk` to install |
| `gearhead-version`, `gearhead-sha256` | | Let apkeep fetch a pinned build; the checksum is required |
| `record` | `failures` | `failures`, `all` or `off`; see [video](testing.md#video---record) |
| `args` | | Extra `aa test` arguments, such as `--filter login` |
| `artifacts-dir` | `openDHU-results` | No spaces |
| `upload`, `artifact-name` | `true`, `openDHU-results` | Artifact upload |

## Android Auto

A Google Play image usually has Android Auto already, so most suites need nothing here. Pass `gearhead-apk`
to use a file you keep, or `gearhead-version` with `gearhead-sha256` to download one. The download comes from
APKPure, so the checksum is what you trust: take it from a bundle you checked yourself. See the README for
`aa setup gearhead`, which these inputs wrap.

## What to expect

- The emulator boots in a few minutes and a suite runs in minutes more, so run this on demand, on a schedule
  or on pull requests that touch the app, not on every push.
- Your suite's `build` step runs on the runner, so install its toolchain first (JDK, Gradle caches).
- The uploaded folder holds `results.html`, `results.json`, `results.xml`, logs, screenshots and `videos/`.
  Download it and open `results.html` to watch a failing test.
- This repository's own `e2e.yml` runs the Android sample this way, on demand from the Actions tab.
- To run the same setup outside GitHub Actions, see [Docker](docker.md).
