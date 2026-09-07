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
  /** Create an overlay video destination inside an atomic linked placement.
   * Ignored for audio-only or still-image insertions. */
  newVideoTrack?: boolean
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

/** Editable video/audio destinations offered by the compact Source Monitor
 * overwrite control. Locked tracks stay inspectable in the timeline but are
 * never offered as a mutation destination. */
export interface SourceOverwriteTrackTargets {
  video: string[]
  audio: string[]
}

/** Resolve the exact existing targets for a placement before converting a
 * rendered (laid) timeline coordinate. A linked A/V placement must compare
 * both destination clocks; defaulting later inside `placeLinkedAV` would make
 * a Source Monitor or drop gesture choose one track's ambiguous crossfade. */
export function resolvePlacementTargets(project: Project | null, opts: Pick<PlaceOptions, 'asset' | 'kind' | 'videoTrack' | 'audioTrack' | 'newVideoTrack' | 'newAudioTrack'>): {
  videoTrack?: string
  audioTrack?: string
} {
  if (opts.kind === 'audio') {
    return { audioTrack: opts.audioTrack ?? firstTrackId(project, 'audio') ?? undefined }
  }
  const videoTrack = opts.videoTrack ?? (opts.newVideoTrack ? undefined : firstTrackId(project, 'video') ?? undefined)
  const audioTrack = opts.kind === 'video' && assetHasAudio(project, opts.asset)
    ? opts.audioTrack ?? (opts.newAudioTrack ? undefined : firstTrackId(project, 'audio') ?? undefined)
    : undefined
  return { videoTrack, audioTrack }
}

export function sourceOverwriteTrackTargets(project: Project | null): SourceOverwriteTrackTargets {
  const targets = (kind: 'video' | 'audio') => project?.tracks
    .filter((track) => track.kind === kind && !track.locked)
    .map((track) => track.id) ?? []
  return { video: targets('video'), audio: targets('audio') }
}

export interface OverwriteSourceRangeOptions {
  asset: string
  atMs: number
  sourceRangeMs: [number, number]
  videoTrack?: string | null
  audioTrack?: string | null
  rationale?: string
}

/** Source Monitor keeps still-image edits practical and bounded: a tenth of a
 * second is the shortest useful visual hold, while one hour avoids an
 * accidental effectively-unbounded timeline replacement. */
export const STILL_OVERWRITE_MIN_DURATION_MS = 100
export const STILL_OVERWRITE_MAX_DURATION_MS = 3_600_000
export const STILL_OVERWRITE_DEFAULT_DURATION_MS = 3_000

export interface OverwriteStillOptions {
  asset: string
  atMs: number
  videoTrack?: string | null
  durationMs: number
  rationale?: string
}

/** A still has no source clock. It can overwrite an unlocked video destination
 * only, so omit timed-media range and audio arguments entirely. */
export async function overwriteSourceStill(opts: OverwriteStillOptions) {
  if (!opts.videoTrack) {
    return {
      ok: false,
      error: { code: 'invalid_args', message: 'Choose a video destination before overwriting.' },
    }
  }
  const durationMs = Math.round(opts.durationMs)
  if (!Number.isFinite(durationMs) || durationMs < STILL_OVERWRITE_MIN_DURATION_MS || durationMs > STILL_OVERWRITE_MAX_DURATION_MS) {
    return {
      ok: false,
      error: { code: 'invalid_args', message: 'Set a still duration from 0.1 to 3,600 seconds before overwriting.' },
    }
  }
  const args: VerbArgs['edit.overwrite'] = {
    asset: opts.asset,
    at_ms: Math.max(0, Math.round(opts.atMs)),
    video_track: opts.videoTrack,
    duration_ms: durationMs,
    rationale: opts.rationale,
  }
  return callVerb('edit.overwrite', args)
}

/** One atomic source-to-timeline overwrite. The engine receives whichever V/A
 * destinations are selected; it owns linked A/V atomicity and range semantics. */
export async function overwriteSourceRange(opts: OverwriteSourceRangeOptions) {
  if (!opts.videoTrack && !opts.audioTrack) {
    return {
      ok: false,
      error: { code: 'invalid_args', message: 'Choose a V or A destination before overwriting.' },
    }
  }
  const args: VerbArgs['edit.overwrite'] = {
    asset: opts.asset,
    at_ms: Math.max(0, Math.round(opts.atMs)),
    src_range_ms: opts.sourceRangeMs,
    rationale: opts.rationale,
  }
  if (opts.videoTrack) args.video_track = opts.videoTrack
  if (opts.audioTrack) args.audio_track = opts.audioTrack
  return callVerb('edit.overwrite', args)
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
  /** Probe fact supplied by the current project snapshot. Only muxed video
   * needs a second linked destination and therefore atomic track creation. */
  hasAudio?: boolean
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
    if (kind === 'video' && opts.hasAudio) {
      return {
        asset: opts.asset,
        kind,
        at_ms: opts.at_ms,
        ripple: false,
        newVideoTrack: true,
        newAudioTrack: true,
        rationale: `place ${opts.asset} on new linked overlay tracks at ${fmtS(opts.at_ms)}s`,
      }
    }
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
        newAudioTrack: kind === 'video' && opts.hasAudio,
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
 * Muxed video creates any missing destinations and both media clips through
 * one engine transaction. Single-leg image and audio placement retain their
 * existing direct edit.insert behavior.
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

  // Muxed video is a single engine transaction. It validates BOTH target legs
  // (and creates either destination if requested) against one staged project;
  // an audio-leg failure cannot leave a visible video clip or orphaned track.
  if (opts.kind === 'video' && assetHasAudio(project, opts.asset)) {
    const targets = resolvePlacementTargets(project, opts)
    const createVideoTrack = opts.newVideoTrack || !targets.videoTrack
    const createAudioTrack = opts.newAudioTrack || !targets.audioTrack
    const r = await callVerb('edit.insert_linked', {
      asset: opts.asset,
      at_ms: opts.at_ms,
      ...(targets.videoTrack ? { video_track: targets.videoTrack } : {}),
      ...(targets.audioTrack ? { audio_track: targets.audioTrack } : {}),
      ...(createVideoTrack ? { create_video_track: true } : {}),
      ...(createAudioTrack ? { create_audio_track: true } : {}),
      ...(opts.src_range_ms ? { src_range_ms: opts.src_range_ms } : {}),
      ripple: opts.ripple ?? true,
      rationale,
    })
    if (!r.ok) {
      return {
        ok: false,
        videoOk: false,
        audioLinked: false,
        videoTrack: targets.videoTrack,
        audioTrack: targets.audioTrack,
        error: errOf(r),
      }
    }
    const receipt = r.result
    if (!receipt) {
      return {
        ok: false,
        videoOk: false,
        audioLinked: false,
        videoTrack: targets.videoTrack,
        audioTrack: targets.audioTrack,
        error: 'The linked insertion committed without a receipt. Refresh the project before making another edit.',
      }
    }
    return {
      ok: true,
      videoOk: true,
      audioLinked: true,
      videoTrack: receipt.video_track,
      audioTrack: receipt.audio_track,
    }
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

  return { ok: true, videoOk: true, audioLinked: false, videoTrack: vTrack }
}

function errOf(r: VerbResult): string | undefined {
  return r.ok ? undefined : (r.error?.message ?? r.error?.code ?? 'error')
}
