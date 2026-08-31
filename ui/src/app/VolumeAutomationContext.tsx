import { createContext, useContext, useEffect, useReducer, useRef, type ReactNode } from 'react'
import type { Keyframe, KfInterp } from '../lib/client'
import { runUserVerb } from '../lib/userActionFeedback'
import {
  createVolumeAutomationMutationController,
  nextVolumeAutomationRequestId,
  volumeAutomationKeyframesFingerprint,
  volumeAutomationNeedsAuthoritativeRefresh,
  volumeAutomationProjectLeaseAdmits,
  volumeAutomationStaticGainState,
  volumeAutomationTrack,
  volumeAutomationUnavailableReason,
  type VolumeAutomationMutationState,
  type VolumeAutomationStaticGainState,
  type VolumeAutomationTransientState,
} from '../panels/Inspector/volumeAutomationModel'

export interface VolumeAutomationInput {
  clipId: string
  durationMs: number
  keyframes: Keyframe[] | null | undefined
  projectRevision?: string | null
  hasSpeedRamp?: boolean
  staticGainDb?: number
}

interface Entry {
  controller: ReturnType<typeof createVolumeAutomationMutationController>
  serverTrack: ReturnType<typeof volumeAutomationTrack>
  fingerprint: string
  projected: { projectRevision: string; track: ReturnType<typeof volumeAutomationTrack> } | null
  refreshRequired: boolean
  input: VolumeAutomationInput
  message: string
}

type UserVerbResponse = Awaited<ReturnType<typeof runUserVerb>>
type MutationControls = { request_id: string; expected_revision: string }

export interface VolumeAutomationView {
  track: ReturnType<typeof volumeAutomationTrack>
  mutationState: VolumeAutomationMutationState
  staticGainState: VolumeAutomationStaticGainState
  unavailableReason: string | null
  message: string
  commit: (points: { t_ms: number; value: number }[], interp: KfInterp, rationale: string) => Promise<void>
  runStaticGain: (action: (controls: MutationControls) => Promise<UserVerbResponse>) => Promise<boolean>
}

/** One app-scoped coordinator serializes every controlled volume write at the
 * project revision, while retaining a per-clip complete-track projection for
 * the sole safe optimistic follow-up: the same clip after its saved SET. */
class VolumeAutomationCoordinator {
  private entries = new Map<string, Entry>()
  private listeners = new Set<() => void>()
  private projectLease = createVolumeAutomationMutationController(null)

  subscribe(listener: () => void) {
    this.listeners.add(listener)
    return () => { this.listeners.delete(listener) }
  }

  dispose() {
    for (const entry of this.entries.values()) entry.controller.dispose()
    this.projectLease.dispose()
    this.entries.clear()
    this.listeners.clear()
  }

  sync(input: VolumeAutomationInput): VolumeAutomationView {
    this.projectLease.observeAuthoritative(input.projectRevision)
    const fingerprint = volumeAutomationKeyframesFingerprint(input.keyframes)
    let entry = this.entries.get(input.clipId)
    if (!entry) {
      entry = {
        controller: createVolumeAutomationMutationController(input.projectRevision),
        serverTrack: volumeAutomationTrack(input.keyframes),
        fingerprint,
        projected: null,
        refreshRequired: false,
        input,
        message: '',
      }
      this.entries.set(input.clipId, entry)
    }
    entry.input = input
    const observed = entry.controller.observeAuthoritative(input.projectRevision)
    if (fingerprint !== entry.fingerprint) {
      entry.serverTrack = volumeAutomationTrack(input.keyframes)
      entry.fingerprint = fingerprint
    }
    if (entry.refreshRequired && observed.applied) entry.refreshRequired = false
    if (entry.projected && entry.projected.projectRevision !== observed.state.projectRevision) {
      entry.projected = null
    } else if (entry.projected && JSON.stringify(entry.projected.track) === JSON.stringify(entry.serverTrack)) {
      entry.projected = null
    }
    return this.view(input.clipId)
  }

  private view(clipId: string): VolumeAutomationView {
    const entry = this.entries.get(clipId)
    if (!entry) throw new Error(`volume automation clip is not registered: ${clipId}`)
    const clipMutationState = entry.controller.state()
    const projectMutationState = this.projectLease.state()
    const leaseAdmitted = this.projectLeaseAdmits(entry, clipMutationState, projectMutationState)
    const mutationState: VolumeAutomationMutationState = {
      projectRevision: projectMutationState.projectRevision,
      inFlight: clipMutationState.inFlight || projectMutationState.inFlight,
    }
    const projected = entry.projected?.projectRevision === mutationState.projectRevision
    const track = projected ? entry.projected!.track : entry.serverTrack
    const transient: VolumeAutomationTransientState = {
      clipId,
      inFlight: mutationState.inFlight,
      effectivePointCount: track.points.length,
      projected,
      projectRevision: mutationState.projectRevision,
      refreshRequired: entry.refreshRequired || (!leaseAdmitted && !projectMutationState.inFlight),
    }
    const staticGainState = volumeAutomationStaticGainState(
      clipId,
      entry.serverTrack.points.length,
      transient,
      entry.input.projectRevision ?? null,
    )
    const unavailableReason = volumeAutomationUnavailableReason(entry.input.durationMs, entry.input.hasSpeedRamp === true)
      ?? (!mutationState.projectRevision
        ? 'Volume automation is waiting for the current project revision.'
        : projectMutationState.inFlight || leaseAdmitted
          ? null
          : 'Refresh project state before editing volume automation.')
    return {
      track,
      mutationState,
      staticGainState,
      unavailableReason,
      message: entry.message,
      commit: async (points, interp, rationale) => this.commit(clipId, points, interp, rationale),
      runStaticGain: (action) => this.runStaticGain(clipId, action),
    }
  }

  private projectLeaseAdmits(
    entry: Entry,
    clipMutationState = entry.controller.state(),
    projectMutationState = this.projectLease.state(),
  ) {
    return volumeAutomationProjectLeaseAdmits(
      projectMutationState,
      clipMutationState,
      entry.input.projectRevision,
      entry.projected?.projectRevision,
    )
  }

  private beginMutation(entry: Entry, requestId: string): MutationControls | null {
    if (!this.projectLeaseAdmits(entry)) return null
    const projectControls = this.projectLease.begin(requestId)
    const clipControls = entry.controller.begin(requestId)
    if (projectControls && clipControls && projectControls.expected_revision === clipControls.expected_revision) return projectControls
    if (projectControls) this.projectLease.complete(requestId, { ok: false, errorCode: 'admission' })
    if (clipControls) entry.controller.complete(requestId, { ok: false, errorCode: 'admission' })
    return null
  }

  private blockedMessage() {
    const state = this.projectLease.state()
    if (state.inFlight) return 'Volume automation is already updating.'
    return state.projectRevision
      ? 'Refresh project state before editing volume automation.'
      : 'Volume automation needs the current project revision before it can save.'
  }

  private settle(entry: Entry, requestId: string, response: UserVerbResponse) {
    const outcome = {
      ok: response?.ok === true,
      projectRevision: response?.project_revision,
      errorCode: response?.error?.code,
    }
    const clipCompletion = entry.controller.complete(requestId, outcome)
    const projectCompletion = this.projectLease.complete(requestId, outcome)
    const saved = response?.ok === true
      && clipCompletion.owned
      && projectCompletion.owned
      && clipCompletion.status === 'saved'
      && projectCompletion.status === 'saved'
      && clipCompletion.state.projectRevision === projectCompletion.state.projectRevision
    const refreshRequired = !saved && (
      !clipCompletion.owned
      || !projectCompletion.owned
      || volumeAutomationNeedsAuthoritativeRefresh(response !== null, clipCompletion)
      || volumeAutomationNeedsAuthoritativeRefresh(response !== null, projectCompletion)
      || clipCompletion.status !== 'failed'
      || projectCompletion.status !== 'failed'
    )
    return { saved, refreshRequired, projectRevision: projectCompletion.state.projectRevision }
  }

  private requireAuthoritativeRefresh(entry: Entry, expectedRevision: string) {
    entry.controller.requireAuthoritativeAfter(expectedRevision)
    this.projectLease.requireAuthoritativeAfter(expectedRevision)
    entry.projected = null
    entry.refreshRequired = true
  }

  private async runStaticGain(clipId: string, action: (controls: MutationControls) => Promise<UserVerbResponse>): Promise<boolean> {
    const entry = this.entries.get(clipId)
    if (!entry) return false
    entry.controller.observeAuthoritative(entry.input.projectRevision)
    const current = this.view(clipId).staticGainState
    if (current.blocked || !this.projectLeaseAdmits(entry)) return false
    const requestId = nextVolumeAutomationRequestId()
    const controls = this.beginMutation(entry, requestId)
    if (!controls) {
      entry.message = this.blockedMessage()
      this.emit()
      return false
    }
    let response: UserVerbResponse = null
    try {
      response = await action(controls)
    } catch {
      response = null
    } finally {
      const completion = this.settle(entry, requestId, response)
      if (completion.saved) {
        // edit.gain has no complete volume-track projection to safely compose
        // with another write, so wait for a project.state refresh.
        this.requireAuthoritativeRefresh(entry, controls.expected_revision)
        entry.message = 'Clip Gain saved. Refresh project state before further volume edits.'
      } else if (completion.refreshRequired) {
        this.requireAuthoritativeRefresh(entry, controls.expected_revision)
        entry.message = 'Could not confirm clip Gain. Refresh project state before editing volume.'
      } else {
        entry.message = 'Could not change clip Gain. Try again.'
      }
      this.emit()
    }
    return response?.ok === true
  }

  private async commit(clipId: string, points: { t_ms: number; value: number }[], interp: KfInterp, rationale: string) {
    const entry = this.entries.get(clipId)
    if (!entry) return
    entry.controller.observeAuthoritative(entry.input.projectRevision)
    const requestId = nextVolumeAutomationRequestId()
    const controls = this.beginMutation(entry, requestId)
    if (!controls) {
      entry.message = this.blockedMessage()
      this.emit()
      return
    }
    entry.message = ''
    this.emit()
    let response: Awaited<ReturnType<typeof runUserVerb>> = null
    try {
      response = await runUserVerb(
        'edit.keyframe',
        { clip: clipId, param: 'volume', points, interp, rationale, ...controls },
        'Could not update volume automation.',
      )
    } finally {
      const completion = this.settle(entry, requestId, response)
      if (completion.saved && completion.projectRevision) {
        entry.projected = { projectRevision: completion.projectRevision, track: { points, interp } }
        entry.refreshRequired = false
        entry.message = points.length
          ? `${points.length} volume point${points.length === 1 ? '' : 's'} saved.`
          : 'Volume automation cleared — static Gain applies again.'
      } else if (completion.refreshRequired) {
        this.requireAuthoritativeRefresh(entry, controls.expected_revision)
        entry.message = 'Could not confirm volume automation. Refresh project state before editing it or static Gain.'
      } else {
        entry.projected = null
        entry.refreshRequired = false
        entry.message = 'Could not save volume automation. Try again.'
      }
      this.emit()
    }
  }

  private emit() {
    for (const listener of this.listeners) listener()
  }
}

const VolumeAutomationContext = createContext<VolumeAutomationCoordinator | null>(null)

export function VolumeAutomationProvider({ children }: { children: ReactNode }) {
  const coordinatorRef = useRef<VolumeAutomationCoordinator | null>(null)
  if (!coordinatorRef.current) coordinatorRef.current = new VolumeAutomationCoordinator()
  useEffect(() => () => coordinatorRef.current?.dispose(), [])
  return <VolumeAutomationContext.Provider value={coordinatorRef.current}>{children}</VolumeAutomationContext.Provider>
}

export function useVolumeAutomation(input: VolumeAutomationInput): VolumeAutomationView {
  const coordinator = useContext(VolumeAutomationContext)
  if (!coordinator) throw new Error('Volume automation must be mounted inside VolumeAutomationProvider.')
  const [, update] = useReducer((value) => value + 1, 0)
  useEffect(() => coordinator.subscribe(update), [coordinator])
  return coordinator.sync(input)
}
