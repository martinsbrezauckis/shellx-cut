import { useCallback, useState } from 'react'
import { callVerb } from '../../lib/client'
import type { LaidItem } from './layout'

interface TimelineRangeSaveConfig {
  allItems: LaidItem[]
  selectedClipIds: string[]
}

export function useTimelineRangeSaves(cfg: { current: TimelineRangeSaveConfig }) {
  const [savingRange, setSavingRange] = useState(false)
  const [savingGif, setSavingGif] = useState(false)
  const [saveNote, setSaveNote] = useState<string | null>(null)

  // "Save to Assets" renders the selected timeline span as a reusable asset.
  const onSaveRange = useCallback(async () => {
    const c = cfg.current
    const sel = c.allItems.filter((i) => c.selectedClipIds.includes(i.id) && (i.kind === 'video' || i.kind === 'audio'))
    if (!sel.length) return
    const start = Math.round(Math.min(...sel.map((i) => i.startMs)))
    const end = Math.round(Math.max(...sel.map((i) => i.startMs + i.durMs)))
    setSavingRange(true)
    setSaveNote('Saving…')
    const r = await callVerb('export.range', { range_ms: [start, end] })
    setSavingRange(false)
    setSaveNote(r.ok ? 'Saved to Assets' : `Save failed: ${r.error?.message ?? r.error?.code ?? 'error'}`)
    setTimeout(() => setSaveNote(null), 4000)
  }, [cfg])

  const onSaveGif = useCallback(async () => {
    const c = cfg.current
    const sel = c.allItems.filter((i) => c.selectedClipIds.includes(i.id) && i.kind === 'video')
    if (!sel.length) return
    const start = Math.round(Math.min(...sel.map((i) => i.startMs)))
    const endFull = Math.round(Math.max(...sel.map((i) => i.startMs + i.durMs)))
    const end = Math.min(endFull, start + 30_000)
    setSavingGif(true)
    setSaveNote('Making GIF…')
    const r = await callVerb('export.gif', { range_ms: [start, end] })
    setSavingGif(false)
    setSaveNote(r.ok ? `GIF → Assets${end < endFull ? ' (first 30s)' : ''}` : `GIF failed: ${r.error?.message ?? r.error?.code ?? 'error'}`)
    setTimeout(() => setSaveNote(null), 4000)
  }, [cfg])

  return {
    savingRange,
    savingGif,
    saveNote,
    onSaveRange,
    onSaveGif,
  }
}
