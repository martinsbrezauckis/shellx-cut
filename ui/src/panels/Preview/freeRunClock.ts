import { sourceMsAtTimelinePosition, timelineMsAtSourcePosition } from '../../lib/mediaTime'
import type { ActiveVideo } from './model'
import {
  acceptPlaybackFrame,
  canStartPreviewMedia,
  PlaybackHandoff,
  mediaTimingCapability,
  playbackVideoKey,
  startsInPlace,
  TimelineMediaClock,
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
  playheadMs: number
  durationMs: number
  frameMs: number
  seekRevision: number
  video: ActiveVideo | null
  voiceoverPlayback: { outMs: number | null } | null
}

export interface FreeRunArgs {
  video: FreeRunVideo
  callbacks: FreeRunFrameCallbacks
  /** The clip placement this clock instance was started for. */
  expectedKey: string | null
  /** Latest rendered state, read on every callback. */
  state: { readonly current: FreeRunState }
  tracker: PlaybackSeekTracker
  /** Publish a playback clock position (not a user seek). */
  publish: (positionMs: number) => void
  /** Stop playback at the end of program content. */
  stop: () => void
  scheduler?: FreeRunScheduler
}

const browserScheduler: FreeRunScheduler = {
  now: () => performance.now(),
  requestFrame: (cb) => requestAnimationFrame(cb),
  cancelFrame: (handle) => cancelAnimationFrame(handle),
}

/** FREE-RUN clock for forward 1x play of a mounted <video>: the element plays
 * itself and its presented media time is mapped back to a timeline position.
 * The timeline stays the authority (see TimelineMediaClock): an element that
 * does not start, stops advancing, fails or jumps ahead is resynchronized while
 * the timeline keeps playing, so no media event order can hold playback.
 * Returns the cleanup that detaches every listener and pauses the element. */
export function runFreeRunClock(args: FreeRunArgs): () => void {
  const { video: v, callbacks, expectedKey, state: cfg, tracker, publish, stop } = args
  const scheduler = args.scheduler ?? browserScheduler
  let rafId = 0
  let fallbackRaf = 0
  let stopped = false
  const handoff = new PlaybackHandoff(tracker)
  let lastDispatch = 0
  let lastFrameAt = scheduler.now()
  let ready = false
  let initialised = false
  let seekTargetMs = 0
  let progressAtResync = Number.NaN
  const clock = new TimelineMediaClock(cfg.current.playheadMs, scheduler.now())
  // Ask the element to show the frame for a timeline position and play.
  // `force` re-issues the seek even at the same position (unsticks a stalled element).
  // An element already parked or playing right at the target (see
  // MEDIA_START_IN_PLACE_MS) is not seeked: it plays from where it stands.
  // Never for a user seek (`userSeek`): a small jump back must still land.
  const seekMedia = (positionMs: number, force = false, userSeek = false) => {
    const c = cfg.current
    if (!c.video) return
    seekTargetMs = sourceMsAtTimelinePosition(c.video, positionMs)
    ready = false
    const inPlace = !force && !userSeek && startsInPlace(v.paused, v.seeking, v.readyState, seekTargetMs - v.currentTime * 1000, c.video.speed)
    if (inPlace) seekTargetMs = v.currentTime * 1000
    if (force || (!inPlace && Math.abs(v.currentTime * 1000 - seekTargetMs) > c.frameMs / 2)) {
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
    if (explicit) clock.restart(c.playheadMs, now)
    if (!canStartPreviewMedia(v.readyState)) {
      initialised = false
      ready = false
      return
    }
    initialised = true
    clock.awaitMedia()
    seekMedia(clock.position, false, explicit)
  }
  // The element stopped advancing or failed while the timeline kept playing.
  // First ask it to seek again; if it still has not advanced by the next
  // attempt, or it failed, reload it (canplay then restarts it at the timeline).
  const resync = () => {
    const stillStuck = clock.lastProgressAt === progressAtResync
    progressAtResync = clock.lastProgressAt
    if (v.error || (initialised && stillStuck)) {
      progressAtResync = Number.NaN
      initialised = false
      ready = false
      v.load()
      return
    }
    if (!initialised) return
    clock.awaitMedia()
    seekMedia(clock.position, true)
  }
  const onSeeked = () => {
    if (!stopped && initialised && !v.seeking && Math.abs(v.currentTime * 1000 - seekTargetMs) <= cfg.current.frameMs * 2) ready = true
  }
  // Cross the clip's out point → jump the playhead to the next clip start;
  // the src swap re-arms this clock, which reseeks + plays the next proxy.
  const advance = (toMs: number) => {
    if (!handoff.begin()) return
    const c = cfg.current
    publish(previewPlaybackClockPosition(toMs, c.durationMs, c.voiceoverPlayback).positionMs)
  }
  const emit = (now: number, timelineMs: number) => {
    const c = cfg.current
    const vid = c.video
    if (!vid) return
    const clipEndMs = vid.startMs + vid.durMs
    if (timelineMs >= c.durationMs - 1) {
      publish(previewPlaybackClockPosition(c.durationMs, c.durationMs, c.voiceoverPlayback).positionMs)
      if (!c.voiceoverPlayback) stop()
      return
    }
    if (timelineMs >= clipEndMs - 1) advance(Math.min(c.durationMs, clipEndMs))
    else if (now - lastDispatch >= 100) {
      lastDispatch = now
      publish(previewPlaybackClockPosition(timelineMs, c.durationMs, c.voiceoverPlayback).positionMs)
    }
  }
  // 'continue' when this clock may run: no handoff pending and no user seek.
  const observe = (): boolean => {
    const c = cfg.current
    if (playbackVideoKey(c.video) !== expectedKey) return false
    const state = handoff.observe(c.playheadMs, c.seekRevision)
    if (state === 'seek') { startPlay(true); return false }
    return state === 'continue'
  }
  const syncFromMediaTime = (now: number, mediaTime: number) => {
    if (stopped || !observe()) return
    const vid = cfg.current.video
    if (vid && acceptPlaybackFrame(expectedKey, vid, mediaTime * 1000, v.currentTime * 1000, ready, v.seeking, cfg.current.frameMs)) {
      const mediaMs = Math.max(timelineMsAtSourcePosition(vid, mediaTime * 1000), vid.startMs)
      emit(now, clock.media(mediaMs, now))
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
      emit(now, clock.tick(now))
      if (clock.resyncDue(now)) resync()
    }
    if (!stopped) fallbackRaf = scheduler.requestFrame(fallbackTick)
  }
  const onReady = () => { if (!initialised) startPlay(false) }
  const onEnded = () => {
    if (!observe()) return
    const c = cfg.current
    if (c.video && v.ended && acceptPlaybackFrame(expectedKey, c.video, v.currentTime * 1000, v.currentTime * 1000, ready, v.seeking, c.frameMs)
      && v.currentTime * 1000 >= c.video.srcOutMs - c.frameMs * 2) {
      const now = scheduler.now()
      emit(now, clock.media(c.video.startMs + c.video.durMs, now))
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
    v.pause()
  }
}
