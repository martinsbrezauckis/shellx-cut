import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import {
  overwriteSourceRange,
  overwriteSourceStill,
  sourceOverwriteTrackTargets,
  STILL_OVERWRITE_DEFAULT_DURATION_MS,
  STILL_OVERWRITE_MAX_DURATION_MS,
  STILL_OVERWRITE_MIN_DURATION_MS,
} from '../src/lib/placement'
import { laidToSharedEditorialPosition } from '../src/panels/Timeline/layout'

const sourceMonitor = readFileSync(new URL('../src/panels/Assets/SourceMonitor.tsx', import.meta.url), 'utf8')
assert.match(
  sourceMonitor,
  /<button\s+ref=\{overwriteButtonRef\}[\s\S]{0,240}data-cut-action="source-overwrite"/,
  'the async overwrite route returns focus to Overwrite itself so Escape remains keyboard-reachable',
)
assert.doesNotMatch(
  sourceMonitor,
  /<button\s+ref=\{overwriteButtonRef\}[\s\S]{0,240}data-cut-source-insert/,
  'the overwrite focus ref cannot drift onto Insert',
)
assert.match(
  sourceMonitor,
  /useEffect\(\(\) => \{[\s\S]{0,500}operation !== null \|\| !overwriteFocusPending\.current[\s\S]{0,500}overwriteButtonRef\.current\?\.focus/,
  'focus restoration waits for React to commit the re-enabled Overwrite button',
)
assert.match(
  sourceMonitor,
  /finally \{\s*overwriteFocusPending\.current = true\s*setOperation\(null\)/,
  'every completed overwrite schedules focus restoration through the post-commit effect',
)
assert.doesNotMatch(
  sourceMonitor,
  /requestAnimationFrame\(\(\) => overwriteButtonRef\.current\?\.focus/,
  'overwrite focus restoration cannot race the React commit through requestAnimationFrame',
)
assert.match(
  sourceMonitor,
  /laidToSharedEditorialPosition\(project, playheadMs, \[videoTarget, overwriteAudioTarget\]\)/,
  'Source Monitor converts the visible playhead through the selected overwrite targets before dispatch',
)
assert.match(sourceMonitor, /kind: 'video' \| 'audio' \| 'image'/, 'Source Monitor admits still-image assets')
assert.match(sourceMonitor, /data-cut-source-still-preview/, 'still sources render an image preview rather than timed media')
assert.match(sourceMonitor, /!isStill && <div className="source-monitor__marks">/, 'still sources do not invent In and Out marks')
assert.match(sourceMonitor, /data-cut-source-still-duration/, 'still overwrite exposes an editable duration control')
assert.match(sourceMonitor, /!isStill && <label className="source-monitor__target-control">/, 'still sources do not expose an audio destination')
assert.match(sourceMonitor, /data-cut-source-video-target-state=\{!sourceHasVideo \? 'source-unavailable' : videoTargets\.length === 0 \? 'unavailable'/, 'a still tells the truth when no unlocked video target exists')
assert.match(sourceMonitor, /source-monitor__duration-error/, 'invalid still duration has explicit visible feedback')
assert.equal(STILL_OVERWRITE_DEFAULT_DURATION_MS, 3_000)
assert.equal(STILL_OVERWRITE_MIN_DURATION_MS, 100)
assert.equal(STILL_OVERWRITE_MAX_DURATION_MS, 3_600_000)
assert.match(
  sourceMonitor,
  /if \(busy \|\| !hasOverwriteTarget \|\| !overwritePosition\.ok \|\|/,
  'an ambiguous shared position cannot dispatch edit.overwrite even if a stale button event arrives',
)

const project = {
  tracks: [
    { id: 'v1', kind: 'video', clips: [] },
    { id: 'v2-locked', kind: 'video', locked: true, clips: [] },
    { id: 'a1', kind: 'audio', clips: [] },
    { id: 'a2-locked', kind: 'audio', locked: true, clips: [] },
    { id: 'c1', kind: 'caption', clips: [] },
  ],
} as any

assert.deepEqual(
  sourceOverwriteTrackTargets(project),
  { video: ['v1'], audio: ['a1'] },
  'only unlocked compatible tracks are offered as Source Monitor overwrite targets',
)

const xfadeProject = {
  tracks: [
    {
      id: 'v1', kind: 'video', clips: [
        { id: 'v-old', asset: 'old', src_in_ms: 0, src_out_ms: 2_000 },
        { id: 'v-next', asset: 'old', src_in_ms: 2_000, src_out_ms: 4_000, xfade_in_ms: 1_000 },
      ],
    },
    {
      id: 'a1', kind: 'audio', clips: [
        { id: 'a-old', asset: 'old', src_in_ms: 0, src_out_ms: 2_000 },
        { id: 'a-next', asset: 'old', src_in_ms: 2_000, src_out_ms: 4_000, xfade_in_ms: 1_000 },
      ],
    },
  ],
} as any
assert.deepEqual(
  laidToSharedEditorialPosition(xfadeProject, 2_500, ['v1', 'a1']),
  { ok: true, atMs: 3_500 },
  'the visible 2500ms playhead converts through the shared laid-to-editorial math for both linked targets',
)

const overlapPosition = laidToSharedEditorialPosition(xfadeProject, 1_500, ['v1', 'a1'])
assert.equal(
  overlapPosition.ok,
  false,
  'a visible playhead covered by both sides of a live crossfade is not an unambiguous overwrite start',
)
assert.match(
  overlapPosition.ok ? '' : overlapPosition.error,
  /live crossfade/i,
  'Source Monitor fails closed instead of resolving the overlap to the left editorial clock and changing pixels before the requested playhead',
)

const divergentXfadeProject = {
  ...xfadeProject,
  tracks: [xfadeProject.tracks[0], { ...xfadeProject.tracks[1], clips: xfadeProject.tracks[1].clips.map((clip: any) => ({ ...clip, xfade_in_ms: 0 })) }],
} as any
const divergentPosition = laidToSharedEditorialPosition(divergentXfadeProject, 2_500, ['v1', 'a1'])
assert.equal(divergentPosition.ok, false, 'one atomic V+A overwrite never guesses between divergent crossfade clocks')
assert.deepEqual(
  sourceOverwriteTrackTargets({ tracks: [] } as any),
  { video: [], audio: [] },
  'an empty timeline offers no implied overwrite target',
)
assert.deepEqual(
  sourceOverwriteTrackTargets({ tracks: [{ id: 'v1', kind: 'video', locked: true, clips: [] }] } as any),
  { video: [], audio: [] },
  'locked-only tracks leave the Source Monitor in its explicit Off state',
)

const previousFetch = globalThis.fetch
const requests: Array<{ url: string; init?: RequestInit }> = []
globalThis.fetch = async (input, init) => {
  requests.push({ url: String(input), init })
  return new Response(JSON.stringify({
    ok: false,
    error: { code: 'track_locked', message: 'Target V v1 is locked. Unlock it and try again.' },
  }), { headers: { 'content-type': 'application/json' } })
}

try {
  const missingTarget = await overwriteSourceRange({
    asset: 'source-a', atMs: 8_000, sourceRangeMs: [2_000, 5_500],
  })
  assert.equal(missingTarget.ok, false)
  assert.equal(missingTarget.error?.message, 'Choose a V or A destination before overwriting.')
  assert.equal(requests.length, 0, 'Off/Off rejects locally without an engine request')

  const result = await overwriteSourceRange({
    asset: 'source-a',
    atMs: 8_000,
    sourceRangeMs: [2_000, 5_500],
    videoTrack: 'v1',
    audioTrack: 'a1',
    rationale: 'overwrite marked source range',
  })
  assert.equal(requests.length, 1, 'V+A overwrite is dispatched as one atomic engine call')
  assert.equal(requests[0].url.endsWith('/api/verb/edit.overwrite'), true)
  assert.deepEqual(JSON.parse(String(requests[0].init?.body)), {
    asset: 'source-a',
    at_ms: 8_000,
    src_range_ms: [2_000, 5_500],
    video_track: 'v1',
    audio_track: 'a1',
    rationale: 'overwrite marked source range',
  })
  assert.equal(result.error?.message, 'Target V v1 is locked. Unlock it and try again.',
    'engine error messages pass through unchanged for Source Monitor recovery')

  const stillOff = await overwriteSourceStill({
    asset: 'still-a', atMs: 8_000, durationMs: 2_500,
  })
  assert.equal(stillOff.ok, false)
  assert.equal(stillOff.error?.message, 'Choose a video destination before overwriting.')
  assert.equal(requests.length, 1, 'a still with Video set to Off stays local')

  const invalidStill = await overwriteSourceStill({
    asset: 'still-a', atMs: 8_000, videoTrack: 'v1', durationMs: 99,
  })
  assert.equal(invalidStill.ok, false)
  assert.equal(invalidStill.error?.message, 'Set a still duration from 0.1 to 3,600 seconds before overwriting.')
  assert.equal(requests.length, 1, 'invalid still duration stays local')

  const still = await overwriteSourceStill({
    asset: 'still-a',
    atMs: 8_000,
    videoTrack: 'v1',
    durationMs: 2_500,
    rationale: 'overwrite still at playhead',
  })
  assert.equal(requests.length, 2, 'a valid still dispatches exactly one engine request')
  assert.equal(requests[1].url.endsWith('/api/verb/edit.overwrite'), true)
  assert.deepEqual(JSON.parse(String(requests[1].init?.body)), {
    asset: 'still-a',
    at_ms: 8_000,
    video_track: 'v1',
    duration_ms: 2_500,
    rationale: 'overwrite still at playhead',
  }, 'still overwrite sends only the video-target duration contract')
  assert.equal(still.error?.message, 'Target V v1 is locked. Unlock it and try again.')
} finally {
  globalThis.fetch = previousFetch
}

console.log('PASS Source Monitor overwrite targets and dispatch contract')
