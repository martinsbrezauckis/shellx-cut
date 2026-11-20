#!/usr/bin/env bash

# Keep a failed structural-audit runtime intact unless its owned server stopped
# and nothing still references the runtime child. Callers provide the project
# root so removal cannot escape the dedicated .scratch prefix.
coverage_audit_cargo_executable() {
  local messages="${1:?Cargo message file is required}"
  jq -er -s '[.[] | select(.reason == "compiler-artifact" and .target.name == "cutd" and (.target.kind | index("bin")) and (.executable | type == "string")) | .executable] | if length == 1 then .[0] else error("expected exactly one cutd executable artifact") end' "$messages"
}

coverage_audit_runtime_has_holders() {
  local runtime_root="${1:?runtime root is required}"
  local scan_status
  if [[ -d /proc ]]; then
    if node --input-type=module - "$runtime_root" <<'NODE'
import { readdirSync, realpathSync } from 'node:fs'

const rootArgument = process.argv[2]
const maximumEntries = 20_000
let checkedEntries = 0
let root
try {
  root = realpathSync(rootArgument)
} catch {
  process.exit(2)
}
const withinRoot = (path) => path === root || path.startsWith(root + '/')
const inspect = (path) => {
  checkedEntries += 1
  if (checkedEntries > maximumEntries) process.exit(2)
  try {
    if (withinRoot(realpathSync(path))) process.exit(0)
  } catch (error) {
    if (error?.code !== 'ENOENT') process.exit(2)
  }
}
let processes
try {
  processes = readdirSync('/proc')
} catch {
  process.exit(2)
}
for (const processId of processes) {
  if (!/^\d+$/.test(processId)) continue
  const rootPath = '/proc/' + processId
  inspect(rootPath + '/cwd')
  let descriptors
  try {
    descriptors = readdirSync(rootPath + '/fd')
  } catch (error) {
    if (error?.code === 'ENOENT') continue
    process.exit(2)
  }
  for (const descriptor of descriptors) inspect(rootPath + '/fd/' + descriptor)
}
// Exit 10 is reserved for this scanner's positive no-holder result. Any Node
// startup, syntax, permission, or runtime failure remains fail-closed.
process.exit(10)
NODE
    then
      return 0
    else
      scan_status=$?
      [[ "$scan_status" == 10 ]] && return 1
      return 0
    fi
  fi
  coverage_audit_lsof_has_holders "$runtime_root"
}

coverage_audit_lsof_has_holders() {
  local runtime_root="${1:?runtime root is required}"
  local scan_status output
  command -v lsof >/dev/null 2>&1 || return 0
  if output="$(lsof -t +D "$runtime_root" 2>&1)"; then
    return 0
  else
    scan_status=$?
  fi
  [[ "$scan_status" == 1 && -z "$output" ]] && return 1
  return 0
}

coverage_audit_cleanup_runtime() {
  local project_root="${1:?project root is required}"
  local runtime_root="${AUDIT_RUNTIME_ROOT:-}"
  local pid="${CUTD_PID:-}"
  local can_remove_runtime=1 attempt
  if [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null; then
    if safe_stop_owned_process_group "$pid"; then
      for attempt in $(seq 1 20); do
        kill -0 "$pid" 2>/dev/null || break
        sleep 0.1
      done
      if ! kill -0 "$pid" 2>/dev/null; then
        wait "$pid" 2>/dev/null || true
      fi
    else
      can_remove_runtime=0
    fi
  fi
  if [[ -n "$pid" ]] && kill -0 "$pid" 2>/dev/null; then
    can_remove_runtime=0
  fi
  if [[ -n "$runtime_root" && -d "$runtime_root" ]] && coverage_audit_runtime_has_holders "$runtime_root"; then
    can_remove_runtime=0
  fi
  if [[ "$can_remove_runtime" == 1 && -n "$runtime_root" && -d "$runtime_root" && "$runtime_root" == "$project_root/.scratch/coverage-audit-state."* ]]; then
    rm -rf -- "$runtime_root"
  elif [[ -n "$runtime_root" && -d "$runtime_root" ]]; then
    echo "[audit] retained runtime evidence after incomplete shutdown: $runtime_root" >&2
  fi
}
