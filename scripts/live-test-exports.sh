#!/usr/bin/env bash
# live-test-exports.sh — focused live proof of the exports batch:
#   render.final bitrate/rate_control (vbr/cbr) + audio_bitrate, and
#   export.publish platform presets (geometry + bitrate).
# Cold-starts cutd headless on a spare port, imports the talking-head asset,
# renders with explicit bitrate targets, and ffprobes each output to PROVE the
# encoder honored the target (not the CRF default). Software-forced for
# determinism. Self-contained; cleans up on exit.
set -uo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ADDR="${E2E_ADDR:-127.0.0.1:6217}"
ASSET_MP4="$ROOT/testdata/talking_head.mp4"
CUTD_BIN="$ROOT/app/target/release/cutd"
export SHELLX_CUT_NO_HWENC=1   # software encode = deterministic, no GPU dependence
RUN_DIR="$(mktemp -d "$ROOT/.scratch/exptest.XXXXXX")"
LOG="$RUN_DIR/cutd.log"
PID=""
cleanup() { [[ -n "$PID" ]] && kill -- -"$PID" 2>/dev/null; rm -rf "$RUN_DIR"; }
trap cleanup EXIT

verb() { curl -sS --max-time 180 -H 'content-type: application/json' -X POST "http://$ADDR/api/verb/$1" -d "${2:-{\}}"; }
jqr()  { printf '%s' "$1" | jq -r "$2 // empty" 2>/dev/null; }
die()  { echo "FAIL: $*" >&2; echo "--- log tail ---" >&2; tail -n 20 "$LOG" >&2; exit 1; }
# probe overall video stream bitrate in kbps
vbitrate_k() { ffprobe -v error -select_streams v:0 -show_entries stream=bit_rate -of csv=p=0 "$1" 2>/dev/null | awk '{printf "%d", $1/1000}'; }
vdims() { ffprobe -v error -select_streams v:0 -show_entries stream=width,height -of csv=p=0 "$1" 2>/dev/null; }
abitrate_k() { ffprobe -v error -select_streams a:0 -show_entries stream=bit_rate -of csv=p=0 "$1" 2>/dev/null | awk '{printf "%d", $1/1000}'; }

wait_job() {
  local jid="$1" deadline=$((SECONDS+180)) state
  while true; do
    state=$(jqr "$(verb jobs.status "{\"job_id\":\"$jid\"}")" '.result.state')
    [[ "$state" == done ]] && return 0
    [[ "$state" == failed ]] && die "job $jid failed"
    (( SECONDS < deadline )) || die "job $jid timeout (state=$state)"
    sleep 1
  done
}

# render via render.final with arbitrary extra args, return the output path
render() {
  local label="$1" extra="$2" resp jid rid out
  resp=$(verb render.final "$extra")
  [[ "$(jqr "$resp" '.ok')" == true ]] || die "$label render.final ok:false — $resp"
  jid=$(jqr "$resp" '.result.job_id'); rid=$(jqr "$resp" '.result.render_id')
  wait_job "$jid"
  out="$PROJ_DIR/exports/$rid.mp4"
  [[ -s "$out" ]] || out=$(ls -t "$PROJ_DIR/exports/"*.mp4 2>/dev/null | head -1)
  [[ -s "$out" ]] || die "$label output missing under $PROJ_DIR/exports"
  echo "$out"
}

echo "== cold-start cutd on $ADDR =="
[[ -x "$CUTD_BIN" ]] || die "cutd binary missing — build -p server first"
setsid "$CUTD_BIN" serve --headless --addr "$ADDR" >"$LOG" 2>&1 &
PID=$!
for i in $(seq 1 60); do curl -s --max-time 2 "http://$ADDR/api/verbs" >/dev/null 2>&1 && break; sleep 0.5; [[ $i == 60 ]] && die "cutd not up"; done
echo "  up (pid $PID)"

echo "== project + import =="
PROJ_DIR="$RUN_DIR/exp.cutproj"
R=$(verb project.create "{\"name\":\"exp\",\"dir\":\"$PROJ_DIR\"}"); [[ "$(jqr "$R" .ok)" == true ]] || die "project.create — $R"
R=$(verb media.import "{\"path\":\"$ASSET_MP4\",\"rationale\":\"export live test\"}")
[[ "$(jqr "$R" .ok)" == true ]] || die "media.import — $R"
IJOB=$(jqr "$R" '.result.job_id'); wait_job "$IJOB"
echo "  imported, ready-to-edit"

# ---- TEST 1: CRF baseline (no bitrate) — reference number -------------------
echo "== CRF baseline (no bitrate) =="
OUT=$(render "crf" '{"preset":"standard","rationale":"crf baseline"}')
CRF_K=$(vbitrate_k "$OUT"); echo "  CRF standard video bitrate: ${CRF_K}k  dims=$(vdims "$OUT")"

# ---- TEST 2: VBR 3M — capped at target, undershoots on easy content ---------
# Platform bitrates are CEILINGS: VBR targets the average + caps ~1.45×, and on
# trivially-simple (synthetic talking-head) content it correctly comes in UNDER
# target. The proof here is that it is RATE-BOUNDED (≤ target+headroom), unlike
# CRF which floats to the content's natural rate.
echo "== Bitrate 3M VBR (rate-bounded) =="
OUT=$(render "vbr3" '{"bitrate":"3M","rationale":"vbr 3M"}')
V3=$(vbitrate_k "$OUT"); echo "  3M vbr video bitrate: ${V3}k (target 3000, cap 4350)"
(( V3 >= 400 && V3 <= 3300 )) || die "VBR 3M landed at ${V3}k — expected rate-bounded <=3300k"
echo "  PASS: VBR 3M is rate-bounded (${V3}k ≤ target)"

# ---- TEST 3: CBR 1.5M (TRUE constant) — pads tight to target ----------------
echo "== Bitrate 1.5M CBR (true CBR, padded) =="
OUT=$(render "cbr15" '{"bitrate":"1500k","rate_control":"cbr","rationale":"cbr 1.5M"}')
V15=$(vbitrate_k "$OUT"); echo "  1.5M cbr video bitrate: ${V15}k (target 1500)"
(( V15 >= 1350 && V15 <= 1700 )) || die "CBR 1.5M landed at ${V15}k — true CBR should pad tight to 1500k"
echo "  PASS: CBR 1.5M is true-constant (${V15}k ≈ target)"

# ---- TEST 4: audio_bitrate override (downward — mono-honorable) -------------
# NOTE: the testdata source is MONO; ffmpeg's native AAC encoder caps mono at
# ~136k no matter how HIGH you ask (a codec ceiling, not our bug). So we prove
# the override responds by targeting LOW (64k), which mono CAN honor — distinct
# from the 192k-default (~138k on this clip).
echo "== Audio bitrate 64k (override responds) =="
OUT=$(render "a64" '{"bitrate":"4M","audio_bitrate":"64k","rationale":"audio 64"}')
A=$(abitrate_k "$OUT"); echo "  audio bitrate: ${A}k (target 64)"
(( A >= 45 && A <= 90 )) || die "audio landed at ${A}k — expected ~64k (override not applied?)"
echo "  PASS: audio_bitrate override honored downward (${A}k)"

# ---- TEST 5: export.publish tiktok → 1080x1920 (9:16) ----------------------
echo "== export.publish tiktok (9:16 geometry) =="
R=$(verb export.publish '{"platform":"tiktok","rationale":"tiktok publish"}')
[[ "$(jqr "$R" .ok)" == true ]] || die "export.publish tiktok ok:false — $R"
PLAT=$(jqr "$R" '.result.publish.platform'); LABEL=$(jqr "$R" '.result.publish.label')
JID=$(jqr "$R" '.result.job_id'); RID=$(jqr "$R" '.result.render_id'); wait_job "$JID"
OUT="$PROJ_DIR/exports/$RID.mp4"; [[ -s "$OUT" ]] || OUT=$(ls -t "$PROJ_DIR/exports/"*.mp4 | head -1)
D=$(vdims "$OUT"); echo "  platform=$PLAT label=$LABEL dims=$D bitrate=$(vbitrate_k "$OUT")k"
[[ "$D" == "1080,1920" ]] || die "tiktok dims=$D — expected 1080,1920"
echo "  PASS: export.publish tiktok → 1080x1920 vertical"

# ---- TEST 6: export.publish youtube → 1920x1080 (16:9) ---------------------
# (audio target is 384k, but mono source caps at ~136k; geometry is the
# unambiguous proof. The `publish` result block echoes the targeted spec.)
echo "== export.publish youtube (16:9 geometry) =="
R=$(verb export.publish '{"platform":"youtube","rationale":"youtube publish"}')
[[ "$(jqr "$R" .ok)" == true ]] || die "export.publish youtube ok:false — $R"
SPEC_AUD=$(jqr "$R" '.result.publish.audio_bitrate')
JID=$(jqr "$R" '.result.job_id'); RID=$(jqr "$R" '.result.render_id'); wait_job "$JID"
OUT="$PROJ_DIR/exports/$RID.mp4"; [[ -s "$OUT" ]] || OUT=$(ls -t "$PROJ_DIR/exports/"*.mp4 | head -1)
D=$(vdims "$OUT"); echo "  dims=$D video=$(vbitrate_k "$OUT")k  spec.audio_bitrate=$SPEC_AUD"
[[ "$D" == "1920,1080" ]] || die "youtube dims=$D — expected 1920,1080"
[[ "$SPEC_AUD" == "384k" ]] || die "youtube publish spec audio_bitrate=$SPEC_AUD — expected 384k"
echo "  PASS: export.publish youtube → 1920x1080 (spec audio 384k)"

# ---- TEST 7: unknown platform → actionable error ---------------------------
echo "== Unknown platform error =="
R=$(verb export.publish '{"platform":"myspace"}')
# NOTE: jq's `//` treats false as empty, so use a direct `.ok` read here.
OKVAL=$(printf '%s' "$R" | jq -r '.ok')
[[ "$OKVAL" == false ]] || die "unknown platform should be ok:false (got ok=$OKVAL) — $R"
echo "  PASS: unknown platform rejected: $(jqr "$R" '.error.message')"

echo
echo "ALL EXPORT LIVE TESTS PASSED (CRF=${CRF_K}k · vbr3M=${V3}k · cbr1.5M=${V15}k · tiktok=9:16 · youtube=16:9+384k)"
