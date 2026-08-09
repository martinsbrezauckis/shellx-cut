// components/inspector/PropertyRow — the compact, uniform building block of
// the Inspector.
//
// ROW GRAMMAR (left→right): label · numeric input · slider · reset-↺.
// One row drives ONE numeric property of the selected clip and maps 1:1 to a verb
// (agent-first preserved): the row never mutates project state itself — it reports
// the value via `onCommit`, and the CALLER fires the verb (so the verb stays the
// single writer, exactly like the chips/buttons it replaces).
//
// ── WHY MODULE SCOPE (the single most important invariant) ────────────────────
// This component is declared at MODULE level, NOT inside the Inspector. A
// component defined inside another component gets a FRESH FUNCTION IDENTITY on
// every parent render, so React treats it as a different component type and
// UNMOUNTS + REMOUNTS its DOM subtree on each render. For a `<input type="range">`
// that is mid-drag, the remount tears down the native pointer-capture → the drag
// is INTERRUPTED and the slider "sticks"/freezes (the reproduced pointer-capture
// bug for the grade sliders). GradeSlider (`panels/Grade/index.tsx:71`) follows the
// same module-scope pattern. Keeping the
// component at module scope holds the input's React identity stable so a drag runs
// to completion. NEVER move this definition inside a component.
//
// ── COMMIT-ON-RELEASE (avoids op-log spam + cross-project frame-cache thrash) ──
// Dragging the slider updates LOCAL state only via `onChange` (smooth, no verb
// per pixel). The verb fires ONCE, through `onCommit`, on pointer-up (slider) or
// blur/Enter (numeric input). Firing per `onChange` would append an op per frame
// and re-key the content-addressed frame cache on every step (the cross-
// project leak class of thrash), so commit-on-release is load-bearing, not just
// tidy.
//
// Deps: react (useState/useEffect), ./inspector-primitives.css (via Inspector's
// inspector.css import — these primitives reuse the `pr-*` token-based classes).
// Callers: panels/Inspector (Transform section + future Crop/Speed/Audio rows).

import { useEffect, useState } from 'react'

/** Props for one Inspector property row. */
export interface PropertyRowProps {
  /** Human label shown at the row start (e.g. "Position X"). */
  label: string
  /** Current committed value (the source of truth; the row re-seeds from it when
   *  it changes externally — e.g. the selection changes or a verb result lands). */
  value: number
  /** Slider/input minimum (inclusive). */
  min: number
  /** Slider/input maximum (inclusive). */
  max: number
  /** Slider/input step granularity. */
  step: number
  /** Optional unit suffix shown after the numeric value (e.g. "%", "px"). */
  unit?: string
  /** The value the reset (↺) returns to (the property's identity/default). */
  default: number
  /** LIVE callback on every drag/type tick — for an optional live preview. The
   *  row does NOT fire the verb here; it only mirrors the local value out. */
  onChange?: (value: number) => void
  /** COMMIT callback — fires ONCE on pointer-up / blur / Enter / reset. The caller
   *  turns this into the actual verb call (the row stays verb-free). */
  onCommit: (value: number) => void
  /** Disable the whole row (no selection / not applicable). */
  disabled?: boolean
  /** data-cut-* selector STEM for the gate + agent layer. The row stamps
   *  `data-cut-prop-input`, `-slider`, `-keyframe`, `-reset` with this value so a
   *  test can target one property unambiguously (e.g. "transform-x"). */
  propKey: string
}

/** Clamp `n` into [min,max] and snap toward `step` (kept simple — the engine is
 *  the real validator; this only keeps the UI value sane before committing). */
function clamp(n: number, min: number, max: number): number {
  if (Number.isNaN(n)) return min
  return Math.min(max, Math.max(min, n))
}

/**
 * One uniform property row. MODULE SCOPE — see the file header for why moving this
 * inside a component freezes the slider mid-drag.
 *
 * Side effects: none beyond invoking the `onChange`/`onCommit` callbacks the
 * caller supplies. Holds a local `draft` so the slider/input stay
 * smooth between commits; re-syncs to `value` whenever the committed value changes.
 */
export default function PropertyRow({
  label,
  value,
  min,
  max,
  step,
  unit,
  default: dflt,
  onChange,
  onCommit,
  disabled = false,
  propKey,
}: PropertyRowProps) {
  // Local draft for smooth dragging/typing. The committed `value` is the source of
  // truth; we re-seed the draft whenever it changes externally (selection change,
  // verb result, reset from elsewhere) so the row reflects reality between drags.
  const [draft, setDraft] = useState<number>(value)
  useEffect(() => {
    setDraft(value)
  }, [value])

  // Mirror live changes out (optional preview); does NOT fire the verb.
  const live = (n: number) => {
    const c = clamp(n, min, max)
    setDraft(c)
    onChange?.(c)
  }
  // Commit-on-release: the ONE place the verb is asked to fire.
  const commit = (n: number) => {
    const c = clamp(n, min, max)
    setDraft(c)
    onCommit(c)
  }

  // Display value — round to the step's decimal precision so e.g. 0.10 reads "0.1"
  // not "0.10000000000000009".
  const decimals = step < 1 ? Math.max(0, -Math.floor(Math.log10(step))) : 0
  const shown = Number.isFinite(draft) ? draft.toFixed(decimals) : ''

  return (
    <div className="pr" data-cut-prop={propKey} aria-disabled={disabled || undefined}>
      <label className="pr__label" htmlFor={`pr-${propKey}`}>{label}</label>

      {/* Numeric field — commit on blur or Enter (typing then tab/click out). */}
      <div className="pr__num-wrap">
        <input
          id={`pr-${propKey}`}
          className="pr__num"
          data-cut-prop-input={propKey}
          type="number"
          min={min}
          max={max}
          step={step}
          value={shown}
          disabled={disabled}
          onChange={(e) => live(Number(e.target.value))}
          onBlur={(e) => commit(Number(e.target.value))}
          onKeyDown={(e) => {
            if (e.key === 'Enter') {
              // Pointer regression: blur commits because onBlur is the
              // SOLE committer. The old code called commit() AND blur(), and the
              // programmatic blur re-fired onBlur→commit, logging TWO ops per Enter
              // (an extra undo step, double verb round-trip + preview refresh).
              ;(e.target as HTMLInputElement).blur()
            }
          }}
        />
        {unit && <span className="pr__unit">{unit}</span>}
      </div>

      {/* Slider — onChange = LOCAL draft only (smooth); verb fires on pointer-up /
          keyboard release via onMouseUp/onKeyUp → commit. */}
      <input
        className="pr__slider"
        data-cut-prop-slider={propKey}
        type="range"
        min={min}
        max={max}
        step={step}
        value={Number.isFinite(draft) ? draft : min}
        disabled={disabled}
        onChange={(e) => live(Number(e.target.value))}
        onMouseUp={(e) => commit(Number((e.target as HTMLInputElement).value))}
        onTouchEnd={(e) => commit(Number((e.target as HTMLInputElement).value))}
        onKeyUp={(e) => commit(Number((e.target as HTMLInputElement).value))}
        aria-label={label}
      />

      {/* Reserved column keeps Reset aligned and leaves room for a future
          keyframe surface once a caller and complete interaction exist. */}
      <span className="pr__action-spacer" aria-hidden="true" />

      {/* Reset ↺ — return to the property's default and COMMIT (so a verb fires). */}
      <button
        type="button"
        className="pr__reset"
        data-cut-prop-reset={propKey}
        disabled={disabled || draft === dflt}
        aria-label={`Reset ${label} to ${dflt}${unit ?? ''}`}
        title={`Reset ${label} to ${dflt}${unit ?? ''}`}
        onClick={() => commit(dflt)}
      >
        ↺
      </button>
    </div>
  )
}
