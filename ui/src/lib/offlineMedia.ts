import type { Project } from './client'
import type { MediaCheckRow } from './clientResults'
import { mediaBasename } from './mediaPath'

export type { MediaCheckResult, MediaCheckRow } from './clientResults'

export interface OfflineAssetView {
  id: string
  label: string
  path: string
  kind: 'video' | 'audio' | 'image' | 'media'
}

export interface OfflineMediaMaps {
  offlineAssetIds: Set<string>
  modifiedMs: Map<string, number>
}

export function offlineMediaMaps(rows: readonly MediaCheckRow[]): OfflineMediaMaps {
  return {
    offlineAssetIds: new Set(rows.filter((row) => !row.exists).map((row) => row.asset)),
    modifiedMs: new Map(rows.flatMap((row) => (
      typeof row.modified_ms === 'number' ? [[row.asset, row.modified_ms] as const] : []
    ))),
  }
}

export function offlineAssetView(
  project: Project | null,
  offlineAssetIds: ReadonlySet<string>,
  assetId: string | null | undefined,
): OfflineAssetView | null {
  if (!project || !assetId || !offlineAssetIds.has(assetId)) return null
  const asset = project.assets[assetId]
  if (!asset) return null
  const rawKind = (asset.probe as { kind?: unknown } | undefined)?.kind
  const kind = rawKind === 'video' || rawKind === 'audio' || rawKind === 'image' ? rawKind : 'media'
  return {
    id: assetId,
    label: mediaBasename(asset.path),
    path: asset.path,
    kind,
  }
}
