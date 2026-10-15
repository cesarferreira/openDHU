#!/bin/bash
# Stand-in for the Google DHU console, for tests only. Mirrors behaviour captured from DHU 2.0:
# silent success for `focus video on|off`, `[I]: Has video focus: <bool>` for the query,
# `[E]: ...` for errors, a bare `> ` prompt with no newline, exit on `quit` or stdin EOF.
# Stream split matches the real DHU (verified): prompt and banner on stdout, [I]/[E]/log lines on stderr.
# FAKE_DHU_MODE: fail-start | late-fail-start | hang-start | (default) normal
case "$FAKE_DHU_MODE" in
  fail-start)
    echo '[I]: Connecting over ADB to localhost:5277...' >&2
    printf '[E]: \033[38;5;196mfailed.\033[0;00m\n' >&2
    echo '[E]: Failed to start Google Automotive Link.' >&2
    exit 1;;
  late-fail-start)
    # stderr closes at once; the explanation arrives on stdout afterwards
    exec 2>&-
    sleep 0.3
    echo '[E]: Failed to start Google Automotive Link (late).'
    exit 1;;
  hang-start)
    echo 'Starting link. Requested protocol version: 1.7' >&2
    exec sleep 30;;
esac
echo 'Android Auto - Desktop Head Unit'
echo '  Build: fake-build-1'
echo '  Version: 0.0-fake'
echo
echo '> Phone reported protocol version 1.7' >&2
echo 'Verify returned: ok' >&2
focus=true
printf '> '
while IFS= read -r line; do
  case "$line" in
    "focus video on") focus=true;;
    "focus video off") focus=false;;
    "focus video toggle") if $focus; then focus=false; else focus=true; fi;;
    "focus video") echo "[I]: Has video focus: $focus" >&2;;
    "focus video "*) echo "[E]: Invalid state '${line#focus video }'" >&2;;
    "screenshot "*)
      path=${line#screenshot }
      if ! $focus; then
        echo "[E]: Don't have video focus - nothing to screenshot." >&2
        printf '> '
        continue
      fi
      case "$FAKE_DHU_MODE" in
        screenshot-nofile) ;;
        screenshot-notpng) echo "this is not a png, just some text padding" > "$path";;
        *) printf '\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x05\x00\x00\x00\x02\xd0\x08\x06\x00\x00\x00' > "$path";;
      esac;;
    "quit"|"exit") exit 0;;
    "crash") exit 3;;
    "hang") exec sleep 30;;
    "") ;;
    *) echo "[E]: \"$line\" is not a known command. Type \"help\" to see a list of commands." >&2;;
  esac
  printf '> '
done
exit 0
