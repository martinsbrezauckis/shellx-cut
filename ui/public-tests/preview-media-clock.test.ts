import assert from 'node:assert/strict'
import { activeVideo } from '../src/panels/Preview/model'
import { runFreeRunClock, type FreeRunState, type FreeRunVideo } from '../src/panels/Preview/freeRunClock'
import { overlayResyncDriftS, PlaybackSeekTracker, playbackVideoKey } from '../src/panels/Preview/playbackClock'

// Forward playback of a clip whose <video> element was just mounted, driven
// through the media event orders observed on the three desktop webviews. The
// timeline clock must keep advancing, never rewind and never jump ahead, even
// when the element stops delivering progress.

const clip = (id: string, a: number, b: number) => ({ id, asset: 'a', src_in_ms: a, src_out_ms: b })
const project: any = { assets: { a: { proxy: '/a.mp4' } }, tracks: [
  { id: 'v1', kind: 'video', clips: [clip('first', 0, 30000), { kind: 'gap', duration_ms: 30000 }, clip('last', 60000, 90000)] },
  { id: 'v2', kind: 'video', clips: [{ kind: 'gap', duration_ms: 30000 }, clip('middle', 30000, 60000)] },
] }

interface Behaviour {
  /** Delay from mount until metadata, data and canplay arrive together. */
  loadMs?: number
  /** Seek latency; null makes that seek never complete. `reloads` counts load() calls. */
  seekMs?: (seekIndex: number, reloads: number) => number | null
  /** Frame callbacks with mediaTime (WKWebView/WebView2); none on WebKitGTK here. */
  frameCallbacks?: boolean
  /** Report a frame from the element's initial position before the start seek. */
  staleInitialFrame?: boolean
  /** Media time (s) at which the element stops advancing without any event. */
  freezeAtS?: number
  /** Like freezeAtS, but the element stays frozen through seeks until it is reloaded. */
  freezeUntilReloadAtS?: number
  /** Media time (s) at which the element fails with this MediaError code. */
  errorAt?: { s: number; code: number }
  /** Media time (s) at which the element's position jumps forward unasked. */
  jumpAt?: { s: number; toS: number }
  /** Already loaded and paused at this media time when the clock starts (the base deck parked it). */
  parkedAtS?: number
  /** Already loaded and PLAYING at this media time (a cut inside one recording). */
  playingAtS?: number
}

class FakeVideo implements FreeRunVideo {
  readyState = 0
  seeking = false
  paused = true
  ended = false
  playbackRate = 1
  error: MediaError | null = null
  seeks: number[] = []
  reloads = 0
  private time = 0
  private listeners = new Map<string, Set<() => void>>()
  private pendingSeek: { doneAt: number } | null = null
  private loadAt: number
  private frozen = false
  constructor(private readonly sim: Sim, readonly b: Behaviour) {
    this.loadAt = sim.now + (b.loadMs ?? 160)
    if (b.parkedAtS !== undefined) { this.readyState = 4; this.time = b.parkedAtS }
    if (b.playingAtS !== undefined) { this.readyState = 4; this.time = b.playingAtS; this.paused = false }
  }
  get currentTime() { return this.time }
  set currentTime(s: number) {
    this.time = s
    this.ended = false
    this.seeking = true
    const latency = (this.b.seekMs ?? (() => 12))(this.seeks.length, this.reloads)
    this.seeks.push(s)
    this.pendingSeek = latency === null ? null : { doneAt: this.sim.now + latency }
    if (this.b.freezeUntilReloadAtS === undefined || this.reloads > 0) this.frozen = false
    this.emit('seeking')
  }
  play() { this.paused = false; this.emit('play'); return Promise.resolve() }
  pause() { this.paused = true }
  load() {
    this.reloads += 1
    this.frozen = false
    this.error = null
    this.readyState = 0
    this.time = 0
    this.seeking = false
    this.pendingSeek = null
    this.loadAt = this.sim.now + (this.b.loadMs ?? 160)
  }
  addEventListener(type: string, cb: () => void) {
    if (!this.listeners.has(type)) this.listeners.set(type, new Set())
    this.listeners.get(type)!.add(cb)
  }
  removeEventListener(type: string, cb: () => void) { this.listeners.get(type)?.delete(cb) }
  emit(type: string) { for (const cb of [...(this.listeners.get(type) ?? [])]) cb() }
  /** Advance the element by one display frame; returns true when a new frame was presented. */
  step(dt: number): boolean {
    if (this.readyState === 0 && !this.error && this.sim.now >= this.loadAt) {
      this.readyState = 4
      if (this.b.staleInitialFrame) this.sim.presentFrame(this.time)
      for (const type of ['loadedmetadata', 'loadeddata', 'canplay', 'canplaythrough']) this.emit(type)
      return false
    }
    if (this.pendingSeek && this.sim.now >= this.pendingSeek.doneAt) {
      this.pendingSeek = null
      this.seeking = false
      this.emit('seeked')
    }
    if (this.error || this.paused || this.seeking || this.readyState < 2 || this.frozen) return false
    this.time += (dt / 1000) * this.playbackRate
    const { freezeAtS, freezeUntilReloadAtS, errorAt, jumpAt } = this.b
    if (freezeAtS !== undefined && this.time >= freezeAtS && this.seeks.length <= 1) { this.time = freezeAtS; this.frozen = true }
    if (freezeUntilReloadAtS !== undefined && this.time >= freezeUntilReloadAtS && this.reloads === 0) { this.time = freezeUntilReloadAtS; this.frozen = true }
    if (errorAt && this.time >= errorAt.s && this.seeks.length <= 1) {
      this.error = { code: errorAt.code } as MediaError
      this.time = 0
      this.emit('error')
      return false
    }
    if (jumpAt && this.time >= jumpAt.s && this.time < jumpAt.toS && this.seeks.length <= 1) this.time = jumpAt.toS
    if (this.time >= 90) { this.time = 90; this.ended = true; this.paused = true; this.emit('ended') }
    return true
  }
}

class Sim {
  now = 1000
  private frameCbs = new Map<number, (now: number) => void>()
  private videoCbs = new Map<number, (now: number, meta: { mediaTime: number }) => void>()
  private nextId = 1
  scheduler = {
    now: () => this.now,
    requestFrame: (cb: (now: number) => void) => { const id = this.nextId++; this.frameCbs.set(id, cb); return id },
    cancelFrame: (id: number) => { this.frameCbs.delete(id) },
  }
  callbacks = {
    request: (cb: (now: number, meta: { mediaTime: number }) => void) => { const id = this.nextId++; this.videoCbs.set(id, cb); return id },
    cancel: (id: number) => { this.videoCbs.delete(id) },
  }
  presentFrame(mediaTime: number) {
    const cbs = [...this.videoCbs.values()]
    this.videoCbs.clear()
    for (const cb of cbs) cb(this.now, { mediaTime })
  }
  runFrame(video: FakeVideo, dt: number) {
    this.now += dt
    const presented = video.step(dt)
    if (presented && video.b.frameCallbacks) this.presentFrame(video.currentTime)
    const cbs = [...this.frameCbs.values()]
    this.frameCbs.clear()
    for (const cb of cbs) cb(this.now)
  }
}

interface Outcome { samples: number[]; video: FakeVideo; stopped: boolean; plateauMs: number; backMs: number; forwardMs: number }

function play(b: Behaviour, startMs = 60030, durationMs = 4000, userSeek?: { atMs: number; toMs: number }): Outcome {
  const sim = new Sim()
  const video = new FakeVideo(sim, b)
  let seekRevision = 0
  const state: { current: FreeRunState } = { current: {
    playheadMs: startMs, durationMs: 90000, frameMs: 1000 / 24, seekRevision, video: activeVideo(project, startMs, new Set()), voiceoverPlayback: null,
  } }
  const tracker = new PlaybackSeekTracker(startMs, seekRevision)
  const expectedKey = playbackVideoKey(state.current.video)
  let stopped = false
  const render = (playheadMs: number) => {
    state.current = { ...state.current, playheadMs, seekRevision, video: activeVideo(project, playheadMs, new Set()) }
  }
  const cleanup = runFreeRunClock({
    video, callbacks: sim.callbacks, expectedKey, state, tracker, scheduler: sim.scheduler,
    publish: (positionMs) => render(tracker.published(positionMs)),
    stop: () => { stopped = true },
  })
  const samples: number[] = []
  for (let elapsed = 0; elapsed < durationMs && !stopped; elapsed += 16) {
    if (userSeek && elapsed >= userSeek.atMs && elapsed < userSeek.atMs + 16) { seekRevision += 1; render(userSeek.toMs) }
    sim.runFrame(video, 16)
    samples.push(state.current.playheadMs)
    if (playbackVideoKey(state.current.video) !== expectedKey) break
  }
  cleanup()
  let plateauMs = 0
  let run = 0
  let backMs = 0
  let forwardMs = 0
  for (let i = 1; i < samples.length; i += 1) {
    const d = samples[i] - samples[i - 1]
    run = d === 0 ? run + 16 : 0
    plateauMs = Math.max(plateauMs, run)
    if (-d > backMs && !(userSeek && Math.abs(samples[i] - userSeek.toMs) < 100)) backMs = -d
    if (d > forwardMs && !(userSeek && Math.abs(samples[i] - userSeek.toMs) < 100)) forwardMs = d
  }
  return { samples, video, stopped, plateauMs, backMs, forwardMs }
}

const failures: string[] = []
function expectSmooth(name: string, o: Outcome, startMs = 60030, minAdvanceMs = 3000) {
  const last = o.samples[o.samples.length - 1]
  const problems = [
    o.plateauMs > 1000 ? `playhead held ${o.plateauMs} ms` : '',
    o.backMs > 250 ? `moved back ${o.backMs} ms` : '',
    o.forwardMs > 500 ? `jumped ahead ${o.forwardMs} ms` : '',
    last - startMs < minAdvanceMs ? `advanced only ${last - startMs} ms` : '',
  ].filter(Boolean)
  if (problems.length) failures.push(`${name}: ${problems.join(', ')}`)
  else console.log(`PASS ${name} (held at most ${o.plateauMs} ms, reached ${last})`)
}

// Until frame callbacks have been seen (WebKitGTK never delivers any), a playing
// overlay tolerates 250 ms of drift before it is seeked; afterwards 60 ms.
expectSmooth('no-frame-callback order before any callback', play({}))
assert.equal(overlayResyncDriftS(), 0.25, 'no frame callbacks seen: tolerant overlay sync')
// WKWebView / WebView2: frame callbacks, start seek completes after canplay.
expectSmooth('frame-callback order', play({ frameCallbacks: true }))
assert.equal(overlayResyncDriftS(), 0.06, 'frame callbacks seen: precise overlay sync as before')
// Original 0.6.115 order: the new element first reports its initial frame (0 s).
const stale = play({ frameCallbacks: true, staleInitialFrame: true })
expectSmooth('stale initial frame is not a jump to zero', stale)
assert.ok(stale.samples.every((p) => p >= 60000), 'the playhead never maps a stale frame back toward zero')
// WebKitGTK healthy order: no frame callbacks, events in one burst, seeked after seeking.
expectSmooth('no-frame-callback order', play({}))
// WebKitGTK: the start seek reports seeking and never seeked; currentTime stays at the target.
const stuck = play({ seekMs: (i) => (i === 0 ? null : 12) })
expectSmooth('start seek that never completes', stuck)
if (stuck.video.seeks.length < 2) failures.push('start seek that never completes: the element was never asked to seek again')
else if (Math.abs(stuck.video.currentTime * 1000 - stuck.samples[stuck.samples.length - 1]) > 300) {
  failures.push(`start seek that never completes: picture ${stuck.video.currentTime} s does not follow the playhead`)
}
// WebKitGTK: a start seek that never completes, and the element never recovers.
expectSmooth('element that never completes any seek', play({ seekMs: () => null }))
// WebKitGTK: no seek completes until the element is reloaded; the picture must recover.
const reload = play({ seekMs: (_i, reloads) => (reloads > 0 ? 12 : null) })
expectSmooth('element whose seeks only complete after a reload', reload)
if (reload.video.reloads < 1) failures.push('element whose seeks only complete after a reload: never reloaded')
else if (Math.abs(reload.video.currentTime * 1000 - reload.samples[reload.samples.length - 1]) > 300) {
  failures.push(`element whose seeks only complete after a reload: picture ${reload.video.currentTime} s does not follow the playhead`)
}
// WebKitGTK: the element freezes while playing and seeks do not wake it; a reload does.
const frozen = play({ freezeUntilReloadAtS: 61.2 })
expectSmooth('element frozen until reloaded', frozen)
if (Math.abs(frozen.video.currentTime * 1000 - frozen.samples[frozen.samples.length - 1]) > 300) {
  failures.push(`element frozen until reloaded: picture ${frozen.video.currentTime} s does not follow the playhead`)
}
// WebKitGTK: the element plays, then its currentTime stops without any event.
expectSmooth('element that stops advancing while playing', play({ freezeAtS: 62.5 }))
// WebKitGTK: MediaError after the start seek (currentTime reads 0 afterwards).
expectSmooth('decoder error after the start seek', play({ errorAt: { s: 60.2, code: 4 } }))
// WebKitGTK: the element's position jumps forward unasked.
expectSmooth('unrequested forward media jump', play({ jumpAt: { s: 61.6, toS: 72.7 } }))
// Base deck: the next clip's element is parked on its first frame (60.000 s)
// and the playhead crosses the cut 30 ms later. It starts from where it stands:
// no seek at the cut, so it plays even where no seek ever completes (WebKitGTK),
// and the picture follows the playhead.
for (const [name, b] of [['healthy', { parkedAtS: 60, frameCallbacks: true }], ['seeks never complete', { parkedAtS: 60, seekMs: () => null }]] as const) {
  const o = play(b)
  expectSmooth(`parked element, ${name}`, o)
  if (o.video.seeks.length) failures.push(`parked element, ${name}: seeked at the cut (${o.video.seeks.join(', ')})`)
  if (o.video.reloads) failures.push(`parked element, ${name}: reloaded ${o.video.reloads} times`)
  if (Math.abs(o.video.currentTime * 1000 - o.samples[o.samples.length - 1]) > 300) {
    failures.push(`parked element, ${name}: picture ${o.video.currentTime} s does not follow the playhead ${o.samples[o.samples.length - 1]}`)
  }
}
// A cut inside one recording: the element keeps playing and is 40 ms past the
// cut when the next clip's clock starts. No seek on the playing element (on
// WebKitGTK that seek often never reports back and held the playhead 0.7 s).
for (const [name, b] of [['healthy', { playingAtS: 60.04, frameCallbacks: true }], ['seeks never complete', { playingAtS: 60.04, seekMs: () => null }]] as const) {
  const o = play(b, 60000)
  expectSmooth(`element playing across a cut, ${name}`, o, 60000)
  if (o.video.seeks.length) failures.push(`element playing across a cut, ${name}: seeked (${o.video.seeks.join(', ')})`)
  // (without frame callbacks the clock first samples the element after 250 ms; a seek here held it 656 ms)
  if (o.plateauMs > 400) failures.push(`element playing across a cut, ${name}: playhead held ${o.plateauMs} ms`)
}
// A small user seek BACK during playback still lands (the in-place start is
// only for the clock's own starts, never for a seek the person made).
{
  // Where the playhead stands 1.5 s in (same deterministic run), then seek 200 ms back from there.
  const standsAt = play({ frameCallbacks: true }, 60030, 1520).samples.at(-1)!
  const target = standsAt - 200
  const o = play({ frameCallbacks: true }, 60030, 3000, { atMs: 1500, toMs: target })
  const at200 = o.samples[Math.ceil(1716 / 16)]
  if (!(at200 < standsAt + 100)) failures.push(`small seek back during playback did not land: ${at200} 200 ms after seeking from ${standsAt} to ${target}`)
  else console.log('PASS small seek back during playback lands')
}
// An element parked far from the playhead (not this clip's start) is still seeked.
const far = play({ parkedAtS: 61 })
if (far.video.seeks.length < 1) failures.push('element ahead of the playhead was not seeked')
// The end of program content still stops playback when the element stalls.
const end = play({ seekMs: () => null }, 88500, 4000)
if (!end.stopped || end.samples[end.samples.length - 1] !== 90000) failures.push(`stalled element at the end: stopped=${end.stopped} last=${end.samples[end.samples.length - 1]}`)
else console.log('PASS stalled element still reaches the end of content')
// A seek during playback lands and stays, in a healthy and in a stalled element.
for (const [name, b] of [['healthy', { frameCallbacks: true }], ['stalled', { seekMs: () => null }]] as const) {
  const o = play(b, 60030, 4000, { atMs: 2000, toMs: 75000 })
  const after = o.samples.slice(Math.ceil(2016 / 16))
  if (!after.length || after.some((p) => p < 75000) || after[after.length - 1] < 76000) failures.push(`seek during ${name} playback did not land and continue: ${after.slice(0, 3)} .. ${after[after.length - 1]}`)
  else console.log(`PASS seek during ${name} playback lands and continues`)
}

assert.deepEqual(failures, [], `\n${failures.join('\n')}`)
console.log('PASS preview media clock event orders')
