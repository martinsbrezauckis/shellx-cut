// Asset ids and op ids repeat between projects, so a frame, source, proxy,
// filmstrip or waveform answer for one project must never be served for
// another. Every media URL and cache key carries the open project's scope.
import assert from 'node:assert/strict'
import { exportUrl, frameUrl, sourceUrl } from '../src/lib/clientUrls'
import { getWaveform, getWindowThumbs } from '../src/lib/clientMedia'
import { mediaScopeOf, scopeMediaUrl, setActiveMediaScope } from '../src/lib/mediaScope'

const identity = (hash: string) => ({
  project_identity: { schema: 'shellx-cut/project-identity/1' as const, origin_path_sha256: hash, project_name: 'p' },
})
const projectA = identity('a'.repeat(64))
const projectB = identity('b'.repeat(64))

// 1. URLs built from asset ids and op ids differ between two projects that
//    share the same ids.
setActiveMediaScope(mediaScopeOf(projectA, 1))
const urlsA = [frameUrl(0, 'op_0004', true), sourceUrl('a1'), exportUrl('exports/_monitor_a.mp3'), scopeMediaUrl('/proxies/a1.mp4'), scopeMediaUrl('/filmstrip/a1.jpg')]
setActiveMediaScope(mediaScopeOf(projectB, 2))
const urlsB = [frameUrl(0, 'op_0004', true), sourceUrl('a1'), exportUrl('exports/_monitor_a.mp3'), scopeMediaUrl('/proxies/a1.mp4'), scopeMediaUrl('/filmstrip/a1.jpg')]
urlsA.forEach((url, i) => assert.notEqual(url, urlsB[i], `same URL for two projects: ${url}`))

// Reopening the same project is a new session: a clean slate too.
setActiveMediaScope(mediaScopeOf(projectA, 3))
assert.notEqual(frameUrl(0, 'op_0004', true), urlsA[0], 'a reopened project must not reuse the old URLs')

// No project open: URLs are unchanged (no stray parameter).
setActiveMediaScope(mediaScopeOf(null, 4))
assert.equal(frameUrl(1500, 'op_0004', true), '/api/frame?at_ms=1500&compose=1&v=op_0004')
assert.equal(sourceUrl('a1'), '/api/source/a1')
assert.equal(scopeMediaUrl('/frames/a.jpg?x=1#f'), '/frames/a.jpg?x=1#f')

// 2. The in-memory waveform and windowed-filmstrip caches are keyed by project.
let calls = 0
Object.assign(globalThis, {
  fetch: async (url: string, init?: { body?: string }) => {
    calls += 1
    const verb = String(url).split('/api/verb/')[1]
    const body = init?.body ? JSON.parse(init.body) : {}
    const result = verb === 'media.waveform'
      ? { asset: body.asset, bucket_count: 1, peaks: [calls / 10], source_ms: 1000, sample_rate: 8000 }
      : { thumbs: `filmstrip/win_${calls}.jpg`, range_ms: body.range_ms, count: 1, h: 40 }
    return { ok: true, status: 200, json: async () => ({ ok: true, result }) }
  },
})

setActiveMediaScope(mediaScopeOf(projectA, 1))
const waveA = await getWaveform('a1')
const waveA2 = await getWaveform('a1')
assert.equal(waveA2, waveA, 'one project reuses its own cached peaks')
const callsAfterA = calls
setActiveMediaScope(mediaScopeOf(projectB, 2))
const waveB = await getWaveform('a1')
assert.equal(calls, callsAfterA + 1, 'another project asks the engine again')
assert.notDeepEqual(waveB?.peaks, waveA?.peaks, 'and gets its own peaks, not the other project\'s')

setActiveMediaScope(mediaScopeOf(projectA, 1))
const thumbA = await getWindowThumbs('a1', 0, 1000, 1, 40)
setActiveMediaScope(mediaScopeOf(projectB, 2))
const thumbB = await getWindowThumbs('a1', 0, 1000, 1, 40)
assert.notEqual(thumbA?.url, thumbB?.url, 'filmstrip tiles are not shared between projects')

console.log('media scope: URLs and caches are per project')
