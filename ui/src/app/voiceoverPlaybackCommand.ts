import type { Dispatch, MutableRefObject, SetStateAction } from 'react'
import type { Project } from '../lib/client'
import type { UiObservableState } from './uiControlState'
import { waitForCommittedState, waitForUiCommitTick } from './uiCommandCommit'
import { activeVoiceoverOwner } from './voiceoverOwnerRuntime'

type VoiceoverCommandError = {
  code: 'invalid_args' | 'not_found' | 'conflict' | 'no_ui_client'
  message: string
}

interface VoiceoverPlaybackCommand {
  args: Record<string, unknown>
  request_id: number
}

interface VoiceoverPlaybackCommandArgs {
  command: VoiceoverPlaybackCommand
  project: Project | null
  beforeRevision: number
  stateRef: MutableRefObject<UiObservableState>
  setPlayheadMs: Dispatch<SetStateAction<number>>
  answer: (
    requested: Record<string, unknown>,
    state: UiObservableState,
    identity: { request_id: string; request_fingerprint: string; bridge_epoch: number },
  ) => void
  reject: (requested: Record<string, unknown>, error: VoiceoverCommandError) => void
}

/** Validates, seeks, starts Preview, and only then returns the exact current
 * durable identity the server may accept for Timeline voiceover recording. */
export async function handleVoiceoverPlaybackCommand({
  command,
  project,
  beforeRevision,
  stateRef,
  setPlayheadMs,
  answer,
  reject,
}: VoiceoverPlaybackCommandArgs): Promise<void> {
  const requestId = typeof command.args.request_id === 'string' ? command.args.request_id : ''
  const requestFingerprint = typeof command.args.request_fingerprint === 'string' ? command.args.request_fingerprint : ''
  const bridgeEpoch = command.args.bridge_epoch
  const acceptedRevision = typeof command.args.accepted_revision === 'string' ? command.args.accepted_revision : ''
  const audioTrack = typeof command.args.audio_track === 'string' ? command.args.audio_track : ''
  const startMs = command.args.start_ms
  const outMs = command.args.out_ms
  const requested = Object.fromEntries(Object.entries(command.args).filter(([, value]) => value !== undefined))
  if (!requestId || !/^[A-Za-z0-9][A-Za-z0-9._:-]{0,127}$/.test(requestId)
    || !/^[a-f0-9]{64}$/i.test(requestFingerprint)
    || !Number.isSafeInteger(bridgeEpoch) || Number(bridgeEpoch) < 1
    || !acceptedRevision || !audioTrack || !Number.isSafeInteger(startMs) || Number(startMs) < 0
    || (outMs !== null && outMs !== undefined && (!Number.isSafeInteger(outMs) || Number(outMs) <= Number(startMs)))) {
    reject(requested, { code: 'invalid_args', message: 'voiceover playback request has an invalid correlated identity or range' })
    return
  }
  if (project?.project_revision !== acceptedRevision) {
    reject(requested, { code: 'conflict', message: 'voiceover playback request is stale for the current project revision' })
    return
  }
  const owner = activeVoiceoverOwner()
  if (!owner
    || owner.request_id !== requestId
    || owner.expected_revision !== acceptedRevision
    || owner.audio_track !== audioTrack
    || owner.start_ms !== Number(startMs)
    || owner.out_ms !== (typeof outMs === 'number' ? outMs : null)) {
    reject(requested, { code: 'conflict', message: 'this Preview tab does not own the active voiceover take' })
    return
  }
  const track = project.tracks.find((candidate) => candidate.id === audioTrack)
  if (!track || track.kind !== 'audio' || track.locked) {
    reject(requested, { code: track ? 'conflict' : 'not_found', message: 'voiceover playback target is no longer an unlocked audio track' })
    return
  }
  const atMs = Number(startMs)
  setPlayheadMs(atMs)
  const state = await waitForCommittedState(
    stateRef,
    beforeRevision,
    (current) => current.playhead_ms === atMs,
  )
  if (!state) {
    reject(requested, { code: 'conflict', message: 'voiceover playhead did not commit before playback' })
    return
  }
  document.dispatchEvent(new CustomEvent('cut:voiceover-playback', {
    detail: {
      request_id: requestId,
      request_fingerprint: requestFingerprint,
      bridge_epoch: Number(bridgeEpoch),
      accepted_revision: acceptedRevision,
      audio_track: audioTrack,
      start_ms: atMs,
      out_ms: typeof outMs === 'number' ? outMs : null,
      owner_session_id: owner.owner.session_id,
      owner_capability: owner.owner.capability,
    },
  }))
  await waitForUiCommitTick()
  if (!document.querySelector('[data-cut-panel="preview"][data-cut-playing="true"]')) {
    reject(requested, { code: 'conflict', message: 'Preview did not begin voiceover playback' })
    return
  }
  answer(requested, state, {
    request_id: requestId,
    request_fingerprint: requestFingerprint,
    bridge_epoch: Number(bridgeEpoch),
  })
}
