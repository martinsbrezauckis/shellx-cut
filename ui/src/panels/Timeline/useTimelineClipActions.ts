import { useCallback, useState, type Dispatch, type SetStateAction } from 'react'
import { callVerb, type Marker, type Project } from '../../lib/client'
import { runUserVerb } from '../../lib/userActionFeedback'
import { adjacentGapSlot, isContiguousRun } from './ClipContextMenuModel'
import { laidToEditorialMs, linkedSiblings, planLinkedSplit, shortDur, timecode, type LaidItem, type Seam, type TrackRow } from './layout'
import { planRippleTrimAtPlayhead, sourceTrimAtTimelinePosition, type RippleTrimSide } from './rippleTrim'
import { timelineEditFailureMessage } from './editFeedback'

// Per-CLIP mute level: a clip has NO `muted` flag in the model, so the context-menu
// "mute clip" sets the clip's gain to nearly silence. This is separate from
// per-track mute, which is non-destructive.
const MUTE_DB = -100

export interface TimelineActionConfig {
  allItems: LaidItem[]
  markers: Marker[]
  playheadMs: number
  selectedClipIds: string[]
  rows: TrackRow[]
  onSeek: (atMs: number) => void
  onSelect: (clipIds: string[]) => void
}

interface UseTimelineClipActionsArgs {
  cfg: { current: TimelineActionConfig }
  setActiveSeam: Dispatch<SetStateAction<Seam | null>>
}

export function useTimelineClipActions({ cfg, setActiveSeam }: UseTimelineClipActionsArgs) {
  const [syncNote, setSyncNote] = useState<string | null>(null)

  const showNote = useCallback((message: string, durationMs = 6000) => {
    setSyncNote(message)
    window.setTimeout(() => setSyncNote((current) => current === message ? null : current), durationMs)
  }, [])

  const showVerbFailure = useCallback((result: Awaited<ReturnType<typeof callVerb>>, fallback: string): boolean => {
    const message = timelineEditFailureMessage(result, fallback)
    if (!message) return false
    showNote(message)
    return true
  }, [showNote])

  const cleanupEmptyTracks = useCallback(async (trackIds: Iterable<string>) => {
    const candidates = new Set(trackIds)
    if (!candidates.size) return
    const sr = await callVerb('project.state', {})
    const ps = sr.ok ? (sr.result as Project | null) : null
    if (!ps) return
    const firstVideo = ps.tracks.find((t) => t.kind === 'video')?.id
    const firstAudio = ps.tracks.find((t) => t.kind === 'audio')?.id
    for (const t of ps.tracks) {
      if (!candidates.has(t.id)) continue
      const isBase = t.id === firstVideo || t.id === firstAudio
      if (isBase || (t.kind !== 'video' && t.kind !== 'audio')) continue
      const hasContent = (t.clips ?? []).some((c) => (c as { asset?: string }).asset)
      if (!hasContent) {
        await runUserVerb(
          'edit.remove_track',
          { track: t.id, force: true, rationale: 'auto-clean: removed an emptied overlay track' },
          `Could not remove empty track ${t.id}.`,
        )
      }
    }
  }, [])

  const addTrack = useCallback(async (kind: 'video' | 'audio') => {
    const result = await callVerb('edit.add_track', {
      kind,
      rationale: `user: add ${kind} track from timeline toolbar`,
    })
    if (!result.ok) {
      showNote(result.error?.message ?? `Could not add ${kind} track.`)
      return
    }
    const trackId = (result.result as { track_id?: string } | undefined)?.track_id
    showNote(`${kind === 'video' ? 'Video' : 'Audio'} track ${trackId ?? ''} added.`.trim(), 4000)
  }, [showNote])

  const rippleTrimAtPlayhead = useCallback(async (side: RippleTrimSide) => {
    const c = cfg.current
    const plan = planRippleTrimAtPlayhead(c.allItems, c.selectedClipIds, c.playheadMs, side)
    if (!plan) {
      showNote('Move the playhead onto a video or audio clip first.')
      return
    }
    const anchor = c.allItems.find((item) => item.id === plan.clipId)
    if (!anchor || (anchor.kind !== 'video' && anchor.kind !== 'audio')) return

    // Exact linked A/V counterpart — the shared engine-mirroring resolver
    // (same criteria the server's resolve_linked_media applies).
    const linked = linkedSiblings(anchor, c.allItems)
    if (linked.length > 1) {
      showNote('This clip has multiple linked-media candidates; detach the duplicate before trimming.')
      return
    }
    const targets = [anchor, ...linked]
    const locked = targets.find((item) => c.rows.some((row) => row.id === item.trackId && row.locked))
    if (locked) {
      showNote(`Unlock track ${locked.trackId} before trimming linked media.`)
      return
    }

    let result
    if (plan.operation === 'delete') {
      const groupId = targets.length > 1 ? `grp-ripple-trim-${crypto.randomUUID()}` : undefined
      const results = await Promise.all(targets.map((item) => callVerb('edit.ripple_delete', {
        track: item.trackId,
        // range_ms is EDITORIAL time (the engine's cumulative-track cursor,
        // app/core/src/edit.rs ripple_delete): laid coords rewind after an
        // upstream crossfade and would remove the wrong span.
        range_ms: [item.editorialStartMs, item.editorialStartMs + item.durMs],
        ripple: true,
        rationale: `user ripple-trim ${side}: remove ${item.id} at playhead boundary`,
        ...(groupId ? { group_id: groupId } : {}),
      })))
      result = results.find((response) => !response.ok) ?? results[0]
      if (results.every((response) => response.ok)) {
        c.onSelect([])
      }
    } else {
      result = await callVerb('edit.trim', {
        clip: plan.clipId,
        ...plan.trim,
        linked: true,
        rationale: `user ripple-trim ${side} of ${plan.clipId} to playhead @ ${timecode(c.playheadMs)}`,
      })
    }
    if (!result?.ok) {
      showNote(result?.error?.message ?? `Could not ripple trim the clip ${side}.`)
      return
    }
    c.onSeek(plan.seekMs)
    showNote(`Ripple trimmed clip ${side} to the playhead.`, 4000)
  }, [cfg, showNote])

  /**
   * Delete the selected clips — ripple (close the gap) or lift (leave it).
   * THE delete implementation for every UI path (toolbar button, Del /
   * Alt+Del keyboard): one place owns the linked-A/V and coordinate rules so
   * the paths can't drift apart again.
   * - Linked A/V: deleting a VIDEO clip also removes its exact linked audio
   *   counterpart (linkedSiblings — the engine's resolve_linked_media
   *   criteria). Import/insert place muxed files as v1 video + a1t audio;
   *   without this the audio is orphaned and keeps the timeline long
   *   (black-tail export). Several exact counterparts = refuse rather than
   *   guess (engine ambiguity policy).
   * - range_ms is EDITORIAL time (engine cumulative-track cursor), not laid.
   * - All ops share one group_id so a single Ctrl+Z restores the whole set.
   */
  const deleteSelection = useCallback(async (ripple: boolean) => {
    const c = cfg.current
    const locked = (trackId: string) => c.rows.some((row) => row.id === trackId && row.locked)
    const sel = c.allItems.filter((i) => c.selectedClipIds.includes(i.id) && i.kind !== 'gap' && !locked(i.trackId))
    if (!sel.length) return
    // Dedup by track + editorial start so a clip and its linked audio (or a
    // multi-select that already includes both halves) aren't queued twice.
    const ranges = new Map<string, { track: string; start: number; dur: number; id: string }>()
    for (const i of sel) {
      ranges.set(`${i.trackId}:${i.editorialStartMs}`, { track: i.trackId, start: i.editorialStartMs, dur: i.durMs, id: i.id })
      if (i.kind !== 'video') continue
      const sibs = linkedSiblings(i, c.allItems)
      if (sibs.length > 1) {
        showNote(`Clip ${i.id} has multiple linked-media candidates; detach the duplicate before deleting.`)
        return
      }
      const sib = sibs[0]
      if (sib && !locked(sib.trackId)) {
        ranges.set(`${sib.trackId}:${sib.editorialStartMs}`, { track: sib.trackId, start: sib.editorialStartMs, dur: sib.durMs, id: sib.id })
      }
    }
    const delGroup = ranges.size > 1 ? `grp-del-${crypto.randomUUID()}` : undefined
    const results = await Promise.all([...ranges.values()].map((r) => runUserVerb('edit.ripple_delete', {
      track: r.track,
      range_ms: [r.start, r.start + r.dur],
      ripple,
      rationale: ripple
        ? `user ripple-delete: ${r.id} on ${r.track} (gap closes) @ ${timecode(r.start)}`
        : `user lift-delete: ${r.id} on ${r.track} (gap stays open) @ ${timecode(r.start)}`,
      ...(delGroup ? { group_id: delGroup } : {}),
    }, `Could not ${ripple ? 'ripple-delete' : 'lift'} clip ${r.id}.`)))
    if (results.some((result) => !result?.ok)) return
    c.onSelect([])
    await cleanupEmptyTracks(new Set([...ranges.values()].map((range) => range.track)))
  }, [cfg, cleanupEmptyTracks, showNote])

  /** Context-menu remove/lift of one clip + its exact linked audio
   * counterpart. Same linkage + coordinate rules as deleteSelection:
   * linkedSiblings for the pair, EDITORIAL range_ms, one group_id. */
  const removeItemById = useCallback(async (itemId: string, ripple: boolean) => {
    const c = cfg.current
    const it = c.allItems.find((i) => i.id === itemId)
    if (!it || it.kind === 'gap') return
    const ranges = new Map<string, { track: string; start: number; dur: number; id: string }>()
    ranges.set(`${it.trackId}:${it.editorialStartMs}`, { track: it.trackId, start: it.editorialStartMs, dur: it.durMs, id: it.id })
    if (it.kind === 'video') {
      const sibs = linkedSiblings(it, c.allItems)
      if (sibs.length > 1) {
        showNote(`Clip ${it.id} has multiple linked-media candidates; detach the duplicate before deleting.`)
        return
      }
      const sib = sibs[0]
      if (sib) {
        ranges.set(`${sib.trackId}:${sib.editorialStartMs}`, { track: sib.trackId, start: sib.editorialStartMs, dur: sib.durMs, id: sib.id })
      }
    }
    const delGroup = ranges.size > 1 ? `grp-del-${crypto.randomUUID()}` : undefined
    const results = await Promise.all([...ranges.values()].map((r) => runUserVerb('edit.ripple_delete', {
      track: r.track,
      range_ms: [r.start, r.start + r.dur],
      ripple,
      rationale: `${ripple ? 'remove' : 'lift'} clip ${r.id} on ${r.track} @ ${timecode(r.start)} (context menu)`,
      ...(delGroup ? { group_id: delGroup } : {}),
    }, `Could not ${ripple ? 'remove' : 'lift'} clip ${r.id}.`)))
    if (results.some((result) => !result?.ok)) return
    c.onSelect([])
    await cleanupEmptyTracks(new Set([...ranges.values()].map((range) => range.track)))
  }, [cfg, cleanupEmptyTracks, showNote])

  const removeTrackById = useCallback(async (trackId: string) => {
    const result = await runUserVerb(
      'edit.remove_track',
      { track: trackId, force: true, rationale: 'remove track + its clips (context menu)' },
      `Could not remove track ${trackId}.`,
    )
    if (!result?.ok) return
    cfg.current.onSelect([])
  }, [cfg])

  /**
   * Dispatch a planned linked split: one edit.split per target (anchor +
   * exact linked A/V counterpart), sharing a group_id so one Ctrl+Z undoes
   * the whole cut. Refusals (ambiguous counterpart / locked counterpart
   * track) surface as notes — splitting only one half would silently desync
   * the pair, the exact razor bug the demo-v2 pass caught.
   */
  const dispatchLinkedSplit = useCallback(async (
    anchor: LaidItem,
    laidCutMs: number,
    why: string,
  ): Promise<boolean> => {
    const c = cfg.current
    const plan = planLinkedSplit(anchor, c.allItems, laidCutMs, (trackId) => c.rows.some((row) => row.id === trackId && row.locked))
    if (plan.kind === 'ambiguous') {
      showNote(`Clip ${anchor.id} has multiple linked-media candidates; detach the duplicate before splitting.`)
      return false
    }
    if (plan.kind === 'locked') {
      showNote(`Unlock track ${plan.trackId} before splitting linked media.`)
      return false
    }
    const groupId = plan.targets.length > 1 ? `grp-split-${crypto.randomUUID()}` : undefined
    const results = await Promise.all(plan.targets.map((t) => runUserVerb('edit.split', {
      track: t.track,
      // at_ms is EDITORIAL time (the engine's split cursor, app/core/src/
      // edit.rs split_pinned) — planLinkedSplit converted the laid cut.
      at_ms: t.atMs,
      rationale: `${why}: split ${t.clipId} on ${t.track} @ ${timecode(t.atMs)}`,
      ...(groupId ? { group_id: groupId } : {}),
    }, `Could not split clip ${t.clipId}.`)))
    return results.every((result) => result?.ok)
  }, [cfg, showNote])

  const splitItemAt = useCallback((itemId: string, atMs: number, source: 'menu' | 'razor' = 'menu') => {
    const c = cfg.current
    const it = c.allItems.find((i) => i.id === itemId)
    if (!it || it.kind === 'gap' || it.kind === 'caption') return
    const within = atMs > it.startMs && atMs < it.startMs + it.durMs
    const cut = within ? atMs : c.playheadMs
    // Boundary guard in LAID space (the cursor's space): a split exactly on an
    // edge is a no-op in the engine; only dispatch inside the clip body.
    if (cut <= it.startMs || cut >= it.startMs + it.durMs) return
    void dispatchLinkedSplit(it, cut, source === 'razor' ? 'user razor' : 'user split (context menu)')
  }, [cfg, dispatchLinkedSplit])

  /**
   * Split at the playhead on the selected clips' tracks, else every unlocked
   * video track (the S / Cmd-B "cut here" convention). The playhead is
   * RENDER/laid time; each track's cut is planned from its covering laid item
   * so the dispatched at_ms is that track's EDITORIAL position, and each
   * video cut propagates to its exact linked audio counterpart.
   */
  const splitAtPlayhead = useCallback(() => {
    const c = cfg.current
    const locked = (trackId: string) => c.rows.some((row) => row.id === trackId && row.locked)
    const selTracks = new Set(c.allItems.filter((i) => c.selectedClipIds.includes(i.id) && !locked(i.trackId)).map((i) => i.trackId))
    const targets = selTracks.size
      ? [...selTracks]
      : c.rows.filter((row) => row.kind === 'video' && !locked(row.id)).map((row) => row.id)
    const seen = new Set<string>()
    for (const track of targets) {
      if (seen.has(track)) continue
      const anchor = c.allItems.find((i) =>
        i.trackId === track
        && (i.kind === 'video' || i.kind === 'audio')
        && c.playheadMs > i.startMs && c.playheadMs < i.startMs + i.durMs)
      if (!anchor) {
        // No media body under the playhead on this track (gap / boundary /
        // past the end) — dispatch the honest miss so the engine's own error
        // surfaces, converting through the track's laid→editorial map.
        const trackItems = c.allItems.filter((i) => i.trackId === track)
        void runUserVerb('edit.split', {
          track,
          at_ms: Math.round(laidToEditorialMs(trackItems, c.playheadMs)),
          rationale: 'split (S / Cmd-B)',
        }, `Could not split track ${track} at the playhead.`)
        continue
      }
      seen.add(track)
      // Mark the linked counterpart's track as handled so a selection that
      // includes both halves doesn't double-split the pair.
      for (const sib of linkedSiblings(anchor, c.allItems)) seen.add(sib.trackId)
      void dispatchLinkedSplit(anchor, c.playheadMs, 'user split (S / Cmd-B)')
    }
  }, [cfg, dispatchLinkedSplit])

  const fadeItem = useCallback((itemId: string, which: 'in' | 'out') => {
    const ms = 500
    const args = which === 'in' ? { clip: itemId, in_ms: ms } : { clip: itemId, out_ms: ms }
    void runUserVerb(
      'edit.fade',
      { ...args, kind: 'both', rationale: `fade ${which} ${ms}ms ${itemId} (context menu)` },
      `Could not fade clip ${itemId}.`,
    )
  }, [])

  const trimItemTo = useCallback((itemId: string, edge: 'start' | 'end', atMs: number) => {
    const c = cfg.current
    const it = c.allItems.find((i) => i.id === itemId)
    if (!it || (it.kind !== 'video' && it.kind !== 'audio')) return
    // Laid coords are fine end-to-end here: the guard and the src conversion
    // both use the WITHIN-clip offset (atMs − startMs), which is identical in
    // laid and editorial time — edit.trim takes source-time args, not at_ms.
    if (atMs <= it.startMs || atMs >= it.startMs + it.durMs) return
    const args = sourceTrimAtTimelinePosition(it, edge, atMs)
    if (!args) return
    void callVerb('edit.trim', {
      clip: it.id,
      ...args,
      linked: true,
      rationale: `trim ${edge} of ${it.id} to playhead @ ${timecode(atMs)} (context menu)`,
    }).then((result) => showVerbFailure(result, `Could not trim the linked clip ${edge}.`))
      .catch(() => showVerbFailure({ ok: false }, `Could not trim the linked clip ${edge}: server unreachable.`))
  }, [cfg, showVerbFailure])

  const reverseItem = useCallback((itemId: string) => {
    void runUserVerb('edit.reverse', { clip: itemId, rationale: `play ${itemId} backward (context menu)` }, `Could not reverse clip ${itemId}.`)
  }, [])

  const freezeItem = useCallback((itemId: string) => {
    void runUserVerb('edit.freeze', { clip: itemId, rationale: `freeze-frame ${itemId} (context menu)` }, `Could not freeze clip ${itemId}.`)
  }, [])

  const stabilizeItem = useCallback((itemId: string) => {
    void runUserVerb('edit.stabilize', { clip: itemId, rationale: `stabilize ${itemId} (context menu)` }, `Could not stabilize clip ${itemId}.`)
  }, [])

  const speedItem = useCallback((itemId: string, factor: number) => {
    void runUserVerb('edit.speed', {
      clip: itemId,
      factor,
      rationale: factor === 1
        ? `user reset ${itemId} to normal speed (context menu)`
        : `user set ${itemId} speed to ${factor}× (context menu)`,
    }, `Could not change the speed of clip ${itemId}.`)
  }, [])

  const crossfadeAdjacent = useCallback((itemId: string) => {
    const c = cfg.current
    const it = c.allItems.find((i) => i.id === itemId)
    if (!it || (it.kind !== 'video' && it.kind !== 'audio')) return
    // Adjacency + at_ms in EDITORIAL time (edit.crossfade's key,
    // app/core/src/edit.rs crossfade). Laid adjacency breaks twice over:
    // an upstream crossfade rewinds every later laid position, and a
    // crossfade ON this very seam pulls the neighbour's laid start into the
    // clip's tail (startMs ≠ end), so the neighbour was never found and
    // re-applying a transition from the menu silently no-opped.
    const onTrack = c.allItems.filter((i) => i.trackId === it.trackId && i.kind !== 'gap')
    const edEndMs = it.editorialStartMs + it.durMs
    const endNeighbour = onTrack.find((i) => i.id !== it.id && i.editorialStartMs === edEndMs)
    const startNeighbour = onTrack.find((i) => i.id !== it.id && i.editorialStartMs + i.durMs === it.editorialStartMs)
    const atMs = endNeighbour ? edEndMs : startNeighbour ? it.editorialStartMs : null
    if (atMs === null) return
    void runUserVerb('edit.crossfade', {
      track: it.trackId,
      at_ms: atMs,
      duration_ms: 500,
      transition: 'dissolve',
      rationale: `crossfade at seam @ ${timecode(atMs)} (context menu)`,
    }, 'Could not apply the crossfade.')
  }, [cfg])

  /** Redirect a video clip to its exact linked audio counterpart (the audio
   * verbs read TrackKind::Audio only). Unambiguous match only — otherwise the
   * clicked clip stays the target and the engine's own guidance surfaces. */
  const audioHalfOf = useCallback((itemId: string): string => {
    const c = cfg.current
    const it = c.allItems.find((i) => i.id === itemId)
    if (!it || it.kind !== 'video') return itemId
    const sibs = linkedSiblings(it, c.allItems)
    return sibs.length === 1 ? sibs[0].id : itemId
  }, [cfg])

  const muteItem = useCallback((itemId: string) => {
    const target = audioHalfOf(itemId)
    void runUserVerb('edit.gain', { clip: target, db: MUTE_DB, rationale: `mute clip ${target} (context menu)` }, `Could not mute clip ${target}.`)
  }, [audioHalfOf])

  const cleanVoiceItem = useCallback((itemId: string) => {
    const target = audioHalfOf(itemId)
    void runUserVerb('audio.cleanup_voice', { clip: target, rationale: `clean voice on ${target} (context menu)` }, `Could not clean the voice on clip ${target}.`)
  }, [audioHalfOf])

  const blurFacesItem = useCallback((itemId: string, atMs: number) => {
    const c = cfg.current
    const it = c.allItems.find((i) => i.id === itemId)
    if (!it || it.kind !== 'video') return
    const within = atMs > it.startMs && atMs < it.startMs + it.durMs
    const frameMs = within ? atMs : c.playheadMs
    const localMs = Math.max(0, Math.round(frameMs - it.startMs))
    void runUserVerb('edit.redact', {
      clip: it.id,
      faces: true,
      mode: 'blur',
      at_ms: localMs,
      rationale: `auto-blur faces in ${it.id} @ ${timecode(frameMs)} (context menu)`,
    }, `Could not blur faces in clip ${it.id}.`)
  }, [cfg])

  const syncByAudio = useCallback(async () => {
    const c = cfg.current
    const sel = c.allItems.filter(
      (i) => c.selectedClipIds.includes(i.id) && (i.kind === 'video' || i.kind === 'audio'),
    )
    if (sel.length < 2) { setSyncNote('Select 2+ media clips to sync.'); window.setTimeout(() => setSyncNote(null), 5000); return }
    setSyncNote('Measuring audio sync…')
    const r = await callVerb('edit.multicam_sync', {
      clips: sel.map((i) => i.id),
      rationale: `user: sync ${sel.length} clips by audio`,
    })
    if (!r.ok || !r.result) {
      setSyncNote(r.error?.message ?? 'could not measure audio sync')
      window.setTimeout(() => setSyncNote(null), 6000)
      return
    }
    const offsets = (r.result as { offsets?: { clip: string; offset_ms: number; score?: number; reference?: boolean }[] }).offsets ?? []
    const byId = new Map(sel.map((i) => [i.id, i]))
    let moved = 0, weak = 0
    for (const o of offsets) {
      if (o.reference || !o.offset_ms) continue
      const it = byId.get(o.clip)
      if (!it) continue
      if ((o.score ?? 1) < 0.3) weak++
      // edit.move at_ms is EDITORIAL time — shift the clip's editorial start
      // by the measured offset (laid startMs understates the position after
      // an upstream crossfade).
      const at = Math.max(0, Math.round(it.editorialStartMs - o.offset_ms))
      await callVerb('edit.move', {
        clip: o.clip,
        to_track: it.trackId,
        at_ms: at,
        ripple: false,
        rationale: `user: align ${o.clip} by audio (${o.offset_ms > 0 ? '+' : ''}${o.offset_ms}ms)`,
      })
      moved++
    }
    setSyncNote(moved
      ? `Synced ${sel.length} clips by audio — aligned ${moved}${weak ? ` (${weak} weak match)` : ''}.`
      : 'Already aligned (no shift needed).')
    window.setTimeout(() => setSyncNote(null), 6000)
  }, [cfg])

  const detachAudioItem = useCallback(async (itemId: string) => {
    const c = cfg.current
    const it = c.allItems.find((i) => i.id === itemId)
    if (!it || it.kind !== 'video') return
    setSyncNote('Detaching audio…')
    const r = await callVerb('edit.detach_audio', { clip: itemId, rationale: `detach audio from ${itemId} (context menu)` })
    if (!r.ok) { setSyncNote(r.error?.message ?? 'could not detach audio'); window.setTimeout(() => setSyncNote(null), 6000); return }
    const res = r.result as { detached?: boolean; audio_track?: string; reason?: string } | undefined
    setSyncNote(res?.detached
      ? `Audio detached → ${res.audio_track ?? 'a new audio track'}`
      : `Audio already on its own track${res?.reason ? ` (${res.reason})` : ''}`)
    window.setTimeout(() => setSyncNote(null), 6000)
  }, [cfg])

  const splitEditItem = useCallback(async (itemId: string, kind: 'j' | 'l') => {
    const c = cfg.current
    const it = c.allItems.find((i) => i.id === itemId)
    if (!it || it.kind !== 'video') return
    // DELIBERATELY laid coords: edit.split_edit resolves its cut against EDL
    // segments (app/core/src/split_edit.rs plan_split_edit matches
    // timeline_out_ms/timeline_in_ms), and the EDL is laid/render time — the
    // one cumulative-track edit verb NOT keyed on editorial time. Adjacency
    // here also correctly excludes crossfaded seams (the verb wants butted
    // clips; a crossfaded neighbour's laid start ≠ this clip's laid end).
    const onTrack = c.allItems.filter((i) => i.trackId === it.trackId && i.kind === 'video')
    const endMs = it.startMs + it.durMs
    const endNeighbour = onTrack.find((i) => i.id !== it.id && i.startMs === endMs)
    const startNeighbour = onTrack.find((i) => i.id !== it.id && i.startMs + i.durMs === it.startMs)
    const atMs = endNeighbour ? endMs : startNeighbour ? it.startMs : null
    if (atMs === null) { setSyncNote('Need an adjacent video cut for a J/L edit.'); window.setTimeout(() => setSyncNote(null), 6000); return }
    setSyncNote(`Rolling ${kind.toUpperCase()}-cut audio…`)
    const r = await callVerb('edit.split_edit', { at_ms: Math.round(atMs), kind, offset_ms: 200, video_track: it.trackId, rationale: `${kind}-cut at ${timecode(atMs)} (context menu)` })
    setSyncNote(r.ok ? `${kind.toUpperCase()}-cut applied — audio rolled 200ms` : (r.error?.message ?? 'could not apply split edit'))
    window.setTimeout(() => setSyncNote(null), 7000)
  }, [cfg])

  const replaceClipSource = useCallback((itemId: string, asset: string) => {
    void runUserVerb('edit.replace', {
      target_clip: itemId,
      asset,
      rationale: `replace ${itemId} source with ${asset} (context menu)`,
    }, `Could not replace clip ${itemId}.`)
  }, [])

  const fitToFillAdjacent = useCallback(async (itemId: string, asset: string) => {
    const c = cfg.current
    const it = c.allItems.find((i) => i.id === itemId)
    if (!it) return
    const slot = adjacentGapSlot(it, c.allItems)
    if (!slot) { setSyncNote('No adjacent gap to fill — delete a clip to open one, or use Replace.'); window.setTimeout(() => setSyncNote(null), 6000); return }
    setSyncNote('Fitting footage to the gap…')
    const r = await callVerb('edit.fit_to_fill', {
      track: slot.track,
      at_ms: slot.at_ms,
      duration_ms: slot.duration_ms,
      asset,
      rationale: `fit ${asset} to the ${slot.duration_ms}ms gap @ ${timecode(slot.at_ms)} (context menu)`,
    })
    if (!r.ok) { setSyncNote(r.error?.message ?? 'could not fit footage to the gap'); window.setTimeout(() => setSyncNote(null), 7000); return }
    const speed = (r.result as { speed?: number } | undefined)?.speed
    setSyncNote(speed ? `Filled the gap — speed-fit ${speed.toFixed(2)}×` : 'Filled the gap (speed-fit)')
    window.setTimeout(() => setSyncNote(null), 6000)
  }, [cfg])

  const nestSelection = useCallback(async () => {
    const c = cfg.current
    const sel = c.allItems.filter((i) => c.selectedClipIds.includes(i.id) && (i.kind === 'video' || i.kind === 'audio'))
    if (!isContiguousRun(sel, c.allItems)) {
      setSyncNote('Select 2+ adjacent clips on ONE track to nest.')
      window.setTimeout(() => setSyncNote(null), 6000)
      return
    }
    setSyncNote('Nesting selection…')
    const r = await callVerb('edit.nest', {
      clips: sel.map((i) => i.id),
      rationale: `nest ${sel.length} clips on ${sel[0].trackId} (context menu)`,
    })
    if (!r.ok) { setSyncNote(r.error?.message ?? 'could not nest the selection'); window.setTimeout(() => setSyncNote(null), 7000); return }
    const res = r.result as { clip_id?: string; nest_id?: string } | undefined
    if (res?.clip_id) c.onSelect([res.clip_id])
    setSyncNote(`Nested ${sel.length} clips → ${res?.nest_id ?? 'a nest'}`)
    window.setTimeout(() => setSyncNote(null), 6000)
  }, [cfg])

  const cutToBeat = useCallback(async () => {
    const c = cfg.current
    const hasBeats = (c.markers ?? []).some((m) => m.label === 'beat')
    if (!hasBeats) { setSyncNote('Add a music bed with beat markers first (audio.add_music).'); window.setTimeout(() => setSyncNote(null), 6000); return }
    setSyncNote('Cutting to beat…')
    const r = await callVerb('edit.cut_to_beat', { mode: 'split', rationale: 'user: cut to beat' })
    if (!r.ok) { setSyncNote(r.error?.message ?? 'could not cut to beat'); window.setTimeout(() => setSyncNote(null), 7000); return }
    const res = r.result as { cuts?: number[]; beats_used?: number } | undefined
    const n = res?.cuts?.length ?? 0
    setSyncNote(n > 0 ? `Cut to beat: ${n} cut${n === 1 ? '' : 's'} on ${res?.beats_used ?? 0} beats` : 'No new cuts (already on the beats / no clips in range)')
    window.setTimeout(() => setSyncNote(null), 7000)
  }, [cfg])

  const multicamSwitch = useCallback(async () => {
    setSyncNote('Auto-switching multicam…')
    const r = await callVerb('edit.multicam_switch', { rationale: 'user: auto multicam switch' })
    if (!r.ok) { setSyncNote(r.error?.message ?? 'could not auto-switch'); window.setTimeout(() => setSyncNote(null), 7000); return }
    const res = r.result as { switches?: number; shots?: unknown[] } | undefined
    const shots = res?.shots?.length ?? 0
    const sw = res?.switches ?? 0
    setSyncNote(`Multicam: ${shots} shot${shots === 1 ? '' : 's'}, ${sw} switch${sw === 1 ? '' : 'es'} → a program track`)
    window.setTimeout(() => setSyncNote(null), 7000)
  }, [])

  const applySpeed = useCallback((factor: number) => {
    const c = cfg.current
    const sel = c.allItems.filter(
      (i) => c.selectedClipIds.includes(i.id) && (i.kind === 'video' || i.kind === 'audio'),
    )
    if (!sel.length) return
    sel.forEach((i) => void runUserVerb('edit.speed', {
      clip: i.id,
      factor,
      rationale: factor === 1
        ? `user reset ${i.id} to normal speed`
        : `user set ${i.id} speed to ${factor}× @ ${timecode(i.startMs)}`,
    }, `Could not change the speed of clip ${i.id}.`))
  }, [cfg])

  const applyCrossfade = useCallback((seam: Seam, durationMs: number, transition = 'dissolve') => {
    void runUserVerb('edit.crossfade', {
      track: seam.trackId,
      // Seam.atMs is the EDITORIAL boundary (edit.crossfade's key). The laid
      // coordinate lives in seam.laidMs and is for drawing only — dispatching
      // it was the harness-caught P1 (not_found on any seam downstream of an
      // existing crossfade).
      at_ms: seam.atMs,
      duration_ms: durationMs,
      ...(durationMs > 0 ? { transition } : {}),
      rationale:
        durationMs > 0
          ? `user crossfade: ${seam.leftId}→${seam.rightId} ${shortDur(durationMs)} ${transition} @ ${timecode(seam.atMs)}`
          : `user crossfade: clear ${seam.leftId}→${seam.rightId} @ ${timecode(seam.atMs)} (back to hard cut)`,
    }, 'Could not update the crossfade.')
    setActiveSeam(null)
  }, [setActiveSeam])

  return {
    syncNote,
    showVerbFailure,
    addTrack,
    rippleTrimAtPlayhead,
    cleanupEmptyTracks,
    deleteSelection,
    removeItemById,
    removeTrackById,
    splitItemAt,
    splitAtPlayhead,
    fadeItem,
    trimItemTo,
    reverseItem,
    freezeItem,
    stabilizeItem,
    speedItem,
    crossfadeAdjacent,
    muteItem,
    cleanVoiceItem,
    blurFacesItem,
    syncByAudio,
    detachAudioItem,
    splitEditItem,
    replaceClipSource,
    fitToFillAdjacent,
    nestSelection,
    cutToBeat,
    multicamSwitch,
    applySpeed,
    applyCrossfade,
  }
}
