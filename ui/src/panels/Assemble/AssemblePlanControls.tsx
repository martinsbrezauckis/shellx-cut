import { useEffect, useRef, useState } from 'react'

import { Icon } from '../../icons'
import type {
  ApplyState,
  RepurposeRequest,
  ReviewedPlan,
  ScriptRequest,
  ShortsMaterialization,
  ShortsRequest,
} from './assemblePlanModel'

type AssembleNumberField = 'count' | 'target' | 'minscore'

const assembleNumberFieldAttributes: Record<AssembleNumberField, Record<string, string>> = {
  count: { 'data-cut-assemble-count': '' },
  target: { 'data-cut-assemble-target': '' },
  minscore: { 'data-cut-assemble-minscore': '' },
}

function clampAssembleNumber(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value))
}

/**
 * Accept browser-style decimal text without `type=number` sanitizing an edit
 * such as `0.` before the next trusted key arrives.
 */
export function assembleNumericDraft(text: string, min: number, max: number, integer = false): number | null {
  if (!/^[+-]?(?:\d+(?:\.\d*)?|\.\d+)(?:e[+-]?\d+)?$/i.test(text)) return null
  const value = Number(text)
  return Number.isFinite(value) && (!integer || Number.isInteger(value)) ? clampAssembleNumber(value, min, max) : null
}

function AssembleNumberInput({
  field,
  value,
  min,
  max,
  step = 1,
  integer = false,
  disabled,
  onValueChange,
}: {
  field: AssembleNumberField
  value: number
  min: number
  max: number
  step?: number
  integer?: boolean
  disabled: boolean
  onValueChange: (value: number) => void
}) {
  const [draft, setDraft] = useState(() => String(value))
  const editing = useRef(false)
  const previewValue = useRef<number | null>(null)
  const hasValueEffect = useRef(false)

  useEffect(() => {
    const firstValueEffect = !hasValueEffect.current
    hasValueEffect.current = true
    // A valid keystroke can immediately update the controlled parent value.
    // Keep its exact text while editing so the next trusted key replaces the
    // selection instead of appending to a clamped value. The initial effect
    // can follow browser focus, so it must not overwrite a just-started edit.
    if (!firstValueEffect && (!editing.current || previewValue.current !== value)) setDraft(String(value))
  }, [value])

  const preview = (text: string) => {
    setDraft(text)
    const next = assembleNumericDraft(text, min, max, integer)
    if (next === null) return
    previewValue.current = next
    onValueChange(next)
  }

  const commit = (text: string) => {
    const next = assembleNumericDraft(text, min, max, integer)
    previewValue.current = null
    if (next === null) {
      setDraft(String(value))
      return
    }
    setDraft(String(next))
    onValueChange(next)
  }

  const stepValue = (direction: 1 | -1) => {
    const current = assembleNumericDraft(draft, min, max, integer) ?? value
    const decimals = step < 1 ? Math.max(0, -Math.floor(Math.log10(step))) : 0
    const next = clampAssembleNumber(Number((current + direction * step).toFixed(decimals)), min, max)
    previewValue.current = next
    setDraft(String(next))
    onValueChange(next)
  }

  return <input
    className="cd-input cd-input--num"
    type="text"
    inputMode={step < 1 ? 'decimal' : 'numeric'}
    role="spinbutton"
    min={min}
    max={max}
    step={step}
    {...assembleNumberFieldAttributes[field]}
    value={draft}
    disabled={disabled}
    aria-valuemin={min}
    aria-valuemax={max}
    aria-valuenow={value}
    aria-valuetext={draft}
    onFocus={() => {
      editing.current = true
      previewValue.current = null
    }}
    onChange={(event) => preview(event.target.value)}
    onBlur={(event) => {
      editing.current = false
      commit(event.target.value)
    }}
    onKeyDown={(event) => {
      if (event.key === 'ArrowUp' || event.key === 'ArrowDown') {
        event.preventDefault()
        stepValue(event.key === 'ArrowUp' ? 1 : -1)
        return
      }
      if (event.key === 'Enter') event.currentTarget.blur()
    }}
  />
}

function ApplyPlanControl({
  mode,
  state,
  disabledReason,
  onApply,
}: {
  mode: 'shorts' | 'repurpose' | 'from_script'
  state: ApplyState
  disabledReason?: string
  onApply: () => void
}) {
  const applying = state === 'applying'
  const applied = state === 'applied'
  return (
    <div className="cd-assemble-apply" data-cut-assemble-apply-state={state}>
      <button
        className="cd-btn cd-btn--primary"
        data-cut-assemble-apply={mode}
        disabled={Boolean(disabledReason) || applying || applied}
        onClick={onApply}
      >
        {applying ? 'Adding to timeline…' : applied ? 'Added to timeline' : 'Add reviewed plan to timeline'}
      </button>
      {disabledReason ? (
        <p className="cd-note cd-note--warn" data-cut-assemble-apply-reason>{disabledReason}</p>
      ) : applied ? (
        <p className="cd-note" data-cut-assemble-apply-result>Added as one editable timeline action. One Undo removes the complete Assemble result.</p>
      ) : (
        <p className="cd-note" data-cut-assemble-apply-ready>Applies the reviewed plan as one editable timeline action. One Undo removes it.</p>
      )}
    </div>
  )
}

export interface AssemblePlanControlsProps {
  mode: 'shorts' | 'repurpose' | 'from_script'
  busy: boolean
  effectiveAsset: string
  count: number
  targetS: number
  prompt: string
  script: string
  minScore: number
  aspect: '9:16' | '1:1' | '4:5' | '16:9'
  shortsPlan: (ReviewedPlan<ShortsRequest> & { materialization: ShortsMaterialization }) | null
  repurposePlan: ReviewedPlan<RepurposeRequest> | null
  scriptPlan: ReviewedPlan<ScriptRequest> | null
  applyState: ApplyState
  onCount: (value: number) => void
  onTargetS: (value: number) => void
  onPrompt: (value: string) => void
  onScript: (value: string) => void
  onMinScore: (value: number) => void
  onAspect: (value: '9:16' | '1:1' | '4:5' | '16:9') => void
  onRunShorts: () => void
  onRunRepurpose: () => void
  onRunFromScript: () => void
  onApplyShorts: () => void
  onApplyRepurpose: () => void
  onApplyFromScript: () => void
}

/** Planner inputs and explicit Apply controls stay outside the drawer shell. */
export function AssemblePlanControls(props: AssemblePlanControlsProps) {
  if (props.mode === 'shorts') {
    const plan = props.shortsPlan
    return <>
      <div className="cd-row">
        <label className="cd-field cd-field--inline">
          <span className="cd-field-label">How many</span>
          <AssembleNumberInput field="count" value={props.count} min={1} max={50} integer
            disabled={props.busy} onValueChange={props.onCount} />
        </label>
        <label className="cd-field cd-field--inline">
          <span className="cd-field-label">Length (s)</span>
          <AssembleNumberInput field="target" value={props.targetS} min={3} max={600}
            disabled={props.busy} onValueChange={props.onTargetS} />
        </label>
        <label className="cd-field cd-field--inline">
          <span className="cd-field-label">Aspect</span>
          <select className="cd-sel cd-sel--sm" data-cut-assemble-aspect value={props.aspect} disabled={props.busy}
            onChange={(e) => props.onAspect(e.target.value as typeof props.aspect)}>
            <option value="9:16">9:16</option><option value="1:1">1:1</option>
            <option value="4:5">4:5</option><option value="16:9">16:9</option>
          </select>
        </label>
      </div>
      <button className="cd-btn cd-btn--primary" data-cut-assemble-run disabled={props.busy || !props.effectiveAsset}
        onClick={props.onRunShorts}>
        {props.busy ? 'Finding ranges…' : <><Icon name="effect" size={14} tone="brand" /> Find short-worthy ranges</>}
      </button>
      <p className="cd-note">Ranks the best moments by engagement and plans a {props.aspect} crop plus transcript captions. Review the ranges, then apply one eligible plan as an editable timeline action.</p>
      {plan && <ApplyPlanControl mode="shorts" state={props.applyState}
        disabledReason={!plan.materialization.eligible
          ? plan.materialization.reason ?? `This plan needs a ${plan.materialization.required_aspect} project; the open project is ${plan.materialization.project_aspect}. Change the project format and plan again.`
          : plan.binding.selected_ranges.length === 0 ? 'This plan has no reviewed transcript ranges to add.' : undefined}
        onApply={props.onApplyShorts} />}
    </>
  }
  if (props.mode === 'repurpose') {
    const plan = props.repurposePlan
    return <>
      <div className="cd-row">
        <label className="cd-field cd-field--inline"><span className="cd-field-label">How many</span>
          <AssembleNumberInput field="count" value={props.count} min={1} max={50} integer
            disabled={props.busy} onValueChange={props.onCount} /></label>
        <label className="cd-field cd-field--inline"><span className="cd-field-label">Target length (s)</span>
          <AssembleNumberInput field="target" value={props.targetS} min={3} max={600}
            disabled={props.busy} onValueChange={props.onTargetS} /></label>
      </div>
      <label className="cd-field"><span className="cd-field-label">Theme / keywords (optional)</span>
        <input className="cd-input" data-cut-assemble-prompt placeholder="e.g. product demo highlights" value={props.prompt} disabled={props.busy}
          onChange={(e) => props.onPrompt(e.target.value)} /></label>
      <button className="cd-btn cd-btn--primary" data-cut-assemble-run disabled={props.busy || !props.effectiveAsset} onClick={props.onRunRepurpose}>
        {props.busy ? 'Finding…' : <><Icon name="split" size={14} tone="brand" /> Find best moments</>}
      </button>
      {plan && <ApplyPlanControl mode="repurpose" state={props.applyState}
        disabledReason={plan.binding.selected_ranges.length === 0 ? 'This plan has no reviewed transcript ranges to add.' : undefined}
        onApply={props.onApplyRepurpose} />}
    </>
  }
  const plan = props.scriptPlan
  return <>
    <label className="cd-field"><span className="cd-field-label">Script (one talking point per line)</span>
      <textarea className="cd-input cd-textarea" rows={5} data-cut-assemble-script disabled={props.busy}
        placeholder={'Welcome to the demo\nHere is the main feature\nAnd how to get started'} value={props.script}
        onChange={(e) => props.onScript(e.target.value)} /></label>
    <label className="cd-field cd-field--inline"><span className="cd-field-label">Min match (0–1)</span>
      <AssembleNumberInput field="minscore" value={props.minScore} min={0} max={1} step={0.05}
        disabled={props.busy} onValueChange={props.onMinScore} /></label>
    <button className="cd-btn cd-btn--primary" data-cut-assemble-run disabled={props.busy || !props.effectiveAsset} onClick={props.onRunFromScript}>
      {props.busy ? 'Matching…' : <><Icon name="text" size={14} tone="brand" /> Match script to footage</>}
    </button>
    {plan && <ApplyPlanControl mode="from_script" state={props.applyState}
      disabledReason={plan.binding.selected_ranges.length === 0 ? 'No script lines matched this source, so there is nothing safe to add.' : undefined}
      onApply={props.onApplyFromScript} />}
  </>
}
