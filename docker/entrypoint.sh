#!/usr/bin/env bash
# Boots a headless emulator, then runs `aa` with the container's arguments (default: `aa test`).
#   docker run ... opendhu test sample --filter login
#   docker run ... opendhu doctor
set -euo pipefail

AVD_NAME="${AVD_NAME:-medium_phone}"
AVD_DEVICE="${AVD_DEVICE:-pixel_6}"
IMAGE="system-images;android-${ANDROID_API};google_apis_playstore;x86_64"

if [[ ! -r /dev/kvm || ! -w /dev/kvm ]]; then
    echo 'opendhu: /dev/kvm is not accessible. Run on a Linux host with KVM and pass --device /dev/kvm.' >&2
    exit 1
fi

if [[ ! -d "$ANDROID_AVD_HOME/$AVD_NAME.avd" ]]; then
    echo "opendhu: creating AVD $AVD_NAME..."
    echo no | avdmanager create avd --force -n "$AVD_NAME" -k "$IMAGE" -d "$AVD_DEVICE"
fi

emulator_pid=''
cleanup() {
    result=$?
    trap - EXIT INT TERM
    if [[ -n "$emulator_pid" ]]; then
        adb emu kill >/dev/null 2>&1 || true
        wait "$emulator_pid" 2>/dev/null || true
    fi
    exit "$result"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

adb start-server >/dev/null
emulator -avd "$AVD_NAME" -no-window -no-snapshot -no-audio -no-boot-anim \
    -gpu swiftshader_indirect -no-metrics > /tmp/emulator.log 2>&1 &
emulator_pid=$!
booted() { [[ "$(adb shell getprop sys.boot_completed 2>/dev/null | tr -d '\r')" == 1 ]]; }
echo 'opendhu: waiting for the emulator to boot...'
for _ in $(seq 1 100); do
    kill -0 "$emulator_pid" 2>/dev/null || { tail -n 40 /tmp/emulator.log >&2; echo 'opendhu: the emulator exited' >&2; exit 1; }
    booted && break
    sleep 3
done
booted || { tail -n 40 /tmp/emulator.log >&2; echo 'opendhu: the emulator did not boot in 5 minutes' >&2; exit 1; }

if [[ -n "${AA_GEARHEAD_APK:-}" ]]; then
    aa setup gearhead --apk "$AA_GEARHEAD_APK" --force
fi

args=("$@")
# Suites run unattended: no window, no animated progress.
if [[ "${args[0]:-}" == test ]]; then args+=(--headless --no-interactive); fi

# aa reuses the running AVD and leaves it for this script to stop.
set +e
aa "${args[@]}"
exit $?
