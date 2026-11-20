import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { cannotDecodeProblem, isDecodeErrorCode, MediaDecodeHealth, NO_VIDEO_PICTURE } from '../src/panels/Preview/mediaDecodeHealth'
import { createUiObservableState, readPreviewMediaProblem, readUiDomState, type UiStateSource } from '../src/app/uiControlState'

// A system without an H.264 decoder for the webview (Linux without the GStreamer
// avdec_h264 / openh264dec plugin) cannot play any preview proxy. The monitor
// must say so in plain words, and ui.state must carry the same fact. A
// transient element error that a reload fixes must not be reported.

// --- the verdict
assert.equal(isDecodeErrorCode(3), true)
assert.equal(isDecodeErrorCode(4), true)
assert.equal(isDecodeErrorCode(2), false, 'a network error is not "cannot decode"')
let health = new MediaDecodeHealth()
assert.equal(health.failed('/p/a1.mp4', 4), 'retry', 'the first failure is retried with a reload')
assert.equal(health.failed('/p/a1.mp4', 4), 'cannot_decode', 'the same failure after the reload is reported')
assert.equal(health.decoded(), true, 'a decoded frame clears the problem')
assert.equal(health.failed('/p/a1.mp4', 3), 'ignore', 'once any proxy decoded, a later error is transient (the clock recovers it)')

// WebKitGTK without an H.264 decoder may load the proxy and play only its AAC
// track: data, but no picture (videoWidth 0). That counts like a decode error.
health = new MediaDecodeHealth()
assert.equal(health.failed('/p/a1.mp4', NO_VIDEO_PICTURE), 'retry')
assert.equal(health.failed('/p/a1.mp4', NO_VIDEO_PICTURE), 'cannot_decode')

health = new MediaDecodeHealth()
assert.equal(health.failed('/p/a1.mp4', 2), 'ignore')
assert.equal(health.failed('/p/a1.mp4', 3), 'retry')
assert.equal(health.decoded(), true)
assert.equal(health.failed('/p/a1.mp4', 3), 'ignore', 'a reload that then plays is not "cannot decode"')

// --- the words
const linux = cannotDecodeProblem('linux', ['a2', 'a1'])
assert.equal(linux.kind, 'cannot_decode')
assert.deepEqual(linux.assetIds, ['a1', 'a2'])
assert.match(linux.title, /cannot play in the preview/)
assert.match(linux.detail, /gstreamer1\.0-libav \(Debian\/Ubuntu\)/)
assert.match(linux.detail, /gstreamer1-plugin-openh264 \(Fedora\)/)
assert.match(linux.detail, /restart ShellX Cut/)
assert.match(linux.detail, /Rendering and export are not affected/)
const other = cannotDecodeProblem('windows', ['a1'])
assert.doesNotMatch(other.detail, /GStreamer/, 'package names are Linux advice only')

// --- ui.state reads what the monitor renders
const notice = {
  dataset: { cutPreviewMediaProblem: 'cannot_decode', cutPreviewMediaAssets: 'a1 a2' },
  textContent: `${linux.title}${linux.detail}`,
  querySelector: (selector: string) => (selector === '[data-cut-preview-media-problem-title]' ? { textContent: linux.title }
    : selector === '[data-cut-preview-media-problem-detail]' ? { textContent: linux.detail } : null),
}
const doc = (el: unknown) => ({
  documentElement: { dataset: {} },
  querySelector: (selector: string) => (selector === '[data-cut-preview-media-problem]' ? el : null),
}) as unknown as Pick<Document, 'querySelector'>
assert.equal(readPreviewMediaProblem(doc(null)), null)
const read = readPreviewMediaProblem(doc(notice))
assert.deepEqual(read?.asset_ids, ['a1', 'a2'])
assert.equal(read?.kind, 'cannot_decode')
assert.match(read?.message ?? '', /gstreamer1\.0-libav/)
assert.equal(read?.message, `${linux.title}. ${linux.detail}`, 'title and detail stay separate words')
const observable = createUiObservableState({
  revision: 1,
  layout: { workspaceMode: 'edit', leftCollapsed: false, leftTab: 'assets', findSurface: 'find-media', railCollapsed: false, railPinned: false, rightTab: 'properties' },
  generateTab: 'templates', wizardOpen: false, envOpen: false, envCategory: 'about', commentsOpen: false,
  activeDrawer: null, highlight: null, playheadMs: 0, selectedClipIds: [], exportRange: null, project: null,
  dom: readUiDomState(doc(notice)),
} as unknown as UiStateSource)
assert.deepEqual(observable.preview.media_problem, read, 'ui.state carries the same fact as the monitor')

// --- wiring: proxy errors reach the verdict, the monitor renders the notice
const deck = readFileSync(new URL('../src/panels/Preview/BaseVideoDeck.tsx', import.meta.url), 'utf8')
const layers = readFileSync(new URL('../src/panels/Preview/PreviewLayers.tsx', import.meta.url), 'utf8')
const preview = readFileSync(new URL('../src/panels/Preview/index.tsx', import.meta.url), 'utf8')
assert.match(deck, /video\.kind === 'proxy'\) \{ decode\.onDecodeError\(e\.currentTarget, video\.assetId\)/)
assert.match(deck, /onLoadedData=\{\(e\) => \{ if \(video\?\.kind === 'proxy'\) decode\.onDecoded\(e\.currentTarget, video\.assetId\) \}\}/)
assert.match(layers, /decode\?\.onDecodeError\(e\.currentTarget, layer\.assetId\)/)
assert.match(preview, /<PreviewDecodeNotice problem=\{decodeProblem\} \/>/)

console.log('PASS preview names a system that cannot decode its video')
