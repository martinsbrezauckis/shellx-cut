import { useCallback, useMemo, useState, type Dispatch, type SetStateAction } from 'react'
import { callVerb, exportUrl, type Project } from '../../lib/client'
import { layoutTrack } from '../Timeline/layout'
import type { PreviewExactReviewState } from './PreviewExactReview'
import type { Rate } from './PreviewTransport'

interface PreviewExportActionsArgs {
  project: Project | null
  playheadMs: number
  selectedClipIds?: string[]
  exportRange?: [number, number] | null
  setRate: Dispatch<SetStateAction<Rate>>
}

export function usePreviewExportActions({
  project,
  playheadMs,
  selectedClipIds,
  exportRange,
  setRate,
}: PreviewExportActionsArgs) {
  const [snapBusy, setSnapBusy] = useState(false)
  const [snapNote, setSnapNote] = useState<string | null>(null)
  const [exact, setExact] = useState<PreviewExactReviewState | null>(null)
  const [exactBusy, setExactBusy] = useState(false)
  const [exactNote, setExactNote] = useState<string | null>(null)
  const [saveBusy, setSaveBusy] = useState(false)

  const snapFrame = useCallback(async () => {
    if (snapBusy) return
    setSnapBusy(true)
    setSnapNote(null)
    const r = await callVerb('export.frame', { at_ms: Math.max(0, Math.round(playheadMs)) })
    setSnapBusy(false)
    setSnapNote(r.ok ? 'Frame saved → Assets' : `Snapshot failed: ${r.error?.message ?? r.error?.code ?? 'error'}`)
    setTimeout(() => setSnapNote(null), 3500)
  }, [snapBusy, playheadMs])

  const sectionRange = useCallback((): [number, number] | null => {
    if (exportRange && exportRange[1] - exportRange[0] >= 100) {
      return [Math.round(exportRange[0]), Math.round(exportRange[1])]
    }
    const sel = selectedClipIds ?? []
    if (project && sel.length) {
      const items = project.tracks
        .flatMap((t) => layoutTrack(t))
        .filter((i) => sel.includes(i.id) && (i.kind === 'video' || i.kind === 'audio'))
      if (items.length) {
        const start = Math.min(...items.map((i) => i.startMs))
        const end = Math.max(...items.map((i) => i.startMs + i.durMs))
        if (end > start) return [Math.round(start), Math.round(end)]
      }
    }
    return null
  }, [project, selectedClipIds, exportRange])

  const hasSection = useMemo(() => {
    if (exportRange && exportRange[1] - exportRange[0] >= 100) return true
    return !!(project && (selectedClipIds?.length ?? 0) > 0)
  }, [project, selectedClipIds, exportRange])

  const renderSection = useCallback(async () => {
    if (exactBusy || !project) return
    const range = sectionRange()
    if (!range) {
      setExactNote('Drag on the ruler to select a span (or select clips) to save to Assets')
      return
    }
    setRate(0)
    setExactBusy(true)
    setExactNote(null)
    const r = await callVerb('export.range', { range_ms: range, to_asset: false })
    setExactBusy(false)
    const path = (r.result as { path?: string } | undefined)?.path
    if (r.ok && path) setExact({ url: exportUrl(path), path, rangeMs: range })
    else setExactNote(`Render failed: ${r.error?.message ?? r.error?.code ?? 'error'}`)
  }, [exactBusy, project, sectionRange, setRate])

  const saveSection = useCallback(async () => {
    if (!exact || saveBusy) return
    setSaveBusy(true)
    const r = await callVerb('media.import', { path: exact.path, rationale: 'save rendered section as a clip' })
    setSaveBusy(false)
    setExactNote(r.ok ? 'Saved to Assets' : `Save failed: ${r.error?.message ?? r.error?.code ?? 'error'}`)
  }, [exact, saveBusy])

  const closeExactReview = useCallback(() => {
    setExact(null)
    setExactNote(null)
  }, [])

  return {
    exact,
    exactBusy,
    exactNote,
    hasSection,
    saveBusy,
    snapBusy,
    snapNote,
    closeExactReview,
    renderSection,
    saveSection,
    snapFrame,
  }
}
