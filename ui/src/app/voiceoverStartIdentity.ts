// Pure selection for the only browser-persisted part of a Timeline voiceover
// start. The caller owns persistence and the server owns admission; this keeps
// an ambiguous response-loss retry separate from a known server refusal.

import type { VoiceoverReattachIdentity } from './voiceoverOwnerRuntime'

/**
 * The sole response disposition that proves the coordinator did not retain a
 * matching active owner and refused this start before admission. All other
 * errors preserve the non-secret retry identity for an exact recovery retry.
 */
export const VOICEOVER_START_RETRY_REJECTED = 'voiceover_start_retry_rejected'

export interface VoiceoverStartTarget {
  projectRevision: string
  audioTrack: string
  startMs: number
  outMs: number | null
}

/**
 * Reuse a retry identity before considering the live UI target. The
 * playhead/range/revision intentionally never override an unresolved A:
 * retrying it must remain exact after a reload or ordinary project edit.
 */
export function selectVoiceoverStartIdentity(
  reattach: VoiceoverReattachIdentity | null,
  target: VoiceoverStartTarget,
  requestId: () => string,
): VoiceoverReattachIdentity {
  if (reattach) return reattach
  return {
    request_id: requestId(),
    expected_revision: target.projectRevision,
    audio_track: target.audioTrack,
    start_ms: target.startMs,
    out_ms: target.outMs,
  }
}

export function voiceoverStartRetryWasRejected(error: { code?: string } | undefined): boolean {
  return error?.code === VOICEOVER_START_RETRY_REJECTED
}
