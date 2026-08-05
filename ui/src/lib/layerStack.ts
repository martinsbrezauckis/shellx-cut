export interface LayerStackTrack {
  id: string
  kind: string
  clips: readonly unknown[]
  locked?: boolean
}

/**
 * The renderer reserves the first non-empty video track as the base canvas.
 * Empty video tracks do not change which populated track owns base semantics.
 */
export function baseVideoTrackId(tracks: readonly LayerStackTrack[]): string | null {
  return tracks.find((track) => track.kind === 'video' && track.clips.length > 0)?.id ?? null
}

export function isTrackLocked(
  tracks: readonly LayerStackTrack[],
  trackId: string | null | undefined,
): boolean {
  return !!trackId && tracks.find((track) => track.id === trackId)?.locked === true
}
