#!/bin/bash
# A scripted stand-in for autolab-native-hu: same JSON-lines protocol, no phone. Modes (FAKE_NATIVE_MODE):
#   never-ready      stays silent, so a start must time out
#   no-confirm       the phone never confirms a focus change (status timed_out)
#   crash-on-focus   exits with status 3 when asked to change focus
#   exit-early       exits with status 4 before announcing ready
#   junk-then-exit   prints a non-protocol line, then exits with status 5
mode=${FAKE_NATIVE_MODE:-}
requested=unknown
observed=on

case "$mode" in
  exit-early) exit 4;;
  junk-then-exit) echo "usb exploded: no accessory"; exit 5;;
  never-ready) cat >/dev/null; exit 0;;
esac

echo '{"event":"ready","backend_version":"fake 0.0","backend_build":"fake-build"}'

while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9]*\).*/\1/p')
  cmd=$(printf '%s' "$line" | sed -n 's/.*"cmd":"\([a-z_]*\)".*/\1/p')
  case "$cmd" in
    quit) exit 0;;
    status)
      echo "{\"id\":$id,\"ok\":{\"type\":\"status\",\"value\":{\"videoFocus\":{\"requested\":\"$requested\",\"backendReported\":\"$requested\",\"observed\":\"$observed\"},\"frames\":3}}}";;
    focus)
      [ "$mode" = crash-on-focus ] && exit 3
      case "$line" in *'"enabled":true'*) want=on;; *) want=off;; esac
      requested=$want
      if [ "$mode" = no-confirm ]; then
        echo "{\"id\":$id,\"ok\":{\"type\":\"focus\",\"value\":{\"status\":\"timed_out\",\"requested\":\"$want\",\"backendReported\":\"$want\",\"observed\":\"$observed\",\"evidence\":[\"the phone did not confirm\"]}}}"
      else
        observed=$want
        echo "{\"id\":$id,\"ok\":{\"type\":\"focus\",\"value\":{\"status\":\"observed\",\"requested\":\"$want\",\"backendReported\":\"$want\",\"observed\":\"$want\",\"evidence\":[\"fake phone said $want\"]}}}"
        echo "{\"event\":\"video\",\"state\":\"$want\"}"
      fi;;
    screenshot)
      path=$(printf '%s' "$line" | sed -n 's/.*"path":"\([^"]*\)".*/\1/p')
      if [ "$observed" = off ]; then
        echo "{\"id\":$id,\"err\":{\"kind\":\"other\",\"message\":\"Don't have video focus - nothing to screenshot.\"}}"
      else
        printf '\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x03\x20\x00\x00\x01\xe0\x08\x06\x00\x00\x00' > "$path"
        echo "{\"id\":$id,\"ok\":{\"type\":\"screenshot\",\"value\":{\"path\":\"$path\",\"width\":800,\"height\":480}}}"
      fi;;
    tap|swipe)
      if [ "$observed" = off ]; then
        echo "{\"id\":$id,\"err\":{\"kind\":\"other\",\"message\":\"The phone is not projecting - nothing to touch.\"}}"
      else
        echo "{\"id\":$id,\"ok\":{\"type\":\"input\",\"value\":{\"status\":\"accepted\",\"evidence\":[\"sent $cmd (fake)\"]}}}"
      fi;;
    *) echo "{\"id\":$id,\"err\":{\"kind\":\"invalid_args\",\"message\":\"unknown command\"}}";;
  esac
done
exit 0
