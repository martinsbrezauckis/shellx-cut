#!/usr/bin/env bash
# make-test-assets.sh — generate ground-truth test footage (end-to-end test contract).
#
# Produces (testdata/ is gitignored and regenerable; script is idempotent —
# it always regenerates deterministically from the fixed script below):
#   testdata/talking_head.mp4        ~65s 1920x1080@30 h264 + 48kHz aac:
#     - espeak-ng TTS speaking the KNOWN script below, with deliberate
#       "um"/"uh"/"so" fillers inside known segments and exact 2-4s silences
#       between segments (inserted as anullsrc, so positions are EXACT)
#     - video: testsrc2 for the first half, testsrc for the second half →
#       one known scene cut at a recorded timestamp (scenedetect ground truth)
#   testdata/insert_clip.mp4         10s second clip for edit.insert tests
#   testdata/talking_head.truth.json ground truth: segment script + windows,
#       filler instances, exact silence spans, scene-cut ms, duration
#
# TTS engine: espeak-ng (apt). The audio_perception MCP / piper voices are
# nicer, but the test asset must be regenerable from a bare checkout in one
# deterministic step — espeak-ng is the only engine guaranteed present and
# scriptable here. If testdata/narration_override/seg<N>.wav files exist
# (e.g. pre-rendered with a better TTS), they are used instead of espeak
# for the matching segment — same measurement-based truth applies.
#
# Ground truth strategy: silences are EXACT (we insert them); word/filler
# positions are WINDOWS (the segment's measured [start,end] on the timeline)
# because TTS word timing is internal to the engine. py/check_truth.py
# asserts instrument output against this file — no eyeballing.
#
# Dependencies: espeak-ng, ffmpeg, ffprobe, python3 (stdlib only).
# Primary callers: developers and perception integration tests.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="$ROOT/testdata"
WORK="$OUT/.work"
mkdir -p "$OUT" "$WORK"

command -v ffmpeg >/dev/null || { echo "ffmpeg not installed" >&2; exit 1; }

# ---------------------------------------------------------------------------
# silent_screen.mp4 — 20s silent screen-demo-like clip (footage-profile contract)
# harness: the silent_screen_demo footage profile's target signature).
# Synthetic "UI" built from drawbox panels (no font dependency, fully
# deterministic) + a 200x140 "dragged window" that moves only during two
# 2s interaction bursts (4-6s rightward, 12-14s downward) and is static
# otherwise. Audio = anullsrc (digital silence).
# Signature BY CONSTRUCTION: 0 transcript words, ~100% silence, 3 frozen
# spans (0-4, 6-12, 14-20s ≈ 80% coverage) split by the two bursts.
# The moving element is deliberately large: a cursor-sized 24px box moves
# <0.1% of the pixels — BELOW freezedetect's -60dB noise floor — and the
# whole clip would read as one wall-to-wall frozen span (= stuck render).
# Consumed by app/perception/tests/silent_profile.rs (talking_head must FAIL this
# footage, silent_screen_demo must PASS it with recorded waivers).
# ---------------------------------------------------------------------------
gen_silent_screen() {
  ffmpeg -nostats -hide_banner -loglevel error -y \
    -f lavfi -i "color=c=0x1e1e28:s=1280x720:r=30:d=20" \
    -f lavfi -i "color=c=0x4a9eff:s=200x140:r=30:d=20" \
    -f lavfi -t 20 -i "anullsrc=r=48000:cl=stereo" \
    -filter_complex "\
[0:v]drawbox=x=0:y=0:w=1280:h=48:color=0x2a2a38:t=fill,\
drawbox=x=40:y=88:w=300:h=592:color=0x252532:t=fill,\
drawbox=x=380:y=88:w=860:h=592:color=0x2e2e3c:t=fill,\
drawbox=x=420:y=128:w=520:h=28:color=0x4a4a5a:t=fill,\
drawbox=x=420:y=176:w=640:h=16:color=0x3a3a48:t=fill,\
drawbox=x=420:y=208:w=600:h=16:color=0x3a3a48:t=fill[bg];\
[bg][1:v]overlay=\
x='if(lt(t,4),420,if(lt(t,6),420+110*(t-4),640))':\
y='if(lt(t,12),300,if(lt(t,14),300+60*(t-12),420))'[v]" \
    -map "[v]" -map 2:a -c:v libx264 -preset veryfast -crf 22 -pix_fmt yuv420p \
    -c:a aac -b:a 128k -shortest "$OUT/silent_screen.mp4"
  echo "OK: $OUT/silent_screen.mp4 (20s silent screen-demo clip)" >&2
}

# Fast path: only the silent screen clip (no espeak/TTS needed) — used by
# the cargo integration test.
if [[ "${1:-}" == "--silent-only" ]]; then
  gen_silent_screen
  exit 0
fi

command -v espeak-ng >/dev/null || { echo "espeak-ng not installed (apt install espeak-ng)" >&2; exit 1; }

# ---------------------------------------------------------------------------
# The KNOWN script. SEGS[i] is spoken; GAPS[i] seconds of exact silence
# follow segment i. Fillers (um/uh/so at sentence starts) are deliberate.
# Changing any text REQUIRES regenerating truth (this script does both).
# ---------------------------------------------------------------------------
SEGS=(
  "Hello and welcome to the shellx cut perception test clip. Every word in this recording comes from a known script."
  "Um, the first feature under test is, uh, silence removal. So the instruments should find every quiet gap in this file."
  "The second feature is filler word detection. Um, the transcript should mark, uh, each filler with word level timestamps."
  "Half way through this clip the video pattern changes completely. The scene detector should report exactly one cut."
  "Loudness is measured with the e b u r one two eight filter. So, um, the integrated value should be steady and quiet."
  "This is the final section of the test recording. After this sentence the clip simply ends. Thank you for listening."
)
GAPS=(2.5 3.0 4.0 2.0 2.5 0)   # exact inserted silence AFTER each segment (s)
# Filler instances per segment index (word listed once per occurrence).
FILLERS=(
  ""               # seg0
  "um uh so"       # seg1
  "um uh"          # seg2
  ""               # seg3
  "so um"          # seg4
  ""               # seg5
)
EDGE_PAD_S=0.2   # head/tail pad — under the 0.3s detector floor on purpose

# ---------------------------------------------------------------------------
# 1) TTS each segment → 48kHz mono WAV; measure exact durations.
#    Edge silence INSIDE each TTS WAV is trimmed (keeping 60ms) so the only
#    head/tail/gap silence in the assembly is what we insert explicitly.
#    Why: espeak leaves ~400ms of trailing silence per utterance; stacked on
#    the 200ms tail pad it made the rendered output end with 514ms of
#    silence, exceeding the silence_at_edges receipt check's 500ms budget.
#    Ground truth must be exact BY CONSTRUCTION; engine slop is not
#    part of the design.
# ---------------------------------------------------------------------------
EDGE_TRIM="silenceremove=start_periods=1:start_threshold=-45dB:start_silence=0.06,areverse,silenceremove=start_periods=1:start_threshold=-45dB:start_silence=0.06,areverse"
seg_durs=()
for i in "${!SEGS[@]}"; do
  override="$OUT/narration_override/seg$i.wav"
  if [[ -f "$override" ]]; then
    src="$override"
    echo "seg$i: using narration override" >&2
  else
    espeak-ng -v en-us+m3 -s 160 -w "$WORK/seg$i.raw.wav" "${SEGS[$i]}"
    src="$WORK/seg$i.raw.wav"
  fi
  ffmpeg -nostats -hide_banner -loglevel error -y -i "$src" \
    -af "$EDGE_TRIM" -ac 1 -ar 48000 -c:a pcm_s16le "$WORK/seg$i.wav"
  seg_durs+=("$(ffprobe -v error -show_entries format=duration \
    -of default=noprint_wrappers=1:nokey=1 "$WORK/seg$i.wav")")
done

# ---------------------------------------------------------------------------
# 2) Exact silences + edge pads, then sample-accurate concat.
# ---------------------------------------------------------------------------
silence_wav() { # $1=duration_s $2=out
  ffmpeg -nostats -hide_banner -loglevel error -y \
    -f lavfi -i "anullsrc=r=48000:cl=mono" -t "$1" -c:a pcm_s16le "$2"
}
silence_wav "$EDGE_PAD_S" "$WORK/pad.wav"
concat_list="$WORK/concat.txt"
: > "$concat_list"
echo "file 'pad.wav'" >> "$concat_list"
for i in "${!SEGS[@]}"; do
  echo "file 'seg$i.wav'" >> "$concat_list"
  gap="${GAPS[$i]}"
  if [[ "$gap" != "0" ]]; then
    silence_wav "$gap" "$WORK/gap$i.wav"
    echo "file 'gap$i.wav'" >> "$concat_list"
  fi
done
echo "file 'pad.wav'" >> "$concat_list"
ffmpeg -nostats -hide_banner -loglevel error -y -f concat -safe 0 \
  -i "$concat_list" -c:a pcm_s16le "$WORK/talking_head.wav"

# ---------------------------------------------------------------------------
# 2b) Master to the receipt-check loudness target: −16 LUFS integrated,
#     true peak ≤ −2 dBTP (1dB of AAC-overshoot headroom under the check's
#     −1 dBTP gate). Raw espeak sits around −21 LUFS, below the receipt
#     target. The fixture is mastered to the target rather than loosening the check.
#     Method: measure integrated (loudnorm pass 1), apply a STATIC gain to
#     hit −16, then a true-peak limiter for the few transients that gain
#     pushes hot. Static gain + limiter keep digital silence EXACTLY zero
#     (0 × g = 0), so the inserted silence spans in the truth stay sample-
#     accurate — full dynamic loudnorm would not guarantee that.
#     The result is measured back and ASSERTED, not assumed.
# ---------------------------------------------------------------------------
measure_loudness() { # $1=wav → "integrated_lufs true_peak_dbtp"
  ffmpeg -nostats -hide_banner -i "$1" \
    -af loudnorm=I=-16:TP=-2:print_format=json -f null - 2>&1 \
    | python3 -c '
import json, sys
raw = sys.stdin.read()
stats = json.loads(raw[raw.rindex("{"):])  # loudnorm prints its JSON last
print(stats["input_i"], stats["input_tp"])'
}
read -r in_i in_tp <<<"$(measure_loudness "$WORK/talking_head.wav")"
gain_db="$(python3 -c "print(round(-16.0 - float('$in_i'), 2))")"
# alimiter limit 0.7943 ≈ −2 dBFS; level=false — do NOT re-normalize after
# limiting (that would undo the target); asc smooths nothing we need.
ffmpeg -nostats -hide_banner -loglevel error -y -i "$WORK/talking_head.wav" \
  -af "volume=${gain_db}dB,alimiter=limit=0.7943:attack=5:release=50:level=false" \
  -c:a pcm_s16le "$WORK/talking_head.master.wav"
read -r out_i out_tp <<<"$(measure_loudness "$WORK/talking_head.master.wav")"
python3 - "$out_i" "$out_tp" <<'PY'
"""Assert the master actually hit the receipt-check window (verify, not hope)."""
import sys
i, tp = float(sys.argv[1]), float(sys.argv[2])
ok = abs(i - (-16.0)) <= 1.5 and tp <= -1.5
print(f"master: integrated {i} LUFS (target -16±1.5), true peak {tp} dBTP (≤ -1.5)", file=sys.stderr)
sys.exit(0 if ok else 1)
PY
mv "$WORK/talking_head.master.wav" "$WORK/talking_head.wav"
total_s="$(ffprobe -v error -show_entries format=duration \
  -of default=noprint_wrappers=1:nokey=1 "$WORK/talking_head.wav")"

# ---------------------------------------------------------------------------
# 3) Video: testsrc2 then testsrc, hard cut at the exact midpoint (truth).
#    Both sources are in constant motion — no black, no frozen frames.
# ---------------------------------------------------------------------------
scene_s="$(python3 -c "print(round(float('$total_s')/2, 3))")"
rest_s="$(python3 -c "print(round(float('$total_s') - float('$scene_s'), 3))")"
ffmpeg -nostats -hide_banner -loglevel error -y \
  -f lavfi -i "testsrc2=size=1920x1080:rate=30:duration=$scene_s" \
  -f lavfi -i "testsrc=size=1920x1080:rate=30:duration=$rest_s" \
  -i "$WORK/talking_head.wav" \
  -filter_complex "[0:v][1:v]concat=n=2:v=1:a=0[v]" \
  -map "[v]" -map 2:a \
  -c:v libx264 -preset fast -pix_fmt yuv420p -r 30 \
  -c:a aac -b:a 192k -ar 48000 -shortest \
  "$OUT/talking_head.mp4"

# ---------------------------------------------------------------------------
# 4) insert_clip.mp4 — 10s second clip for edit.insert tests.
# ---------------------------------------------------------------------------
espeak-ng -v en-us+f4 -s 150 -w "$WORK/insert.raw.wav" \
  "This is the insert clip. It exists for insert and overlay tests."
ffmpeg -nostats -hide_banner -loglevel error -y -i "$WORK/insert.raw.wav" \
  -ac 1 -ar 48000 -c:a pcm_s16le "$WORK/insert.wav"
ffmpeg -nostats -hide_banner -loglevel error -y \
  -f lavfi -i "testsrc2=size=1920x1080:rate=30:duration=10" \
  -f lavfi -i "anullsrc=r=48000:cl=mono" \
  -i "$WORK/insert.wav" \
  -filter_complex "[2:a][1:a]amix=inputs=2:duration=first:dropout_transition=0[a]" \
  -map 0:v -map "[a]" -t 10 \
  -c:v libx264 -preset fast -pix_fmt yuv420p -c:a aac -b:a 192k -ar 48000 \
  "$OUT/insert_clip.mp4"

# ---------------------------------------------------------------------------
# 5) Ground truth JSON — computed from the SAME measured durations used to
#    assemble the audio, so silence spans are exact by construction.
# ---------------------------------------------------------------------------
SEG_DURS="${seg_durs[*]}" SEG_GAPS="${GAPS[*]}" SEG_FILLERS="$(printf '%s|' "${FILLERS[@]}")" \
SEG_TEXTS="$(printf '%s|' "${SEGS[@]}")" TOTAL_S="$total_s" SCENE_S="$scene_s" PAD_S="$EDGE_PAD_S" \
python3 - "$OUT/talking_head.truth.json" <<'PY'
"""Assemble talking_head.truth.json from the env the shell measured."""
import json, os, sys

durs = [float(d) for d in os.environ["SEG_DURS"].split()]
gaps = [float(g) for g in os.environ["SEG_GAPS"].split()]
fillers = os.environ["SEG_FILLERS"].split("|")[:-1]   # trailing | from printf
texts = os.environ["SEG_TEXTS"].split("|")[:-1]
pad = float(os.environ["PAD_S"])

segments, silences = [], []
cursor = pad
for i, dur in enumerate(durs):
    start, end = cursor, cursor + dur
    segments.append({
        "idx": i,
        "text": texts[i],
        "start_ms": int(start * 1000),
        "end_ms": int(end * 1000),
        "fillers": fillers[i].split() if fillers[i] else [],
    })
    cursor = end
    if gaps[i] > 0:  # the EXACT inserted silence after this segment
        silences.append({"start_ms": int(cursor * 1000),
                         "end_ms": int((cursor + gaps[i]) * 1000)})
        cursor += gaps[i]

truth = {
    "schema": "shellx-cut/truth/1",
    "media": "talking_head.mp4",
    "duration_ms": int(float(os.environ["TOTAL_S"]) * 1000),
    "edge_pad_ms": int(pad * 1000),
    "segments": segments,
    "inserted_silences_ms": silences,
    "scene_change_ms": int(float(os.environ["SCENE_S"]) * 1000),
    "filler_lexicon": ["um", "uh", "so"],
}
with open(sys.argv[1], "w") as f:
    json.dump(truth, f, indent=2)
print(f"truth written: {sys.argv[1]}", file=sys.stderr)
PY

gen_silent_screen

echo "OK: $OUT/talking_head.mp4 (${total_s%.*}s), insert_clip.mp4, talking_head.truth.json, silent_screen.mp4" >&2
