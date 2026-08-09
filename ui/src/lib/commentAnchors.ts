import type { Clip, Comment, CommentAnchor, Project, Track } from './clientModel'

export type CommentAnchorStatus = 'anchored' | 'absolute' | 'stale'

export interface ResolvedCommentTime {
  atMs: number
  endMs?: number
  status: CommentAnchorStatus
}

interface ClipSpan {
  clipId: string
  startMs: number
  durationMs: number
}

function clipId(clip: Clip): string | null {
  return 'id' in clip ? clip.id : null
}

function clipDurationMs(clip: Clip): number {
  if ('duration_ms' in clip && clip.kind === 'gap') return Math.max(0, clip.duration_ms)
  if ('range_ms' in clip) return Math.max(0, clip.range_ms[1] - clip.range_ms[0])
  if ('src_in_ms' in clip && 'src_out_ms' in clip) {
    const raw = Math.max(0, clip.src_out_ms - clip.src_in_ms)
    const speed = typeof clip.speed === 'number' && Number.isFinite(clip.speed) && clip.speed > 0 ? clip.speed : 1
    return speed !== 1 ? Math.round(raw / speed) : raw
  }
  return 0
}

function trackSpans(track: Track): ClipSpan[] {
  if (track.kind === 'caption') {
    return (track.clips ?? []).flatMap((clip) => {
      if (!('range_ms' in clip)) return []
      const id = clipId(clip)
      if (!id) return []
      return [{ clipId: id, startMs: clip.range_ms[0], durationMs: Math.max(0, clip.range_ms[1] - clip.range_ms[0]) }]
    })
  }

  const spans: ClipSpan[] = []
  let cursor = 0
  let prevMediaDur: number | null = null
  for (const clip of track.clips ?? []) {
    const dur = clipDurationMs(clip)
    const xfade = prevMediaDur != null && 'src_in_ms' in clip
      ? Math.min(prevMediaDur, dur, Math.max(0, clip.xfade_in_ms ?? 0))
      : 0
    const start = Math.max(0, cursor - xfade)
    const id = clipId(clip)
    if (id) spans.push({ clipId: id, startMs: start, durationMs: dur })
    cursor = start + dur
    prevMediaDur = 'src_in_ms' in clip ? dur : null
  }
  return spans
}

function findAnchorSpan(project: Project | null, anchor: CommentAnchor): ClipSpan | null {
  if (!project) return null
  const primary = project.tracks?.find((track) => track.id === anchor.track_id)
  const primarySpan = primary ? trackSpans(primary).find((span) => span.clipId === anchor.clip_id) : undefined
  if (primarySpan) return primarySpan
  for (const track of project.tracks ?? []) {
    if (track.id === anchor.track_id) continue
    const span = trackSpans(track).find((candidate) => candidate.clipId === anchor.clip_id)
    if (span) return span
  }
  return null
}

export function resolveCommentTime(project: Project | null, comment: Comment): ResolvedCommentTime {
  const rangeLen = comment.end_ms != null ? Math.max(0, comment.end_ms - comment.at_ms) : undefined
  const anchor = comment.anchor ?? null
  if (!anchor) {
    return {
      atMs: comment.at_ms,
      ...(rangeLen != null ? { endMs: comment.at_ms + rangeLen } : {}),
      status: 'absolute',
    }
  }

  const span = findAnchorSpan(project, anchor)
  if (!span) {
    return {
      atMs: comment.at_ms,
      ...(rangeLen != null ? { endMs: comment.at_ms + rangeLen } : {}),
      status: 'stale',
    }
  }

  const offset = Math.min(Math.max(0, anchor.offset_ms), span.durationMs)
  const atMs = span.startMs + offset
  return {
    atMs,
    ...(rangeLen != null ? { endMs: atMs + rangeLen } : {}),
    status: anchor.offset_ms <= span.durationMs ? 'anchored' : 'stale',
  }
}
