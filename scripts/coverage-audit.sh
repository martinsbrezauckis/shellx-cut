#!/usr/bin/env bash
# coverage-audit.sh — contract gate: REST and MCP both expose 100% of
# schema/verbs.json, the machine-readable API contract.
#
# REST side: POST every verb with EMPTY args {} against a live cutd, except
# debug.screenshot, which receives an invalid monitor type so this structural
# gate never launches a real OS capture/portal on a headless verifier.
#   PASS = a structured envelope came back ({ok:…}; ok:false with
#          error{code,message,cause} is a PASS — validation errors prove the
#          verb is routed and argument-checked).
#   FAIL = connection error, HTTP 404/5xx, non-JSON, or missing .ok (panic,
#          unrouted verb, or transport breakage).
# MCP side: scripts/mcp-probe.mjs (initialize + tools/list over stdio) must
#   list every verb as a tool, dots→underscores (the REST-to-MCP tool-name mapping contract).
#
# By default the audit COLD-STARTS ITS OWN throwaway cutd with NO project
# open — so "mutating" verbs can't damage anything (they all error
# structurally with no_project, which is exactly what we want to observe).
# Set CUTD_ADDR to audit an already-running server instead — but know that
# every verb WILL be posted at it, including project.close.
#
# Env knobs:
#   CUTD_ADDR    use an existing server (skip spawn); e.g. 127.0.0.1:6161
#   AUDIT_ADDR   bind address for the throwaway server (default 127.0.0.1:6167)
#
# Exit: 0 only when BOTH sides report N/N. Dependencies: cargo, curl, jq, node.
# Callers: humans and CI.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VERBS_JSON="$ROOT/schema/verbs.json"
CUTD_BIN="$ROOT/app/target/debug/cutd"
ADDR="${CUTD_ADDR:-}"
AUDIT_ADDR="${AUDIT_ADDR:-127.0.0.1:6167}"
CUTD_PID=""

# cleanup — kill the throwaway server (never a user's own CUTD_ADDR server).
cleanup() {
  if [[ -n "$CUTD_PID" ]] && kill -0 "$CUTD_PID" 2>/dev/null; then
    kill -- -"$CUTD_PID" 2>/dev/null || kill "$CUTD_PID" 2>/dev/null || true
    wait "$CUTD_PID" 2>/dev/null || true
  fi
}
trap cleanup EXIT

die() { echo "AUDIT FAIL: $*" >&2; exit 1; }

for tool in jq curl node cargo; do
  command -v "$tool" >/dev/null || die "'$tool' not found on PATH — required by coverage-audit.sh"
done
[[ -s "$VERBS_JSON" ]] || die "$VERBS_JSON missing or empty — the verb registry IS the contract"

VERBS=()
while IFS= read -r verb; do
  VERBS+=("$verb")
done < <(jq -r '.verbs[].name' "$VERBS_JSON")
TOTAL=${#VERBS[@]}
[[ "$TOTAL" -gt 0 ]] || die "no verbs parsed from $VERBS_JSON (schema shape changed?)"

# ------------------------------------------------------------ build + spawn --
if ! build_out=$(cargo build --manifest-path "$ROOT/app/Cargo.toml" -p server 2>&1); then
  printf '%s\n' "$build_out" | tail -n 40 >&2
  die "cargo build -p server failed — cannot audit an unbuildable server"
fi

SERVER_LOG=""
if [[ -z "$ADDR" ]]; then
  ADDR="$AUDIT_ADDR"
  curl -s --max-time 2 "http://$ADDR/api/verbs" >/dev/null 2>&1 \
    && die "something already listens on $ADDR — set AUDIT_ADDR to a free port (or CUTD_ADDR to audit it deliberately)"
  mkdir -p "$ROOT/.scratch"
  SERVER_LOG="$(mktemp "$ROOT/.scratch/coverage-audit.XXXXXX.log")"
  if command -v setsid >/dev/null 2>&1; then
    setsid "$CUTD_BIN" serve --headless --addr "$ADDR" >"$SERVER_LOG" 2>&1 &
  else
    "$CUTD_BIN" serve --headless --addr "$ADDR" >"$SERVER_LOG" 2>&1 &
  fi
  CUTD_PID=$!
  for i in $(seq 1 60); do
    curl -s --max-time 2 "http://$ADDR/api/verbs" >/dev/null 2>&1 && break
    kill -0 "$CUTD_PID" 2>/dev/null || { tail -n 15 "$SERVER_LOG" >&2; die "cutd exited during startup — log tail above ($SERVER_LOG)"; }
    [[ "$i" == 60 ]] && die "cutd not answering on $ADDR after 30s ($SERVER_LOG)"
    sleep 0.5
  done
  echo "[audit] throwaway cutd on $ADDR (pid $CUTD_PID, no project open — nothing to mutate)"
else
  curl -s --max-time 3 "http://$ADDR/api/verbs" >/dev/null 2>&1 \
    || die "no cutd answering at CUTD_ADDR=$ADDR"
  echo "[audit] using existing server at $ADDR (every verb WILL be posted at it)"
fi

# ------------------------------------------------------------ REST coverage --
rest_pass=0
rest_failed=()
for v in "${VERBS[@]}"; do
  payload='{}'
  [[ "$v" == "debug.screenshot" ]] && payload='{"monitor":"structural-audit"}'
  # -w token splits body from HTTP status; curl transport errors → empty.
  out=$(curl -sS --max-time 30 -H 'content-type: application/json' \
        -X POST "http://$ADDR/api/verb/$v" -d "$payload" \
        -w $'\n%{http_code}' 2>&1) || out=""
  code="${out##*$'\n'}"
  body="${out%$'\n'*}"
  if [[ -z "$out" ]]; then
    echo "  FAIL $v — transport error (connection refused/timeout)"
    rest_failed+=("$v")
  elif [[ "$code" == 404 || "$code" =~ ^5 ]]; then
    echo "  FAIL $v — HTTP $code (unrouted or panicked): $(printf '%s' "$body" | head -c 200)"
    rest_failed+=("$v")
  elif ! printf '%s' "$body" | jq -e 'has("ok")' >/dev/null 2>&1; then
    echo "  FAIL $v — non-envelope response: $(printf '%s' "$body" | head -c 200)"
    rest_failed+=("$v")
  else
    # Structured envelope — ok:true or a structured error both prove routing.
    status=$(printf '%s' "$body" | jq -r 'if .ok then "ok" else (.error.code // "error") end')
    echo "  PASS $v ($status)"
    rest_pass=$((rest_pass + 1))
  fi
done
echo "REST coverage: $rest_pass/$TOTAL"

# ------------------------------------------------------------- MCP coverage --
# tools/list via the stdio probe; tool name = verb with dots → underscores.
if ! tools_json=$(node "$ROOT/scripts/mcp-probe.mjs"); then
  die "mcp-probe.mjs failed — MCP side unreachable (see stderr above)"
fi
mcp_pass=0
mcp_failed=()
for v in "${VERBS[@]}"; do
  tool="${v//./_}"
  if printf '%s' "$tools_json" | jq -e --arg t "$tool" '.tools | index($t) != null' >/dev/null 2>&1; then
    mcp_pass=$((mcp_pass + 1))
  else
    echo "  FAIL mcp:$tool — not in tools/list"
    mcp_failed+=("$tool")
  fi
done
echo "MCP coverage:  $mcp_pass/$TOTAL"

# ------------------------------------------------------------------ verdict --
if [[ ${#rest_failed[@]} -eq 0 && ${#mcp_failed[@]} -eq 0 ]]; then
  echo "COVERAGE AUDIT PASS — $TOTAL/$TOTAL verbs on REST and MCP"
  [[ -n "$SERVER_LOG" ]] && rm -f "$SERVER_LOG"
  exit 0
fi
[[ ${#rest_failed[@]} -gt 0 ]] && echo "REST missing: ${rest_failed[*]}" >&2
[[ ${#mcp_failed[@]} -gt 0 ]] && echo "MCP missing:  ${mcp_failed[*]}" >&2
[[ -n "$SERVER_LOG" ]] && echo "server log kept: $SERVER_LOG" >&2
die "coverage incomplete (REST $rest_pass/$TOTAL, MCP $mcp_pass/$TOTAL)"
