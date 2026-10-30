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
  --tmpfs /home/opendhu:rw,mode=0755,uid="$(id -u)",gid="$(id -g)" \
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

## Run the sample suite

The sample's suite builds its APK before every run, so Gradle runs inside the container (the image has JDK 17;
the Android platform is downloaded on first use). Mount a Gradle cache to keep that fast:

```bash
make docker-test                                  # builds the image, then runs `aa test sample`
make docker-test DOCKER_TEST_ARGS="sample --filter favorite"
```

The same without `make`, from the repository root:

```bash
docker run --rm --device /dev/kvm --group-add "$(stat -c %g /dev/kvm)" --user "$(id -u):$(id -g)" \
  --tmpfs /home/opendhu:rw,mode=0755,uid="$(id -u)",gid="$(id -g)" \
  -v "$PWD:/work" \
  -v "$HOME/.cache/opendhu/avd:/avd" \
  -v "$HOME/.cache/opendhu/gradle:/home/opendhu/.gradle" \
  -v "$HOME/.config/opendhu/credentials:/home/opendhu/.config/opendhu/credentials:ro" \
  opendhu test sample
```

Reports land in `build/test-results` in the checkout. The first run downloads Gradle and the Android platform and
creates the AVD, so it is slow.

## What is and is not provisioned

The entrypoint creates the AVD when `/avd` has none, boots it with `-no-window -no-snapshot -gpu
swiftshader_indirect`, waits for Android to finish booting and stops it on exit. The adb key pair and debug
keystore are kept in `/avd/.android`, because the AVD trusts the key it first booted with: keep `/avd` in a volume
or bind mount, and delete it entirely (not just part of it) to start over.

A fresh AVD does **not** have Android Auto ready, and `aa` cannot finish that for you. Do this once, then keep the
AVD in `/avd`:

1. **Install Android Auto.** Supply an APK, split directory or `.xapk` with `AA_GEARHEAD_APK`
   (`make docker-test DOCKER_GEARHEAD_APK=/path/to/splits`). You can pull the splits from an emulator or phone that
   already has it with `adb shell pm path com.google.android.projection.gearhead` and `adb pull`.
2. **Enable developer mode.** Open Android Auto settings (`adb shell am start -a
   com.google.android.projection.gearhead.SETTINGS`), tap *Version* about ten times and accept the development
   prompt. Without it `aa` stops with "Provision this emulator baseline".
3. **Allow notification access.** With a session running (`aa session start --reverse 10101`), the head unit asks
   for notification access: tap *Go to phone*, then turn on Android Auto under *Notification read, reply & control*
   and allow it. Without it the car never shows the launcher and suites fail at their first `app.open` step.

The container has no display, so drive these with `adb shell input tap` and `adb exec-out screencap -p` (or `aa
screenshot`/`aa tap` for the head unit) from `docker run -d --entrypoint bash ...` running the emulator. Stop the
emulator with `adb emu kill` so the AVD state is saved. One AVD is shared by all runs, so use a dedicated runner.

## Building on the image

The image holds only `aa` and the emulator. A project whose suite builds an app (a JDK, Gradle, Node, ...) should
derive from it and add its toolchain:

```dockerfile
FROM opendhu
USER root
RUN apt-get update && apt-get install -y openjdk-21-jdk-headless && rm -rf /var/lib/apt/lists/*
```
