export type TrackOrderDirection = 'back' | 'forward'

export interface TrackOrderTrack {
  id: string
  kind: string
}

export interface TrackOrderStatus {
  index: number
  count: number
  canMoveBack: boolean
  canMoveForward: boolean
}

export function trackOrderStatus(tracks: TrackOrderTrack[], trackId: string): TrackOrderStatus | null {
  const track = tracks.find((t) => t.id === trackId)
  if (!track) return null
  const sameKind = tracks.filter((t) => t.kind === track.kind)
  const index = sameKind.findIndex((t) => t.id === trackId)
  if (index < 0) return null
  return {
    index,
    count: sameKind.length,
    canMoveBack: index > 0,
    canMoveForward: index < sameKind.length - 1,
  }
}

export function trackReorderTargetIndex(
  tracks: TrackOrderTrack[],
  trackId: string,
  direction: TrackOrderDirection,
): number | null {
  const status = trackOrderStatus(tracks, trackId)
  if (!status) return null
  if (direction === 'back') return status.canMoveBack ? status.index - 1 : null
  return status.canMoveForward ? status.index + 1 : null
}
