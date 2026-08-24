import type { Keyframe, KfInterp } from '../../lib/client'

/** The Inspector keeps normal audio automation within a deliberately legible
 * range. The engine accepts an open linear multiplier for API compatibility;
 * existing higher values remain visible in the point list without being
 * rewritten until the editor changes that point. */
export const VOLUME_AUTOMATION_MAX_PERCENT = 400

export interface VolumeAutomationPoint {
  t_ms: number
  value: number
}

export interface VolumeAutomationMutationControls {
  request_id: string
  expected_revision: string
}

export interface VolumeAutomationMutationState {
  projectRevision: string | null
  inFlight: boolean
}

/** Transient editor state supplied to the parent Volume section. It keeps
 * sibling static-Gain controls safe while a controlled keyframe save has not
 * yet appeared in the normal project refresh. */
export interface VolumeAutomationTransientState {
  clipId: string
  inFlight: boolean
  effectivePointCount: number
  projected: boolean
  /** Returned revision for a confirmed projected save, ahead of project props. */
  projectRevision?: string | null
  /** A request may have committed, but its current track/revision is unknown. */
  refreshRequired: boolean
}

export interface VolumeAutomationStaticGainState {
  projectRevision: string | null
  effectivePointCount: number
  inFlight: boolean
  refreshRequired: boolean
  blocked: boolean
}

function pointCount(value: number): number {
  return Number.isFinite(value) ? Math.max(0, Math.round(value)) : 0
}

/** Resolve the editor's optimistic/in-flight track ahead of stale project
 * props. A transient from another selected clip is intentionally ignored. */
export function volumeAutomationStaticGainState(
  clipId: string,
  serverPointCount: number,
  transient: VolumeAutomationTransientState | null | undefined,
  authoritativeRevision: string | null,
): VolumeAutomationStaticGainState {
  const current = transient?.clipId === clipId ? transient : null
  // Only an active optimistic projection may supersede project props. A failed
  // or conflicted completion reports `projected:false`; its captured track can
  // be older than an external project refresh and must never reopen Gain.
  const effectivePointCount = current?.projected
    ? pointCount(current.effectivePointCount)
    : pointCount(serverPointCount)
  const inFlight = current?.inFlight === true
  const refreshRequired = current?.refreshRequired === true
  const projectRevision = current?.projected ? revisionOrNull(current.projectRevision) ?? authoritativeRevision : authoritativeRevision
  return { projectRevision, effectivePointCount, inFlight, refreshRequired, blocked: !projectRevision || inFlight || refreshRequired || effectivePointCount > 0 }
}

/** The parent keeps this ref in addition to React state. Reporting an editor
 * save updates it synchronously, so an already-mounted Gain/reset control
 * cannot dispatch in the tiny interval before React rerenders it disabled. */
export function createVolumeAutomationStaticGainGuard() {
  let transient: VolumeAutomationTransientState | null = null
  let authoritativeClipId: string | null = null
  let authoritativePointCount = 0
  let authoritativeRevision: string | null = null
  const state = (): VolumeAutomationStaticGainState => {
    if (!authoritativeClipId) return { projectRevision: null, effectivePointCount: 0, inFlight: false, refreshRequired: true, blocked: true }
    return volumeAutomationStaticGainState(authoritativeClipId, authoritativePointCount, transient, authoritativeRevision)
  }
  return {
    observeAuthoritative(clipId: string, serverPointCount: number, projectRevision: string | null | undefined) {
      authoritativeClipId = clipId
      authoritativePointCount = pointCount(serverPointCount)
      authoritativeRevision = revisionOrNull(projectRevision)
    },
    report(next: VolumeAutomationTransientState) {
      transient = { ...next, effectivePointCount: pointCount(next.effectivePointCount) }
    },
    state,
    runIfAllowed(handlerClipId: string, action: (controls: VolumeAutomationMutationControls) => void): boolean {
      const current = state()
      if (handlerClipId !== authoritativeClipId || current.blocked || !current.projectRevision) return false
      action({ request_id: nextVolumeAutomationRequestId(), expected_revision: current.projectRevision })
      return true
    },
  }
}

export type VolumeAutomationMutationOutcome = {
  ok: boolean
  projectRevision?: string | null
  errorCode?: string | null
}

export type VolumeAutomationMutationCompletion = {
  owned: boolean
  status: 'saved' | 'external-refresh' | 'conflict' | 'failed' | 'unavailable'
  state: VolumeAutomationMutationState
}

/** A missing response or revision cannot prove whether a controlled mutation
 * committed. A normal explicit server rejection is safe to leave on current
 * props; conflicts without a newer observed revision are not. */
export function volumeAutomationNeedsAuthoritativeRefresh(
  responseReceived: boolean,
  completion: Pick<VolumeAutomationMutationCompletion, 'status' | 'state'>,
): boolean {
  return !responseReceived
    || completion.status === 'unavailable'
    || (completion.status === 'conflict' && !completion.state.projectRevision)
}

const PROJECT_REVISION = /^op_[0-9]{6,}$/

function revisionOrNull(value: string | null | undefined): string | null {
  return typeof value === 'string' && PROJECT_REVISION.test(value) ? value : null
}

/** Project revisions are opaque identifiers, but the schema deliberately
 * constrains them to zero-padded operation numbers. Comparing their digit
 * suffixes lets a local successful result reject an older project.state refresh
 * instead of reopening a pre-save track. */
export function compareProjectRevisions(left: string, right: string): number {
  const leftDigits = left.slice(3).replace(/^0+/, '') || '0'
  const rightDigits = right.slice(3).replace(/^0+/, '') || '0'
  if (leftDigits.length !== rightDigits.length) return leftDigits.length < rightDigits.length ? -1 : 1
  return leftDigits === rightDigits ? 0 : leftDigits < rightDigits ? -1 : 1
}

function newerRevision(current: string | null, candidate: string | null): string | null {
  if (!candidate) return current
  if (!current || compareProjectRevisions(candidate, current) > 0) return candidate
  return current
}

/** Owns the narrow optimistic-concurrency protocol for one automation editor.
 *
 * A real in-flight lock blocks duplicate UI events synchronously. Incoming
 * project snapshots can be older than a just-accepted mutation, so they never
 * unlock a request or replace its revision base. Every follow-up mutation uses
 * the revision returned by the previous durable response; a conflict without a
 * newer authoritative snapshot clears that base and disables the editor. */
export function createVolumeAutomationMutationController(initialRevision: string | null | undefined) {
  let projectRevision = revisionOrNull(initialRevision)
  let refreshAfterRevision: string | null = null
  let active: VolumeAutomationMutationControls | null = null
  let observedWhileLocked: string | null = null
  let disposed = false

  const state = (): VolumeAutomationMutationState => ({ projectRevision, inFlight: active !== null })

  return {
    state,
    observeAuthoritative(revision: string | null | undefined) {
      const received = revisionOrNull(revision)
      if (disposed) return { applied: false, state: state() }
      if (active) {
        observedWhileLocked = newerRevision(observedWhileLocked, received)
        return { applied: false, state: state() }
      }
      if (refreshAfterRevision) {
        if (!received || compareProjectRevisions(received, refreshAfterRevision) <= 0) {
          projectRevision = null
          return { applied: false, state: state() }
        }
        refreshAfterRevision = null
      }
      if (!received) {
        projectRevision = null
        return { applied: true, state: state() }
      }
      if (!projectRevision || compareProjectRevisions(received, projectRevision) >= 0) {
        projectRevision = received
        return { applied: true, state: state() }
      }
      return { applied: false, state: state() }
    },
    begin(requestId: string): VolumeAutomationMutationControls | null {
      if (disposed || active || !projectRevision) return null
      active = { request_id: requestId, expected_revision: projectRevision }
      observedWhileLocked = null
      return active
    },
    /** Unknown responses may have committed. Only a project.state revision
     * strictly after this request's base can make this editor safe again. */
    requireAuthoritativeAfter(expectedRevision: string) {
      refreshAfterRevision = revisionOrNull(expectedRevision)
      projectRevision = null
    },
    complete(requestId: string, outcome: VolumeAutomationMutationOutcome): VolumeAutomationMutationCompletion {
      if (disposed || !active || active.request_id !== requestId) {
        return { owned: false, status: 'failed', state: state() }
      }
      const request = active
      active = null
      const responseRevision = revisionOrNull(outcome.projectRevision)
      const externalRevision = observedWhileLocked && compareProjectRevisions(observedWhileLocked, request.expected_revision) > 0
        ? observedWhileLocked
        : null
      observedWhileLocked = null

      if (outcome.ok && responseRevision) {
        if (externalRevision && compareProjectRevisions(externalRevision, responseRevision) > 0) {
          projectRevision = externalRevision
          return { owned: true, status: 'external-refresh', state: state() }
        }
        projectRevision = responseRevision
        return { owned: true, status: 'saved', state: state() }
      }
      if (outcome.errorCode === 'conflict') {
        // A conflict may only retry from a project.state snapshot observed after
        // the rejected base. Otherwise leave the base absent and fail closed.
        projectRevision = externalRevision
        return { owned: true, status: 'conflict', state: state() }
      }
      if (externalRevision) {
        projectRevision = externalRevision
        return { owned: true, status: 'external-refresh', state: state() }
      }
      projectRevision = outcome.ok ? null : request.expected_revision
      return { owned: true, status: outcome.ok ? 'unavailable' : 'failed', state: state() }
    },
    dispose() {
      disposed = true
      active = null
      observedWhileLocked = null
      projectRevision = null
    },
  }
}

let volumeAutomationRequestSequence = 0

/** Unique per UI save, valid for the server's durable request-id schema. */
export function nextVolumeAutomationRequestId(): string {
  volumeAutomationRequestSequence += 1
  const random = globalThis.crypto?.randomUUID?.()
  return `ui-volume-${random ?? `${Date.now().toString(36)}-${volumeAutomationRequestSequence}`}`
}

export const VOLUME_AUTOMATION_INTERPOLATIONS: ReadonlyArray<{ value: KfInterp; label: string; group: 'Basic' | 'Advanced' }> = [
  { value: 'linear', label: 'Linear', group: 'Basic' },
  { value: 'hold', label: 'Hold', group: 'Basic' },
  { value: 'ease_in_out_cubic', label: 'Smooth', group: 'Basic' },
  { value: 'ease_in_quad', label: 'Ease in (quad)', group: 'Advanced' },
  { value: 'ease_out_quad', label: 'Ease out (quad)', group: 'Advanced' },
  { value: 'ease_in_out_quad', label: 'Ease in/out (quad)', group: 'Advanced' },
  { value: 'ease_in_cubic', label: 'Ease in (cubic)', group: 'Advanced' },
  { value: 'ease_out_cubic', label: 'Ease out (cubic)', group: 'Advanced' },
  { value: 'ease_in_expo', label: 'Ease in (expo)', group: 'Advanced' },
  { value: 'ease_out_expo', label: 'Ease out (expo)', group: 'Advanced' },
  { value: 'ease_in_out_expo', label: 'Ease in/out (expo)', group: 'Advanced' },
  { value: 'ease_in_back', label: 'Ease in (back)', group: 'Advanced' },
  { value: 'ease_out_back', label: 'Ease out (back)', group: 'Advanced' },
  { value: 'ease_in_out_back', label: 'Ease in/out (back)', group: 'Advanced' },
  { value: 'ease_in_elastic', label: 'Ease in (elastic)', group: 'Advanced' },
  { value: 'ease_out_elastic', label: 'Ease out (elastic)', group: 'Advanced' },
  { value: 'ease_in_out_elastic', label: 'Ease in/out (elastic)', group: 'Advanced' },
  { value: 'ease_in_bounce', label: 'Ease in (bounce)', group: 'Advanced' },
  { value: 'ease_out_bounce', label: 'Ease out (bounce)', group: 'Advanced' },
  { value: 'ease_in_out_bounce', label: 'Ease in/out (bounce)', group: 'Advanced' },
]

export function volumeAutomationTrack(keyframes: Keyframe[] | null | undefined): { points: VolumeAutomationPoint[]; interp: KfInterp } {
  const track = (keyframes ?? []).find((candidate) => candidate.param === 'volume')
  return {
    points: (track?.points ?? []).slice().sort((a, b) => a.t_ms - b.t_ms),
    interp: track?.interp ?? 'linear',
  }
}

/** Only semantically changed server keyframes supersede the local optimistic
 * track. A shallow parent re-render must not discard a just-saved point. */
export function volumeAutomationKeyframesFingerprint(keyframes: Keyframe[] | null | undefined): string {
  return JSON.stringify(keyframes ?? [])
}

export function volumeAutomationInterpolation(value: string, fallback: KfInterp): KfInterp {
  return VOLUME_AUTOMATION_INTERPOLATIONS.some((option) => option.value === value)
    ? value as KfInterp
    : fallback
}

export function clampAutomationTime(value: number, durationMs: number): number {
  const max = Math.max(0, Math.round(durationMs))
  if (!Number.isFinite(value)) return 0
  return Math.min(max, Math.max(0, Math.round(value)))
}

export function clampAutomationPercent(value: number): number {
  if (!Number.isFinite(value)) return 100
  return Math.min(VOLUME_AUTOMATION_MAX_PERCENT, Math.max(0, Math.round(value)))
}

/** `edit.keyframe` refuses clips with a non-linear speed ramp. Keep that
 * engine rule visible before an editor can appear to accept an impossible edit. */
export function volumeAutomationUnavailableReason(durationMs: number, hasSpeedRamp: boolean): string | null {
  if (hasSpeedRamp) return 'Clear the Speed ramp before editing volume automation.'
  if (!Number.isFinite(durationMs) || durationMs <= 0) {
    return 'This audio clip has no usable duration yet, so automation is unavailable.'
  }
  return null
}

/** `edit.keyframe` has SET semantics. Replace exactly one timestamp in the
 * projected list before sending the complete ordered track back to the engine. */
export function replaceVolumeAutomationPoint(
  points: VolumeAutomationPoint[],
  tMs: number,
  percent: number,
): VolumeAutomationPoint[] {
  const point = { t_ms: tMs, value: percent / 100 }
  const withoutAtTime = points.filter((candidate) => candidate.t_ms !== tMs)
  return [...withoutAtTime, point].sort((a, b) => a.t_ms - b.t_ms)
}

export function removeVolumeAutomationPoint(points: VolumeAutomationPoint[], tMs: number): VolumeAutomationPoint[] {
  return points.filter((candidate) => candidate.t_ms !== tMs)
}

export function formatAutomationTime(tMs: number): string {
  return `${(tMs / 1000).toFixed(tMs % 1000 === 0 ? 0 : 2)}s`
}

export function formatAutomationPercent(value: number): string {
  return `${Math.round(value * 100)}%`
}
