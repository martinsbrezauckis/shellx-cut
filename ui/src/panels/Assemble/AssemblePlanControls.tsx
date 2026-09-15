import { Icon } from '../../icons'
import type {
  ApplyState,
  RepurposeRequest,
  ReviewedPlan,
  ScriptRequest,
  ShortsMaterialization,
  ShortsRequest,
} from './assemblePlanModel'

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
  const resetCount = (value: number) => props.onCount(Math.max(1, Math.min(50, value || 5)))
  const resetTarget = (value: number) => props.onTargetS(Math.max(3, Math.min(600, value || 30)))
  if (props.mode === 'shorts') {
    const plan = props.shortsPlan
    return <>
      <div className="cd-row">
        <label className="cd-field cd-field--inline">
          <span className="cd-field-label">How many</span>
          <input className="cd-input cd-input--num" type="number" min={1} max={50}
            data-cut-assemble-count value={props.count} disabled={props.busy}
            onChange={(e) => resetCount(Number(e.target.value))} />
        </label>
        <label className="cd-field cd-field--inline">
          <span className="cd-field-label">Length (s)</span>
          <input className="cd-input cd-input--num" type="number" min={3} max={600}
            data-cut-assemble-target value={props.targetS} disabled={props.busy}
            onChange={(e) => resetTarget(Number(e.target.value))} />
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
          <input className="cd-input cd-input--num" type="number" min={1} max={50} data-cut-assemble-count value={props.count} disabled={props.busy}
            onChange={(e) => resetCount(Number(e.target.value))} /></label>
        <label className="cd-field cd-field--inline"><span className="cd-field-label">Target length (s)</span>
          <input className="cd-input cd-input--num" type="number" min={3} max={600} data-cut-assemble-target value={props.targetS} disabled={props.busy}
            onChange={(e) => resetTarget(Number(e.target.value))} /></label>
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
      <input className="cd-input cd-input--num" type="number" min={0} max={1} step={0.05} data-cut-assemble-minscore value={props.minScore} disabled={props.busy}
        onChange={(e) => props.onMinScore(Math.max(0, Math.min(1, Number(e.target.value) || 0.35)))} /></label>
    <button className="cd-btn cd-btn--primary" data-cut-assemble-run disabled={props.busy || !props.effectiveAsset} onClick={props.onRunFromScript}>
      {props.busy ? 'Matching…' : <><Icon name="text" size={14} tone="brand" /> Match script to footage</>}
    </button>
    {plan && <ApplyPlanControl mode="from_script" state={props.applyState}
      disabledReason={plan.binding.selected_ranges.length === 0 ? 'No script lines matched this source, so there is nothing safe to add.' : undefined}
      onApply={props.onApplyFromScript} />}
  </>
}
