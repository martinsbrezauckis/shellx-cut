#!/bin/sh
set -eu

: "${SHELLX_CUT_WDIO_REAL_APP:?SHELLX_CUT_WDIO_REAL_APP is required}"
: "${SHELLX_CUT_WDIO_APP_CWD:?SHELLX_CUT_WDIO_APP_CWD is required}"

test -x "$SHELLX_CUT_WDIO_REAL_APP" || {
  echo "native app is not executable: $SHELLX_CUT_WDIO_REAL_APP" >&2
  exit 126
}
test -d "$SHELLX_CUT_WDIO_APP_CWD" || {
  echo "isolated native app cwd does not exist: $SHELLX_CUT_WDIO_APP_CWD" >&2
  exit 126
}

cd -- "$SHELLX_CUT_WDIO_APP_CWD"
exec "$SHELLX_CUT_WDIO_REAL_APP" "$@"
