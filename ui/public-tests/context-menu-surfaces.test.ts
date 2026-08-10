// Focused UX-CONTEXT-01 contract: each new context surface has one exact
// target, and class-invalid/ambiguous mutations are never routed optimistically.
import { strict as assert } from 'node:assert'
import { readFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import {
  gapFillState,
  isFiniteTimelineContextPoint,
  removeTrackState,
  resolveTimelineContextTarget,
} from '../src/panels/Timeline/TimelineSurfaceMenuModel'
import { parseSpeedFactor, speedFactorReason } from '../src/panels/Timeline/speedFactor'
import type { Track } from '../src/lib/client'
import type { LaidItem } from '../src/panels/Timeline/layout'

const tracks = [
  { id: 'v1', kind: 'video', clips: [] },
  { id: 'a1', kind: 'audio', clips: [] },
  { id: 'v2', kind: 'video', clips: [] },
  { id: 'locked-v', kind: 'video', locked: true, clips: [] },
] as unknown as Track[]

function item(id: string, kind: LaidItem['kind'], trackId: string, startMs: number, durMs: number): LaidItem {
  return {
    id,
    kind,
    contentClass: kind,
    trackId,
    startMs,
    editorialStartMs: startMs,
    durMs,
    label: id,
    ...(kind === 'video' || kind === 'audio' ? { asset: `${id}.asset`, srcInMs: 0, srcOutMs: durMs } : {}),
  }
}

const media = item('video-1', 'video', 'v1', 0, 1000)
const gap = item('gap-v1-1000', 'gap', 'v1', 1000, 1000)
const lockedMedia = item('locked-video', 'video', 'locked-v', 0, 1000)
const items = [media, gap, lockedMedia]

// Malformed automation/browser coordinates fail closed before DOM hit-testing;
// they must never reach elementsFromPoint or timeline arithmetic.
{
  assert.equal(isFiniteTimelineContextPoint(20, 30), true)
  assert.equal(isFiniteTimelineContextPoint(Number.NaN, 30), false)
  assert.equal(isFiniteTimelineContextPoint(20, Number.POSITIVE_INFINITY), false)
}

// Target resolution is mutually exclusive. Locked wins, and a stale/mismatched
// DOM claim is refused instead of falling back to a track or unrelated clip.
{
  assert.deepEqual(
    resolveTimelineContextTarget({ itemId: media.id, gapId: null, trackId: 'v1', x: 1, y: 2, atMs: 10, items, tracks }),
    { kind: 'clip', itemId: media.id },
  )
  assert.deepEqual(
    resolveTimelineContextTarget({ itemId: null, gapId: gap.id, trackId: 'v1', x: 3, y: 4, atMs: 1001, items, tracks }),
    { kind: 'gap', x: 3, y: 4, itemId: gap.id },
  )
  assert.deepEqual(
    resolveTimelineContextTarget({ itemId: lockedMedia.id, gapId: null, trackId: 'locked-v', x: 5, y: 6, atMs: 7, items, tracks }),
    { kind: 'locked', x: 5, y: 6, trackId: 'locked-v', itemId: lockedMedia.id, atMs: 7 },
    'locked-track ownership beats the contained media context menu',
  )
  assert.equal(
    resolveTimelineContextTarget({ itemId: 'stale-clip', gapId: null, trackId: 'v1', x: 0, y: 0, atMs: 0, items, tracks }).kind,
    'none',
    'stale clip identity is refused instead of becoming an empty-track mutation',
  )
  assert.equal(
    resolveTimelineContextTarget({ itemId: media.id, gapId: gap.id, trackId: 'v1', x: 0, y: 0, atMs: 0, items, tracks }).kind,
    'none',
    'contradictory clip and gap identity is refused',
  )
  assert.equal(
    resolveTimelineContextTarget({ itemId: media.id, gapId: null, trackId: 'a1', x: 0, y: 0, atMs: 0, items, tracks }).kind,
    'none',
    'a clip cannot be dispatched through a mismatched track owner',
  )
}

// Gaps only admit a copied clip of the same kind and only where the existing
// fit-to-fill engine constraint (source span / gap span) lies in 0.25–4×.
{
  const targetTrack = tracks[0]
  const atMin = item('source-min', 'video', 'v1', 0, 250)
  const atMax = item('source-max', 'video', 'v1', 0, 4000)
  const tooSlow = item('source-too-slow', 'video', 'v1', 0, 249)
  const tooFast = item('source-too-fast', 'video', 'v1', 0, 4001)
  const audio = item('source-audio', 'audio', 'a1', 0, 1000)
  assert.equal(gapFillState(gap, targetTrack, atMin).enabled, true, '0.25× is a valid fit boundary')
  assert.equal(gapFillState(gap, targetTrack, atMax).enabled, true, '4× is a valid fit boundary')
  assert.equal(gapFillState(gap, targetTrack, tooSlow).enabled, false, 'a factor below 0.25× stays disabled')
  assert.equal(gapFillState(gap, targetTrack, tooFast).enabled, false, 'a factor above 4× stays disabled')
  assert.equal(gapFillState(gap, targetTrack, audio).enabled, false, 'an audio clipboard item cannot fill a video gap')
  assert.equal(gapFillState(gap, targetTrack, null).enabled, false, 'a missing/stale clipboard identity cannot dispatch')
}

// The first video/audio tracks are project structure, not destructive targets.
{
  assert.equal(removeTrackState(tracks[0], tracks).enabled, false, 'base video remove stays disabled')
  assert.equal(removeTrackState(tracks[1], tracks).enabled, false, 'base audio remove stays disabled')
  assert.equal(removeTrackState(tracks[2], tracks).enabled, true, 'overlay video may be removed after confirmation')
}

// Custom speed preserves the engine's actual numeric window: there is no
// menu-only step rounding that would turn a valid engine factor into another.
{
  assert.equal(parseSpeedFactor(0.25), 0.25)
  assert.equal(parseSpeedFactor(4), 4)
  assert.equal(parseSpeedFactor('0.251'), 0.251, 'valid precision is not UI-rounded')
  assert.equal(parseSpeedFactor('0.249'), null)
  assert.equal(parseSpeedFactor('4.001'), null)
  assert.equal(parseSpeedFactor('wat'), null)
  assert.equal(speedFactorReason('0.249') !== null, true)
}

// Lightweight source contracts keep every visible context route declared and
// prohibit the retired prompt-based speed path.
const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const read = (relative: string) => readFileSync(resolve(root, relative), 'utf8')
for (const [file, selector] of [
  ['src/panels/Timeline/TimelineSurfaceContextMenu.tsx', 'data-cut-timeline-empty-menu'],
  ['src/panels/Timeline/TimelineSurfaceContextMenu.tsx', 'data-cut-gap-menu'],
  ['src/panels/Timeline/TimelineSurfaceContextMenu.tsx', 'data-cut-locked-track-menu'],
  ['src/panels/Preview/PreviewContextMenu.tsx', 'data-cut-preview-menu'],
  ['src/panels/Assets/AssetContextMenu.tsx', 'data-cut-asset-menu'],
  ['src/panels/Projects/ProjectContextMenu.tsx', 'data-cut-project-menu'],
] as const) {
  assert.match(read(file), new RegExp(selector), `${file} declares ${selector}`)
}
assert.doesNotMatch(read('src/panels/Timeline/ClipContextMenu.tsx'), /window\.prompt/, 'custom speed is native UI, not window.prompt')
assert.match(read('src/panels/Timeline/CustomSpeedMenuEditor.tsx'), /step="any"/, 'custom speed does not invent a stricter UI step')
assert.match(read('src/components/ContextMenuFrame.tsx'), /event\.key (?:!==|===) 'Escape'/, 'new menus preserve keyboard dismissal')
assert.match(read('src/components/ContextMenuFrame.tsx'), /tabIndex=\{-1\}/, 'new menus take keyboard focus for menu navigation')
assert.match(read('src/panels/Timeline/useTimelineContextMenus.ts'), /document\.elementsFromPoint/, 'gap ownership survives an overlapping trim affordance')
assert.match(read('src/panels/Timeline/useTimelineContextMenus.ts'), /isFiniteTimelineContextPoint\(event\.clientX, event\.clientY\)/, 'non-finite coordinates are refused before DOM hit-testing')

console.log('PASS context-menu surfaces: exact targets, locked/gap refusal, and engine-speed bounds')
