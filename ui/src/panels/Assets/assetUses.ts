import type { SequenceIndexClipRow, SequenceIndexResult } from '../../lib/client'

/** One exact, path-light timeline occurrence of an imported asset. */
export interface AssetUse {
  sequenceId: string
  sequenceName: string
  active: boolean
  clipId: string
  trackId: string
  trackKind: 'video' | 'audio' | 'caption'
  atMs: number
  /** Sequence Index observed this occurrence's shared source as offline. */
  offline: boolean
}

export interface AssetUsesResult {
  uses: AssetUse[]
  /** The shared bounded index did not return every clip in the project. */
  truncated: boolean
}

function isExactAssetUse(row: SequenceIndexClipRow | { kind: 'marker' }, assetId: string): row is SequenceIndexClipRow {
  return row.kind === 'clip' && row.asset === assetId
}

/**
 * The Sequence Index is the shared source of truth for cross-sequence
 * navigation. Filter its stable asset ids locally rather than matching visible
 * labels, which can be duplicated or renamed.
 */
export function assetUsesFromSequenceIndex(index: SequenceIndexResult, assetId: string): AssetUsesResult {
  const uses = index.results
    .filter((row): row is SequenceIndexClipRow => isExactAssetUse(row, assetId))
    .map((row) => ({
      sequenceId: row.sequence_id,
      sequenceName: row.sequence_name,
      active: row.active,
      clipId: row.id,
      trackId: row.track_id,
      trackKind: row.track_kind,
      atMs: Math.max(0, Math.round(row.at_ms)),
      offline: row.offline,
    }))
    .sort((a, b) => a.sequenceName.localeCompare(b.sequenceName) || a.trackId.localeCompare(b.trackId) || a.atMs - b.atMs || a.clipId.localeCompare(b.clipId))
  return { uses, truncated: index.truncated }
}
