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
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$ROOT/ui/src"

FUNCTIONAL_SVG_SPECS=(
  "ui/src/panels/Layer/index.tsx|data-cut-layer-kf-curve"
  "ui/src/panels/Preview/GuideOverlay.tsx|data-cut-preview-guides"
  "ui/src/panels/Preview/MaskOverlay.tsx|data-cut-mask-shape"
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
  marker="$(marker_for "$rel" || true)"
  count="$(grep -o '<svg' "$file" | wc -l | tr -d ' ')"
  if [ -z "$marker" ]; then
    echo "FAIL: unmanaged inline SVG in $rel; route fixed glyphs through <Icon name=…>."
    fail=1
    continue
  fi
  if [ "$count" -ne 1 ] || ! grep -q "$marker" "$file"; then
    echo "FAIL: reviewed functional SVG $rel must contain exactly one <svg> and marker $marker."
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
