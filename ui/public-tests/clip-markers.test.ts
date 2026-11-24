// ui/public-tests/clip-markers.test.ts — ClipView clip marker ticks
// (Premiere/Resolve source-time clip markers across trim, split, speed, reverse).

import { strict as assert } from 'node:assert'
import { readFileSync } from 'node:fs'
import { register } from 'node:module'
import { resolve } from 'node:path'

register('./lib/css-module-loader.mjs', import.meta.url)

const { createElement } = await import('react')
const { renderToStaticMarkup } = await import('react-dom/server')
const { OfflineMediaContext } = await import('../src/app/OfflineMediaContext')
const { default: ClipView } = await import('../src/panels/Timeline/ClipView')
const { layoutTrack } = await import('../src/panels/Timeline/layout')
type Project = import('../src/lib/client').Project
type LaidItem = import('../src/panels/Timeline/layout').LaidItem

const uiRoot = resolve(import.meta.dirname, '..')

function renderClip(item: LaidItem, zoom = 1): string {
  return renderToStaticMarkup(
    createElement(
      OfflineMediaContext.Provider,
      { value: { offlineAssets: new Set<string>(), relinkingAssets: new Set<string>(), onRelinkAsset: async () => {} } },
      createElement(ClipView, {
        item,
        zoom,
        fps: 30,
        selected: false,
        dragging: false,
        locked: false,
        onClipDown: () => {},
        onRelinkAsset: async () => {},
      }),
    ),
  )
}

function markerAttrs(html: string): Array<{ id: string; title: string; leftPx: number }> {
  const matches = [...html.matchAll(/<span[^>]*data-cut-clip-marker="([^"]+)"[^>]*>/g)]
  return matches.map((m) => {
    const tag = m[0]
    const title = tag.match(/title="([^"]*)"/)?.[1] ?? ''
    const left = tag.match(/left:\s*([0-9.]+)px/)?.[1]
    return {
      id: m[1],
      title,
      leftPx: left !== undefined ? Number(left) : Number.NaN,
    }
  })
}

const markers = [
  { id: 'cm1', source_ms: 1_000, label: 'First mark' },
  { id: 'cm2', source_ms: 4_000, label: 'After pause' },
]

// 1. layoutTrack propagates clip markers onto LaidItem.
const project = {
  tracks: [{
    id: 'v1',
    kind: 'video',
    clips: [{
      id: 'c1',
      asset: 'rec-asset',
      src_in_ms: 0,
      src_out_ms: 6_000,
      markers,
    }],
  }],
} as unknown as Project

const laid = layoutTrack(project.tracks[0])
assert.equal(laid.length, 1)
assert.deepEqual((laid[0] as any).markers, markers, 'layoutTrack must propagate clip.markers')

// 2. Unedited recording clip renders 2 ticks at zoom=1 (50px/s -> 50px and 200px).
const fullHtml = renderClip(laid[0], 1)
const fullTicks = markerAttrs(fullHtml)
assert.deepEqual(
  fullTicks.map((t) => ({ id: t.id, title: t.title })),
  [
    { id: 'cm1', title: 'First mark' },
    { id: 'cm2', title: 'After pause' },
  ],
)
assert.ok(Math.abs(fullTicks[0].leftPx - 50) < 0.5, `expected ~50px, got ${fullTicks[0].leftPx}`)
assert.ok(Math.abs(fullTicks[1].leftPx - 200) < 0.5, `expected ~200px, got ${fullTicks[1].leftPx}`)

// 3. Trim keeps out-of-range marker hidden and widening restores it.
const trimmedItem: LaidItem = { ...laid[0], srcInMs: 2_000, srcOutMs: 6_000, durMs: 4_000 }
const trimmedTicks = markerAttrs(renderClip(trimmedItem, 1))
assert.equal(trimmedTicks.length, 1)
assert.equal(trimmedTicks[0].id, 'cm2')
assert.ok(Math.abs(trimmedTicks[0].leftPx - 100) < 0.5, `expected (4000-2000)ms at 50px/s = 100px, got ${trimmedTicks[0].leftPx}`)

// 4. Split at 2500ms: left draws cm1 (50px), right draws cm2 (75px).
const leftItem: LaidItem = { ...laid[0], srcInMs: 0, srcOutMs: 2_500, durMs: 2_500 }
const rightItem: LaidItem = { ...laid[0], id: 'c2', startMs: 2_500, srcInMs: 2_500, srcOutMs: 6_000, durMs: 3_500 }
assert.deepEqual(markerAttrs(renderClip(leftItem, 1)).map((t) => t.id), ['cm1'])
const rightTicks = markerAttrs(renderClip(rightItem, 1))
assert.deepEqual(rightTicks.map((t) => t.id), ['cm2'])
assert.ok(Math.abs(rightTicks[0].leftPx - 75) < 0.5)

// 5. Speed 2x & Reverse map source time onto timeline ticks.
const fastItem: LaidItem = { ...laid[0], speed: 2, durMs: 3_000 }
const fastTicks = markerAttrs(renderClip(fastItem, 1))
assert.ok(Math.abs(fastTicks[0].leftPx - 25) < 0.5)
assert.ok(Math.abs(fastTicks[1].leftPx - 100) < 0.5)

const revItem: LaidItem = { ...laid[0], reverse: true, durMs: 6_000 }
const revTicks = markerAttrs(renderClip(revItem, 1))
assert.ok(Math.abs(revTicks[0].leftPx - 250) < 0.5)
assert.ok(Math.abs(revTicks[1].leftPx - 100) < 0.5)

// 6. Both dark and light theme readability CSS & non-blocking click/drag check.
const css = readFileSync(resolve(uiRoot, 'src/panels/Timeline/timeline.css'), 'utf8')
const clipViewSrc = readFileSync(resolve(uiRoot, 'src/panels/Timeline/ClipView.tsx'), 'utf8')
assert.match(css, /\.tl-clip-marker\b/, 'timeline.css must define .tl-clip-marker')
assert.match(css, /\[data-theme=["']light["']\]\s*\.tl-clip-marker/, 'timeline.css must define light-theme .tl-clip-marker readability rule')
assert.doesNotMatch(clipViewSrc, /tl-clip-marker[^>]*onMouseDown/, 'clip marker tick must not intercept mousedown')

console.log('PASS clip-markers unit contract')
