// panels/Grade — the color-grade drawer (0.5.0 UI for edit.grade).
// Role: a right-side drawer (the MusicBed drawer family) that drives ONE verb —
// edit.grade — to apply a parametric color grade (ffmpeg eq + white balance +
// optional .cube LUT) to the SELECTED media clip.
//
// PLACEMENT: color grading is contextual to the selected clip and is a lower-
// frequency action than basic timeline editing, so it is not a permanent top-level button.
// So the launch point is a "Grade" button on the timeline toolbar that ENABLES
// only when a media clip is selected (beside the Speed control — same shape of
// "edit the selected clip" action). It opens this drawer for that clip.
//
// LIVE EDITOR, not just a setter: the drawer SEEDS its sliders from the clip's
// current grade (clip.grade in the project snapshot), so re-opening shows where
// the clip is and edits from there. Identity values clear the grade (the engine
// stores None). A "Reset to neutral" fires an all-identity grade.
//
// TRUST STORY: no live preview in the drawer. Fires the verb; the Preview poster
// (cache-busted by headOpId on op_applied) shows the ACTUALLY graded frame at
// the playhead. Receipt = the verb result (old_grade → grade). The receipt
// PERSISTS across project snapshot refreshes of the SAME clip (the apply itself
// triggers one — clearing it there made the receipt vanish moments after apply)
// and clears only when the selection moves to
// a different clip, the drawer unmounts, or a new apply replaces it. Relay NOT
// claimed. Sliders step 0.01 so grades applied by the agent API at fine
// precision stay reproducible/tunable from the UI (0.05 locked them out).
//
// Callers: App.tsx (mounted when open, with the selected clip id). Deps:
// lib/client, ../drawer.css.

import { useEffect, useMemo, useState } from 'react'
import { callVerb, type ClipGrade, type Project } from '../../lib/client'
import { isTauri, pickCube } from '../../lib/tauri'
import '../drawer.css'

export interface GradeDrawerProps {
  project: Project | null
  /** The clip to grade (App passes selectedClipIds[0]). */
  clipId: string | null
}

/** edit.grade result. grade is null when an identity grade cleared it. */
interface GradeResult {
  clip: string
  grade: ClipGrade | null
  old_grade: ClipGrade | null
}

/** Identity grade (matches cut-core defaults: 1/0/1/1, no temp/lut). */
const NEUTRAL = { contrast: 1, brightness: 0, saturation: 1, gamma: 1 }

/** Find a MEDIA clip by id and return its current grade (or null). null clip =
 *  not found / not a media clip. */
function findMediaGrade(project: Project | null, clipId: string | null): { found: boolean; grade: ClipGrade | null } {
  if (!project || !clipId) return { found: false, grade: null }
  for (const t of project.tracks) {
    for (const c of t.clips) {
      if ('asset' in c && c.id === clipId) {
        return { found: true, grade: c.grade ?? null }
      }
    }
  }
  return { found: false, grade: null }
}

/** A single grade slider (label + live value + range input). Defined at MODULE level
 *  on purpose: a component defined INSIDE GradeDrawer gets a fresh identity on every
 *  render, so React unmounts+remounts this `<input>` on each onChange — which INTERRUPTS
 *  the native pointer-drag → the slider sticks/freezes mid-drag and grading feels like it
 *  "hangs" (reproduced pointer-capture bug). Module scope keeps the input identity
 *  stable so a drag runs to completion. */
function GradeSlider({ label, attr, value, set, min, max, step }: {
  label: string; attr: string; value: number; set: (n: number) => void; min: number; max: number; step: number
}) {
  return (
    <label className="cd-field">
      <span className="cd-field-label">
        {/* 2-decimal display matches the 0.01 step; an API-precision seed
            beyond 2 decimals still holds its EXACT value on the input itself.
            Programmatic consumers read this numerically (parseFloat), so fixed formatting
            stays compatible. */}
        {label} <span className="cd-val" data-cut-grade-val={attr}>{value.toFixed(2)}</span>
      </span>
      <input
        className="cd-range"
        data-cut-grade-input={attr}
        type="range"
        min={min}
        max={max}
        step={step}
        value={value}
        onChange={(e) => set(Number(e.target.value))}
      />
    </label>
  )
}

export default function GradeDrawer({ project, clipId }: GradeDrawerProps) {
  const { found, grade } = useMemo(() => findMediaGrade(project, clipId), [project, clipId])
  const gradeSeedKey = [
    clipId ?? '',
    grade?.contrast ?? NEUTRAL.contrast,
    grade?.brightness ?? NEUTRAL.brightness,
    grade?.saturation ?? NEUTRAL.saturation,
    grade?.gamma ?? NEUTRAL.gamma,
    grade?.temperature_k ?? '',
    grade?.lut ?? '',
  ].join('|')
  const desktop = isTauri()

  // Seed from the clip's current grade (or neutral). temperature is opt-in.
  const [contrast, setContrast] = useState(grade?.contrast ?? NEUTRAL.contrast)
  const [brightness, setBrightness] = useState(grade?.brightness ?? NEUTRAL.brightness)
  const [saturation, setSaturation] = useState(grade?.saturation ?? NEUTRAL.saturation)
  const [gamma, setGamma] = useState(grade?.gamma ?? NEUTRAL.gamma)
  const [tempOn, setTempOn] = useState(grade?.temperature_k != null)
  const [tempK, setTempK] = useState<number>(grade?.temperature_k ?? 6500)
  const [lut, setLut] = useState<string>(grade?.lut ?? '')
  const [busy, setBusy] = useState(false)
  const [result, setResult] = useState<GradeResult | null>(null)
  const [err, setErr] = useState<string | null>(null)

  // The right rail keeps this component mounted while selection changes. Keep
  // local editor state aligned with the newly selected clip's stored grade.
  // SLIDERS ONLY: the apply receipt is deliberately NOT cleared here. Applying
  // a grade triggers a snapshot refresh whose snapshot carries the new grade,
  // so gradeSeedKey changes and this effect re-runs for the SAME clip —
  // clearing `result` here wipes the receipt moments after every apply. Receipt
  // lifecycle lives in the
  // [clipId]-keyed effect below.
  useEffect(() => {
    setContrast(grade?.contrast ?? NEUTRAL.contrast)
    setBrightness(grade?.brightness ?? NEUTRAL.brightness)
    setSaturation(grade?.saturation ?? NEUTRAL.saturation)
    setGamma(grade?.gamma ?? NEUTRAL.gamma)
    setTempOn(grade?.temperature_k != null)
    setTempK(grade?.temperature_k ?? 6500)
    setLut(grade?.lut ?? '')
  }, [gradeSeedKey])

  // Receipt lifecycle: the receipt (and any error) belongs to the clip it was
  // earned on. Clear ONLY when the SELECTED CLIP changes — same-clip snapshot
  // refreshes (including the one our own apply triggers) keep it visible.
  // The other exit paths need no handling here: closing the drawer unmounts
  // this component (state discarded) and fire() clears/replaces the receipt at
  // the start of the next apply.
  useEffect(() => {
    setResult(null)
    setErr(null)
  }, [clipId])

  /** The grade values fire() sends. Defaults to the live slider state; reset()
   *  passes an explicit identity set so the verb call doesn't race the async
   *  setState below (state updates aren't visible until the next render). */
  type GradeVals = { contrast: number; brightness: number; saturation: number; gamma: number; tempOn: boolean; tempK: number; lut: string }

  const fire = async (override?: GradeVals) => {
    if (!found || !clipId || busy) return
    const v: GradeVals = override ?? { contrast, brightness, saturation, gamma, tempOn, tempK, lut }
    setBusy(true)
    setErr(null)
    setResult(null)
    try {
      const r = await callVerb('edit.grade', {
        clip: clipId,
        contrast: v.contrast,
        brightness: v.brightness,
        saturation: v.saturation,
        gamma: v.gamma,
        ...(v.tempOn ? { temperature_k: Math.round(v.tempK) } : {}),
        ...(v.lut.trim() ? { lut: v.lut.trim() } : {}),
        rationale: override ? 'user: reset grade to neutral' : 'user: color grade',
      })
      if (r.ok) {
        setResult(r.result as GradeResult)
        // Flip the Preview to COMPOSED so the graded frame is actually visible
        // (the raw proxy never shows a grade) — makes the receipt below true.
        document.dispatchEvent(new CustomEvent('cut:show-composed'))
      } else {
        setErr(`${r.error?.code ?? 'failed'}: ${r.error?.message ?? 'edit.grade failed'}`)
      }
    } catch {
      setErr('server unreachable')
    } finally {
      setBusy(false)
    }
  }

  /** "Reset to neutral" — resets the sliders AND fires an all-identity edit.grade
   *  so the clip is actually neutralized (temp off + lut cleared → the engine
   *  stores None), matching the button's "neutral" promise. Previously this only
   *  reset local state, leaving the clip's grade untouched (the dead-control bug).
   *  Reuses fire() with explicit identity values to avoid the async-setState race. */
  /** Open the native .cube picker (desktop only) and seed the LUT path from it —
   *  the PRIMARY affordance so users don't hand-type an absolute server path. The
   *  mono input stays the manual/advanced fallback (and the only path on a
   *  browser/remote build, where isTauri() is false). */
  const pickLut = async () => {
    const p = await pickCube()
    if (p) setLut(p)
  }

  const reset = () => {
    if (busy) return
    setContrast(NEUTRAL.contrast)
    setBrightness(NEUTRAL.brightness)
    setSaturation(NEUTRAL.saturation)
    setGamma(NEUTRAL.gamma)
    setTempOn(false)
    setLut('')
    if (found && clipId) {
      void fire({
        contrast: NEUTRAL.contrast,
        brightness: NEUTRAL.brightness,
        saturation: NEUTRAL.saturation,
        gamma: NEUTRAL.gamma,
        tempOn: false,
        tempK,
        lut: '',
      })
    }
  }

  const body = (
        <div className="cd-body">
          {!found ? (
            <div className="cd-empty" data-cut-grade-empty>
              Select a video clip on the timeline to grade it.
            </div>
          ) : (
            <>
              <p className="cd-note" data-cut-grade-clip>Grading clip <code>{clipId}</code>.</p>

              {/* 0.01 step: grades the agent applies via the API at fine
                  precision must stay reproducible/tunable from these sliders
                  (0.05 quantization would lock them out).
                  Temperature below keeps its designed 100 K step: Kelvin
                  scale, engine rounds to an integer. */}
              <GradeSlider label="Contrast" attr="contrast" value={contrast} set={setContrast} min={0} max={2} step={0.01} />
              <GradeSlider label="Brightness" attr="brightness" value={brightness} set={setBrightness} min={-1} max={1} step={0.01} />
              <GradeSlider label="Saturation" attr="saturation" value={saturation} set={setSaturation} min={0} max={2} step={0.01} />
              <GradeSlider label="Gamma" attr="gamma" value={gamma} set={setGamma} min={0.1} max={3} step={0.01} />

              {/* white balance — opt-in */}
              <label className="cd-toggle" data-cut-grade-temp-toggle>
                <input
                  type="checkbox"
                  data-cut-grade-temp-on
                  checked={tempOn}
                  onChange={(e) => setTempOn(e.target.checked)}
                />
                <span className="cd-field-label">White balance (Kelvin)</span>
              </label>
              {tempOn && (
                <label className="cd-field">
                  <span className="cd-field-label">
                    Temperature <span className="cd-val" data-cut-grade-val="temperature_k">{Math.round(tempK)}K</span>
                  </span>
                  <input
                    className="cd-range"
                    data-cut-grade-input="temperature_k"
                    type="range"
                    min={2000}
                    max={12000}
                    step={100}
                    value={tempK}
                    onChange={(e) => setTempK(Number(e.target.value))}
                  />
                </label>
              )}

              {/* LUT (.cube): native picker is the PRIMARY affordance. The raw
                  path field stays as an Advanced fallback for browsers/rigs and
                  fixture injection. Fenced engine-side: must end .cube + exist. */}
              <label className="cd-field">
                <span className="cd-field-label">Look-up table (.cube)</span>
                <div className="cd-lut-row">
                  {desktop && (
                    <button
                      type="button"
                      className="cd-btn cd-btn--ghost"
                      data-cut-grade-lut-pick
                      onClick={() => void pickLut()}
                      style={{ flexShrink: 0 }}
                    >
                      Choose .cube…
                    </button>
                  )}
                  <span className="cd-lut-chip" data-cut-grade-lut-picked title={lut || 'No LUT selected'}>
                    {lut ? lut.split(/[\\/]/).pop() : 'No LUT selected'}
                  </span>
                </div>
                <details className="cd-advanced" data-cut-grade-lut-advanced open={!desktop}>
                  <summary data-cut-grade-lut-advanced-toggle>Advanced path</summary>
                  <input
                    className="cd-input cd-input--mono"
                    data-cut-grade-lut
                    placeholder="Paste .cube path"
                    value={lut}
                    onChange={(e) => setLut(e.target.value)}
                  />
                  <p className="cd-note">Use this when the native picker is unavailable or a test fixture needs an exact file path.</p>
                </details>
                <p className="cd-note">Optional 3D color preset. Choose a .cube file; Apply commits it to the selected clip.</p>
              </label>

              <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--space-3)' }}>
                <button
                  className="cd-btn cd-btn--primary"
                  data-cut-grade-apply
                  disabled={busy}
                  onClick={() => void fire()}
                  style={{ flex: 1 }}
                >
                  {busy ? 'Grading…' : 'Apply grade'}
                </button>
                <button className="cd-reset" data-cut-grade-reset onClick={reset} type="button" disabled={busy}>
                  Reset to neutral
                </button>
              </div>

              <p className="cd-note">The preview refreshes to show the graded frame. An all-neutral grade clears it.</p>

              {err && (
                <div className="cd-err" data-cut-grade-error role="alert">{err}</div>
              )}

              {result && (
                <div className="cd-result" data-cut-grade-result>
                  <div className="cd-result-head">
                    {result.grade ? 'grade applied' : 'grade cleared (neutral)'} · {result.clip}
                  </div>
                  {result.grade && (
                    <dl className="cd-result-grid">
                      <dt>contrast</dt><dd>{result.grade.contrast}</dd>
                      <dt>brightness</dt><dd>{result.grade.brightness}</dd>
                      <dt>saturation</dt><dd>{result.grade.saturation}</dd>
                      <dt>gamma</dt><dd>{result.grade.gamma}</dd>
                      {result.grade.temperature_k != null && (<><dt>temp</dt><dd>{result.grade.temperature_k}K</dd></>)}
                      {result.grade.lut && (<><dt>lut</dt><dd>{result.grade.lut.split('/').pop()}</dd></>)}
                    </dl>
                  )}
                  <div className="cd-result-foot">
                    Scrub the preview over <strong>{result.clip}</strong> to see the graded frame.
                  </div>
                </div>
              )}
            </>
          )}
        </div>
  )

  return (
    <section className="cd-embed" data-cut-grade data-cut-grade-open="true" data-cut-grade-embed aria-label="Color grade">
      {body}
    </section>
  )
}
