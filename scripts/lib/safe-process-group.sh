#!/usr/bin/env bash

# Stop only a positively identified child-owned process group. Negating 0 or 1
# has system-wide signal semantics on Unix and must always fail closed.
safe_stop_owned_process_group() {
  local owned_pid="${1:-}"
  if [[ ! "$owned_pid" =~ ^[0-9]+$ ]] || (( owned_pid <= 1 )); then
    printf 'refusing unsafe owned process group: %q\n' "$owned_pid" >&2
    return 64
  fi
  kill -- "-$owned_pid" 2>/dev/null || kill "$owned_pid" 2>/dev/null || true
}
