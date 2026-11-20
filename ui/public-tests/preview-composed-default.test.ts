import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { loadComposedPreference } from '../src/panels/Preview/usePreviewViewOptions'
import { composedFrameOnTop, shouldUseLivePreviewSurface } from '../src/panels/Preview/composite'
import { HIDDEN_ELEMENT_WORDS, hiddenInLivePreview, livePreviewNotice } from '../src/panels/Preview/livePreviewCoverage'
import { COMPOSED_REST_MS } from '../src/panels/Preview/ComposedFrameOverlay'
import { createUiObservableState, readPreviewView, readUiDomState, type UiStateSource } from '../src/app/uiControlState'

// Item 147 (owner decision 2026-11-20): the preview shows the composited
// program picture by default, as large editors do. Paused, the exact composed
// frame is drawn over the still-mounted live preview; playing, the live
// preview shows and SAYS what it leaves out at that point; ui.state says what
// the monitor shows; the person's OFF is remembered.

// --- default ON, the person's OFF remembered ---------------------------------
const store = (value: string | null) => ({ getItem: () => value })
assert.equal(loadComposedPreference(store(null)), true, 'a fresh install shows the composed picture')
assert.equal(loadComposedPreference(store('on')), true)
assert.equal(loadComposedPreference(store('off')), false, 'the person turned it off: remembered')
assert.equal(loadComposedPreference({ getItem: () => { throw new Error('blocked') } }), true, 'no storage: default ON')
assert.equal(loadComposedPreference(null), true)

// --- the live stage is never unmounted for the exact frame --------------------
assert.equal(shouldUseLivePreviewSurface(true), true)
assert.equal(composedFrameOnTop(true, 0, true), true, 'paused + composed: exact frame on top')
assert.equal(composedFrameOnTop(true, 1, true), false, 'playing: the live preview')
assert.equal(composedFrameOnTop(false, 0, true), false, 'OFF: live preview only')
assert.ok(COMPOSED_REST_MS >= 100 && COMPOSED_REST_MS <= 250, 'requested once the playhead rests, not per scrub step')

// --- what the live preview leaves out ------------------------------------------
const media = (id: string, a: number, b: number, extra: Record<string, unknown> = {}) => ({ id, asset: 'a', src_in_ms: a, src_out_ms: b, ...extra })
const project = (v1: unknown[], extraTracks: unknown[] = [], extra: Record<string, unknown> = {}): any => ({
  assets: { a: { path: '/m/a.mp4', proxy: 'proxies/a.mp4' }, t: { path: '/p/titles/title_ab12.mov' } },
  tracks: [{ id: 'v1', kind: 'video', clips: v1 }, ...extraTracks],
  ...extra,
})
const plain = project([media('c1', 0, 10000)])
assert.deepEqual(hiddenInLivePreview(plain, 5000), [], 'a plain clip: nothing hidden, no notice')
assert.equal(livePreviewNotice([], true), null)
const titled = project([media('c1', 0, 10000)], [{ id: 'title1', kind: 'video', clips: [{ kind: 'gap', duration_ms: 2000 }, { id: 't1', asset: 't', src_in_ms: 0, src_out_ms: 3000 }] }])
assert.deepEqual(hiddenInLivePreview(titled, 1000), [], 'before the title: nothing hidden')
assert.deepEqual(hiddenInLivePreview(titled, 3000), ['titles'], 'a title overlay movie is not drawn live')
assert.deepEqual(hiddenInLivePreview(titled, 5500), [], 'after the title')
assert.deepEqual(hiddenInLivePreview(titled, 3000, undefined, new Set(['t1'])), [], 'a title movie this webview plays (WebKitGTK) is drawn live: not reported')
const faded = project([media('c1', 0, 10000, { fade: { in_ms: 1000, out_ms: 2000 } })])
assert.deepEqual(hiddenInLivePreview(faded, 500), ['fades'])
assert.deepEqual(hiddenInLivePreview(faded, 5000), [], 'the middle of a faded clip is drawn as it is')
assert.deepEqual(hiddenInLivePreview(faded, 9000), ['fades'])
const rich = project([media('c1', 0, 4000), media('c2', 4000, 8000, {
  xfade_in_ms: 500, effects: [{ type: 'blur', radius: 4 }], animation: { kind: 'kenburns' }, grade: { contrast: 1, brightness: 0, saturation: 1, gamma: 1.2 },
})], [], { adjustments: [{ range_ms: [0, 2000], grade: null, effects: [] }] })
assert.deepEqual(hiddenInLivePreview(rich, 1000), ['adjustments'])
assert.deepEqual(hiddenInLivePreview(rich, 3600), ['transitions', 'effects', 'motion', 'color'], 'crossfade pulls the clip start back into 3.5..4 s')
const cssGrade = project([media('c1', 0, 4000, { grade: { contrast: 1.2, brightness: 0.1, saturation: 1, gamma: 1 } })])
assert.deepEqual(hiddenInLivePreview(cssGrade, 1000), [], 'contrast/brightness/saturation are drawn live (CSS approximation)')
const hiddenTrack = project([media('c1', 0, 10000)], [{ id: 'title1', kind: 'video', visible: false, clips: [{ id: 't1', asset: 't', src_in_ms: 0, src_out_ms: 3000 }] }])
assert.deepEqual(hiddenInLivePreview(hiddenTrack, 1000), [], 'a hidden track hides nothing')
assert.equal(livePreviewNotice(['titles', 'fades'], true), 'Playback preview: titles and fades appear when paused')
assert.equal(livePreviewNotice(['titles'], false), 'Titles are not shown. Turn COMPOSED on to see them')
assert.equal(Object.keys(HIDDEN_ELEMENT_WORDS).length, 7)

// --- ui.state carries the mode and what the monitor shows ----------------------
const el = (dataset: Record<string, string>) => ({ dataset })
const doc = (root: unknown, frame: unknown) => ({
  documentElement: { dataset: {} },
  querySelector: (selector: string) => selector.startsWith('[data-cut-panel="preview"]') ? root
    : selector === '[data-cut-composed-frame-state]' ? frame : null,
}) as unknown as Pick<Document, 'querySelector'>
assert.equal(readPreviewView(doc(null, null)), null, 'no monitor: no preview view')
const paused = readPreviewView(doc(
  el({ cutPreviewMode: 'composed', cutPreviewShowing: 'composed_frame', cutPreviewHidden: '', cutPreviewMedia: 'proxy' }),
  el({ cutComposedFrameState: 'shown', cutComposedFrameAt: '3000', cutComposedFrameLatency: '412', cutComposedFrameRequests: '2' }),
))
assert.deepEqual(paused, { mode: 'composed', showing: 'composed_frame', hidden_elements: [], media: 'proxy', composed_frame: { state: 'shown', at_ms: 3000, reason: null, latency_ms: 412, requests: 2 } })
const playing = readPreviewView(doc(
  el({ cutPreviewMode: 'composed', cutPreviewShowing: 'live_preview', cutPreviewHidden: 'titles fades' }),
  el({ cutComposedFrameState: 'off', cutComposedFrameRequests: '2' }),
))
assert.deepEqual(playing?.hidden_elements, ['titles', 'fades'], 'playing over a title: the state says what is missing')
assert.equal(playing?.media, null, 'no media attribute: not reported as proxy or original')
const failed = readPreviewView(doc(
  el({ cutPreviewMode: 'composed', cutPreviewShowing: 'live_preview', cutPreviewHidden: 'titles' }),
  el({ cutComposedFrameState: 'failed', cutComposedFrameAt: '3000', cutComposedFrameError: 'ffmpeg exited with exit status: 1', cutComposedFrameRequests: '1' }),
))
assert.equal(failed?.composed_frame?.state, 'failed')
assert.equal(failed?.composed_frame?.reason, 'ffmpeg exited with exit status: 1', 'a failed exact frame is never silent')
const observable = createUiObservableState({
  revision: 1,
  layout: { workspaceMode: 'edit', leftCollapsed: false, leftTab: 'assets', findSurface: 'find-media', railCollapsed: false, railPinned: false, rightTab: 'properties' },
  generateTab: 'templates', wizardOpen: false, envOpen: false, envCategory: 'about', commentsOpen: false,
  activeDrawer: null, highlight: null, playheadMs: 0, selectedClipIds: [], exportRange: null, project: null,
  dom: readUiDomState(doc(
    el({ cutPreviewMode: 'live', cutPreviewShowing: 'live_preview', cutPreviewHidden: 'titles' }),
    el({ cutComposedFrameState: 'off', cutComposedFrameRequests: '0' }),
  )),
} as unknown as UiStateSource)
assert.equal(observable.preview.mode, 'live')
assert.equal(observable.preview.showing, 'live_preview')
assert.deepEqual(observable.preview.hidden_elements, ['titles'])
assert.equal(observable.preview.media_problem, null)

// --- wiring ----------------------------------------------------------------------
const preview = readFileSync(new URL('../src/panels/Preview/index.tsx', import.meta.url), 'utf8')
const overlay = readFileSync(new URL('../src/panels/Preview/ComposedFrameOverlay.tsx', import.meta.url), 'utf8')
const controller = readFileSync(new URL('../src/app/useUiCommandController.ts', import.meta.url), 'utf8')
assert.match(preview, /<ComposedFrameOverlay enabled=\{composedOnTop\}/, 'the exact frame is drawn inside the live stage')
assert.match(preview, /data-cut-preview-mode=\{composed \? 'composed' : 'live'\}/)
assert.match(preview, /data-cut-preview-media=\{liveBaseVideo \? \(liveBaseVideo\.kind === 'proxy' \? 'proxy' : 'original'\) : ''\}/, 'ui.state.preview.media says which file the live preview plays')
assert.match(preview, /data-cut-preview-showing=\{exactOnScreen \? 'composed_frame'/, 'composed_frame is reported only while the exact frame is on screen (not while waited for or failed)')
assert.doesNotMatch(preview, /useState\(false\)\s*\n\s*\/\/ #193b/, 'COMPOSED is no longer a session flag that starts OFF')
assert.match(overlay, /data-cut-poster\s+data-cut-composed-frame/, 'the exact frame keeps the poster selector')
assert.match(overlay, /setShownKey\(null\)/, 'a new position or edit drops the old picture at once')
assert.match(overlay, /data-cut-composed-frame-failure/, 'a failure is said on the picture')
assert.match(controller, /case 'ui\.preview':/)
assert.match(readFileSync(new URL('../src/app/previewModeCommand.ts', import.meta.url), 'utf8'), /current\.preview\?\.mode === mode/, 'ui.preview is confirmed by the committed preview mode')

console.log('PASS preview shows the composed picture by default and says what playback leaves out')
