import type { ActiveVideo } from './model'

/** Shared whole-millisecond playhead representation used by App and Preview. */
export function normalizePlayheadMs(positionMs: number): number {
  return Math.max(0, Math.round(positionMs))
}

/** Wait for enough media data before attempting or marking startup complete. */
export function canStartPreviewMedia(readyState: number): boolean {
  return readyState >= 2
}

/** Keep the normal cadence, but enter the next clip as soon as its start is
 * crossed. Waiting for another 100 ms tick in a gap postpones its parked video. */
export function shouldPublishPlaybackTick(now: number, lastDispatch: number, positionMs: number, rate: number, nextVideoStartMs: number | null): boolean {
  return now - lastDispatch >= 100 || (rate === 1 && nextVideoStartMs !== null && positionMs >= nextVideoStartMs)
}

/** Distinguish the clock's asynchronous React updates from a user seek. */
export class PlaybackSeekTracker {
  private observed: number
  private pending: number[] = []
  constructor(positionMs: number, private revision = 0) { this.observed = normalizePlayheadMs(positionMs) }
  published(positionMs: number) {
    const normalized = normalizePlayheadMs(positionMs)
    this.pending.push(normalized)
    if (this.pending.length > 32) this.pending.shift()
    return normalized
  }
  external(positionMs: number, revision = this.revision): boolean {
    positionMs = normalizePlayheadMs(positionMs)
    if (revision !== this.revision) {
      this.revision = revision
      this.observed = positionMs
      this.pending = []
      return true
    }
    if (positionMs === this.observed) return false
    this.observed = positionMs
    const index = this.pending.lastIndexOf(positionMs)
    if (index >= 0) {
      this.pending.splice(0, index + 1)
      return false
    }
    this.pending = []
    return true
  }
}

/** A boundary publication waits for a clip change, but a seek back into the
 * same clip cancels that handoff and must restart its media clock. */
export class PlaybackHandoff {
  private waiting = false
  constructor(private readonly tracker: PlaybackSeekTracker) {}
  begin(): boolean {
    if (this.waiting) return false
    this.waiting = true
    return true
  }
  complete() { this.waiting = false }
  observe(positionMs: number, revision?: number): 'seek' | 'wait' | 'continue' {
    if (this.tracker.external(positionMs, revision)) {
      this.waiting = false
      return 'seek'
    }
    return this.waiting ? 'wait' : 'continue'
  }
}

export function playbackVideoKey(video: ActiveVideo | null): string | null {
  return video ? `${video.clipId}:${video.src}:${video.startMs}:${video.srcInMs}:${video.srcOutMs}:${video.speed}` : null
}

/** Ordinary pieces with the same source-to-timeline offset share a decoder
 * and clock. Other speeds retain their existing per-placement clock. */
export function playbackBindingKey(video: ActiveVideo | null, slot: number | null): string | null {
  if (!video || slot === null) return null
  return video.speed === 1
    ? `${slot}:${video.src}:${video.startMs - video.srcInMs}`
    : `${slot}:${playbackVideoKey(video)}`
}

/** A frame belongs to the current seek epoch and clip, and is near the
 * element's actual clock. The latter rejects queued frames from an older seek. */
export function acceptPlaybackFrame(
  expectedKey: string | null, video: ActiveVideo | null, mediaMs: number,
  elementMs: number, ready: boolean, seeking: boolean, frameMs: number,
  sourceOutMs = video?.srcOutMs,
): boolean {
  if (!ready || seeking || !video || playbackVideoKey(video) !== expectedKey) return false
  if (!Number.isFinite(mediaMs) || !Number.isFinite(elementMs)) return false
  const tolerance = Math.max(250, frameMs * 2) * Math.max(1, video.speed)
  return mediaMs >= video.srcInMs - tolerance && mediaMs <= (sourceOutMs ?? video.srcOutMs) + tolerance
    && Math.abs(mediaMs - elementMs) <= tolerance
}

/** Forward media callbacks may describe a decoded frame just before a cut or
 * arrive out of order. Never move the timeline behind this seek epoch. */
export function forwardPlaybackPosition(positionMs: number, previousMs: number, clipStartMs: number): number {
  return Math.max(positionMs, previousMs, clipStartMs)
}

/** Longest time the clock waits for a started or playing media element to
 * advance before the timeline continues on its own. */
export const MEDIA_STALL_MS = 600
/** How far a media position may run ahead of the elapsed timeline time. */
export const MEDIA_LEAD_MS = 500
/** How close a recovered media position must be to the timeline to lead again. */
export const MEDIA_REJOIN_MS = 200
/** Minimum time between attempts to move a lagging element to the timeline. */
export const MEDIA_RESYNC_INTERVAL_MS = 1000
/** An element that already stands this close to the start position is not
 * seeked when its clock starts; it plays from where it stands. Two cases:
 * - parked: paused and a little BEHIND (the base deck parked the next clip on
 *   its first frame and the playhead crossed the cut a few ms later); the
 *   timeline waits those ms, it never rewinds;
 * - in place: already playing across a cut inside one recording, a few ms
 *   past the cut; the timeline takes its position.
 * One more seek at that moment is exactly what some webviews (WebKitGTK) fail
 * to complete, which held the playhead and froze or blanked the picture. */
export const MEDIA_START_IN_PLACE_MS = 250

/** What this webview's media elements have shown about their timing, learned
 * from behaviour rather than guessed from the user agent. `frameCallbacks`:
 * requestVideoFrameCallback has delivered at least one presented frame
 * (WebView2 and WKWebView do; WebKitGTK exposes the API but delivers none, and
 * there a seek on a playing element is also the operation that most often
 * never completes). */
export const mediaTimingCapability = { frameCallbacks: false }

/** Drift (s) at which a PLAYING overlay element is moved back to the
 * timeline: 60 ms where frame callbacks prove precise media timing, else
 * MEDIA_START_IN_PLACE_MS, so small drift does not cost a fragile seek. */
export function overlayResyncDriftS(): number {
  return mediaTimingCapability.frameCallbacks ? 0.06 : MEDIA_START_IN_PLACE_MS / 1000
}

/** Whether a starting clock can use the element where it stands (see
 * MEDIA_START_IN_PLACE_MS). `behindMs` = target - element position. */
export function startsInPlace(paused: boolean, seeking: boolean, readyState: number, behindMs: number, speed: number): boolean {
  if (seeking || readyState < 2) return false
  const limit = MEDIA_START_IN_PLACE_MS * Math.max(1, speed)
  return paused ? behindMs >= 0 && behindMs <= limit : Math.abs(behindMs) <= limit
}

/** The timeline position during forward playback of a media element.
 *
 * While the element advances normally, its presented time leads and the
 * timeline follows it, never backwards. If the element stops advancing (a
 * start or seek that never completes, a stall, a decoder error) for longer
 * than MEDIA_STALL_MS, or reports a position further ahead than the elapsed
 * time allows, the timeline continues on elapsed time instead and the element
 * is asked to resynchronize. It leads again once it is advancing close to the
 * timeline. The timeline therefore never waits indefinitely, never rewinds and
 * never jumps ahead because of the element. */
export const PLAYBACK_PUBLISH_CADENCE_MS = 32

/** Continuous wall-clock transport clock for an active playback session.
 *
 * Cuts, gaps, asset swaps, and slow decoders never stop or rewind the timeline
 * clock. Only rate changes and explicit seeks (`seekRevision`) re-anchor it. */
export class PlaybackSessionClock {
  private anchorMs: number
  private anchorAt: number
  private currentRate: number
  private seekRevision: number
  private lastPublishedMs = Number.NEGATIVE_INFINITY
  private lastDispatchAt = Number.NEGATIVE_INFINITY
  constructor(positionMs: number, now: number, rate = 1, seekRevision = 0) {
    this.anchorMs = positionMs
    this.anchorAt = now
    this.currentRate = rate
    this.seekRevision = seekRevision
  }
  /** Live timeline position at `now`. */
  position(now: number): number {
    return this.anchorMs + Math.max(0, now - this.anchorAt) * this.currentRate
  }
  /** Re-anchor immediately when an explicit seek or rate change commits. */
  reanchor(positionMs: number, now: number, rate = this.currentRate, seekRevision = this.seekRevision) {
    this.anchorMs = positionMs
    this.anchorAt = now
    this.currentRate = rate
    this.seekRevision = seekRevision
    this.lastPublishedMs = normalizePlayheadMs(positionMs)
    this.lastDispatchAt = now
  }
  /** Idempotently align to a new `seekRevision`, returning the live position. */
  syncExternal(positionMs: number, seekRevision: number, now: number, rate = this.currentRate): number {
    if (seekRevision !== this.seekRevision || rate !== this.currentRate) {
      this.reanchor(positionMs, now, rate, seekRevision)
      return positionMs
    }
    return this.position(now)
  }
  /** Publish on boundary crossings or cadence, skipping duplicate whole-ms ticks. */
  shouldPublish(now: number, positionMs: number, boundaryMs: number | null, cadenceMs = PLAYBACK_PUBLISH_CADENCE_MS): boolean {
    const normalized = normalizePlayheadMs(positionMs)
    if (normalized === this.lastPublishedMs) return false
    if (boundaryMs !== null) {
      if (this.currentRate > 0 && this.lastPublishedMs < boundaryMs && normalized >= boundaryMs) return true
      if (this.currentRate < 0 && this.lastPublishedMs > boundaryMs && normalized <= boundaryMs) return true
    }
    return now - this.lastDispatchAt >= cadenceMs
  }
  markPublished(now: number, positionMs: number) {
    this.lastPublishedMs = normalizePlayheadMs(positionMs)
    this.lastDispatchAt = now
  }
}

export function nextPlaybackBoundaryMs(
  rate: number,
  video: Pick<ActiveVideo, 'startMs' | 'durMs'> | null,
  nextVideoStartMs: number | null,
): number | null {
  if (rate > 0) return video ? video.startMs + video.durMs : nextVideoStartMs
  if (rate < 0) return video ? video.startMs : null
  return null
}

/** Tracks whether an active media element is advancing close to the live clock
 * so stalls, errors, or unrequested jumps trigger seek/reload recovery while the
 * session clock keeps moving uninterrupted. */
export class MediaProgressMonitor {
  private mode: 'tracking' | 'resyncing' = 'tracking'
  private progressAt: number
  private mediaMs = -Infinity
  private resyncAt = -Infinity
  constructor(now: number) {
    this.progressAt = now
  }
  get following(): 'media' | 'timeline' { return this.mode === 'tracking' ? 'media' : 'timeline' }
  get lastProgressAt(): number { return this.progressAt }
  restart(now: number) {
    this.mode = 'tracking'
    this.progressAt = now
    this.resyncAt = -Infinity
    this.awaitMedia()
  }
  awaitMedia() {
    this.mediaMs = -Infinity
  }
  sample(timelineMs: number, clockMs: number, now: number, seeking: boolean, paused: boolean) {
    if (timelineMs > this.mediaMs) {
      if (this.mediaMs !== -Infinity) this.progressAt = now
      this.mediaMs = timelineMs
    }
    const drifted = timelineMs - clockMs > MEDIA_LEAD_MS || (!seeking && !paused && clockMs - timelineMs > MEDIA_LEAD_MS)
    if (drifted) {
      this.mode = 'resyncing'
      return
    }
    if (this.mode === 'resyncing' && now - this.progressAt <= MEDIA_STALL_MS && Math.abs(timelineMs - clockMs) <= MEDIA_REJOIN_MS) {
      this.mode = 'tracking'
    }
  }
  resyncDue(now: number, hasError = false): boolean {
    if (hasError || (this.mode === 'tracking' && now - this.progressAt > MEDIA_STALL_MS)) {
      this.mode = 'resyncing'
    }
    if (this.mode !== 'resyncing' || now - this.resyncAt < MEDIA_RESYNC_INTERVAL_MS) return false
    this.resyncAt = now
    return true
  }
}

/** @deprecated Kept for direct unit compatibility; Preview uses PlaybackSessionClock + MediaProgressMonitor. */
export class TimelineMediaClock {
  private mode: 'media' | 'timeline' = 'media'
  private positionMs: number
  private anchorAt: number
  private progressAt: number
  private mediaMs = -Infinity
  private resyncAt = -Infinity
  constructor(positionMs: number, now: number) {
    this.positionMs = positionMs
    this.anchorAt = now
    this.progressAt = now
  }
  get position(): number { return this.positionMs }
  get following(): 'media' | 'timeline' { return this.mode }
  /** When the element was last seen advancing (or the clock last restarted). */
  get lastProgressAt(): number { return this.progressAt }
  /** An explicit seek moves the timeline and gives the element a fresh start. */
  restart(positionMs: number, now: number) {
    this.mode = 'media'
    this.positionMs = positionMs
    this.anchorAt = now
    this.progressAt = now
    this.awaitMedia()
  }
  /** A start or seek was issued to the element: its next sample counts as
   * progress. The stall allowance is not extended, so a slow load and a slow
   * start together still wait at most MEDIA_STALL_MS. */
  awaitMedia() {
    this.mediaMs = -Infinity
  }
  /** An accepted media sample, already mapped to a timeline position. Progress
   * means a position beyond the previous sample: a single sample after a seek
   * only reports where the element stands, not that it is playing. */
  media(timelineMs: number, now: number): number {
    if (timelineMs > this.mediaMs) {
      if (this.mediaMs !== -Infinity) this.progressAt = now
      this.mediaMs = timelineMs
    }
    if (this.mode === 'media') {
      if (timelineMs > this.positionMs + (now - this.anchorAt) + MEDIA_LEAD_MS) {
        this.followTimeline(now)
        return this.positionMs
      }
      if (timelineMs > this.positionMs) {
        this.positionMs = timelineMs
        this.anchorAt = now
      }
      return this.positionMs
    }
    const clock = this.tick(now)
    if (now - this.progressAt <= MEDIA_STALL_MS && Math.abs(timelineMs - clock) <= MEDIA_REJOIN_MS) {
      this.mode = 'media'
      if (timelineMs > this.positionMs) this.positionMs = timelineMs
    }
    return this.positionMs
  }
  /** Advance without a media sample (called on every animation frame). */
  tick(now: number): number {
    if (this.mode === 'media' && now - this.progressAt > MEDIA_STALL_MS) this.followTimeline(now)
    if (this.mode === 'timeline') {
      this.positionMs += Math.max(0, now - this.anchorAt)
      this.anchorAt = now
    }
    return this.positionMs
  }
  /** True (at most once per interval) while the element should be moved to the timeline. */
  resyncDue(now: number): boolean {
    if (this.mode !== 'timeline' || now - this.resyncAt < MEDIA_RESYNC_INTERVAL_MS) return false
    this.resyncAt = now
    return true
  }
  private followTimeline(now: number) {
    this.mode = 'timeline'
    this.anchorAt = now
  }
}
