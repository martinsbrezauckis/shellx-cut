#!/usr/bin/env bash
# e2e.sh — THE definition of done (end-to-end test contract). Full pipeline against a real cutd:
#   build → test assets → cold-start headless on a temp project → import →
#   wait job chain → checkpoint → remove_silences(natural) → remove_fillers →
#   cut_words(known span) → captions.generate → render.final → receipt asserts
#   (ALL public verb contract checks PASS — receipt.pass true — plus lufs carries a
#   measured number and duration shrank) →
#   export.srt + export.xml(fcpxml) non-empty → project.diff lists the ops.
#
# Exit nonzero on the FIRST failure, with a clear "FAIL <step>" line and a
# response excerpt — diagnostics are the product here: the server may not be
# fully built yet and this script must say exactly what is missing, not die
# cryptically. Every step prints one PASS line on success.
#
# Surfaces driven: REST POST /api/verb/{name} (curl) — the same surface agents
# use, so green here means the agent loop works. Envelope contract (public verb contract;
# schema/verbs.json is the machine-readable source of truth):
#   {ok, result?, op_ids?, warnings?[], error?{code,message,clip_id?,at_ms?,cause,suggested_action?}}
#
# Env knobs:
#   E2E_ADDR        bind address for the throwaway cutd (default 127.0.0.1:6166
#                   — NOT the dev default 6161, so a running dev server is
#                   never clobbered; cutd serve --addr makes this safe)
#   E2E_JOB_TIMEOUT seconds to wait for the import job chain / render job
#                   (default 900 — first whisperX run downloads models)
#   E2E_KEEP=1      keep the temp project dir even on success
#
# Dependencies: bash, curl, jq, cargo (rust), ffmpeg+espeak-ng (via
# make-test-assets.sh — owned by the perception track; we only invoke it).
# Callers: humans, CI, and integration agents.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ADDR="${E2E_ADDR:-127.0.0.1:6166}"
JOB_TIMEOUT="${E2E_JOB_TIMEOUT:-900}"
CUTD_BIN="$ROOT/app/target/debug/cutd"
ASSET_MP4="$ROOT/testdata/talking_head.mp4"

# Scratch area for this run (gitignored). Kept on failure for post-mortem.
mkdir -p "$ROOT/.scratch"
RUN_DIR="$(mktemp -d "$ROOT/.scratch/e2e-run.XXXXXX")"
SERVER_LOG="$RUN_DIR/cutd.log"
CUTD_PID=""
E2E_OK=0   # flipped to 1 only by the final summary

# ---------------------------------------------------------------- helpers --

# cleanup — trap target: kill cutd (and its ffmpeg children via process
# group), keep artifacts on failure, delete them on success unless E2E_KEEP.
cleanup() {
  local code=$?
  if [[ -n "$CUTD_PID" ]] && kill -0 "$CUTD_PID" 2>/dev/null; then
    # cutd spawns ffmpeg/python subprocesses; nuke the whole process group.
    kill -- -"$CUTD_PID" 2>/dev/null || kill "$CUTD_PID" 2>/dev/null || true
    wait "$CUTD_PID" 2>/dev/null || true
  fi
  if [[ "$E2E_OK" == 1 && "${E2E_KEEP:-0}" != 1 ]]; then
    rm -rf "$RUN_DIR"
  else
    echo "[e2e] artifacts kept at: $RUN_DIR (server log: cutd.log)" >&2
  fi
  exit "$code"
}
trap cleanup EXIT

pass() { echo "PASS: $*"; }

# fail STEP MESSAGE [RESPONSE] — one honest failure line + excerpt + log tail.
fail() {
  local step="$1" msg="$2" resp="${3:-}"
  echo "FAIL: $step — $msg" >&2
  if [[ -n "$resp" ]]; then
    echo "  response excerpt: $(printf '%s' "$resp" | head -c 800)" >&2
  fi
  if [[ -s "$SERVER_LOG" ]]; then
    echo "  --- cutd log tail ---" >&2
    tail -n 15 "$SERVER_LOG" >&2
  fi
  exit 1
}

# verb NAME JSON — POST /api/verb/NAME, echo the envelope body. Distinguishes
# transport failure (connection refused / timeout) from verb errors so the
# caller's diagnostic is actionable. HTTP is 200 even on verb errors (the
# envelope's .ok is the contract — app/server/src/http.rs).
verb() {
  local name="$1" args="${2:-{\}}" body
  if ! body=$(curl -sS --max-time 120 -H 'content-type: application/json' \
      -X POST "http://$ADDR/api/verb/$name" -d "$args" 2>&1); then
    fail "verb $name" "transport error talking to cutd at $ADDR (is it still running?)" "$body"
  fi
  printf '%s' "$body"
}

# assert_ok STEP RESPONSE — envelope must be JSON with .ok == true.
assert_ok() {
  local step="$1" resp="$2"
  if ! printf '%s' "$resp" | jq -e '.ok == true' >/dev/null 2>&1; then
    fail "$step" "verb returned ok:false (or non-JSON)" "$resp"
  fi
}

# jqr RESPONSE FILTER — extract with jq -r; empty string when path missing.
jqr() { printf '%s' "$1" | jq -r "$2 // empty" 2>/dev/null || true; }

# wait_job STEP JOB_ID — poll jobs.status until state=done; fail on
# state=failed or timeout. JobRecord per verbs.json: {state, progress, error?}.
# (WS job_progress events are the fast path for live agents; polling is the
# honest bash equivalent.)
wait_job() {
  local step="$1" job_id="$2" deadline=$((SECONDS + JOB_TIMEOUT)) resp state last_echo=0
  [[ -n "$job_id" ]] || fail "$step" "no job_id to poll (verb result missing job_id)"
  while true; do
    resp=$(verb jobs.status "{\"job_id\":\"$job_id\"}")
    assert_ok "$step (jobs.status $job_id)" "$resp"
    state=$(jqr "$resp" '.result.state')
    case "$state" in
      done) return 0 ;;
      failed) fail "$step" "job $job_id failed" "$resp" ;;
      queued|running|"")
        if (( SECONDS - last_echo >= 15 )); then
          echo "  … $step: job $job_id state=${state:-?} progress=$(jqr "$resp" '.result.progress')" >&2
          last_echo=$SECONDS
        fi
        ;;
      *) fail "$step" "job $job_id in unknown state '$state'" "$resp" ;;
    esac
    (( SECONDS < deadline )) || fail "$step" "job $job_id not done after ${JOB_TIMEOUT}s (raise E2E_JOB_TIMEOUT?)" "$resp"
    sleep 2
  done
}

# -------------------------------------------------------- step 0: preflight --
for tool in cargo curl jq; do
  command -v "$tool" >/dev/null || fail "preflight" "'$tool' not found on PATH — required by e2e.sh (apt/rustup install it)"
done
pass "preflight — cargo, curl, jq present"

# ------------------------------------------------------ step 1: build check --
# Build only the server package (pulls core/media/perception transitively).
if ! build_out=$(cargo build --manifest-path "$ROOT/app/Cargo.toml" -p server 2>&1); then
  printf '%s\n' "$build_out" | tail -n 40 >&2
  fail "build" "cargo build -p server failed — see compiler output above (other tracks may be mid-edit)"
fi
[[ -x "$CUTD_BIN" ]] || fail "build" "build succeeded but $CUTD_BIN missing — did the [[bin]] name change from 'cutd'?"
pass "build — cutd compiled ($CUTD_BIN)"

# ------------------------------------------------------ step 2: test assets --
# make-test-assets.sh is OWNED by the perception track (end-to-end test contract); we only
# invoke it. Idempotent by contract — skip if the asset already exists.
if [[ ! -s "$ASSET_MP4" ]]; then
  if ! assets_out=$("$ROOT/scripts/make-test-assets.sh" 2>&1); then
    printf '%s\n' "$assets_out" | tail -n 20 >&2
    fail "test-assets" "make-test-assets.sh failed — perception track owns it (needs espeak-ng + ffmpeg, end-to-end test contract)"
  fi
fi
[[ -s "$ASSET_MP4" ]] || fail "test-assets" "$ASSET_MP4 still missing after make-test-assets.sh ran"
pass "test-assets — talking_head.mp4 present ($(du -h "$ASSET_MP4" | cut -f1))"

# ------------------------------------------- step 3: cold-start cutd headless --
# Fresh server, no project preloaded (the e2e creates its own — true cold
# start). setsid → own process group so cleanup can kill ffmpeg children too;
# macOS has no setsid, so fall back to a plain background launch there (cleanup
# already degrades from a process-group kill to a plain kill, line 54).
if curl -s --max-time 2 "http://$ADDR/api/verbs" >/dev/null 2>&1; then
  fail "cold-start" "something already listens on $ADDR — set E2E_ADDR to a free port"
fi
if command -v setsid >/dev/null 2>&1; then
  setsid "$CUTD_BIN" serve --headless --addr "$ADDR" >"$SERVER_LOG" 2>&1 &
else
  "$CUTD_BIN" serve --headless --addr "$ADDR" >"$SERVER_LOG" 2>&1 &
fi
CUTD_PID=$!
for i in $(seq 1 60); do
  curl -s --max-time 2 "http://$ADDR/api/verbs" >/dev/null 2>&1 && break
  kill -0 "$CUTD_PID" 2>/dev/null || fail "cold-start" "cutd exited during startup — log tail follows"
  [[ "$i" == 60 ]] && fail "cold-start" "cutd not answering GET /api/verbs on $ADDR after 30s"
  sleep 0.5
done
pass "cold-start — cutd serving on $ADDR (pid $CUTD_PID, headless)"

# --------------------------------------------------- step 4: project.create --
RESP=$(verb project.create "{\"name\":\"e2e\",\"dir\":\"$RUN_DIR/e2e.cutproj\"}")
assert_ok "project.create" "$RESP"
PROJ_PATH=$(jqr "$RESP" '.result.path')
[[ -n "$PROJ_PATH" ]] || fail "project.create" "result.path missing from envelope" "$RESP"
pass "project.create — $PROJ_PATH"

# ----------------------------------------------------- step 5: media.import --
RESP=$(verb media.import "{\"path\":\"$ASSET_MP4\",\"rationale\":\"e2e talking-head source\"}")
assert_ok "media.import" "$RESP"
ASSET_ID=$(jqr "$RESP" '.result.asset_id')
IMPORT_JOB=$(jqr "$RESP" '.result.job_id')
[[ -n "$ASSET_ID" ]] || fail "media.import" "result.asset_id missing" "$RESP"
pass "media.import — asset=$ASSET_ID job=$IMPORT_JOB"

# ------------------------------ step 6: wait import + enrich via jobs.status --
# Import is now probe→proxy→filmstrip→READY-TO-EDIT (fast); transcribe+perception
# moved to a separate background ENRICH job decouple — a slow
# Transcription must never block editing; large files must not freeze import at
# 55% for ~2h). The `enrich_job` id is in the import-chain job's FINISH result
# (it is spawned at ready-to-edit), not the verb response — so read it AFTER the
# import job is done. Then wait for enrich (transcript+perception).
wait_job "import-chain" "$IMPORT_JOB"
IMPORT_DONE=$(verb jobs.status "{\"job_id\":\"$IMPORT_JOB\"}")
ENRICH_JOB=$(jqr "$IMPORT_DONE" '.result.result.enrich_job')
[[ -n "$ENRICH_JOB" ]] || fail "media.import" "import-chain result.enrich_job missing (decouple contract)" "$IMPORT_DONE"
pass "media.import — ready-to-edit, enrich job=$ENRICH_JOB"
wait_job "enrich" "$ENRICH_JOB"
# Capture the ORIGINAL duration now — the receipt assert needs it (shrank?).
RESP=$(verb media.probe "{\"asset\":\"$ASSET_ID\"}")
assert_ok "media.probe" "$RESP"
ORIG_MS=$(jqr "$RESP" '.result.duration_ms')
[[ "$ORIG_MS" =~ ^[0-9]+$ && "$ORIG_MS" -gt 0 ]] || fail "media.probe" "duration_ms not a positive integer" "$RESP"
# Transcript drives steps 8–15 (the transcript-editing flow + the caption receipt
# check + SRT export). STT (Parakeet) is OPTIONAL / fetch-on-consent; when the
# perception venv is absent the enrich job skips transcription (transcript:false).
# Detect that honestly here and exit with an explicit dependency error instead
# of calling transcript.get blind and reporting a misleading
# "transcript missing" mid-run — this e2e exercises the transcript flow and needs it.
RESP=$(verb transcript.get "{\"asset\":\"$ASSET_ID\"}")
if [[ "$(jqr "$RESP" '.ok')" != "true" ]]; then
  echo
  echo "E2E DEPENDENCY MISSING — transcript not produced (perception / STT runtime absent)."
  echo "  This e2e exercises the TRANSCRIPT-editing flow (transcript.remove_silences /"
  echo "  remove_fillers / cut_words + captions.generate + the caption receipt check),"
  echo "  which needs the Parakeet STT perception venv. The core import/edit/render path"
  echo "  is fine; only the transcript-dependent assertions can't run."
  echo "  → Install it (fetch-on-consent): run the  system.setup_perception  verb, then re-run."
  echo "  (transcript.get error: $(jqr "$RESP" '.error.code') — $(jqr "$RESP" '.error.message'))"
  exit 3  # `trap cleanup EXIT` tears down cutd + scratch; distinct code = dependency, not a product failure
fi
WORD_COUNT=$(jqr "$RESP" '.result.words | length')
[[ "${WORD_COUNT:-0}" -gt 0 ]] || fail "transcript.get" "transcript present but has no words (empty enrichment)" "$RESP"
TRANSCRIPT_JSON="$RESP"
pass "import-chain — done; duration=${ORIG_MS}ms, transcript=${WORD_COUNT} words"

# ------------------------------------------------ step 7: project.checkpoint --
RESP=$(verb project.checkpoint '{"name":"e2e-start","rationale":"baseline before the e2e edit pass"}')
assert_ok "project.checkpoint" "$RESP"
pass "project.checkpoint — e2e-start"

# --------------------------------------- step 8: transcript.remove_silences --
# aggressiveness REQUIRED (the required-argument contract); testdata has known 2–4s silences, so
# at least one span must go.
RESP=$(verb transcript.remove_silences '{"aggressiveness":"natural","rationale":"e2e silence pass"}')
assert_ok "transcript.remove_silences" "$RESP"
SIL_SPANS=$(jqr "$RESP" '.result.spans_removed')
SIL_OPS=$(jqr "$RESP" '.op_ids | length')
[[ "${SIL_SPANS:-0}" -ge 1 ]] || fail "transcript.remove_silences" "expected ≥1 removed span (testdata has known silences)" "$RESP"
[[ "${SIL_OPS:-0}" -ge 1 ]] || fail "transcript.remove_silences" "no op_ids — one op per span is the contract" "$RESP"
pass "transcript.remove_silences — $SIL_SPANS spans, $SIL_OPS ops"

# ---------------------------------------- step 9: transcript.remove_fillers --
RESP=$(verb transcript.remove_fillers '{"rationale":"e2e filler pass"}')
assert_ok "transcript.remove_fillers" "$RESP"
FILLERS=$(jqr "$RESP" '.result.fillers_removed')
[[ "${FILLERS:-0}" -ge 1 ]] || fail "transcript.remove_fillers" "expected ≥1 filler removed (testdata scripts deliberate um/uh/so)" "$RESP"
pass "transcript.remove_fillers — $FILLERS filler runs removed"

# ------------------------------------------- step 10: transcript.cut_words --
# Pick a KNOWN span: 3 consecutive word indices, none of them filler-shaped
# (fillers are already gone from the timeline — cutting them again would be
# a bogus request, and an honest server may refuse it). The jq finds runs of
# 3 consecutive surviving indices and takes the middle run.
SPAN=$(printf '%s' "$TRANSCRIPT_JSON" | jq -c '
  [.result.words[]
   | select((.word | ascii_downcase | gsub("[^a-z]"; "")) as $w
            | (["um","uh","erm","ah","like","so","well","you","know","mean","right","okay"] | index($w)) == null)
   | .idx] as $ok
  | [range(0; ([($ok|length)-2, 0] | max))
     | select($ok[.]+1 == $ok[.+1] and $ok[.]+2 == $ok[.+2])] as $runs
  | if ($runs|length) == 0 then empty
    else $ok[$runs[($runs|length)/2 | floor]] as $s | [$s, $s+2] end')
[[ -n "$SPAN" ]] || fail "transcript.cut_words" "could not find 3 consecutive non-filler words to cut — transcript too short?" "$TRANSCRIPT_JSON"
RESP=$(verb transcript.cut_words "{\"asset\":\"$ASSET_ID\",\"word_range\":$SPAN,\"rationale\":\"e2e known-span cut\"}")
assert_ok "transcript.cut_words" "$RESP"
CW_OPS=$(jqr "$RESP" '.op_ids | length')
[[ "${CW_OPS:-0}" -ge 1 ]] || fail "transcript.cut_words" "no op appended for the cut" "$RESP"
pass "transcript.cut_words — span $SPAN cut ('$(jqr "$RESP" '.result.text')')"

# ---------------------------------------------- step 11: captions.generate --
RESP=$(verb captions.generate '{"rationale":"e2e caption pass"}')
assert_ok "captions.generate" "$RESP"
CAPS=$(jqr "$RESP" '.result.caption_count')
[[ "${CAPS:-0}" -ge 1 ]] || fail "captions.generate" "caption_count is 0 — no captions produced from transcript" "$RESP"
pass "captions.generate — $CAPS captions on track $(jqr "$RESP" '.result.track_id')"

# -------------------------------------------------- step 12: render.final --
RESP=$(verb render.final '{"rationale":"e2e final render"}')
assert_ok "render.final" "$RESP"
RENDER_JOB=$(jqr "$RESP" '.result.job_id')
RENDER_ID=$(jqr "$RESP" '.result.render_id')
wait_job "render.final" "$RENDER_JOB"
pass "render.final — job done (render_id=$RENDER_ID)"

# ---------------------------------- step 13: receipt (wait + assert facts) --
# the event-ordering contract: receipt_ready always follows render_done. Without a WS client we
# poll verify.checks until the receipt exists — the same ordering guarantee,
# observed via REST. 60s grace is generous (checks are deterministic + fast).
# Receipt shape: RenderReceipt {render_id, checks[{name,pass,details,evidence}],
# pass, duration_ms, output_hash, …} (app/core/src/receipt.rs).
RECEIPT=""
RESP=""
for i in $(seq 1 30); do
  RESP=$(verb verify.checks "{\"render_id\":\"$RENDER_ID\"}")
  if printf '%s' "$RESP" | jq -e '.ok == true and ((.result.checks // .result.receipt.checks // []) | length > 0)' >/dev/null 2>&1; then
    RECEIPT=$(printf '%s' "$RESP" | jq -c '.result.receipt // .result')
    break
  fi
  sleep 2
done
[[ -n "$RECEIPT" ]] || fail "receipt" "verify.checks never returned a receipt with checks (receipt_ready missing after render_done?)" "$RESP"

ck() { printf '%s' "$RECEIPT" | jq -e --arg n "$1" '.checks[] | select(.name == $n) | .pass == true' >/dev/null 2>&1; }
ck cut_on_word        || fail "receipt" "cut_on_word check did not PASS — a cut landed inside a word" "$RECEIPT"
ck caption_presence   || fail "receipt" "caption_presence check did not PASS" "$RECEIPT"
# lufs: the check must exist AND carry a measured number (details/evidence
# hold integrated LUFS per app/perception/src/checks.rs::lufs).
printf '%s' "$RECEIPT" | jq -e '.checks[] | select(.name == "lufs") | [.details, .evidence] | [.. | numbers] | length > 0' >/dev/null 2>&1 \
  || fail "receipt" "lufs check missing or carries no measured number" "$RECEIPT"
# FULL receipt verdict: every check green (receipt.pass = all 6 of public verb contract).
# The fixture is mastered to −16 LUFS and edge-trimmed; the duration check also
# guards against treating absolute caption ranges as additive durations. Never
# loosen a check to get past this assertion.
printf '%s' "$RECEIPT" | jq -e '.pass == true' >/dev/null 2>&1 \
  || fail "receipt" "receipt.pass != true — failing checks: $(printf '%s' "$RECEIPT" | jq -c '[.checks[] | select(.pass | not) | {name, evidence}]')" "$RECEIPT"
OUT_MS=$(printf '%s' "$RECEIPT" | jq -r '.duration_ms // empty')
[[ "$OUT_MS" =~ ^[0-9]+$ ]] || fail "receipt" "receipt.duration_ms missing/non-numeric" "$RECEIPT"
[[ "$OUT_MS" -lt "$ORIG_MS" ]] || fail "receipt" "duration did not shrink: output ${OUT_MS}ms vs source ${ORIG_MS}ms" "$RECEIPT"
CHECKS_N=$(printf '%s' "$RECEIPT" | jq '[.checks[] | select(.pass)] | length')
pass "receipt — ALL checks PASS ($CHECKS_N/$(printf '%s' "$RECEIPT" | jq '.checks | length')), duration ${ORIG_MS}→${OUT_MS}ms"

# ------------------------------------------------------- step 14: exports --
RESP=$(verb export.srt '{}')
assert_ok "export.srt" "$RESP"
SRT_PATH=$(jqr "$RESP" '.result.path')
[[ -n "$SRT_PATH" && -s "$SRT_PATH" ]] || fail "export.srt" "exported file missing or empty at '$SRT_PATH'" "$RESP"
RESP=$(verb export.xml '{"format":"fcpxml"}')
assert_ok "export.xml" "$RESP"
XML_PATH=$(jqr "$RESP" '.result.path')
[[ -n "$XML_PATH" && -s "$XML_PATH" ]] || fail "export.xml" "exported file missing or empty at '$XML_PATH'" "$RESP"
pass "exports — srt ($(wc -c <"$SRT_PATH")B) + fcpxml ($(wc -c <"$XML_PATH")B) non-empty"

# -------------------------------------------------- step 15: project.diff --
# from = the e2e-start checkpoint, to = current log head (last op id).
RESP=$(verb project.ops '{}')
assert_ok "project.ops" "$RESP"
HEAD_OP=$(jqr "$RESP" '.result.ops | last | .op_id')
[[ -n "$HEAD_OP" ]] || fail "project.diff" "could not resolve log head from project.ops" "$RESP"
RESP=$(verb project.diff "{\"from\":\"e2e-start\",\"to\":\"$HEAD_OP\"}")
assert_ok "project.diff" "$RESP"
DIFF_OPS=$(jqr "$RESP" '.result.ops | length')
[[ "${DIFF_OPS:-0}" -ge 3 ]] || fail "project.diff" "expected ≥3 ops since e2e-start (silences+fillers+cut at minimum), got ${DIFF_OPS:-0}" "$RESP"
printf '%s' "$RESP" | jq -e '.result.ops[] | select(.verb == "transcript.cut_words")' >/dev/null 2>&1 \
  || fail "project.diff" "diff does not list the transcript.cut_words op" "$RESP"
pass "project.diff — $DIFF_OPS ops between e2e-start and $HEAD_OP"

E2E_OK=1
echo "E2E PASS — full end-to-end test contract pipeline green (project: $PROJ_PATH)"
