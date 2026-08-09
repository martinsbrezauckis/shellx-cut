#!/usr/bin/env bash
# Regenerate the small, self-owned clip embedded by cutd for the First edit path.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="$ROOT/app/server/assets/first-edit-sample.mp4"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

for tool in espeak-ng ffmpeg ffprobe; do
  command -v "$tool" >/dev/null || {
    printf 'missing required tool: %s\n' "$tool" >&2
    exit 1
  }
done

mkdir -p "$(dirname "$OUT")"

segments=(
  "Welcome to your first Shell X Cut edit."
  "This sample has two quiet gaps for a calm first cut."
  "Preview the plan, add captions, and render your first version."
)

for i in "${!segments[@]}"; do
  espeak-ng -v en-us+m3 -s 150 -p 45 -a 145 \
    -w "$WORK/segment-$i.raw.wav" "${segments[$i]}"
  ffmpeg -nostats -hide_banner -loglevel error -y \
    -i "$WORK/segment-$i.raw.wav" \
    -af "silenceremove=start_periods=1:start_threshold=-45dB:start_silence=0.05,areverse,silenceremove=start_periods=1:start_threshold=-45dB:start_silence=0.05,areverse" \
    -ar 48000 -ac 1 -c:a pcm_s16le "$WORK/segment-$i.wav"
done

ffmpeg -nostats -hide_banner -loglevel error -y \
  -f lavfi -i "anullsrc=r=48000:cl=mono" -t 2.2 \
  -c:a pcm_s16le "$WORK/gap.wav"

printf "%s\n" \
  "file '$WORK/segment-0.wav'" \
  "file '$WORK/gap.wav'" \
  "file '$WORK/segment-1.wav'" \
  "file '$WORK/gap.wav'" \
  "file '$WORK/segment-2.wav'" > "$WORK/audio.txt"

ffmpeg -nostats -hide_banner -loglevel error -y \
  -f concat -safe 0 -i "$WORK/audio.txt" \
  -af "alimiter=limit=0.75:level=false" -ar 48000 -ac 1 -c:a pcm_s16le "$WORK/audio.wav"

duration="$(ffprobe -v error -show_entries format=duration \
  -of default=noprint_wrappers=1:nokey=1 "$WORK/audio.wav")"

ffmpeg -nostats -hide_banner -loglevel error -y \
  -f lavfi -i "color=c=0x17191f:s=640x360:r=24:d=$duration" \
  -i "$WORK/audio.wav" \
  -filter_complex "[0:v]drawbox=x=0:y=0:w=640:h=54:color=0x242832:t=fill,drawbox=x=36:y=86:w=568:h=224:color=0x20242d:t=fill,drawbox=x='48+mod(t*42\\,420)':y=118:w=96:h=12:color=0x36c98f:t=fill,drawbox=x=72:y='158+mod(t*22\\,92)':w=310:h=18:color=0x697386:t=fill,drawbox=x=72:y=276:w=460:h=8:color=0x3a4250:t=fill,drawbox=x='72+mod(t*31\\,420)':y=276:w=68:h=8:color=0x54a8ff:t=fill[v]" \
  -map "[v]" -map 1:a -shortest \
  -c:v libx264 -preset slow -crf 27 -pix_fmt yuv420p -r 24 \
  -c:a aac -b:a 96k -movflags +faststart "$OUT"

printf 'generated %s (%s bytes, %ss)\n' "$OUT" "$(wc -c < "$OUT")" "$duration"
