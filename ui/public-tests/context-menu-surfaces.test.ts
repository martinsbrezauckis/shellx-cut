// Focused UX-CONTEXT-01 contract: each new context surface has one exact
// target, and class-invalid/ambiguous mutations are never routed optimistically.
import { strict as assert } from 'node:assert'
import { readFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import {
  gapFillState,
  isFiniteTimelineContextPoint,
  removeTrackState,
  resolveTimelineContextTarget,
} from '../src/panels/Timeline/TimelineSurfaceMenuModel'
import { parseSpeedFactor, speedFactorReason } from '../src/panels/Timeline/speedFactor'
import TimelineSurfaceContextMenu from '../src/panels/Timeline/TimelineSurfaceContextMenu'
import TimelineTrackContextMenu from '../src/panels/Timeline/TimelineTrackContextMenu'
import { LibraryContextMenus } from '../src/panels/Library/LibraryContextMenus'
import type { Track } from '../src/lib/client'
import type { LaidItem } from '../src/panels/Timeline/layout'

const tracks = [
  { id: 'v1', kind: 'video', clips: [] },
  { id: 'a1', kind: 'audio', clips: [] },
  { id: 'v2', kind: 'video', clips: [] },
  { id: 'cap1', kind: 'caption', clips: [] },
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
  assert.deepEqual(
    resolveTimelineContextTarget({ itemId: lockedMedia.id, gapId: null, trackId: 'locked-v', headerTrackId: 'locked-v', x: 8, y: 9, atMs: 10, items, tracks }),
    { kind: 'track', x: 8, y: 9, trackId: 'locked-v' },
    'the track header owns its own context menu even when the track is locked',
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

// Track headers own discrete track actions. The base tracks cannot be removed;
// visibility/mute/solo stay compact and no mixer or reorder controls leak in.
{
  const project = { tracks } as unknown as import('../src/lib/client').Project
  const common = { project, allItems: items, onSelect: () => {}, onRemoveTrack: () => {}, onClose: () => {} }
  const renderTrack = (trackId: string) => renderToStaticMarkup(createElement(TimelineTrackContextMenu, {
    ...common, menu: { kind: 'track', x: 20, y: 30, trackId },
  }))
  const baseVideo = renderTrack('v1')
  assert.match(baseVideo, /data-cut-track-ctx="lock"/, 'a header exposes track lock')
  assert.match(baseVideo, /data-cut-track-ctx="visibility"/, 'video headers own visibility')
  assert.doesNotMatch(baseVideo, /data-cut-track-ctx="remove"/, 'base video removal stays hidden, not merely disabled')
  assert.doesNotMatch(baseVideo, /data-cut-track-ctx="mute"|data-cut-track-ctx="solo"/, 'video headers do not acquire audio controls')

  const overlay = renderTrack('v2')
  assert.match(overlay, /data-cut-track-ctx="remove"/, 'non-base tracks own their confirmed removal action')

  const audio = renderTrack('a1')
  assert.match(audio, /data-cut-track-ctx="mute"/, 'audio headers own mute')
  assert.match(audio, /data-cut-track-ctx="solo"/, 'audio headers own solo')
  assert.doesNotMatch(audio, /data-cut-track-ctx="visibility"/, 'audio headers do not acquire video visibility')
  assert.doesNotMatch(audio, /mixer|reorder/i, 'dense mixer and reorder controls remain outside the header menu')

  const captions = renderTrack('cap1')
  assert.match(captions, /data-cut-track-ctx="visibility"/, 'caption headers retain their existing visibility control')
}

// Contextual paste binds to the resolved timeline lane and position. A mismatch
// stays visibly disabled with an accessible reason instead of falling back to
// the playhead.
{
  const project = { tracks } as unknown as import('../src/lib/client').Project
  const common = {
    project,
    allItems: items,
    durationMs: 8_000,
    onSeek: () => {},
    onExportRange: () => {},
    onAddTrack: () => {},
    onPasteAt: () => {},
    onClose: () => {},
  }
  const pasteAtVideo = renderToStaticMarkup(createElement(TimelineSurfaceContextMenu, {
    ...common,
    menu: { kind: 'empty', x: 40, y: 50, atMs: 1_250, trackId: 'v1' },
    clipboardClipId: media.id,
    clipboardKind: 'video',
  }))
  assert.match(pasteAtVideo, /role="menu"/, 'the shared menu frame renders semantic menu ownership')
  assert.match(pasteAtVideo, /data-cut-timeline-ctx="empty-paste"/, 'empty lanes expose a target-owned paste action')
  assert.match(pasteAtVideo, /Paste copied clip here/, 'paste copy describes its local target instead of the playhead')
  assert.doesNotMatch(pasteAtVideo, /data-cut-ctx="paste"/, 'clip menus no longer own silent playhead paste')

  const mismatch = renderToStaticMarkup(createElement(TimelineSurfaceContextMenu, {
    ...common,
    menu: { kind: 'empty', x: 40, y: 50, atMs: 1_250, trackId: 'v1' },
    clipboardClipId: media.id,
    clipboardKind: 'audio',
  }))
  assert.match(mismatch, /data-cut-timeline-ctx="empty-paste"[^>]*disabled/, 'cross-kind timeline paste fails closed')
  assert.match(mismatch, /aria-description="The copied audio clip needs an audio track"/, 'disabled paste exposes its reason to assistive technology')
}

// Library cards use the same focused, clamped menu frame. Their context menu
// owns library placement and recovery rather than borrowing clip-menu verbs.
{
  const missingLibraryItem = {
    id: 'library-missing',
    name: 'Moved interview.mov',
    type: 'video',
    favorite: false,
    tags: [],
    media_ok: false,
    src_path: '/old/location/interview.mov',
  } as never
  const html = renderToStaticMarkup(createElement(LibraryContextMenus, {
    folderMenu: null,
    cardMenu: { x: 12, y: 18, id: 'library-missing' },
    cardMenuItem: missingLibraryItem,
    hasProject: false,
    busy: null,
    folders: ['Interviews'],
    onCloseFolderMenu: () => {},
    onCloseCardMenu: () => {},
    onStartRename: () => {},
    onRemoveFolder: () => {},
    onAddToProject: () => {},
    onInsertAtPlayhead: () => {},
    onMoveTo: () => {},
    onToggleFavorite: () => {},
    onEditTags: () => {},
    onRelink: () => {},
    onMakePortable: () => {},
    onRemove: () => {},
  }))
  assert.match(html, /data-cut-library-card-menu/, 'Library card menu renders through the shared frame')
  assert.match(html, /data-cut-library-card-ctx="insert"[^>]*disabled[^>]*aria-description="Open a project first"/, 'Library insertion stays disabled with an accessible project precondition')
  assert.match(html, /data-cut-library-card-ctx="move"/, 'Library card owns a compact move-to-folder parent')
  assert.match(html, /data-cut-library-card-ctx="relink"/, 'missing external Library media owns relink')
  assert.doesNotMatch(html, /data-cut-library-card-move="/, 'folder choices stay collapsed until Move to folder opens')
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
  ['src/panels/Timeline/TimelineTrackContextMenu.tsx', 'data-cut-track-menu'],
  ['src/panels/Timeline/TimelineTrackContextMenu.tsx', 'data-cut-locked-track-menu'],
  ['src/panels/Preview/PreviewContextMenu.tsx', 'data-cut-preview-menu'],
  ['src/panels/Assets/AssetContextMenu.tsx', 'data-cut-asset-menu'],
  ['src/panels/Projects/ProjectContextMenu.tsx', 'data-cut-project-menu'],
] as const) {
  assert.match(read(file), new RegExp(selector), `${file} declares ${selector}`)
}
const previewContextMenu = read('src/panels/Preview/PreviewContextMenu.tsx')
const previewCss = read('src/panels/Preview/preview.css')
assert.match(
  previewContextMenu,
  /monitor\.addEventListener\('contextmenu', onContextMenu\)/,
  'the Preview monitor owns the native context gesture at its stable parent',
)
assert.match(
  previewContextMenu,
  /data-cut-preview-menu-button[\s\S]*aria-haspopup="menu"/,
  'Preview exposes the same menu through a visible keyboard-focusable button',
)
assert.match(
  previewCss,
  /\.pv-stage\s*>\s*\.pv-base\s*,\s*\.pv-stage\s*>\s*img\[data-cut-poster\]\s*\{[^}]*pointer-events\s*:\s*none/s,
  'passive base video and poster cannot consume the monitor-center context gesture',
)
assert.match(
  previewCss,
  /\.pv-xform\s*\{[^}]*pointer-events\s*:\s*auto/s,
  'selected transform handles remain intentionally interactive above the passive base media',
)
assert.match(
  previewCss,
  /\.pv-redact-capture\s*\{[^}]*pointer-events\s*:\s*auto/s,
  'the armed redact capture layer remains intentionally interactive above the passive base media',
)
assert.doesNotMatch(read('src/panels/Timeline/ClipContextMenu.tsx'), /window\.prompt/, 'custom speed is native UI, not window.prompt')
assert.match(read('src/panels/Timeline/CustomSpeedMenuEditor.tsx'), /step="any"/, 'custom speed does not invent a stricter UI step')
assert.match(read('src/components/ContextMenuFrame.tsx'), /event\.key (?:!==|===) 'Escape'/, 'new menus preserve keyboard dismissal')
assert.match(read('src/components/ContextMenuFrame.tsx'), /tabIndex=\{-1\}/, 'new menus take keyboard focus for menu navigation')
assert.match(read('src/components/ContextMenuFrame.tsx'), /ArrowDown[\s\S]*ArrowUp[\s\S]*Home[\s\S]*End/, 'the shared frame provides practical menu-key navigation')
assert.match(read('src/panels/Timeline/useTimelineContextMenus.ts'), /document\.elementsFromPoint/, 'gap ownership survives an overlapping trim affordance')
assert.match(read('src/panels/Timeline/useTimelineContextMenus.ts'), /isFiniteTimelineContextPoint\(event\.clientX, event\.clientY\)/, 'non-finite coordinates are refused before DOM hit-testing')
assert.match(read('src/panels/Timeline/TimelineTrackRow.tsx'), /data-cut-track-header[\s\S]*onKeyDown=\{openKeyboardMenu\}/, 'track headers support Menu and Shift+F10 keyboard entry')
assert.match(read('src/panels/Timeline/TimelineTrackRow.tsx'), /data-cut-track-menu-button[\s\S]*aria-haspopup="menu"/, 'track headers expose a visible menu button when native context input is unavailable')
assert.match(read('src/panels/Timeline/ClipView.tsx'), /onKeyDown=\{openKeyboardMenu\}/, 'clip menus support Menu and Shift+F10 keyboard entry')
assert.match(read('src/panels/Library/LibraryActions.tsx'), /data-cut-library-menu-button[\s\S]*aria-haspopup="menu"/, 'Library items expose a visible menu button')
assert.match(read('src/panels/Assets/index.tsx'), /data-cut-asset-menu-button[\s\S]*aria-haspopup="menu"/, 'Assets expose a visible menu button')
assert.match(read('src/panels/Timeline/index.tsx'), /TimelineContextMenuLayer/, 'Timeline delegates context-menu rendering to its bounded owner')
assert.match(read('src/panels/Timeline/TimelineContextMenuLayer.tsx'), /ClipContextMenu[\s\S]*TimelineSurfaceContextMenu[\s\S]*TimelineTrackContextMenu/, 'the bounded owner preserves every timeline context surface')
assert.match(read('src/panels/Library/index.tsx'), /LibraryContextMenuLayer/, 'Library delegates context-menu wiring to its bounded owner')
assert.match(read('src/panels/Library/LibraryContextMenuLayer.tsx'), /LibraryContextMenus/, 'the bounded Library layer preserves its established menu owner')
assert.match(read('src/panels/Library/LibraryContextMenus.tsx'), /data-cut-library-card-move[\s\S]*No folder/, 'Library cards expose their own compact move-to-folder submenu')
for (const file of ['src/panels/Library/LibraryCard.tsx', 'src/panels/Library/LibraryRow.tsx', 'src/panels/Library/LibraryFolders.tsx']) {
  const source = read(file)
  assert.match(source, /ContextMenu/, `${file} supports the Menu key`)
  assert.match(source, /Shift\+F10/, `${file} supports Shift+F10 keyboard entry`)
}

console.log('PASS context-menu surfaces: exact targets, locked/gap refusal, and engine-speed bounds')
