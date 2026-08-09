import { useEffect, type Dispatch, type RefObject, type SetStateAction } from 'react'
import { callVerb } from '../../lib/client'
import { ASSET_DRAG_DROP, ASSET_DRAG_MOVE, type AssetDragDetail } from '../../lib/dnd'
import { placeLinkedAV, planTimelineAssetDrop } from '../../lib/placement'
import type { TrackRow } from './layout'
import type { AssetDropState } from './TimelineOverlays'

const isObject = (v: unknown): v is object => v !== null && typeof v === 'object'

function assetDragDetailFrom(v: unknown): AssetDragDetail | null {
  if (!isObject(v)) return null
  const asset = Reflect.get(v, 'asset')
  const kind = Reflect.get(v, 'kind')
  const clientX = Reflect.get(v, 'clientX')
  const clientY = Reflect.get(v, 'clientY')
  const alt = Reflect.get(v, 'alt')
  if (typeof asset !== 'string' || typeof kind !== 'string' || typeof clientX !== 'number' || typeof clientY !== 'number') return null
  return { asset, kind, clientX, clientY, alt: alt === true }
}

interface TimelineAssetDropArgs {
  scrollRef: RefObject<HTMLDivElement | null>
  clientXToMs: (clientX: number) => number
  clientYToRow: (clientY: number) => TrackRow | null
  setAssetDnd: Dispatch<SetStateAction<AssetDropState | null>>
  /** LAID pointer position → EDITORIAL at_ms for edit.insert (the engine's
   * cumulative-track cursor; the two diverge after an upstream crossfade).
   * The hover PREVIEW stays laid — it draws where the pointer is. */
  dropMsToEditorial: (laidMs: number, row: TrackRow | null) => number
}

export function useTimelineAssetDrop({
  scrollRef,
  clientXToMs,
  clientYToRow,
  setAssetDnd,
  dropMsToEditorial,
}: TimelineAssetDropArgs) {
  useEffect(() => {
    const within = (x: number, y: number) => {
      const el = scrollRef.current
      if (!el) return false
      const r = el.getBoundingClientRect()
      return x >= r.left && x <= r.right && y >= r.top && y <= r.bottom
    }
    const onMove = (e: Event) => {
      const d = e instanceof CustomEvent ? assetDragDetailFrom(e.detail) : null
      if (!d) { setAssetDnd(null); return }
      if (!within(d.clientX, d.clientY)) { setAssetDnd(null); return }
      const row = clientYToRow(d.clientY)
      setAssetDnd({ atMs: clientXToMs(d.clientX), trackId: row?.id ?? null })
    }
    const onDrop = async (e: Event) => {
      const d = e instanceof CustomEvent ? assetDragDetailFrom(e.detail) : null
      setAssetDnd(null)
      if (!d) return
      if (!within(d.clientX, d.clientY)) return
      const kind = d.kind === 'audio' ? 'audio' : d.kind === 'image' ? 'image' : 'video'
      const durMs = d.kind === 'image' ? 3000 : undefined
      const row = clientYToRow(d.clientY)
      // Pointer position is LAID; edit.insert keys on EDITORIAL time.
      const atMs = dropMsToEditorial(clientXToMs(d.clientX), row)
      const plan = planTimelineAssetDrop({ asset: d.asset, kind, at_ms: atMs, duration_ms: durMs, target: row, overlay: d.alt })
      if (!plan) return
      if (plan.createTrackKind && plan.useCreatedTrackFor) {
        const r = await callVerb('edit.add_track', {
          kind: plan.createTrackKind,
          rationale: plan.createTrackKind === 'audio' ? 'new audio line for a dropped clip' : 'new overlay line for a dropped clip',
        })
        const newTrack = r.ok ? (r.result as { track_id?: string } | null)?.track_id : undefined
        if (!newTrack) return
        if (plan.useCreatedTrackFor === 'video') plan.videoTrack = newTrack
        if (plan.useCreatedTrackFor === 'audio') plan.audioTrack = newTrack
      }
      const { createTrackKind, useCreatedTrackFor, ...place } = plan
      await placeLinkedAV(place)
    }
    document.addEventListener(ASSET_DRAG_MOVE, onMove)
    document.addEventListener(ASSET_DRAG_DROP, onDrop)
    return () => {
      document.removeEventListener(ASSET_DRAG_MOVE, onMove)
      document.removeEventListener(ASSET_DRAG_DROP, onDrop)
    }
  }, [clientXToMs, clientYToRow, dropMsToEditorial, scrollRef, setAssetDnd])
}
