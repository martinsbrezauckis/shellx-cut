// GuideOverlay.tsx — framing guides over the Preview stage.
//
// Role: a pure, non-interactive SVG layer inside `pv-stage` that draws
// rule-of-thirds lines and/or broadcast safe-area rectangles over the frame.
// Percent-based coordinates so it tracks the letterboxed stage at any size.
// Safe-area convention: action-safe 90%, title-safe 80% (the classic SMPTE
// conventional default margins) — a framing aid, not a compliance
// certification. pointer-events:none, so playback/handle interaction is
// untouched. Callers: panels/Preview/index.tsx. Deps: react + preview.css.

import type { GuideMode } from './usePreviewViewOptions'

/** Inset rectangle at `pct` of the frame (90 → 5% margin on every side). */
function insetRect(pct: number, className: string, label: string) {
  const inset = (100 - pct) / 2
  return (
    <g className={className}>
      <rect
        x={`${inset}%`}
        y={`${inset}%`}
        width={`${pct}%`}
        height={`${pct}%`}
      />
      <text x={`${inset + 1}%`} y={`${inset + 3.5}%`}>{label}</text>
    </g>
  )
}

export function GuideOverlay({ mode }: { mode: GuideMode }) {
  if (mode === 'off') return null
  const thirds = mode === 'thirds' || mode === 'both'
  const safe = mode === 'safe' || mode === 'both'
  return (
    <svg
      className="pv-guides"
      data-cut-preview-guides={mode}
      aria-hidden="true"
      width="100%"
      height="100%"
    >
      {thirds && (
        <g className="pv-guides__thirds">
          <line x1="33.333%" y1="0" x2="33.333%" y2="100%" />
          <line x1="66.667%" y1="0" x2="66.667%" y2="100%" />
          <line x1="0" y1="33.333%" x2="100%" y2="33.333%" />
          <line x1="0" y1="66.667%" x2="100%" y2="66.667%" />
        </g>
      )}
      {safe && (
        <>
          {insetRect(90, 'pv-guides__safe pv-guides__safe--action', 'action safe')}
          {insetRect(80, 'pv-guides__safe pv-guides__safe--title', 'title safe')}
        </>
      )}
    </svg>
  )
}
