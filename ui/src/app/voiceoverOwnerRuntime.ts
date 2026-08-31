// Volatile Timeline voiceover ownership for this browser tab. The server
// capability is deliberately never written to browser storage, logs, URLs, or
// state telemetry. Only the non-secret retry identity survives a tab reload.

export interface VoiceoverOwnerClaim {
  session_id: string
  capability: string
}

export interface VoiceoverReattachIdentity {
  request_id: string
  expected_revision: string
  audio_track: string
  start_ms: number
  out_ms: number | null
}

export interface ActiveVoiceoverOwner extends VoiceoverReattachIdentity {
  owner: VoiceoverOwnerClaim
}

const REATTACH_STORAGE_KEY = 'shellx-cut/voiceover-reattach/1'
let activeOwner: ActiveVoiceoverOwner | null = null

function storage(): Storage | null {
  try {
    return typeof window === 'undefined' ? null : window.sessionStorage
  } catch {
    return null
  }
}

function isIdentity(value: unknown): value is VoiceoverReattachIdentity {
  if (!value || typeof value !== 'object') return false
  const candidate = value as Record<string, unknown>
  return typeof candidate.request_id === 'string'
    && typeof candidate.expected_revision === 'string'
    && typeof candidate.audio_track === 'string'
    && Number.isSafeInteger(candidate.start_ms)
    && (candidate.out_ms === null || Number.isSafeInteger(candidate.out_ms))
}

function persistReattach(identity: VoiceoverReattachIdentity) {
  try {
    storage()?.setItem(REATTACH_STORAGE_KEY, JSON.stringify(identity))
  } catch {}
}

/** Store only the public start identity before sending voiceover.start. */
export function persistVoiceoverReattach(identity: VoiceoverReattachIdentity) {
  persistReattach(identity)
}

/** Read only non-secret identity used to reattach after a lost response. */
export function readVoiceoverReattach(): VoiceoverReattachIdentity | null {
  try {
    const raw = storage()?.getItem(REATTACH_STORAGE_KEY)
    if (!raw) return null
    const parsed: unknown = JSON.parse(raw)
    return isIdentity(parsed)
      ? {
          request_id: parsed.request_id,
          expected_revision: parsed.expected_revision,
          audio_track: parsed.audio_track,
          start_ms: parsed.start_ms,
          out_ms: parsed.out_ms,
        }
      : null
  } catch {
    return null
  }
}

export function rememberVoiceoverOwner(
  identity: VoiceoverReattachIdentity,
  owner: VoiceoverOwnerClaim,
): ActiveVoiceoverOwner {
  const next = {
    ...identity,
    owner,
  }
  activeOwner = next
  // Deliberately omit both server claim fields: they exist only in this
  // module's active memory and are discarded on reload/process exit.
  persistReattach(identity)
  return next
}

export function activeVoiceoverOwner(): ActiveVoiceoverOwner | null {
  return activeOwner
}

export function clearVoiceoverOwner() {
  activeOwner = null
  try {
    storage()?.removeItem(REATTACH_STORAGE_KEY)
  } catch {}
}
