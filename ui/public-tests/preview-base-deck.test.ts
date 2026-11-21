import assert from 'node:assert/strict'
import { activeVideo } from '../src/panels/Preview/model'
import { continuesInPlace, continuousBaseVideoEnd, EMPTY_DECK, nextDeck, PREROLL_MS, upcomingBaseVideo, type DeckState } from '../src/panels/Preview/baseDeckPlan'
import { liveBaseGap, MAX_OVERLAYS, resolveOverlays, resolveUpcomingOverlays } from '../src/panels/Preview/composite'

// The base preview video is a deck of two elements: the clip under the
// playhead plays in the FRONT element while the next base clip is already
// loaded and parked on its first frame in the STANDBY element. At the cut the
// standby only has to play; no element is created or seeked at the cut.
const frameMs = 1000 / 30
const clip = (id: string, a: number, b: number) => ({ id, asset: 'a', src_in_ms: a, src_out_ms: b })
const gap = (ms: number) => ({ kind: 'gap', duration_ms: ms })
// Issue #2 arrangement: one 90 s recording cut at 30 s and 60 s, the middle
// part moved to V2 (V1 has a gap 30..60 s).
const issue2: any = { assets: { a: { proxy: '/a.mp4' } }, tracks: [
  { id: 'v1', kind: 'video', clips: [clip('first', 0, 30000), gap(30000), clip('last', 60000, 90000)] },
  { id: 'v2', kind: 'video', clips: [gap(30000), clip('middle', 30000, 60000)] },
] }
const none = new Set<string>()
const at = (project: any, ms: number) => activeVideo(project, ms, none)
const next = (project: any, ms: number) => upcomingBaseVideo(project, ms, none)

// --- what is parked: the next base clip, at its own first frame, within the horizon
assert.equal(PREROLL_MS, 3000)
assert.equal(next(issue2, 50000), null, 'nothing is loaded 10 s ahead of the cut')
const last = next(issue2, 58000)!
assert.equal(last.clipId, 'last')
assert.equal(last.srcMs, 60000, 'parked on the first frame after the gap')
assert.equal(next(issue2, 60000), null, 'the clip under the playhead is not "upcoming"')

// --- issue #2 playback: V1 -> gap (V2 plays) -> V1 again
let deck: DeckState = EMPTY_DECK
deck = nextDeck(deck, at(issue2, 10000), next(issue2, 10000), frameMs)
assert.equal(deck.front, 0)
assert.equal(deck.slots[0]?.video.clipId, 'first')
deck = nextDeck(deck, null, next(issue2, 30000), frameMs)
assert.equal(deck.front, null, 'nothing of the base track is on screen in its gap')
deck = nextDeck(deck, null, next(issue2, 58000), frameMs)
assert.equal(deck.front, null)
assert.equal(deck.slots[1]?.video.clipId, 'last', 'the clip after the gap is parked in the other element')
assert.equal(deck.slots[1]?.parkMs, 60000)
const parked = deck.slots[1]
deck = nextDeck(deck, at(issue2, 60000), next(issue2, 60000), frameMs)
assert.equal(deck.front, 1, 'at the cut the parked element comes on screen')
assert.equal(deck.slots[1], parked, 'and keeps its entry: no new source, no reload')

// --- a cut every second between non-adjacent source places: the two elements alternate
const shuffle: any = { assets: { a: { proxy: '/a.mp4' } }, tracks: [
  { id: 'v1', kind: 'video', clips: [clip('s0', 0, 1000), clip('s1', 14000, 15000), clip('s2', 28000, 29000), clip('s3', 42000, 43000)] },
] }
deck = EMPTY_DECK
const fronts: Array<number | null> = []
for (const ms of [0, 500, 1000, 1500, 2000, 3000]) {
  const before = deck
  deck = nextDeck(deck, at(shuffle, ms), next(shuffle, ms), frameMs)
  fronts.push(deck.front)
  const upcoming = next(shuffle, ms)
  if (upcoming) {
    const standby = deck.slots[deck.front === 0 ? 1 : 0]
    assert.equal(standby?.video.clipId, upcoming.clipId, `at ${ms} ms the next clip waits in the standby element`)
    assert.equal(standby?.parkMs, upcoming.srcMs)
  }
  if (ms === 500) assert.equal(deck, before, 'an unchanged deck is the same object (no re-render churn)')
}
assert.deepEqual(fronts, [0, 0, 1, 1, 0, 1])

// --- a plain cut inside one recording: the playing element just carries on
const contiguous: any = { assets: { a: { proxy: '/a.mp4' } }, tracks: [
  { id: 'v1', kind: 'video', clips: [clip('c0', 0, 1000), clip('c1', 1000, 2000)] },
] }
assert.equal(continuesInPlace(at(contiguous, 0)!, at(contiguous, 1000)!, frameMs), true)
assert.equal(continuesInPlace(at(shuffle, 0)!, at(shuffle, 1000)!, frameMs), false)
deck = nextDeck(EMPTY_DECK, at(contiguous, 0), next(contiguous, 0), frameMs)
assert.deepEqual([deck.front, deck.slots[1]], [0, null], 'no standby for a cut that continues in place')
deck = nextDeck(deck, at(contiguous, 1000), next(contiguous, 1000), frameMs)
assert.equal(deck.front, 0, 'the same element plays across the cut')
assert.equal(deck.slots[0]?.video.clipId, 'c1')

// The clock may use the actual presented frame across a run of source-contiguous
// short cuts. A gap or source jump ends that permission before another picture.
const pieces = (prefix: string, firstSourceMs: number) => Array.from({ length: 20 }, (_, i) =>
  clip(`${prefix}${i}`, firstSourceMs + i * 200, firstSourceMs + (i + 1) * 200))
const manyCuts: any = { assets: { a: { proxy: '/a.mp4' } }, tracks: [
  { id: 'v1', kind: 'video', clips: [...pieces('a', 0), gap(200), ...pieces('b', 4000), ...pieces('c', 10000)] },
] }
assert.equal(continuousBaseVideoEnd(manyCuts, at(manyCuts, 0), none, frameMs), 4000, 'the first 20 pieces end at a gap')
assert.equal(continuousBaseVideoEnd(manyCuts, at(manyCuts, 2400), none, frameMs), 4000, 'a later piece has the same run end')
assert.equal(continuousBaseVideoEnd(manyCuts, at(manyCuts, 4000), none, frameMs), null, 'a gap has no playing base clip')
assert.equal(continuousBaseVideoEnd(manyCuts, at(manyCuts, 4200), none, frameMs), 8200, 'the second run ends at a source jump')
assert.equal(continuousBaseVideoEnd(manyCuts, at(manyCuts, 8200), none, frameMs), 12200, 'the third run reaches the timeline end')
assert.equal(continuousBaseVideoEnd(null, at(manyCuts, 0), none, frameMs), 200, 'unknown project cannot extend a run')

const speedChange: any = { assets: manyCuts.assets, tracks: [
  { id: 'v1', kind: 'video', clips: [clip('normal', 0, 200), { ...clip('fast', 200, 600), speed: 2 }, clip('normal-again', 600, 800)] },
] }
assert.equal(continuousBaseVideoEnd(speedChange, at(speedChange, 0), none, frameMs), 200, 'speed change ends the run')
assert.equal(continuousBaseVideoEnd(speedChange, at(speedChange, 200), none, frameMs), 400, 'non-1x playback stays inside its clip')
const freezeChange: any = { assets: manyCuts.assets, tracks: [
  { id: 'v1', kind: 'video', clips: [clip('normal', 0, 200), { ...clip('frozen', 200, 400), freeze: { at_ms: 250 } }, clip('after', 400, 600)] },
] }
assert.equal(continuousBaseVideoEnd(freezeChange, at(freezeChange, 0), none, frameMs), 200, 'frozen clip ends the run')
assert.equal(continuousBaseVideoEnd(freezeChange, at(freezeChange, 200), none, frameMs), 400, 'a frozen clip cannot extend its run')
const reverseChange: any = { assets: manyCuts.assets, tracks: [
  { id: 'v1', kind: 'video', clips: [clip('normal', 0, 200), { ...clip('reversed', 200, 400), reverse: true }] },
] }
assert.equal(continuousBaseVideoEnd(reverseChange, at(reverseChange, 0), none, frameMs), 200, 'reverse clip ends the run')

// --- a seek to a clip nobody parked moves the on-screen element (one element as before)
deck = nextDeck(EMPTY_DECK, at(shuffle, 0), next(shuffle, 0), frameMs)
deck = nextDeck(deck, at(shuffle, 3500), next(shuffle, 3500), frameMs)
assert.equal(deck.front, 0)
assert.equal(deck.slots[0]?.video.clipId, 's3')

// --- overlays: the V2 clip is parked before it appears; nothing beyond the horizon or the cap
const upcomingAt = (ms: number) => resolveUpcomingOverlays(issue2, ms, 'v1', none, resolveOverlays(issue2, ms, 'v1', none).overlays)
assert.deepEqual(upcomingAt(26000), [], 'not loaded 4 s ahead')
const middle = upcomingAt(28000)
assert.equal(middle.length, 1)
assert.equal(middle[0].clipId, 'middle')
assert.equal(middle[0].startMs, 30000)
assert.deepEqual(upcomingAt(45000), [], 'an active overlay is not also upcoming')
const full = Array.from({ length: MAX_OVERLAYS }, () => middle[0])
assert.deepEqual(resolveUpcomingOverlays(issue2, 28000, 'v1', none, full), [], 'parking never exceeds the overlay decode cap')
const hidden: any = { ...issue2, tracks: [issue2.tracks[0], { ...issue2.tracks[1], visible: false }] }
assert.deepEqual(resolveUpcomingOverlays(hidden, 28000, 'v1', none, []), [], 'a hidden track is not parked')

// --- which gaps stay on the live stage
assert.equal(liveBaseGap(true, 1, 1), true, 'forward play through a gap stays live')
assert.equal(liveBaseGap(true, 1, 0), true, 'also over an empty gap while playing')
assert.equal(liveBaseGap(true, 0, 1), true, 'paused with an overlay: live (the composed frame is drawn on top), no surface swap at play or pause')
assert.equal(liveBaseGap(true, 0, 0), false, 'paused over an empty gap: the engine frame (and its "no clip" note)')
assert.equal(liveBaseGap(true, -1, 1), false, 'reverse/shuttle keep the seek path and its frames')
assert.equal(liveBaseGap(false, 1, 1), false, 'not a gap')

console.log('PASS preview base deck parks the next clip and swaps it in at the cut')
