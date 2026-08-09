// StoryboardOverlay.tsx — the "see the whole edit at a glance" modal
// Reuses the family scrim/modal chrome
// shared with the env wizard & keymap overlay — scrim below the 54px top bar,
// --surface body, hairlines, --shadow-modal, Esc / click-outside / × close).
// Role: display ONLY. It renders the contact sheet returned by render.storyboard
// (a JPEG of N evenly-spaced composed-timeline frames tiled into a grid). The
// image arrives as base64 in the verb envelope (inline:true), so we render it
// directly as a data: URL — no extra /api/proxies fetch. Pure view: this never
// dispatches an edit verb and the engine creates no op for render.storyboard,
// so the zero-local-mutation invariant (zero-local-mutation contract) holds.
// Callers: topbar/ (owns the open/busy/result/error state). Deps: react +
// lib/client (StoryboardResult type) + storyboard.css.

import type { StoryboardResult } from '../lib/client'
import { useBlockingOverlay } from '../components/overlay/useBlockingOverlay'
import './storyboard.css'

export interface StoryboardOverlayProps {
  /** true while render.storyboard is in flight (the modal shows a spinner). */
  busy: boolean
  /** The contact sheet, once the verb returns ok. null while busy / on error. */
  result: StoryboardResult | null
  /** A human-readable verb error (e.g. "timeline is empty") — shown honestly,
   *  never swallowed. null when there is no error. */
  error: string | null
  /** Close the overlay (Esc / scrim click / × button). */
  onClose: () => void
}

/** Format a duration in ms as a compact, glanceable `M:SS.s` (mono = fact). */
function fmtDuration(ms: number): string {
  const totalS = ms / 1000
  const m = Math.floor(totalS / 60)
  const s = totalS - m * 60
  return m > 0 ? `${m}:${s.toFixed(1).padStart(4, '0')}` : `${s.toFixed(1)}s`
}

/**
 * Contact-sheet overlay. Rendered only while open (the caller mounts it on
 * demand). Always shows the chrome; the body is busy / error / image depending
 * on state, so the user sees progress and never a blank frame.
 */
export default function StoryboardOverlay({ busy, result, error, onClose }: StoryboardOverlayProps) {
  const overlay = useBlockingOverlay<HTMLDivElement>(onClose)

  return (
    <div className="sb-scrim" data-cut-storyboard-scrim onMouseDown={overlay.onScrimMouseDown}>
      <div
        ref={overlay.dialogRef}
        className="sb-modal"
        data-cut-storyboard
        data-cut-storyboard-open="true"
        data-cut-storyboard-state={busy ? 'busy' : error ? 'error' : result ? 'ready' : 'empty'}
        role="dialog"
        aria-modal="true"
        aria-label="Storyboard"
        data-cut-blocking-overlay
        tabIndex={-1}
        onKeyDown={overlay.onDialogKeyDown}
        onMouseDown={(e) => e.stopPropagation()}
      >
        <header className="sb-head">
          <div>
            <h2 className="sb-title">Storyboard</h2>
            <p className="sb-sub">
              {/* meta = fact, in mono; only meaningful once we have a sheet */}
              {result ? (
                <span className="sb-meta" data-cut-storyboard-meta>
                  {result.count} frames · {result.grid[0]}×{result.grid[1]} grid · {fmtDuration(result.duration_ms)}
                </span>
              ) : busy ? (
                'Sampling the composed timeline…'
              ) : (
                'A glance at the whole edit.'
              )}
            </p>
          </div>
          <button className="sb-close" data-cut-storyboard-close aria-label="Close" onClick={onClose}>
            ×
          </button>
        </header>

        <div className="sb-body">
          {busy ? (
            <div className="sb-status sb-status--busy" data-cut-storyboard-busy>
              <span className="sb-spinner" aria-hidden="true" />
              <span>generating…</span>
            </div>
          ) : error ? (
            // Surface the verb error verbatim — a stub/empty timeline must say so,
            // never silently show nothing.
            <div className="sb-status sb-status--error" data-cut-storyboard-error>
              {error}
            </div>
          ) : result?.base64 ? (
            <img
              className="sb-img"
              data-cut-storyboard-img
              src={`data:${result.mime || 'image/jpeg'};base64,${result.base64}`}
              alt={`Storyboard — ${result.count} frames of the composed timeline`}
            />
          ) : (
            // ok envelope without base64 (caller asked inline:true, so this is
            // an unexpected engine shape) — name it rather than render blank.
            <div className="sb-status sb-status--error" data-cut-storyboard-error>
              no inline image returned
            </div>
          )}
        </div>
      </div>
    </div>
  )
}
