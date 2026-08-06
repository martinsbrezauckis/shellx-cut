#!/bin/sh
# run-under-terminal.sh — run a command file under a Terminal.app-hosted shell on this Mac
# and marshal its REAL exit code back to the (typically ssh) caller.
#
# WHY THIS EXISTS — macOS TCC attributes a screen-capture grant to the RESPONSIBLE PROCESS of
# the launching chain, not to the binary. A process spawned from an ssh session is
# responsible-to sshd, which holds no Screen Recording grant, so ScreenCaptureKit degrades and
# every live screen_record row fails. Probed 2026-08-06 on the macOS rig with ONE identical
# installed 0.6.106 binary and ONE ssh session, only the launch path differing:
#     direct exec from the ssh chain              -> screen_capture DEGRADED
#     LaunchServices (open -n)                    -> screen_capture OK
#     direct exec re-parented under Terminal.app  -> screen_capture OK
# The WebDriver suite cannot take the LaunchServices path: the instrumented candidate is built
# --no-bundle (there is no .app to open), the wdio tauri service must spawn and own the process
# to attach WebDriver, and the exact-source claim contract requires the action matrix to come
# from the candidate rather than the installed app. Hosting the SAME direct spawn under
# Terminal — which already holds the grant — is therefore the fix, and it needs no change to
# machine configuration.
#
# EXIT-CODE SENTINEL — Terminal's `do script` DETACHES, so the caller cannot observe the hosted
# process's status; a naive implementation would report success for a run that never started.
# The wrapper generated here makes its LAST act an atomic write of the command's exit status to
# a sentinel file, and this script treats a missing, unreadable, or non-numeric sentinel as a
# HARD FAILURE. A detached run can never report false success.
#
# usage: run-under-terminal.sh <command-file> <out-dir>
#   <command-file>  sh script to execute. Built by the caller and written to disk, so it MUST
#                   NOT contain secrets.
#   <out-dir>       run directory; receives hosted-run.log (tailed live to this script's
#                   stdout so progress stays observable) and hosted-exit-code (the sentinel).
# env:
#   RUN_UNDER_TERMINAL_TIMEOUT_S  give up waiting after N seconds (default 25200 = 7h, chosen
#                                 to sit above the suite's own 6h mocha budget)
# exits: the hosted command's status, or 2 usage / 3 no usable console / 4 timeout /
#        5 unusable sentinel.

set -eu

COMMAND_FILE=${1:-}
OUT_DIR=${2:-}
if [ -z "$COMMAND_FILE" ] || [ -z "$OUT_DIR" ]; then
  echo "usage: run-under-terminal.sh <command-file> <out-dir>" >&2
  exit 2
fi
if [ ! -f "$COMMAND_FILE" ]; then
  echo "run-under-terminal: command file not found: $COMMAND_FILE" >&2
  exit 2
fi
# The generated wrapper single-quotes these paths for the hosted shell; a single quote inside
# one would break out of that quoting, so refuse it rather than emit a malformed wrapper.
case "$COMMAND_FILE$OUT_DIR" in
  *"'"*)
    echo "run-under-terminal: paths must not contain single quotes" >&2
    exit 2
    ;;
esac

mkdir -p "$OUT_DIR"
TIMEOUT_S=${RUN_UNDER_TERMINAL_TIMEOUT_S:-25200}

# Fail fast when there is no usable console session: `do script` would open nothing to run in,
# and TCC would not hand out the capture grant anyway. Hanging here would be worse than exiting.
CONSOLE_USER=$(stat -f%Su /dev/console 2>/dev/null || echo "")
ME=$(id -un)
if [ "$CONSOLE_USER" != "$ME" ]; then
  echo "run-under-terminal: no console session for '$ME' (console user='${CONSOLE_USER:-none}')." >&2
  echo "  Terminal hosting requires this user logged in at the Mac's console. Log in, or pass" >&2
  echo "  --no-terminal-host to spawn directly (screen capture will then degrade)." >&2
  exit 3
fi
if ioreg -n Root -d1 2>/dev/null |
  grep -E '"?CGSSessionScreenIsLocked"?[[:space:]]*=[[:space:]]*Yes' >/dev/null; then
  echo "run-under-terminal: the Mac console is locked; unlock it before a hosted run" >&2
  exit 3
fi

LOG="$OUT_DIR/hosted-run.log"
SENTINEL="$OUT_DIR/hosted-exit-code"
WRAPPER="$OUT_DIR/hosted-wrapper.sh"
rm -f "$LOG" "$SENTINEL" "$SENTINEL.partial" "$WRAPPER"
: > "$LOG"

# Sentinel write is the wrapper's last act; the temp+mv makes it atomic so the poll below can
# never read a half-written status.
cat > "$WRAPPER" <<WRAPPER_EOF
#!/bin/sh
/bin/sh '$COMMAND_FILE' >> '$LOG' 2>&1
status=\$?
printf '%s\n' "\$status" > '$SENTINEL.partial'
mv '$SENTINEL.partial' '$SENTINEL'
exit \$status
WRAPPER_EOF
chmod +x "$WRAPPER"

echo "run-under-terminal: hosting $COMMAND_FILE under Terminal (log: $LOG)"
# Capture the hosted window's id so it can be closed deterministically afterwards. Matching on
# window NAME does not work: the wrapper exits and the window reverts to the shell's own title,
# so a name match silently never fires and windows pile up across runs.
WINDOW_ID=$(osascript <<OSA_EOF 2>/dev/null || echo ""
tell application "Terminal"
  do script "'$WRAPPER'"
  return id of front window
end tell
OSA_EOF
)

# Stream the hosted output so a long run stays observable from the ssh side.
tail -f "$LOG" &
TAIL_PID=$!
trap 'kill "$TAIL_PID" 2>/dev/null || true' EXIT INT TERM

waited=0
while [ ! -f "$SENTINEL" ]; do
  if [ "$waited" -ge "$TIMEOUT_S" ]; then
    echo "run-under-terminal: hosted run did not finish within ${TIMEOUT_S}s (no exit-code sentinel at $SENTINEL)" >&2
    exit 4
  fi
  sleep 2
  waited=$((waited + 2))
done

sleep 1
# Braces + wait keep the shell's job-control "Terminated" notice off the caller's transcript.
{ kill "$TAIL_PID"; wait "$TAIL_PID"; } 2>/dev/null || true
trap - EXIT INT TERM

# Best-effort tidy so repeat runs do not pile up console windows.
case "$WINDOW_ID" in
  '' | *[!0-9]*) : ;;
  *) osascript -e "tell application \"Terminal\" to close (every window whose id is $WINDOW_ID)" >/dev/null 2>&1 || true ;;
esac

STATUS=$(cat "$SENTINEL" 2>/dev/null || echo "")
case "$STATUS" in
  '' | *[!0-9]*)
    echo "run-under-terminal: exit-code sentinel missing or non-numeric (got '${STATUS}') — refusing to report success" >&2
    exit 5
    ;;
esac

echo "run-under-terminal: hosted run exited ${STATUS}"
exit "$STATUS"
