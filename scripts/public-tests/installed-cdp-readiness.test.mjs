import test from 'node:test'
import assert from 'node:assert/strict'
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import {
  installedCdpPage,
  readTestOwnedSlowFfmpegMarker,
  waitForInstalledCdpPage,
  waitForInstalledDomRoot,
  waitForTestOwnedSlowFfmpegMarker,
} from '../lib/installed-cdp-readiness.mjs'

function fakeClock() {
  let value = 1_000
  return {
    now: () => value,
    sleep: async (ms) => { value += ms },
  }
}

test('installed CDP readiness selects only the real Cut page after the listener answers', async () => {
  const clock = fakeClock()
  let requests = 0
  const result = await waitForInstalledCdpPage({
    cdpBase: 'http://127.0.0.1:9223',
    timeoutMs: 100,
    pollMs: 5,
    now: clock.now,
    sleepImpl: clock.sleep,
    getJsonImpl: async () => {
      requests += 1
      return requests === 1
        ? [{ type: 'service_worker', url: 'http://127.0.0.1:6161/' }]
        : [{ type: 'page', url: 'http://127.0.0.1:6161/', webSocketDebuggerUrl: 'ws://127.0.0.1:9223/devtools/page/1' }]
    },
  })
  assert.equal(requests, 2)
  assert.equal(result.page.webSocketDebuggerUrl, 'ws://127.0.0.1:9223/devtools/page/1')
  assert.equal(installedCdpPage([{ type: 'page', url: 'https://example.test/' }]), null)
})

test('installed DOM readiness requires the real app-root selector, not merely CDP availability', async () => {
  const clock = fakeClock()
  let attempts = 0
  const result = await waitForInstalledDomRoot({
    page: { webSocketDebuggerUrl: 'ws://127.0.0.1:9223/devtools/page/1' },
    timeoutMs: 100,
    pollMs: 5,
    now: clock.now,
    sleepImpl: clock.sleep,
    evaluateImpl: async () => ++attempts === 2,
  })
  assert.equal(attempts, 2)
  assert.equal(result.selector, '[data-cut-app-root]')
})

test('test-owned slow FFmpeg markers remain optional and bounded', async () => {
  const clock = fakeClock()
  let reads = 0
  const marker = await waitForTestOwnedSlowFfmpegMarker({
    markerPath: '/private/fixture/slow-ffmpeg.json',
    timeoutMs: 100,
    pollMs: 5,
    now: clock.now,
    sleepImpl: clock.sleep,
    readMarker: () => {
      reads += 1
      return reads === 3 ? {
        atEpochMs: 1_050,
        evidence: { markerSchema: 'shellx-cut/test-owned-slow-ffmpeg-marker@1' },
      } : null
    },
  })
  assert.equal(reads, 3)
  assert.equal(marker.atEpochMs, 1_050)
})

test('slow FFmpeg marker accepts only the explicit test-owned contract', () => {
  const root = mkdtempSync(join(tmpdir(), 'shellx-cut-slow-ffmpeg-marker-'))
  const markerPath = join(root, 'marker.json')
  try {
    writeFileSync(markerPath, JSON.stringify({
      schema: 'shellx-cut/test-owned-slow-ffmpeg-marker@1',
      testOwned: true,
      startedAtEpochMs: 1_234,
    }))
    assert.equal(readTestOwnedSlowFfmpegMarker(markerPath).atEpochMs, 1_234)
    writeFileSync(markerPath, JSON.stringify({ startedAtEpochMs: 1_234 }))
    assert.throws(() => readTestOwnedSlowFfmpegMarker(markerPath), /explicit test-owned marker/)
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
})
