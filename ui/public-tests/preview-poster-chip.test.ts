import { strict as assert } from 'node:assert'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import PreviewMonitorBadges from '../src/panels/Preview/PreviewMonitorBadges'
import type { Project } from '../src/lib/client'
import { posterChip, type PosterChipInput } from '../src/panels/Preview/posterChip'

/** One base track: a gap of `lead` ms (when > 0) then media clips back to back. */
function project(clips: Array<{ id: string; asset: string; dur: number }>, lead = 0): Project {
  const laid: unknown[] = []
  if (lead > 0) laid.push({ kind: 'gap', duration_ms: lead })
  for (const c of clips) laid.push({ id: c.id, asset: c.asset, src_in_ms: 0, src_out_ms: c.dur })
  return {
    settings: { width: 1920, height: 1080, fps: 30, audio_rate: 48000 },
    assets: { a1: { path: '/m/a1.mp4' }, img: { path: '/m/p.png' } },
    tracks: [{ id: 'v1', kind: 'video', visible: true, clips: laid }],
  } as unknown as Project
}

function input(over: Partial<PosterChipInput>): PosterChipInput {
  return {
    project: project([{ id: 'c1', asset: 'a1', dur: 10_000 }]),
    playheadMs: 5_000,
    durationMs: 10_000,
    proxyBuilding: false,
    composed: false,
    failedSources: new Set(),
    imageAssets: new Set(['img']),
    overlayCount: 0,
    ...over,
  }
}

// Work in progress and the composed view keep their labels.
assert.equal(posterChip(input({ proxyBuilding: true }))?.label, 'PROXY RENDERING…')
assert.equal(posterChip(input({ composed: true }))?.label, 'COMPOSED FRAME')

// A still image at the playhead is a picture, not a missing preview.
const withImage = project([{ id: 'c2', asset: 'img', dur: 5_000 }])
assert.equal(posterChip(input({ project: withImage, playheadMs: 0 })), null, 'an image clip needs no chip')

// A clip that lives on an overlay track while the base is empty still shows a picture.
assert.equal(
  posterChip(input({ project: project([]), playheadMs: 3_000, overlayCount: 1 })),
  null,
  'nothing wrong when an overlay clip is shown',
)

// The end of the timeline says so instead of "no preview".
const end = posterChip(input({ playheadMs: 10_000 }))
assert.equal(end?.label, 'END OF TIMELINE')
assert.equal(end?.kind, 'end')
assert.ok(end?.note)

// A gap says there is no clip and what to do.
const gap = posterChip(input({ project: project([{ id: 'c1', asset: 'a1', dur: 4_000 }], 6_000), playheadMs: 2_000 }))
assert.equal(gap?.label, 'NO CLIP HERE')
assert.match(gap?.title ?? '', /Move the playhead onto a clip/)

// A clip that cannot play live is the one case that is a real shortfall, and it
// says that plainly.
const failed = posterChip(input({ failedSources: new Set(['a1']) }))
assert.equal(failed?.kind, 'live-failed')
assert.match(failed?.title ?? '', /could not play live/)

// A playable clip under the playhead never gets a "no preview" style chip.
assert.equal(posterChip(input({})), null)

// The old wording is gone for every state.
for (const chip of [end, gap, failed]) assert.doesNotMatch(chip?.label ?? '', /no preview/i)

// The real badge component: at the end of the timeline the chip names the state
// and the words "no preview" never appear; an image clip shows no chip at all.
function badgesMarkup(chip: ReturnType<typeof posterChip>): string {
  return renderToStaticMarkup(
    createElement(PreviewMonitorBadges, {
      showVideo: false,
      video: null,
      proxyBuilding: false,
      overlaysDropped: 0,
      posterActive: true,
      showSpinner: false,
      posterChip: chip,
      livePreviewNotice: null,
    }),
  )
}
const endMarkup = badgesMarkup(end)
assert.match(endMarkup, /END OF TIMELINE/)
assert.doesNotMatch(endMarkup, /no preview/i)
assert.match(endMarkup, /data-cut-poster-chip="end"/)
assert.doesNotMatch(badgesMarkup(null), /pv-chip/, 'no chip when the frame needs no explanation')

console.log('poster chip tests passed')
