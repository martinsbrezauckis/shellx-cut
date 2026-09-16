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

import { useEffect, useRef, useState } from 'react'

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
 * Parse text from the editable numeric field without treating an unfinished
 * sign or decimal as a value. `type="number"` sanitizes `-` and `-.` before
 * React can retain them, so the field is text-backed while it is being edited.
 */
export function propertyRowNumericDraft(text: string, min: number, max: number): number | null {
  // Keep the browser number-field grammar without its eager sanitization. This
  // rejects Number-only spellings such as `0x10` while retaining normal decimal
  // and exponent entry (`-.5`, `1.`, `1e-3`).
  if (!/^[+-]?(?:\d+(?:\.\d*)?|\.\d+)(?:e[+-]?\d+)?$/i.test(text)) return null
  const value = Number(text)
  return Number.isFinite(value) ? clamp(value, min, max) : null
}

/**
 * One uniform property row. MODULE SCOPE — see the file header for why moving this
 * inside a component freezes the slider mid-drag.
 *
 * Side effects: none beyond invoking the `onChange`/`onCommit` callbacks the
 * caller supplies. Holds a local numeric `draft` for the slider plus an editing
 * text draft for the field, so a human can type a signed or decimal value without
 * formatting it away before the one blur/Enter commit.
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
  const [inputDraft, setInputDraft] = useState<string | null>(null)
  const inputPreviewValue = useRef<number | null>(null)
  const editingInput = useRef(false)
  const hasValueEffect = useRef(false)
  useEffect(() => {
    const firstValueEffect = !hasValueEffect.current
    hasValueEffect.current = true
    setDraft(value)
    // A live preview can feed the same value back through the parent while the
    // user is still completing (for example) `-6.`. Keep that text intact; a
    // different external value replaces the edit draft. The first effect can
    // arrive after the initial browser focus, so it must not erase that input.
    if (!firstValueEffect && (!editingInput.current || inputPreviewValue.current !== value)) setInputDraft(null)
  }, [value])

  // Mirror live changes out (optional preview); does NOT fire the verb.
  const live = (n: number, retainInputDraft = false) => {
    const c = clamp(n, min, max)
    setDraft(c)
    if (!retainInputDraft) {
      inputPreviewValue.current = null
      setInputDraft(null)
    }
    onChange?.(c)
  }
  // Commit-on-release: the ONE place the verb is asked to fire.
  const commit = (n: number) => {
    const c = clamp(n, min, max)
    inputPreviewValue.current = null
    setInputDraft(null)
    setDraft(c)
    onCommit(c)
  }

  const previewInput = (text: string) => {
    setInputDraft(text)
    const next = propertyRowNumericDraft(text, min, max)
    if (next === null) return
    inputPreviewValue.current = next
    live(next, true)
  }

  const commitInput = (text: string) => {
    const next = propertyRowNumericDraft(text, min, max)
    if (next !== null) {
      commit(next)
      return
    }
    // An unfinished sign/decimal is not a numeric edit. Restore the last
    // committed source value and issue no verb.
    inputPreviewValue.current = null
    setInputDraft(null)
    setDraft(value)
  }

  // Display value — round to the step's decimal precision so e.g. 0.10 reads "0.1"
  // not "0.10000000000000009".
  const decimals = step < 1 ? Math.max(0, -Math.floor(Math.log10(step))) : 0
  const shown = Number.isFinite(draft) ? draft.toFixed(decimals) : ''
  const inputValue = inputDraft ?? shown

  const stepInput = (direction: 1 | -1) => {
    const current = propertyRowNumericDraft(inputValue, min, max) ?? draft
    const next = clamp(Number((current + direction * step).toFixed(decimals)), min, max)
    inputPreviewValue.current = next
    setInputDraft(next.toFixed(decimals))
    live(next, true)
  }

  return (
    <div className="pr" data-cut-prop={propKey} aria-disabled={disabled || undefined}>
      <label className="pr__label" htmlFor={`pr-${propKey}`}>{label}</label>

      {/* Numeric field — commit on blur or Enter (typing then tab/click out). */}
      <div className="pr__num-wrap">
        <input
          id={`pr-${propKey}`}
          className="pr__num"
          data-cut-prop-input={propKey}
          type="text"
          inputMode="decimal"
          role="spinbutton"
          aria-valuemin={min}
          aria-valuemax={max}
          aria-valuenow={draft}
          aria-valuetext={inputValue}
          value={inputValue}
          disabled={disabled}
          onFocus={() => {
            editingInput.current = true
            inputPreviewValue.current = null
          }}
          onChange={(e) => previewInput(e.target.value)}
          onBlur={(e) => {
            editingInput.current = false
            commitInput(inputDraft ?? e.target.value)
          }}
          onKeyDown={(e) => {
            if (e.key === 'ArrowUp' || e.key === 'ArrowDown') {
              e.preventDefault()
              stepInput(e.key === 'ArrowUp' ? 1 : -1)
              return
            }
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
        onKeyUp={(e) => {
          if (['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 'Home', 'End', 'PageUp', 'PageDown'].includes(e.key)) {
            commit(Number((e.target as HTMLInputElement).value))
          }
        }}
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
