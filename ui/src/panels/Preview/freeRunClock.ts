import { sourceMsAtTimelinePosition, timelineMsAtSourcePosition } from '../../lib/mediaTime'
import type { ActiveVideo } from './model'
import { continuesInPlace } from './baseDeckPlan'
import {
  acceptPlaybackFrame,
  canStartPreviewMedia,
  MediaProgressMonitor,
  nextPlaybackBoundaryMs,
  PlaybackHandoff,
  PlaybackSessionClock,
  mediaTimingCapability,
  playbackVideoKey,
  playbackBindingKey,
  startsInPlace,
  type PlaybackSeekTracker,
} from './playbackClock'
import { previewPlaybackClockPosition } from './useVoiceoverPlayback'

type VideoFrameCallback = (now: number, meta: { mediaTime: number }) => void

/** The parts of HTMLVideoElement the free-run clock reads and drives. */
export type FreeRunVideo = Pick<HTMLVideoElement,
  'readyState' | 'currentTime' | 'seeking' | 'paused' | 'ended' | 'playbackRate' | 'error'
  | 'play' | 'pause' | 'load' | 'addEventListener' | 'removeEventListener'>

export interface FreeRunFrameCallbacks {
  request: (cb: VideoFrameCallback) => number
  cancel: (handle: number) => void
}

/** Animation-frame clock; injectable so the clock can be driven deterministically. */
export interface FreeRunScheduler {
  now: () => number
  requestFrame: (cb: (now: number) => void) => number
  cancelFrame: (handle: number) => void
}

export interface FreeRunState {
  rate: number
  playheadMs: number
  durationMs: number
  frameMs: number
  seekRevision: number
  video: ActiveVideo | null
  /** End of the uninterrupted source run, before a gap or different mapping. */
  continuousEndMs?: number
  nextVideoStartMs?: number | null
  voiceoverPlayback: { outMs: number | null } | null
}

export interface FreeRunArgs {
  video: FreeRunVideo
  callbacks: FreeRunFrameCallbacks
  /** Initial placement; contiguous pieces retain this clock instance. */
  expectedKey: string | null
  /** Latest rendered state, read on every callback. */
  state: { readonly current: FreeRunState }
  tracker: PlaybackSeekTracker
  /** Publish a playback clock position (not a user seek). */
  publish: (positionMs: number) => void
  /** Stop playback at the end of program content. */
  stop: () => void
  /** The deck still shows this element after the next render. */
  isCurrentVideo?: () => boolean
  scheduler?: FreeRunScheduler
  /** Shared continuous session clock across cuts and gaps when owned by Preview. */
  sessionClock?: PlaybackSessionClock
}

const browserScheduler: FreeRunScheduler = {
  now: () => performance.now(),
  requestFrame: (cb) => requestAnimationFrame(cb),
  cancelFrame: (handle) => cancelAnimationFrame(handle),
}

/** FREE-RUN sync for forward 1x play of a mounted <video>. A continuous
 * PlaybackSessionClock owns wall-clock timeline progression across cuts and
 * gaps while MediaProgressMonitor tracks element progress and issues
 * seek/reload recovery when an element stalls, errors, or jumps. Contiguous
 * cuts keep their decoder playing without pausing. */
export function runFreeRunClock(args: FreeRunArgs): () => void {
  const { video: v, callbacks, state: cfg, tracker, publish, stop } = args
  let expectedKey = args.expectedKey
  let clockVideo = cfg.current.video
  const scheduler = args.scheduler ?? browserScheduler
  const initialSeekRevision = cfg.current.seekRevision
  const ownsPublish = !args.sessionClock
  const clock = args.sessionClock ?? new PlaybackSessionClock(cfg.current.playheadMs, scheduler.now(), cfg.current.rate, initialSeekRevision)
  const monitor = new MediaProgressMonitor(scheduler.now())
  let lastSeekRevision = initialSeekRevision
  let rafId = 0
  let fallbackRaf = 0
  let stopped = false
  const handoff = new PlaybackHandoff(tracker)
  let lastFrameAt = scheduler.now()
  let ready = false
  let initialised = false
  let seekTargetMs = 0
  let progressAtResync = Number.NaN
  // Ask the element to show the frame for a timeline position and play.
  // `force` re-issues the seek even at the same position (unsticks a stalled element).
  // An element already parked or playing right at the target (see
  // MEDIA_START_IN_PLACE_MS) is not seeked: it plays from where it stands.
  // Never for a user seek (`userSeek`): a small jump back must still land.
  const seekMedia = (positionMs: number, force = false, userSeek = false) => {
    const c = cfg.current
    if (!c.video) return
    const end = Math.max(c.video.startMs + c.video.durMs, c.continuousEndMs ?? 0)
    const sourceOutMs = c.video.srcInMs + (end - c.video.startMs) * c.video.speed
    seekTargetMs = sourceMsAtTimelinePosition({ ...c.video, srcOutMs: sourceOutMs }, positionMs)
    ready = false
    const behindMs = seekTargetMs - v.currentTime * 1000
    const clipStartInPlace = !force && !userSeek && v.paused && !v.seeking
      && canStartPreviewMedia(v.readyState)
      && Math.abs(seekTargetMs - c.video.srcInMs) <= c.frameMs * 2
      && behindMs >= -Math.max(120, c.frameMs * 3)
      && behindMs <= 250 * Math.max(1, c.video.speed)
    const inPlace = clipStartInPlace
      || (!force && !userSeek && startsInPlace(v.paused, v.seeking, v.readyState, behindMs, c.video.speed))
    // Preserve an in-flight standby park seek already within MEDIA_START_IN_PLACE_MS
    // instead of cancelling it with another currentTime write.
    const inFlightInPlace = !force && !userSeek && v.seeking && startsInPlace(v.paused, false, v.readyState, behindMs, c.video.speed)
    if (inPlace || inFlightInPlace) seekTargetMs = v.currentTime * 1000
    if (force || (!inPlace && !inFlightInPlace && Math.abs(v.currentTime * 1000 - seekTargetMs) > c.frameMs / 2)) {
      v.currentTime = seekTargetMs / 1000
    }
    ready = !v.seeking && Math.abs(v.currentTime * 1000 - seekTargetMs) <= c.frameMs
    v.playbackRate = c.video.speed
    if (v.paused) void v.play().catch(() => {})
  }
  // `explicit`: a user seek moved the playhead, so the timeline restarts there.
  const startPlay = (explicit: boolean) => {
    const c = cfg.current
    if (!c.video || playbackVideoKey(c.video) !== expectedKey) return
    const now = scheduler.now()
    if (explicit && ownsPublish) clock.reanchor(c.playheadMs, now, c.rate, c.seekRevision)
    const targetMs = clock.syncExternal(c.playheadMs, c.seekRevision, now, c.rate)
    if (!canStartPreviewMedia(v.readyState)) {
      initialised = false
      ready = false
      return
    }
    initialised = true
    monitor.restart(now)
    seekMedia(targetMs, false, explicit)
  }
  // The element stopped advancing or failed while the timeline kept playing.
  // First ask it to seek again; if it still has not advanced by the next
  // attempt, or it failed, reload it (canplay then restarts it at the timeline).
  const resync = () => {
    const now = scheduler.now()
    const stillStuck = monitor.lastProgressAt === progressAtResync
    progressAtResync = monitor.lastProgressAt
    if (v.error || (initialised && stillStuck)) {
      progressAtResync = Number.NaN
      initialised = false
      ready = false
      v.load()
      return
    }
    if (!initialised) return
    monitor.awaitMedia()
    seekMedia(clock.syncExternal(cfg.current.playheadMs, cfg.current.seekRevision, now, cfg.current.rate), true)
  }
  const onSeeked = () => {
    if (!stopped && initialised && !v.seeking && Math.abs(v.currentTime * 1000 - seekTargetMs) <= cfg.current.frameMs * 2) ready = true
  }
  // Standalone runner only: publish boundary transitions and cadence ticks.
  const emitTick = (now: number, timelineMs: number) => {
    const c = cfg.current
    const vid = c.video
    if (!vid) return
    const position = previewPlaybackClockPosition(timelineMs, c.durationMs, c.voiceoverPlayback, c.rate)
    if (position.stop) {
      clock.markPublished(now, position.positionMs)
      publish(position.positionMs)
      if (!c.voiceoverPlayback) stop()
      return
    }
    const boundary = nextPlaybackBoundaryMs(c.rate, vid, c.nextVideoStartMs ?? null)
    if (!clock.shouldPublish(now, position.positionMs, boundary)) return
    if (boundary !== null && position.positionMs >= boundary && !handoff.begin()) return
    clock.markPublished(now, position.positionMs)
    publish(position.positionMs)
  }
  // 'continue' when this clock may run: no handoff pending and no user seek.
  const observe = (): boolean => {
    const c = cfg.current
    if (c.rate !== 1 || !(args.isCurrentVideo?.() ?? true)) return false
    const key = playbackVideoKey(c.video)
    const changed = key !== expectedKey
    if (changed) {
      if (!clockVideo || !c.video || playbackBindingKey(clockVideo, 0) !== playbackBindingKey(c.video, 0)) return false
      expectedKey = key
      clockVideo = c.video
      handoff.complete()
    }
    if (!ownsPublish) {
      if (c.seekRevision !== lastSeekRevision) {
        lastSeekRevision = c.seekRevision
        startPlay(true)
        return false
      }
      return true
    }
    const state = handoff.observe(c.playheadMs, c.seekRevision)
    if (state === 'seek') { startPlay(true); return false }
    return changed || state === 'continue'
  }
  const syncFromMediaTime = (now: number, mediaTime: number) => {
    if (stopped || !observe()) return
    const vid = cfg.current.video
    if (vid && mediaTime * 1000 >= vid.srcInMs - cfg.current.frameMs
      && acceptPlaybackFrame(expectedKey, vid, mediaTime * 1000, v.currentTime * 1000, ready, v.seeking, cfg.current.frameMs,
        vid.srcInMs + (Math.max(vid.startMs + vid.durMs, cfg.current.continuousEndMs ?? 0) - vid.startMs) * vid.speed)) {
      const mediaMs = Math.max(timelineMsAtSourcePosition(vid, mediaTime * 1000), vid.startMs)
      monitor.sample(mediaMs, clock.position(now), now, v.seeking, v.paused)
    }
  }
  const onFrame: VideoFrameCallback = (now, meta) => {
    if (stopped) return
    mediaTimingCapability.frameCallbacks = true
    lastFrameAt = now
    syncFromMediaTime(now, meta.mediaTime)
    rafId = callbacks.request(onFrame)
  }
  const fallbackTick = (now: number) => {
    if (stopped) return
    // Runs every animation frame, also for an ended, paused or failed element:
    // it notices a seek that cancels a handoff and keeps the timeline moving.
    if (!v.paused && v.readyState >= 2 && now - lastFrameAt > 250) syncFromMediaTime(now, v.currentTime)
    if (!stopped && observe()) {
      if (ownsPublish) emitTick(now, clock.position(now))
      if (monitor.resyncDue(now, !!v.error)) resync()
    }
    if (!stopped) fallbackRaf = scheduler.requestFrame(fallbackTick)
  }
  const onReady = () => { if (!initialised) startPlay(false) }
  const onEnded = () => {
    if (!ownsPublish || !observe()) return
    const c = cfg.current
    if (c.video && v.ended && acceptPlaybackFrame(expectedKey, c.video, v.currentTime * 1000, v.currentTime * 1000, ready, v.seeking, c.frameMs)
      && v.currentTime * 1000 >= c.video.srcOutMs - c.frameMs * 2) {
      const now = scheduler.now()
      emitTick(now, Math.max(clock.position(now), c.video.startMs + c.video.durMs))
    }
  }
  v.addEventListener('canplay', onReady)
  v.addEventListener('seeked', onSeeked)
  v.addEventListener('ended', onEnded)
  if (v.readyState >= 2) startPlay(false)
  rafId = callbacks.request(onFrame)
  fallbackRaf = scheduler.requestFrame(fallbackTick)
  return () => {
    stopped = true
    v.removeEventListener('canplay', onReady)
    v.removeEventListener('seeked', onSeeked)
    v.removeEventListener('ended', onEnded)
    callbacks.cancel(rafId)
    scheduler.cancelFrame(fallbackRaf)
    const c = cfg.current
    const continuing = c.rate === 1 && c.seekRevision === initialSeekRevision
      && clockVideo && c.video && continuesInPlace(clockVideo, c.video, c.frameMs)
      && (args.isCurrentVideo?.() ?? true)
    if (!continuing) v.pause()
  }
}
