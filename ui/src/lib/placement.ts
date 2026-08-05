// lib/placement.ts — how a media asset becomes timeline clips.
//
// Role: the SINGLE place that decides which track(s) an inserted asset lands on,
// shared by every placement path (Assets "Insert" button, post-import auto-place,
// timeline drag-drop). Before this module each path open-coded `edit.insert` onto
// a single video track, which caused two placement regressions:
//
//   1. "second clip has no sound" — the renderer mixes audio from AUDIO tracks
//      only (cut-media build_graph). A video-with-audio clip placed on a VIDEO
//      track alone is silent in both the preview mix and the export. The engine's
//      first-import auto-place already mirrors video→v1 + audio→a1t; every other
//      path dropped the audio. This module makes that LINKED A/V pair the rule.
//
//   2. "second clip instantly becomes an overlay" — the drag path created a new
//      overlay video track for any drop below the lanes, so a normal "add another
//      clip to cut" gesture stacked it as PiP. The DEFAULT is now append to the
//      base track; an overlay is an explicit opt-in (Alt-drop / a chosen lane).
//
// Each call resolves fresh track ids from project.state (no stale closure) and
// emits real verbs (every mutation stays one op in the review rail).

import { callVerb, type Project, type TrackKind, type VerbArgs, type VerbResult } from './client'

type InsertArgs = VerbArgs['edit.insert']

/** First track id of a kind in document order, or null when the project has none. */
function firstTrackId(project: Project | null, kind: 'video' | 'audio'): string | null {
  return project?.tracks?.find((t) => t.kind === kind)?.id ?? null
}

/** Does an asset carry an audio stream? (probe.has_audio — set by media.probe). */
export function assetHasAudio(project: Project | null, assetId: string): boolean {
  const probe = project?.assets?.[assetId]?.probe as { has_audio?: boolean } | undefined
  return !!probe?.has_audio
}

/** Realized timeline end on a track id — the append point, in ms. */
export function trackEndMs(project: Project | null, trackId: string): number {
  const t = project?.tracks?.find((tr) => tr.id === trackId)
  if (!t) return 0
  let cursor = 0
  for (const c of t.clips ?? []) {
    if ('duration_ms' in c && c.kind === 'gap') {
      cursor += c.duration_ms
      continue
    }
    if ('src_out_ms' in c && 'src_in_ms' in c) {
      const raw = Math.max(0, c.src_out_ms - c.src_in_ms)
      const speed = typeof c.speed === 'number' && Number.isFinite(c.speed) && c.speed > 0 ? c.speed : 1
      const dur = speed !== 1 ? Math.round(raw / speed) : raw
      const overlap = Math.max(0, c.xfade_in_ms ?? 0)
      cursor = Math.max(0, cursor - Math.min(overlap, dur)) + dur
    }
  }
  return cursor
}

export interface PlaceOptions {
  asset: string
  /** Probe kind: 'video' | 'audio' | 'image'. */
  kind: string
  at_ms: number
  /** Ripple downstream on insert (open a gap, keep AV in sync mid-timeline). */
  ripple?: boolean
  /** Image-only clip length (stills have no intrinsic duration). */
  duration_ms?: number
  /** Timed-media source selection. The same range is applied to both halves of
   * a linked video/audio placement so their source clocks stay aligned. */
  src_range_ms?: [number, number]
  /** Explicit video track (overlay placement); default = base video track. */
  videoTrack?: string
  /** Explicit audio track for the linked audio; default = base audio track. */
  audioTrack?: string
  /** Overlay placement: route the linked audio to its OWN new audio track so a
   *  PiP's sound doesn't clobber the main dialog mix. Ignored if audioTrack set. */
  newAudioTrack?: boolean
  rationale?: string
  /** Pre-fetched project state (skip the round-trip); fetched if omitted. */
  project?: Project | null
}

export interface TimelineDropTarget {
  id: string
  kind: TrackKind
  kindIndex: number
  locked?: boolean
}

export interface PlacementPlan extends PlaceOptions {
  /** Explicit overlay/separate-track placement can ask the caller to create the
   *  track first, then pass that new id back as `videoTrack` or `audioTrack`. */
  createTrackKind?: 'video' | 'audio'
  useCreatedTrackFor?: 'video' | 'audio'
}

/** Result of a placement: the clip(s) the verbs created, for selection/undo UX. */
export interface PlaceResult {
  ok: boolean
  videoOk: boolean
  audioLinked: boolean
  videoTrack?: string
  audioTrack?: string
  error?: string
}

function fmtS(ms: number): string {
  return (Math.max(0, Math.round(ms)) / 1000).toFixed(2)
}

/** Default Assets "Insert" behavior: add material to the base timeline, the
 * same way standard rough cuts are built. Overlay/separate tracks
 * are explicit choices, not the default Insert button behavior. */
export function planAssetInsertAtPlayhead(opts: {
  asset: string
  kind: string
  at_ms: number
  duration_ms?: number
}): PlacementPlan {
  const kind = opts.kind === 'audio' ? 'audio' : opts.kind === 'image' ? 'image' : 'video'
  const plan: PlacementPlan = {
    asset: opts.asset,
    kind,
    at_ms: opts.at_ms,
    ripple: true,
    rationale: `add ${opts.asset} to the base timeline at ${fmtS(opts.at_ms)}s`,
  }
  if (kind === 'image' && opts.duration_ms) plan.duration_ms = opts.duration_ms
  return plan
}

/** Timeline drag/drop behavior:
 *  - base track or empty area = insert into the story/base timeline;
 *  - existing overlay/extra track = place on top without rippling the base;
 *  - Alt-drop = create a new overlay/separate track, then place there.
 */
export function planTimelineAssetDrop(opts: {
  asset: string
  kind: string
  at_ms: number
  duration_ms?: number
  target?: TimelineDropTarget | null
  overlay?: boolean
}): PlacementPlan | null {
  const kind = opts.kind === 'audio' ? 'audio' : opts.kind === 'image' ? 'image' : 'video'
  if (opts.target?.locked) return null
  const base: PlacementPlan = {
    asset: opts.asset,
    kind,
    at_ms: opts.at_ms,
    ripple: true,
    rationale: `drop ${opts.asset} into the base timeline at ${fmtS(opts.at_ms)}s`,
  }
  if (kind === 'image' && opts.duration_ms) base.duration_ms = opts.duration_ms

  if (opts.overlay) {
    const createTrackKind = kind === 'audio' ? 'audio' : 'video'
    const plan: PlacementPlan = {
      asset: opts.asset,
      kind,
      at_ms: opts.at_ms,
      ripple: false,
      createTrackKind,
      useCreatedTrackFor: createTrackKind,
      newAudioTrack: kind !== 'audio',
      rationale: `place ${opts.asset} on a new ${createTrackKind === 'audio' ? 'audio' : 'overlay'} track at ${fmtS(opts.at_ms)}s`,
    }
    if (kind === 'image' && opts.duration_ms) plan.duration_ms = opts.duration_ms
    return plan
  }

  const target = opts.target
  if (target && target.kindIndex > 0) {
    if (kind !== 'audio' && target.kind === 'video') {
      const plan: PlacementPlan = {
        asset: opts.asset,
        kind,
        at_ms: opts.at_ms,
        ripple: false,
        videoTrack: target.id,
        newAudioTrack: true,
        rationale: `place ${opts.asset} on overlay track ${target.id} at ${fmtS(opts.at_ms)}s`,
      }
      if (kind === 'image' && opts.duration_ms) plan.duration_ms = opts.duration_ms
      return plan
    }
    if (kind === 'audio' && target.kind === 'audio') {
      return {
        asset: opts.asset,
        kind,
        at_ms: opts.at_ms,
        ripple: false,
        audioTrack: target.id,
        rationale: `place ${opts.asset} on audio track ${target.id} at ${fmtS(opts.at_ms)}s`,
      }
    }
  }

  return base
}

async function addTrack(kind: 'video' | 'audio', rationale: string): Promise<string | undefined> {
  const r = await callVerb('edit.add_track', { kind, rationale })
  return r.ok ? (r.result as { track_id?: string } | null)?.track_id ?? undefined : undefined
}

/**
 * Place `asset` on the timeline as a LINKED audio/video pair (the fix for the
 * silent-second-clip bug). Mirrors the engine's first-import auto-place:
 *
 *   - kind 'video' WITH audio → insert on the video track AND an audio track.
 *   - kind 'video' without audio, or 'image' → video track only.
 *   - kind 'audio' → audio track only.
 *
 * Creates a base audio track if the project has none (or always, for overlays).
 * Verbs run sequentially so each reads the prior's committed state.
 */
export async function placeLinkedAV(opts: PlaceOptions): Promise<PlaceResult> {
  let project = opts.project ?? null
  if (!project) {
    const sr = await callVerb('project.state', {})
    project = sr.ok ? (sr.result as Project) : null
  }
  const rationale = opts.rationale ?? `place ${opts.asset}`

  // Audio-only asset → straight onto an audio track.
  if (opts.kind === 'audio') {
    const track = opts.audioTrack ?? firstTrackId(project, 'audio') ?? (await addTrack('audio', 'audio for a placed clip')) ?? 'a1t'
    const r = await callVerb('edit.insert', {
      asset: opts.asset, track, at_ms: opts.at_ms, src_range_ms: opts.src_range_ms,
      ripple: opts.ripple ?? true, rationale,
    })
    return { ok: r.ok, videoOk: r.ok, audioLinked: false, audioTrack: track, error: errOf(r) }
  }

  // Video / image → the video track is the primary clip.
  const primaryRipple = opts.ripple ?? true
  const vTrack = opts.videoTrack ?? firstTrackId(project, 'video') ?? 'v1'
  const vArgs: InsertArgs = {
    asset: opts.asset, track: vTrack, at_ms: opts.at_ms,
    src_range_ms: opts.src_range_ms, ripple: primaryRipple, rationale,
  }
  if (opts.kind === 'image' && opts.duration_ms) vArgs.duration_ms = opts.duration_ms
  const vr = await callVerb('edit.insert', vArgs)
  if (!vr.ok) return { ok: false, videoOk: false, audioLinked: false, videoTrack: vTrack, error: errOf(vr) }

  // Linked audio — only for video that actually carries sound.
  let audioLinked = false
  let aTrack = opts.audioTrack
  if (opts.kind === 'video' && assetHasAudio(project, opts.asset)) {
    if (!aTrack) {
      aTrack = opts.newAudioTrack
        ? await addTrack('audio', 'overlay: linked audio on its own track')
        : firstTrackId(project, 'audio') ?? (await addTrack('audio', 'linked audio for a placed clip'))
    }
    if (aTrack) {
      const linkedAudioRipple = false
      const ar = await callVerb('edit.insert', {
        asset: opts.asset, track: aTrack, at_ms: opts.at_ms,
        src_range_ms: opts.src_range_ms, ripple: linkedAudioRipple,
        rationale: `linked audio: ${opts.asset} → ${aTrack}`,
      })
      audioLinked = ar.ok
    }
  }
  return { ok: true, videoOk: true, audioLinked, videoTrack: vTrack, audioTrack: aTrack }
}

function errOf(r: VerbResult): string | undefined {
  return r.ok ? undefined : (r.error?.message ?? r.error?.code ?? 'error')
}
