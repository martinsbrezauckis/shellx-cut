import { useMemo, type CSSProperties, type Ref } from 'react'
import { Icon } from '../../icons'
import { useOfflineMedia } from '../../app/OfflineMediaContext'
import type { Project } from '../../lib/client'
import { offlineAssetView, type OfflineAssetView } from '../../lib/offlineMedia'
import { activeBaseAssetId } from './model'
import { assetMediaKind } from '../Assets/mediaReadiness'
import { overlayBoxStyle, type OverlayLayer } from './composite'

export function usePreviewOfflineMedia(
  project: Project | null,
  playheadMs: number,
  overlays: OverlayLayer[],
) {
  const media = useOfflineMedia()
  const resolved = useMemo(() => {
    // A missing title or shape movie is not offline media to relink: Cut draws
    // it again before the next exact frame (or says, on the picture, why it
    // cannot), so the stage stays live and asks for that frame.
    const offline = new Set([...media.offlineAssetIds].filter((id) => !isDrawnTitle(project, id)))
    return {
      baseOffline: offlineAssetView(project, offline, activeBaseAssetId(project, playheadMs)),
      onlineOverlays: overlays.filter((overlay) => !offline.has(overlay.assetId)),
      offlineOverlays: overlays.flatMap((overlay) => {
        const asset = offlineAssetView(project, offline, overlay.assetId)
        return asset ? [{ overlay, asset }] : []
      }),
    }
  }, [media.offlineAssetIds, overlays, playheadMs, project])
  return { ...media, ...resolved }
}

function isDrawnTitle(project: Project | null, assetId: string): boolean {
  const asset = project?.assets[assetId]
  return !!asset && assetMediaKind({ path: asset.path, probe: (asset.probe ?? {}) as { kind?: string } }) === 'title'
}

export function PreviewOfflineAsset({
  asset,
  relinking,
  overlay = false,
  style,
  onRelink,
}: {
  asset: OfflineAssetView
  relinking: boolean
  overlay?: boolean
  style?: CSSProperties
  onRelink: (assetId: string) => Promise<boolean>
}) {
  return (
    <div
      className={`pv-offline${overlay ? ' pv-offline--overlay' : ''}`}
      data-cut-preview-offline={asset.id}
      data-cut-preview-offline-kind={overlay ? 'overlay' : 'base'}
      style={style}
      role="status"
    >
      <Icon name="warning" size={overlay ? 16 : 20} />
      <span className="pv-offline__copy">
        <strong>{overlay ? 'Overlay source is offline' : 'Source file is offline'}</strong>
        <span title={asset.label}>{asset.label}</span>
      </span>
      <button
        type="button"
        className="pv-offline__relink"
        data-cut-action="preview-relink-offline"
        data-cut-preview-relink={asset.id}
        disabled={relinking}
        onClick={() => void onRelink(asset.id)}
      >
        {relinking ? 'Relinking…' : 'Relink…'}
      </button>
    </div>
  )
}

export function PreviewOfflineStage({
  stageRef,
  stageBox,
  asset,
  relinking,
  onRelink,
}: {
  stageRef: Ref<HTMLDivElement>
  stageBox: { w: number; h: number }
  asset: OfflineAssetView
  relinking: boolean
  onRelink: (assetId: string) => Promise<boolean>
}) {
  return (
    <div
      ref={stageRef}
      className="pv-stage"
      data-cut-stage
      data-cut-preview-surface="offline"
      style={{ width: stageBox.w || undefined, height: stageBox.h || undefined }}
    >
      <PreviewOfflineAsset asset={asset} relinking={relinking} onRelink={onRelink} />
    </div>
  )
}

export function PreviewOfflineOverlays({
  entries,
  relinkingAssetId,
  onRelink,
}: {
  entries: Array<{ overlay: OverlayLayer; asset: OfflineAssetView }>
  relinkingAssetId: string | null
  onRelink: (assetId: string) => Promise<boolean>
}) {
  return entries.map(({ overlay, asset }) => (
    <PreviewOfflineAsset
      key={overlay.clipId}
      asset={asset}
      overlay
      style={overlayBoxStyle(overlay.transform)}
      relinking={relinkingAssetId === asset.id}
      onRelink={onRelink}
    />
  ))
}

export function PreviewFrameError() {
  return (
    <div className="pv-frame-error" data-cut-preview-frame-error role="alert">
      <Icon name="warning" size={20} />
      <span className="pv-offline__copy">
        <strong>Preview frame could not be loaded</strong>
        <span>Check whether the source file is still available, then try again.</span>
      </span>
    </div>
  )
}
