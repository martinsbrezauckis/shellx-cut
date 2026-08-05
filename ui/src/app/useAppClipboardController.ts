import { useCallback, useEffect, useRef, useState, type Dispatch, type SetStateAction } from 'react'
import { callVerb, type Project, type Track } from '../lib/client'
import { shouldIgnoreGlobalShortcut } from '../lib/dom'
import { type ClipSnapshot, pasteTargetTrack, snapshotClip } from './model'

interface AppClipboardControllerArgs {
  project: Project | null
  playheadMs: number
  selectedClipIds: string[]
  setSelectedClipIds: Dispatch<SetStateAction<string[]>>
}

export function useAppClipboardController({
  project,
  playheadMs,
  selectedClipIds,
  setSelectedClipIds,
}: AppClipboardControllerArgs) {
  const clipboardRef = useRef<ClipSnapshot | null>(null)
  const [clipboardHasContent, setClipboardHasContent] = useState(false)
  // Paste-attributes needs the SOURCE clip id (the copied clip must still
  // exist on the timeline; the server verb re-validates that honestly).
  const [clipboardClipId, setClipboardClipId] = useState<string | null>(null)
  const [clipboardNotice, setClipboardNotice] = useState<string | null>(null)
  const liveRef = useRef({ project, playheadMs, selectedClipIds })
  const clipboardWarnedAtRef = useRef(0)
  const clipboardNoticeTimerRef = useRef<number | null>(null)
  liveRef.current = { project, playheadMs, selectedClipIds }

  const clearClipboard = useCallback(() => {
    clipboardRef.current = null
    setClipboardHasContent(false)
    setClipboardClipId(null)
  }, [])

  const copyClip = useCallback((clipId: string): boolean => {
    const snap = snapshotClip(liveRef.current.project, clipId)
    if (!snap) return false
    clipboardRef.current = snap
    setClipboardHasContent(true)
    setClipboardClipId(snap.clipId)
    return true
  }, [])

  const cutClip = useCallback(async (clipId: string) => {
    if (!copyClip(clipId)) return
    const project = liveRef.current.project
    const snap = clipboardRef.current
    if (!snap || !project) return

    const clipTimelineDur = (c: Track['clips'][number]): number =>
      'duration_ms' in c
        ? c.duration_ms
        : 'range_ms' in c
          ? c.range_ms[1] - c.range_ms[0]
          : Math.round((c.src_out_ms - c.src_in_ms) / (c.speed ?? 1))
    const ranges: { track: string; start: number; dur: number }[] = []
    let srcStart: number | null = null
    let srcDur = 0
    {
      const track = project.tracks.find((t) => t.id === snap.trackId)
      let cur = 0
      for (const c of track?.clips ?? []) {
        if ('id' in c && c.id === snap.clipId) { srcStart = cur; srcDur = clipTimelineDur(c); break }
        cur += clipTimelineDur(c)
      }
    }
    if (srcStart !== null) {
      ranges.push({ track: snap.trackId, start: srcStart, dur: srcDur })
      if (snap.kind === 'video') {
        for (const t of project.tracks) {
          if (t.kind !== 'audio') continue
          let cur = 0
          for (const c of t.clips) {
            if ('id' in c && 'asset' in c && c.asset === snap.asset && cur === srcStart) {
              ranges.push({ track: t.id, start: cur, dur: clipTimelineDur(c) })
            }
            cur += clipTimelineDur(c)
          }
        }
      }
    }
    const cutGroup = ranges.length > 1 ? `grp-cut-${crypto.randomUUID()}` : undefined
    await Promise.all(ranges.map((r) => callVerb('edit.ripple_delete', {
      track: r.track,
      range_ms: [r.start, r.start + r.dur],
      ripple: true,
      rationale: `cut clip on ${r.track} @ ${r.start}ms (Ctrl+X / context menu)`,
      ...(cutGroup ? { group_id: cutGroup } : {}),
    })))
    setSelectedClipIds([])
    void callVerb('ui.select', { clip_ids: [] })
  }, [copyClip, setSelectedClipIds])

  const pasteClip = useCallback(async () => {
    const snap = clipboardRef.current
    if (!snap) return
    const { project, playheadMs: at, selectedClipIds: sel } = liveRef.current
    const activeTrackId = sel.length > 0 ? (snapshotClip(project, sel[0])?.trackId ?? null) : null
    const toTrack = pasteTargetTrack(project, snap, activeTrackId)
    await callVerb('edit.paste', {
      clip: snap.clipId,
      asset: snap.asset,
      src_range_ms: snap.srcRange,
      to_track: toTrack,
      at_ms: Math.max(0, Math.round(at)),
      rationale: `paste clip onto ${toTrack} @ ${Math.round(at)}ms (Ctrl+V / context menu)`,
    })
  }, [])

  const warnMultiSelectionClipboard = useCallback((action: string) => {
    const now = Date.now()
    if (now - clipboardWarnedAtRef.current < 1200) return
    clipboardWarnedAtRef.current = now
    setClipboardNotice(`${action} supports one selected clip at a time.`)
    if (clipboardNoticeTimerRef.current != null) window.clearTimeout(clipboardNoticeTimerRef.current)
    clipboardNoticeTimerRef.current = window.setTimeout(() => setClipboardNotice(null), 3500)
  }, [])

  useEffect(() => () => {
    if (clipboardNoticeTimerRef.current != null) window.clearTimeout(clipboardNoticeTimerRef.current)
  }, [])

  useEffect(() => {
    const onClipKey = (e: KeyboardEvent) => {
      if (shouldIgnoreGlobalShortcut(e)) return
      if (!(e.ctrlKey || e.metaKey) || e.shiftKey || e.altKey) return
      const k = e.key.toLowerCase()
      if (k === 'c') {
        const sel = liveRef.current.selectedClipIds
        if (sel.length === 0) return
        e.preventDefault()
        if (sel.length > 1) { warnMultiSelectionClipboard('Copy'); return }
        copyClip(sel[0])
      } else if (k === 'x') {
        const sel = liveRef.current.selectedClipIds
        if (sel.length === 0) return
        e.preventDefault()
        if (sel.length > 1) { warnMultiSelectionClipboard('Cut'); return }
        void cutClip(sel[0])
      } else if (k === 'v') {
        if (!clipboardRef.current) return
        e.preventDefault()
        void pasteClip()
      }
    }
    window.addEventListener('keydown', onClipKey)
    return () => window.removeEventListener('keydown', onClipKey)
  }, [copyClip, cutClip, pasteClip, warnMultiSelectionClipboard])

  return { clipboardHasContent, clipboardClipId, clipboardNotice, copyClip, cutClip, pasteClip, clearClipboard }
}
