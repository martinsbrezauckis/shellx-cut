import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { placeLinkedAV, planTimelineAssetDrop, resolvePlacementTargets } from '../src/lib/placement'
import { laidToLinkedPlacementPosition } from '../src/panels/Timeline/layout'

const sourceMonitor = readFileSync(new URL('../src/panels/Assets/SourceMonitor.tsx', import.meta.url), 'utf8')
const timelineDrop = readFileSync(new URL('../src/panels/Timeline/useTimelineAssetDrop.ts', import.meta.url), 'utf8')

// Exercise the Source Monitor's target/clock/dispatch pipeline for each known
// stream shape, not just the spelling of its linked-target admission expression.
const savedFetch = globalThis.fetch
try {
  for (const [kind, hasAudio, targets, verb, destination] of [
    ['video', true, { videoTrack: 'v1', audioTrack: 'a1t' }, 'edit.insert_linked', { video_track: 'v1', audio_track: 'a1t' }],
    ['audio', true, { audioTrack: 'a1t' }, 'edit.insert', { track: 'a1t' }],
    ['video', false, { videoTrack: 'v1', audioTrack: undefined }, 'edit.insert', { track: 'v1' }],
  ] as const) {
    const project = {
      assets: { source: { probe: { kind, has_audio: hasAudio, duration_ms: 6000 } } },
      tracks: [{ id: 'v1', kind: 'video', clips: [] }, { id: 'a1t', kind: 'audio', clips: [] }],
    } as any
    const resolved = resolvePlacementTargets(project, { asset: 'source', kind })
    assert.deepEqual(resolved, targets)
    const position = laidToLinkedPlacementPosition(project, 2500,
      [resolved.videoTrack, hasAudio ? resolved.audioTrack : undefined],
      kind === 'video' && hasAudio && (!resolved.videoTrack || !resolved.audioTrack))
    assert.deepEqual(position, { ok: true, atMs: 2500 })
    const calls: unknown[] = []
    globalThis.fetch = async (input, init) => {
      calls.push([String(input).split('/api/verb/')[1], JSON.parse(String(init?.body))])
      return new Response(JSON.stringify({ ok: true, result: { video_track: 'v1', audio_track: 'a1t' } }))
    }
    const result = await placeLinkedAV({ asset: 'source', kind, at_ms: 2500,
      src_range_ms: [100, 1100], ripple: true, rationale: 'source range', ...resolved, project })
    assert.equal(result.ok, true)
    assert.deepEqual(calls, [[verb, { asset: 'source', at_ms: 2500, ...destination,
      src_range_ms: [100, 1100], ripple: true, rationale: 'source range' }]])
  }
} finally { globalThis.fetch = savedFetch }
console.log('PASS Source Monitor normal, sound-only and silent source target/clock/dispatch cases')

assert.match(
  sourceMonitor,
  /laidToLinkedPlacementPosition\(\s*project,\s*playheadMs,/,
  'Source Monitor resolves a linked insert through both destination clocks before it calls the engine',
)
assert.match(
  sourceMonitor,
  /asset\.kind === 'video' && !!asset\.hasAudio && \(!insertTargets\.videoTrack \|\| !insertTargets\.audioTrack\)/,
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
    placement: 'overwrite',
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

for (const kind of ['video', 'image', 'audio']) {
  for (const target of [null, { id: 'v1', kind: 'video', kindIndex: 0 }, { id: 'v2', kind: 'video', kindIndex: 1 }, { id: 'a1t', kind: 'audio', kindIndex: 0 }, { id: 'a2t', kind: 'audio', kindIndex: 1 }, { id: 'cap1', kind: 'caption', kindIndex: 0 }]) {
    for (const alt of [false, true]) {
      const plan = planTimelineAssetDrop({ asset: 'source', kind, at_ms: 1000, target: target as any, alt })!
      assert.equal(plan.ripple, alt, `${kind} on ${target?.id ?? 'empty area'}: plain overwrites, Alt inserts`)
      assert.equal(plan.videoTrack, kind !== 'audio' && target?.kind === 'video' ? target.id : undefined)
      assert.equal(plan.audioTrack, kind === 'audio' && target?.kind === 'audio' ? target.id : undefined)
      assert.equal(plan.newVideoTrack, undefined, 'Alt never creates an overlay')
    }
  }
}
try {
  for (const overlaps of [0, 1, 2]) {
    for (const kind of ['video', 'audio', 'image']) {
      for (const alt of [false, true]) {
        const clips = Array.from({ length: overlaps }, (_, i) => ({ id: `old-${i}`, asset: 'old', src_in_ms: 0, src_out_ms: 1000 }))
        const project = { assets: { source: { probe: { kind, has_audio: kind === 'video', duration_ms: 2000 } } },
          tracks: [{ id: 'v1', kind: 'video', clips }, { id: 'a1t', kind: 'audio', clips }] } as any
        requests.length = 0
        globalThis.fetch = async (input, init) => {
          requests.push({ url: String(input), body: JSON.parse(String(init?.body ?? '{}')) })
          return new Response(JSON.stringify({ ok: true, result: { video_track: 'v1', audio_track: 'a1t' } }), { headers: { 'content-type': 'application/json' } })
        }
        const plan = planTimelineAssetDrop({ asset: 'source', kind, at_ms: 500, alt, duration_ms: kind === 'image' ? 3000 : undefined,
          target: kind === 'audio' ? { id: 'a1t', kind: 'audio', kindIndex: 0 } : { id: 'v1', kind: 'video', kindIndex: 0 } })!
        assert.equal((await placeLinkedAV({ ...plan, project })).ok, true)
        assert.equal(requests.length, 1, 'one edit, including linked audio, regardless of overlap count')
        assert.ok(requests[0].url.endsWith(`/api/verb/${alt ? kind === 'video' ? 'edit.insert_linked' : 'edit.insert' : 'edit.overwrite'}`), `${kind} overlaps=${overlaps} Alt=${alt}`)
        const body = requests[0].body as any
        if (!alt) {
          assert.equal(body.video_track, kind === 'audio' ? undefined : 'v1')
          assert.equal(body.audio_track, kind === 'image' ? undefined : 'a1t')
        } else assert.equal(body.ripple, true)
      }
    }
  }
} finally { globalThis.fetch = previousFetch }
console.log('PASS linked insert coordinate admission; 36 asset drop decisions; 18 empty/one/two-overlap dispatch cases')
