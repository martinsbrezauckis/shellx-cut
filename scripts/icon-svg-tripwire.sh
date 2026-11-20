#!/usr/bin/env bash
# icon-svg-tripwire.sh — normalize the icon system through a deterministic ratchet.
#
# Every glyph must render through the single <Icon name=…> wrapper (ui/src/icons/).
# Functional vector surfaces are different: a data plot, framing overlay, or
# interactive geometry editor cannot be represented by a fixed Icon registry
# entry. This gate permits only an exact, marker-bound list of those surfaces and
# fails every other inline <svg> outside ui/src/icons/.
#
# Run from the repository root:
#   bash scripts/icon-svg-tripwire.sh
# New functional SVGs require an explicit path + stable-marker review here.
#
# Spec format: "path|marker[,marker...][|count]". Every listed marker must appear
# in the file; count is the exact number of <svg> occurrences (default 1). A file
# that holds self-owned image fixtures as data (not glyphs drawn into the UI) may
# list one marker per fixture and the matching count.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$ROOT/ui/src"

FUNCTIONAL_SVG_SPECS=(
  "ui/src/panels/Layer/index.tsx|data-cut-layer-kf-curve"
  "ui/src/panels/Preview/GuideOverlay.tsx|data-cut-preview-guides"
  "ui/src/panels/Preview/MaskOverlay.tsx|data-cut-mask-shape"
  # Data curve of a clip's volume automation; its points are the clip's data.
  "ui/src/panels/Timeline/TimelineVolumeAutomationLane.tsx|tl-volume-lane__curve"
  # Offline mock fixtures: two self-owned image strings served as data, not drawn glyphs.
  "ui/src/panels/Review/mock.ts|MANUAL_DEMO_IMAGE_SVG,MOCK_MEDIA_FIXTURE_SVG|2"
)

marker_for() {
  local wanted="$1" spec rel
  for spec in "${FUNCTIONAL_SVG_SPECS[@]}"; do
    rel="${spec%%|*}"
    if [ "$rel" = "$wanted" ]; then
      echo "${spec#*|}"
      return 0
    fi
  done
  return 1
}

fail=0
while IFS= read -r file; do
  [ -n "$file" ] || continue
  rel="${file#"$ROOT/"}"
  entry="$(marker_for "$rel" || true)"
  count="$(grep -o '<svg' "$file" | wc -l | tr -d ' ')"
  if [ -z "$entry" ]; then
    echo "FAIL: unmanaged inline SVG in $rel; route fixed glyphs through <Icon name=…>."
    fail=1
    continue
  fi
  markers="${entry%%|*}"
  expected="1"
  case "$entry" in *"|"*) expected="${entry#*|}" ;; esac
  markers_ok=1
  IFS=',' read -r -a marker_list <<< "$markers"
  for marker in "${marker_list[@]}"; do
    grep -q "$marker" "$file" || markers_ok=0
  done
  if [ "$count" -ne "$expected" ] || [ "$markers_ok" -ne 1 ]; then
    echo "FAIL: reviewed functional SVG $rel must contain exactly $expected <svg> and marker(s) $markers."
    fail=1
  fi
done < <(grep -rl '<svg' "$SRC" --include='*.tsx' --include='*.ts' --exclude-dir=icons || true)

for spec in "${FUNCTIONAL_SVG_SPECS[@]}"; do
  rel="${spec%%|*}"
  if [ ! -f "$ROOT/$rel" ] || ! grep -q '<svg' "$ROOT/$rel"; then
    echo "FAIL: stale functional SVG allowlist entry $rel."
    fail=1
  fi
done

if [ "$fail" -ne 0 ]; then
  exit 1
fi

echo "ICON TRIPWIRE PASS — 0 unmanaged inline icons; ${#FUNCTIONAL_SVG_SPECS[@]} exact functional SVG surfaces reviewed."
