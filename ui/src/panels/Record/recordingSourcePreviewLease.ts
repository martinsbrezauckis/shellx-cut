import type { ScreenRecordSourcePreviewStatus } from '../../lib/clientResults'

export interface RecordingSourcePreviewLease {
  sourceKey: string
  generation: number
  leaseNonce: string
}

export function recordingSourcePreviewGeneration(status: ScreenRecordSourcePreviewStatus): number | null {
  return Number.isSafeInteger(status.generation) && status.generation !== null && status.generation > 0
    ? status.generation
    : null
}

export function recordingSourcePreviewRetainsLease(status: ScreenRecordSourcePreviewStatus): boolean {
  return status.state === 'starting' || status.state === 'ready' || status.state === 'paused'
}

export function recordingSourcePreviewLeaseNonce(status: ScreenRecordSourcePreviewStatus): string | null {
  return typeof status.lease_nonce === 'string' && /^[a-f0-9]{32}$/.test(status.lease_nonce)
    ? status.lease_nonce
    : null
}

/** A frame/status read is usable only by the lease that acknowledged its start. */
export function recordingSourcePreviewLeaseMatches(
  status: ScreenRecordSourcePreviewStatus,
  expectedGeneration: number | null,
  expectedLeaseNonce: string | null,
): boolean {
  return recordingSourcePreviewGeneration(status) === expectedGeneration
    && recordingSourcePreviewLeaseNonce(status) === expectedLeaseNonce
}

/** A terminal, malformed, or generation-less status can never retain a UI lease. */
export function recordingSourcePreviewLease(
  sourceKey: string | null,
  status: ScreenRecordSourcePreviewStatus,
): RecordingSourcePreviewLease | null {
  const generation = recordingSourcePreviewGeneration(status)
  const leaseNonce = recordingSourcePreviewLeaseNonce(status)
  return sourceKey !== null && generation !== null && leaseNonce !== null && recordingSourcePreviewRetainsLease(status)
    ? { sourceKey, generation, leaseNonce }
    : null
}

/**
 * A failed start has no admitted result generation for this panel. If a later
 * status read is still active, it may belong to a concurrent client or prior
 * request, so do not adopt a lease that this panel cannot prove it owns.
 */
export function recordingSourcePreviewFailedStartStatus(
  status: ScreenRecordSourcePreviewStatus,
): ScreenRecordSourcePreviewStatus {
  if (recordingSourcePreviewRetainsLease(status)
    && recordingSourcePreviewGeneration(status) !== null
  ) return { state: 'unavailable', recursion: 'none', has_frame: false, generation: null }
  // A malformed terminal status must not perpetuate an owner nonce after its
  // generation ended; a nonce is valid only as part of an active lease pair.
  const { lease_nonce: _leaseNonce, ...terminal } = status
  return terminal
}

/** Destructive controls must bind their request to the exact lease they observed. */
export function recordingSourcePreviewControlArgs(
  generation: number | null,
  leaseNonce: string | null,
): { expected_generation: number; expected_lease_nonce: string } | null {
  return Number.isSafeInteger(generation) && generation !== null && generation > 0
    && typeof leaseNonce === 'string' && /^[a-f0-9]{32}$/.test(leaseNonce)
    ? { expected_generation: generation, expected_lease_nonce: leaseNonce }
    : null
}
