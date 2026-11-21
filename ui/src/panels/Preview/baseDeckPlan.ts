// Two-element "deck" for the base-track preview video.
//
// A media element that is mounted, given a new source or sent to a new place
// at the very moment of a cut has to load and seek before it can show the right
// frame. Some webviews (WebKitGTK on Linux) sometimes never finish that seek,
// freeze or show the file's first frame for a while, so the person sees the
// wrong picture after the cut. The deck removes that dependency: while one
// element (the FRONT) plays the clip under the playhead, the other (the
// STANDBY) is already loaded and parked on the first frame of the next base
// clip, hidden. At the cut the standby becomes the front and only has to play.
// The element that left becomes the standby for the clip after that.
//
// The same decision runs on every webview; a healthy element simply never
// needs the clock's recovery path. This module is pure (no DOM, no React).
import type { Project } from '../../lib/client'
import { baseVideoTrackId } from '../../lib/layerStack'
import { layoutTrack } from '../Timeline/layout'
import { activeVideo, type ActiveVideo } from './model'
import { playbackBindingKey, playbackVideoKey } from './playbackClock'

/** How far ahead of its start the next base clip is loaded and parked. Long
 * enough for a slow first load plus a seek, short enough that scrubbing does
 * not load clips the person will not reach. */
export const PREROLL_MS = 3000

/** The base clip that starts next after `atMs`, within `horizonMs`, as it is
 * at its own first frame (`srcMs` = where its element must be parked). */
export function upcomingBaseVideo(
  project: Project | null, atMs: number, failed: Set<string>, horizonMs = PREROLL_MS,
): ActiveVideo | null {
  if (!project) return null
  const trackId = baseVideoTrackId(project.tracks)
  const track = project.tracks.find((candidate) => candidate.id === trackId)
  if (!track || track.visible === false) return null
  for (const item of layoutTrack(track)) {
    if (item.kind !== 'video' || !item.asset) continue
    if (item.startMs > atMs && item.startMs <= atMs + horizonMs) return activeVideo(project, item.startMs, failed)
    if (item.startMs > atMs + horizonMs) break
  }
  return null
}

/** The playing element can carry on into `next` without being moved: same
 * file and speed, adjacent on the timeline and contiguous in the source
 * (a plain cut inside one recording). Such a cut needs no standby. */
export function continuesInPlace(current: ActiveVideo, next: ActiveVideo, frameMs: number): boolean {
  return current.src === next.src
    && current.speed === next.speed
    && !current.reverse && !next.reverse
    && current.freezeAtMs == null && next.freezeAtMs == null
    && Math.abs(current.startMs + current.durMs - next.startMs) <= 1
    && Math.abs(current.srcOutMs - next.srcInMs) <= frameMs / 2
}

/** Last timeline millisecond covered by the current uninterrupted source run.
 * The clock may follow the presented media position across these cuts, but it
 * must stop at a gap, a source jump, or a clip requiring a different decoder. */
export function continuousBaseVideoEnd(
  project: Project | null, active: ActiveVideo | null, failed: Set<string>, frameMs: number,
): number | null {
  if (!active) return null
  let endMs = active.startMs + active.durMs
  if (!project || active.speed !== 1 || active.reverse || active.freezeAtMs != null) return endMs
  const track = project.tracks.find((candidate) => candidate.id === baseVideoTrackId(project.tracks))
  if (!track || track.visible === false) return endMs
  const items = layoutTrack(track)
  const index = items.findIndex((item) => item.kind === 'video' && item.id === active.clipId && item.startMs === active.startMs)
  if (index < 0) return endMs
  const binding = playbackBindingKey(active, 0)
  let previous = active
  for (const item of items.slice(index + 1)) {
    if (item.kind !== 'video' || !item.asset || Math.abs(item.startMs - endMs) > 1) break
    const next = activeVideo(project, item.startMs, failed)
    if (!next || next.speed !== 1 || playbackBindingKey(next, 0) !== binding || !continuesInPlace(previous, next, frameMs)) break
    endMs = next.startMs + next.durMs
    previous = next
  }
  return endMs
}

/** One element of the deck: the clip it holds and where it waits (source ms). */
export interface DeckSlot {
  key: string
  video: ActiveVideo
  parkMs: number
}

export type SlotIndex = 0 | 1

/** Which clip each of the two elements holds, and which one is on screen
 * (null while the base track has no clip under the playhead). */
export interface DeckState {
  slots: readonly [DeckSlot | null, DeckSlot | null]
  front: SlotIndex | null
}

export const EMPTY_DECK: DeckState = { slots: [null, null], front: null }

const other = (index: SlotIndex): SlotIndex => (index === 0 ? 1 : 0)

function slotFor(video: ActiveVideo): DeckSlot {
  return { key: playbackVideoKey(video) ?? '', video, parkMs: video.srcMs }
}

/** The next deck for the clip under the playhead (`active`) and the clip that
 * starts next (`upcoming`). Stable: a clip keeps the element that already
 * holds it, and the on-screen element keeps playing across a cut that
 * continues in place, exactly like the single element did before. */
export function nextDeck(previous: DeckState, active: ActiveVideo | null, upcoming: ActiveVideo | null, frameMs: number): DeckState {
  const slots: [DeckSlot | null, DeckSlot | null] = [previous.slots[0], previous.slots[1]]
  const holding = (key: string): SlotIndex | null => (slots[0]?.key === key ? 0 : slots[1]?.key === key ? 1 : null)
  let front: SlotIndex | null = null
  if (active) {
    const entry = slotFor(active)
    const upcomingKey = upcoming ? playbackVideoKey(upcoming) : null
    // 1. an element already holds this clip (the parked standby at a cut);
    // 2. otherwise the element on screen takes it (continuing in place, or a
    //    seek elsewhere: it is moved there, as with a single element);
    // 3. otherwise (coming out of a gap or a still) an element that is not
    //    parked for the upcoming clip.
    front = holding(entry.key)
      ?? previous.front
      ?? (slots[0] && slots[0].key === upcomingKey ? 1 : 0)
    // Same key = same file and placement; keep the entry so an unchanged deck
    // stays the same object (srcMs of `active` moves with the playhead).
    if (slots[front]?.key !== entry.key) slots[front] = entry
  }
  if (upcoming && !(active && continuesInPlace(active, upcoming, frameMs))) {
    const entry = slotFor(upcoming)
    const held = holding(entry.key)
    if (held === null || held === front) {
      const target = front === null ? (slots[0] === null || slots[1] !== null ? 0 : 1) : other(front)
      slots[target] = entry
    }
  }
  if (slots[0] === previous.slots[0] && slots[1] === previous.slots[1] && front === previous.front) return previous
  return { slots, front }
}
