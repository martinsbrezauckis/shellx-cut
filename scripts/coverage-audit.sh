#!/usr/bin/env bash
# coverage-audit.sh — contract gate: REST and MCP both expose 100% of
# schema/verbs.json, the machine-readable API contract.
#
# REST side: POST every verb against a fresh isolated cutd. Zero-required
# object schemas receive []; all other object schemas receive {}. Both are
# structurally invalid, so the request cannot reach a route handler.
# This is registry and argument-validation coverage, not handler-behavior proof.
#   PASS = exactly {ok:false,error:{code:"invalid_args",…}}, proving the verb
#          is routed and rejected by its argument schema.
#   FAIL = connection error, HTTP 404/5xx, any accepted handler, or another
#          envelope (panic, unrouted verb, or changed validation behavior).
# MCP side: scripts/mcp-probe.mjs (initialize + tools/list over stdio) must
#   list every verb as a tool, dots→underscores (the REST-to-MCP tool-name mapping contract).
#
# The audit always cold-starts its own throwaway cutd with isolated Cut state.
# A missing project does not protect global setup, tool, model, or preference
# routes, so auditing an existing server is deliberately unsupported.
#
# Env knobs:
#   AUDIT_ADDR   bind address for the throwaway server (default 127.0.0.1:6167)
#   AUDIT_REST_TIMEOUT_SECONDS    ordinary structural verb timeout (default 30)
#
# Exit: 0 only when BOTH sides report N/N. Dependencies: cargo, curl, jq, node.
# Callers: humans and CI.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
source "$ROOT/scripts/lib/safe-process-group.sh"
source "$ROOT/scripts/lib/coverage-audit-request.sh"
source "$ROOT/scripts/lib/coverage-audit-runtime.sh"
VERBS_JSON="$ROOT/schema/verbs.json"
CUTD_BIN=""
AUDIT_ADDR="${AUDIT_ADDR:-127.0.0.1:6167}"
REST_TIMEOUT_SECONDS="${AUDIT_REST_TIMEOUT_SECONDS:-30}"
CUTD_PID=""
AUDIT_RUNTIME_ROOT=""
AUDIT_POLICY_PLAN=""

# cleanup — stop only the throwaway server and retain evidence on uncertainty.
cleanup() {
  coverage_audit_cleanup_runtime "$ROOT"
}
trap cleanup EXIT

die() { echo "AUDIT FAIL: $*" >&2; exit 1; }

[[ -z "${CUTD_ADDR:-}" ]] || die "CUTD_ADDR is unsupported: coverage audit always owns an isolated cutd"

for tool in jq curl node cargo; do
  command -v "$tool" >/dev/null || die "'$tool' not found on PATH — required by coverage-audit.sh"
done
[[ "$REST_TIMEOUT_SECONDS" =~ ^[1-9][0-9]*$ ]] || die "AUDIT_REST_TIMEOUT_SECONDS must be a positive integer"
[[ -s "$VERBS_JSON" ]] || die "$VERBS_JSON missing or empty — the verb registry IS the contract"

VERBS=()
while IFS= read -r verb; do
  VERBS+=("$verb")
done < <(jq -r '.verbs[].name' "$VERBS_JSON")
TOTAL=${#VERBS[@]}
[[ "$TOTAL" -gt 0 ]] || die "no verbs parsed from $VERBS_JSON (schema shape changed?)"

mkdir -p "$ROOT/.scratch"
AUDIT_RUNTIME_ROOT="$(mktemp -d "$ROOT/.scratch/coverage-audit-state.XXXXXX")"
mkdir -p "$AUDIT_RUNTIME_ROOT"/{state,projects,data,cache,tmp,model-cache}
AUDIT_POLICY_PLAN="$AUDIT_RUNTIME_ROOT/structural-payloads.tsv"
if ! node "$ROOT/scripts/lib/coverage-audit-policy.mjs" \
  --schema "$VERBS_JSON" \
  --contracts "$ROOT/app/core/src/verb_contract.rs" >"$AUDIT_POLICY_PLAN"; then
  die "coverage payload policy rejected the current schema or verb contracts"
fi
AUDIT_PAYLOADS=()
plan_index=0
while IFS=$'\t' read -r verb payload; do
  [[ "$plan_index" -lt "$TOTAL" ]] || die "coverage payload policy emitted an unexpected entry: $verb"
  expected_verb="${VERBS[$plan_index]}"
  [[ "$verb" == "$expected_verb" && ( "$payload" == '{}' || "$payload" == '[]' ) ]] \
    || die "coverage payload policy entry $plan_index must be $expected_verb with an invalid JSON payload"
  AUDIT_PAYLOADS[$plan_index]="$payload"
  plan_index=$((plan_index + 1))
done <"$AUDIT_POLICY_PLAN"
[[ "$plan_index" -eq "$TOTAL" ]] || die "coverage payload policy omitted ${VERBS[$plan_index]}"

# ------------------------------------------------------------ build + spawn --
if ! cargo build --manifest-path "$ROOT/app/Cargo.toml" -p server --bin cutd --locked --message-format=json-render-diagnostics >"$AUDIT_RUNTIME_ROOT/cargo-messages.jsonl" 2>"$AUDIT_RUNTIME_ROOT/cargo-stderr.log"; then
  tail -n 40 "$AUDIT_RUNTIME_ROOT/cargo-stderr.log" >&2
  die "cargo build -p server failed — cannot audit an unbuildable server"
fi
CUTD_BIN="$(coverage_audit_cargo_executable "$AUDIT_RUNTIME_ROOT/cargo-messages.jsonl")" || die "Cargo did not report exactly one cutd executable"
[[ "$CUTD_BIN" == /* && -f "$CUTD_BIN" && -x "$CUTD_BIN" ]] || die "Cargo reported an invalid cutd executable"
cutd_sha256() { node -e 'const fs=require("node:fs"),crypto=require("node:crypto");const h=crypto.createHash("sha256");const s=fs.createReadStream(process.argv[1]);s.on("data",c=>h.update(c));s.on("end",()=>console.log(h.digest("hex")));s.on("error",()=>process.exit(1));' "$CUTD_BIN"; }
CUTD_SHA256="$(cutd_sha256)" || die "cannot hash Cargo-reported cutd"
echo "[audit] Cargo cutd: $CUTD_BIN sha256=$CUTD_SHA256"

# Keep Cut's mutable state, appdata, models, caches, projects, and temporary
# files inside one owned child. HOME is intentionally preserved: the audit does
# not repoint or inspect provider-owned homes/authentication.
export SHELLX_CUT_HOME="$AUDIT_RUNTIME_ROOT/state"
export SHELLX_CUT_PROJECTS_DIR="$AUDIT_RUNTIME_ROOT/projects"
export XDG_DATA_HOME="$AUDIT_RUNTIME_ROOT/data"
export XDG_CACHE_HOME="$AUDIT_RUNTIME_ROOT/cache"
export TMPDIR="$AUDIT_RUNTIME_ROOT/tmp"
export HF_HOME="$AUDIT_RUNTIME_ROOT/model-cache"
unset SHELLX_CUT_PYTHON SHELLX_CUT_SIDECAR_DIR SHELLX_CUT_UV SHELLX_CUT_FFMPEG SHELLX_CUT_FFMPEG_DIR FFMPEG_BIN MATTE_MODEL MATTE_RUNNER_PY MATTE_RUNNER_SCRIPT

SERVER_LOG=""
ADDR="$AUDIT_ADDR"
curl -s --max-time 2 "http://$ADDR/api/verbs" >/dev/null 2>&1 \
  && die "something already listens on $ADDR — set AUDIT_ADDR to a free port"
SERVER_LOG="$AUDIT_RUNTIME_ROOT/coverage-audit-server.log"
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
echo "[audit] isolated throwaway cutd on $ADDR (pid $CUTD_PID)"

# ------------------------------------------------------------ REST coverage --
rest_pass=0
rest_failed=()
for ((plan_index = 0; plan_index < TOTAL; plan_index = plan_index + 1)); do
  v="${VERBS[$plan_index]}"
  payload="${AUDIT_PAYLOADS[$plan_index]}"
  timeout_seconds="$REST_TIMEOUT_SECONDS"
  # -w token splits body from HTTP status; curl transport errors → empty.
  out=$(coverage_audit_request "$ADDR" "$v" "$payload" "$timeout_seconds" 2>&1) || out=""
  code="${out##*$'\n'}"
  body="${out%$'\n'*}"
  if [[ -z "$out" ]]; then
    echo "  FAIL $v — transport error (connection refused/timeout)"
    rest_failed+=("$v")
  elif [[ "$code" == 404 || "$code" =~ ^5 ]]; then
    echo "  FAIL $v — HTTP $code (unrouted or panicked): $(printf '%s' "$body" | head -c 200)"
    rest_failed+=("$v")
  else
    response_verdict="$(coverage_audit_response_verdict "$body")"
    case "$response_verdict" in
      validation)
        echo "  PASS $v (invalid_args)"
        rest_pass=$((rest_pass + 1))
        ;;
      accepted)
        echo "  FAIL $v — handler accepted an intentionally invalid structural payload"
        rest_failed+=("$v")
        ;;
      *)
        echo "  FAIL $v — expected invalid_args validation envelope: $(printf '%s' "$body" | head -c 200)"
        rest_failed+=("$v")
        ;;
    esac
  fi
done
echo "REST coverage: $rest_pass/$TOTAL"

# ------------------------------------------------------------- MCP coverage --
# tools/list via the stdio probe; tool name = verb with dots → underscores.
[[ "$(cutd_sha256)" == "$CUTD_SHA256" ]] || die "Cargo-reported cutd changed after REST launch"
if ! tools_json=$(CUTD_PROXY_ADDR="$ADDR" CUTD_PROXY_ACTOR="" SHELLX_CUT_AGENT_CONTAINED="" node "$ROOT/scripts/mcp-probe.mjs" --cutd "$CUTD_BIN"); then
  die "mcp-probe.mjs failed — MCP side unreachable (see stderr above)"
fi
[[ "$(cutd_sha256)" == "$CUTD_SHA256" ]] || die "Cargo-reported cutd changed during MCP probe"
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
  exit 0
fi
[[ ${#rest_failed[@]} -gt 0 ]] && echo "REST missing: ${rest_failed[*]}" >&2
[[ ${#mcp_failed[@]} -gt 0 ]] && echo "MCP missing:  ${mcp_failed[*]}" >&2
[[ -n "$SERVER_LOG" ]] && echo "server log kept: $SERVER_LOG" >&2
die "coverage incomplete (REST $rest_pass/$TOTAL, MCP $mcp_pass/$TOTAL)"
