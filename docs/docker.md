# Running openDHU in Docker

`docker/Dockerfile` packages `aa`, a headless Android emulator (Google Play image, so Android Auto is
available) and the tools `aa test` needs (`adb`, ffmpeg, tesseract). It is the same setup as the
[GitHub Action](ci.md), for servers, local CI and machines that should not carry an Android SDK.

## Requirements

- A **Linux host with KVM**. The container needs `--device /dev/kvm`; without hardware acceleration the
  emulator is too slow to be useful.
- **Not macOS or Windows, including Apple Silicon.** Docker Desktop provides no `/dev/kvm`, and the image is
  x86_64 because Google ships no arm64 Linux emulator. On a Mac, install `aa` natively (see the README).
- Development apps need [head-unit credentials](native-backend.md#debugging-knobs): mount
  `~/.config/opendhu/credentials` into the container.

## Build

```bash
make docker-build   # docker build -f docker/Dockerfile -t opendhu .
```

Build arguments: `ANDROID_API` (system image API level, default `36`), `UBUNTU`.

## Run

Mount your project at `/work`, where `opendhu.yaml` and the suites live. Arguments after the image name are
passed to `aa`; with none, `aa test` runs every suite. `test` adds `--headless --no-interactive`.

```bash
docker run --rm --device /dev/kvm --group-add "$(stat -c %g /dev/kvm)" \
  --user "$(id -u):$(id -g)" \
  -v "$PWD:/work" \
  -v "$HOME/.cache/opendhu/avd:/avd" \
  -v "$HOME/.config/opendhu/credentials:/home/opendhu/.config/opendhu/credentials:ro" \
  opendhu test sample --filter login
```

Reports land in `build/test-results` in the mounted project, owned by you.

| | |
|---|---|
| `AVD_NAME` | AVD to create and boot, default `medium_phone`; it must equal `environment.emulator` in the suite |
| `AVD_DEVICE` | `avdmanager` device definition for a new AVD, default `pixel_6` |
| `AA_GEARHEAD_APK` | Path inside the container of an Android Auto APK, split directory or `.xapk` to install first |

## What is and is not provisioned

The entrypoint creates the AVD when `/avd` has none, boots it with `-no-window -no-snapshot -gpu
swiftshader_indirect`, waits for Android to finish booting and stops it on exit. `aa` itself handles the
Android Auto welcome screen and unknown sources. Consent and developer settings in Android Auto are not
scripted, so provision them once, or supply an APK, then **keep `/avd` in a volume or bind mount** so the
provisioned emulator is reused. One AVD is shared by all runs, so use a dedicated runner.

## Building on the image

The image holds only `aa` and the emulator. A project whose suite builds an app (a JDK, Gradle, Node, ...) should
derive from it and add its toolchain:

```dockerfile
FROM opendhu
USER root
RUN apt-get update && apt-get install -y openjdk-21-jdk-headless && rm -rf /var/lib/apt/lists/*
```
