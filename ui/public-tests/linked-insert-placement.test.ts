import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { placeLinkedAV, planTimelineAssetDrop } from '../src/lib/placement'
import { laidToLinkedPlacementPosition } from '../src/panels/Timeline/layout'

const sourceMonitor = readFileSync(new URL('../src/panels/Assets/SourceMonitor.tsx', import.meta.url), 'utf8')
const timelineDrop = readFileSync(new URL('../src/panels/Timeline/useTimelineAssetDrop.ts', import.meta.url), 'utf8')

assert.match(
  sourceMonitor,
  /laidToLinkedPlacementPosition\(\s*project,\s*playheadMs,/,
  'Source Monitor resolves a linked insert through both destination clocks before it calls the engine',
)
assert.match(
  sourceMonitor,
  /asset\.kind === 'video' && asset\.hasAudio && \(!insertTargets\.videoTrack \|\| !insertTargets\.audioTrack\)/,
  'Source Monitor treats either missing muxed-A/V destination as a new identity-clock linked target',
)
assert.match(
  timelineDrop,
  /laidToLinkedPlacementPosition\(project, laidMs, \[targets\.videoTrack, targets\.audioTrack\], createsLinkedTarget\)/,
  'Timeline drops use the same shared linked-placement coordinate admission',
)
assert.match(
  timelineDrop,
  /Could not place this asset:/,
  'Timeline drops surface a rejected atomic placement instead of silently ignoring it',
)

assert.deepEqual(
  planTimelineAssetDrop({
    asset: 'muxed-source',
    kind: 'video',
    at_ms: 7_000,
    hasAudio: true,
    target: { id: 'v2', kind: 'video', kindIndex: 1 },
  }),
  {
    asset: 'muxed-source',
    kind: 'video',
    at_ms: 7_000,
    ripple: false,
    videoTrack: 'v2',
    newAudioTrack: true,
    rationale: 'place muxed-source on overlay track v2 at 7.00s',
  },
  'a muxed overlay drop supplies a distinct linked audio destination from probe evidence',
)

const matchingCrossfades = {
  tracks: [
    {
      id: 'v1', kind: 'video', clips: [
        { id: 'v-old', asset: 'old', src_in_ms: 0, src_out_ms: 2_000 },
        { id: 'v-next', asset: 'old', src_in_ms: 2_000, src_out_ms: 4_000, xfade_in_ms: 1_000 },
      ],
    },
    {
      id: 'a1t', kind: 'audio', clips: [
        { id: 'a-old', asset: 'old', src_in_ms: 0, src_out_ms: 2_000 },
        { id: 'a-next', asset: 'old', src_in_ms: 2_000, src_out_ms: 4_000, xfade_in_ms: 1_000 },
      ],
    },
  ],
} as any

assert.deepEqual(
  laidToLinkedPlacementPosition(matchingCrossfades, 2_500, ['v1', 'a1t'], false),
  { ok: true, atMs: 3_500 },
  'existing aligned V/A tracks convert one visible crossfade position into their shared editorial insertion point',
)

const overlap = laidToLinkedPlacementPosition(matchingCrossfades, 1_500, ['v1', 'a1t'], false)
assert.equal(overlap.ok, false, 'a visible overlap cannot be chosen as a linked insertion coordinate')
assert.match(overlap.ok ? '' : overlap.error, /live crossfade/i)

const divergentCrossfades = {
  ...matchingCrossfades,
  tracks: [
    matchingCrossfades.tracks[0],
    { ...matchingCrossfades.tracks[1], clips: matchingCrossfades.tracks[1].clips.map((clip: any) => ({ ...clip, xfade_in_ms: 0 })) },
  ],
} as any
const divergent = laidToLinkedPlacementPosition(divergentCrossfades, 2_500, ['v1', 'a1t'], false)
assert.equal(divergent.ok, false, 'existing V/A targets with divergent crossfade clocks are rejected')

const missingAudio = laidToLinkedPlacementPosition(matchingCrossfades, 2_500, ['v1'], true)
assert.equal(missingAudio.ok, false, 'a crossfaded existing video target cannot be paired with a missing identity-clock audio target')
assert.match(missingAudio.ok ? '' : missingAudio.error, /new linked track/i)

assert.deepEqual(
  laidToLinkedPlacementPosition(matchingCrossfades, 2_500, [], true),
  { ok: true, atMs: 2_500 },
  'two new linked tracks share the identity clock and remain admissible',
)

const previousFetch = globalThis.fetch
const requests: Array<{ url: string; body: unknown }> = []
globalThis.fetch = async (input, init) => {
  requests.push({ url: String(input), body: JSON.parse(String(init?.body ?? '{}')) })
  return new Response(JSON.stringify({ ok: true, result: null }), { headers: { 'content-type': 'application/json' } })
}
try {
  const missingReceipt = await placeLinkedAV({
    asset: 'muxed-source',
    kind: 'video',
    at_ms: 3_500,
    src_range_ms: [100, 1_100],
    project: {
      assets: { 'muxed-source': { probe: { kind: 'video', has_audio: true, duration_ms: 6_000 } } },
      tracks: [{ id: 'v1', kind: 'video', clips: [] }, { id: 'a1t', kind: 'audio', clips: [] }],
    } as any,
  })
  assert.equal(requests.length, 1)
  assert.equal(requests[0]?.url.endsWith('/api/verb/edit.insert_linked'), true)
  assert.deepEqual(requests[0]?.body, {
    asset: 'muxed-source', at_ms: 3_500, video_track: 'v1', audio_track: 'a1t', src_range_ms: [100, 1_100], ripple: true, rationale: 'place muxed-source',
  })
  assert.equal(missingReceipt.ok, false, 'a committed response without an atomic receipt is not safe to retry')
  assert.match(missingReceipt.error ?? '', /committed without a receipt.*Refresh the project/i)

  requests.length = 0
  globalThis.fetch = async (input, init) => {
    requests.push({ url: String(input), body: JSON.parse(String(init?.body ?? '{}')) })
    return new Response(JSON.stringify({
      ok: false,
      error: { code: 'conflict', message: 'audio destination is locked' },
    }), { headers: { 'content-type': 'application/json' } })
  }
  const rejectedSecondLeg = await placeLinkedAV({
    asset: 'muxed-source',
    kind: 'video',
    at_ms: 3_500,
    project: {
      assets: { 'muxed-source': { probe: { kind: 'video', has_audio: true, duration_ms: 6_000 } } },
      tracks: [{ id: 'v1', kind: 'video', clips: [] }, { id: 'a1t', kind: 'audio', clips: [] }],
    } as any,
  })
  assert.equal(requests.length, 1, 'a rejected audio leg reaches one atomic endpoint rather than a second client insertion')
  assert.deepEqual(
    rejectedSecondLeg,
    { ok: false, videoOk: false, audioLinked: false, videoTrack: 'v1', audioTrack: 'a1t', error: 'audio destination is locked' },
    'a rejected second leg reports no partial video placement to Source Monitor or Timeline',
  )
} finally {
  globalThis.fetch = previousFetch
}

console.log('PASS linked insert coordinate admission')
