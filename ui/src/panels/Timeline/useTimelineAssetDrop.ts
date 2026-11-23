import { useEffect, type Dispatch, type RefObject, type SetStateAction } from 'react'
import type { Project } from '../../lib/client'
import { ASSET_DRAG_DROP, ASSET_DRAG_MOVE, type AssetDragDetail } from '../../lib/dnd'
import { assetHasAudio, placeLinkedAV, planTimelineAssetDrop, resolvePlacementTargets } from '../../lib/placement'
import { publishUserActionMessage } from '../../lib/userActionFeedback'
import { laidToLinkedPlacementPosition, trackRows, type TrackRow } from './layout'
import type { AssetDropState } from './TimelineOverlays'

function assetDragDetailFrom(v: unknown): AssetDragDetail | null {
  if (!v || typeof v !== 'object') return null
  const { asset, kind, clientX, clientY, alt, shift } = v as AssetDragDetail
  if (typeof asset !== 'string' || typeof kind !== 'string' || typeof clientX !== 'number' || typeof clientY !== 'number') return null
  return { asset, kind, clientX, clientY, alt: alt === true, shift: shift === true }
}

interface TimelineAssetDropArgs {
  project: Project | null
  scrollRef: RefObject<HTMLDivElement | null>
  clientXToMs: (clientX: number) => number
  clientYToRow: (clientY: number) => TrackRow | null
  snapAsset: (atMs: number, durationMs: number, shift: boolean) => number
  setAssetDnd: Dispatch<SetStateAction<AssetDropState | null>>
}

export function useTimelineAssetDrop({ project, scrollRef, clientXToMs, clientYToRow, snapAsset, setAssetDnd }: TimelineAssetDropArgs) {
  useEffect(() => {
    const resolve = (e: Event) => {
      const d = e instanceof CustomEvent ? assetDragDetailFrom(e.detail) : null
      const r = scrollRef.current?.getBoundingClientRect()
      if (!d || !r || d.clientX < r.left || d.clientX > r.right || d.clientY < r.top || d.clientY > r.bottom) return null
      const kind = d.kind === 'audio' ? 'audio' : d.kind === 'image' ? 'image' : 'video'
      const probe = project?.assets[d.asset]?.probe as { duration_ms?: number } | undefined
      const durationMs = kind === 'image' ? 3000 : probe?.duration_ms ?? 0
      const row = clientYToRow(d.clientY)
      const laidMs = snapAsset(clientXToMs(d.clientX), durationMs, !!d.shift)
      const plan = planTimelineAssetDrop({ asset: d.asset, kind, at_ms: 0,
        duration_ms: kind === 'image' ? durationMs : undefined,
        hasAudio: assetHasAudio(project, d.asset), target: row, alt: d.alt })
      if (!plan) return { error: 'This track is locked. Unlock it before dropping a clip.' }
      const targets = resolvePlacementTargets(project, plan)
      const createsLinkedTarget = kind === 'video' && assetHasAudio(project, d.asset)
        && (plan.newVideoTrack || plan.newAudioTrack || !targets.videoTrack || !targets.audioTrack)
      const position = project
        ? laidToLinkedPlacementPosition(project, laidMs, [targets.videoTrack, targets.audioTrack], createsLinkedTarget)
        : { ok: true as const, atMs: Math.round(laidMs) }
      if (!position.ok) return { error: position.error }
      plan.at_ms = position.atMs
      plan.videoTrack ??= targets.videoTrack
      plan.audioTrack ??= targets.audioTrack
      const destination = trackRows(project).find(t => t.id === (kind === 'audio' ? targets.audioTrack : targets.videoTrack))
      return { plan, preview: { atMs: laidMs, durationMs, trackId: destination?.id ?? null,
        top: destination?.top ?? 0, height: destination?.height ?? 40, insert: !!d.alt } }
    }
    const onMove = (e: Event) => {
      const resolved = resolve(e)
      setAssetDnd(resolved?.preview ?? null)
    }
    const onDrop = async (e: Event) => {
      const resolved = resolve(e)
      setAssetDnd(null)
      if (!resolved) return
      if (resolved.error) { publishUserActionMessage(resolved.error); return }
      if (!resolved.plan) return
      const result = await placeLinkedAV({ ...resolved.plan, project })
      if (!result.ok) publishUserActionMessage(`Could not place this asset: ${result.error ?? 'the editor rejected the drop'}`)
    }
    document.addEventListener(ASSET_DRAG_MOVE, onMove)
    document.addEventListener(ASSET_DRAG_DROP, onDrop)
    return () => {
      document.removeEventListener(ASSET_DRAG_MOVE, onMove)
      document.removeEventListener(ASSET_DRAG_DROP, onDrop)
    }
  }, [clientXToMs, clientYToRow, project, scrollRef, snapAsset, setAssetDnd])
}
