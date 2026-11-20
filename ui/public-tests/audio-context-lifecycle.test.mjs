import test from 'node:test'
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve, dirname } from 'node:path'
import { fileURLToPath } from 'node:url'
import { stripTypeScriptTypes } from 'node:module'
import vm from 'node:vm'
const root = process.env.CUT_AUDIO_LIFECYCLE_SOURCE_ROOT || resolve(dirname(fileURLToPath(import.meta.url)), '..')
const flush = () => new Promise(resolve => setImmediate(resolve))
function harness(panel, { failSource = false, failAnalyser = false, failDisconnect = false, pendingDecode = false, tracks = ['track'] } = {}) {
  const source = readFileSync(resolve(root, 'src/panels', panel, panel === 'Preview' ? 'usePreviewAudioMeter.ts' : 'index.tsx'), 'utf8')
  const start = panel === 'Preview' ? 0 : source.indexOf('  const ctxRef =')
  const end = panel === 'Preview' ? source.length : source.indexOf('  const setGain =', start)
  assert.ok(start >= 0 && end > start, 'owning production lifecycle region exists')
  const contexts = [], writes = [], cleanups = [], verbs = []
  let decodeComplete
  class Context {
    state = 'running'; closes = 0; resumes = 0; disconnects = 0
    destination = {}; constructor() { contexts.push(this) }
    createMediaElementSource() {
      if (failSource) throw new Error('source setup refused')
      return { connect() {}, disconnect: () => { this.disconnects++; if (failDisconnect) throw new Error('disconnect refused') } }
    }
    createAnalyser() { if (failAnalyser) throw new Error('analyser setup refused'); return { connect() {} } }
    resume() { this.resumes++; return Promise.resolve() }
    close() { this.closes++; this.state = 'closed'; return Promise.resolve() }
    decodeAudioData() {
      const audio = { numberOfChannels: 1, sampleRate: 48000, getChannelData() { return new Float32Array([1]) } }
      return pendingDecode ? new Promise(resolve => { decodeComplete = () => resolve(audio) }) : Promise.resolve(audio)
    }
  }
  const globals = {
    window: { AudioContext: Context }, useRef: value => ({ current: value }),
    useState: value => [value, value => writes.push(value)], useCallback: fn => fn,
    useEffect: fn => { const cleanup = fn(); if (typeof cleanup === 'function') cleanups.push(cleanup) },
    audioRef: { current: {} }, doctor: {}, isFfmpegMissing: () => false, openVideoToolsSettings() {},
    performance: { now: () => 1 }, requestAnimationFrame: () => 1, cancelAnimationFrame() {},
    playheadMs: 0, audioTrackIds: tracks, headOpId: 'op', measurementKey: 'current',
    currentMeasurementKey: { current: 'current' }, mixerProjectScope: () => 'project', project: {},
    MAX_METER_MS: 10000, AbortController, mixerSnapshotMatches: () => true,
    exportUrl: path => path, fetch: async () => ({ arrayBuffer: async () => new ArrayBuffer(1) }),
    callVerb: async name => { verbs.push(name); return { ok: true, result: name === 'export.audio' ? { path: '/stem', duration_ms: 1000 } : {} } },
  }
  const output = panel === 'Preview' ? '\n globalThis.setup = usePreviewAudioMeter(audioRef).setupMeter' : ''
  const owningCode = source.slice(start, end).replace(/^import .* from 'react'\n/m, '').replace('export function usePreviewAudioMeter', 'function usePreviewAudioMeter')
  vm.runInNewContext(stripTypeScriptTypes(owningCode + output, { mode: 'transform' }), globals)
  return { contexts, writes, verbs, setup: globals.setup,
    unmount() { for (const cleanup of cleanups) cleanup() }, finishDecode() { assert.ok(decodeComplete); decodeComplete() } }
}
test('Preview keeps one live graph through replay and closes its owned context once on unmount', () => {
  const h = harness('Preview'); h.setup(); h.setup()
  assert.equal(h.contexts.length, 1); assert.equal(h.contexts[0].closes, 0)
  h.unmount(); h.unmount()
  assert.equal(h.contexts[0].closes, 1); assert.equal(h.contexts[0].disconnects, 1)
  h.setup(); assert.equal(h.contexts.length, 1, 'detached playback event cannot reacquire audio')
})
test('Preview releases a partially constructed context when source creation refuses', () => {
  const h = harness('Preview', { failSource: true }); h.setup()
  assert.equal(h.contexts[0].closes, 1); assert.equal(h.writes.length, 0)
  h.unmount(); assert.equal(h.contexts[0].closes, 1)
})
test('a remounted Preview owns a distinct live context; old cleanup cannot close it', () => {
  const old = harness('Preview'); old.setup(); old.unmount()
  const fresh = harness('Preview'); fresh.setup(); old.unmount()
  assert.equal(fresh.contexts[0].closes, 0); fresh.unmount(); assert.equal(fresh.contexts[0].closes, 1)
})
test('Mixer closes only at unmount and rejects pending decode publication and further stem work', async () => {
  const h = harness('Mixer', { pendingDecode: true, tracks: ['a', 'b'] }); await flush()
  assert.equal(h.contexts.length, 1); assert.equal(h.contexts[0].closes, 0)
  h.unmount(); h.unmount(); h.finishDecode(); await flush()
  assert.equal(h.contexts[0].closes, 1); assert.equal(h.writes.length, 0)
  assert.equal(h.verbs.filter(name => name === 'export.audio').length, 1)
})
test('Mixer publishes a decoded stem while mounted, then releases its context', async () => {
  const h = harness('Mixer'); await flush(); assert.equal(h.writes.length, 1)
  assert.equal(h.contexts[0].closes, 0); h.unmount(); assert.equal(h.contexts[0].closes, 1)
})
test('empty Mixer owns no audio context and unmount does not create one', async () => {
  const h = harness('Mixer', { tracks: [] }); await flush(); h.unmount(); assert.equal(h.contexts.length, 0)
})

test('Preview closes a partially linked graph even if disconnect refuses', () => {
  const h = harness('Preview', { failAnalyser: true, failDisconnect: true }); h.setup()
  assert.equal(h.contexts[0].disconnects, 1); assert.equal(h.contexts[0].closes, 1)
  assert.equal(h.writes.length, 0); h.unmount(); assert.equal(h.contexts[0].closes, 1)
})
test('Preview unmount still closes its context when disconnect refuses', () => {
  const h = harness('Preview', { failDisconnect: true }); h.setup(); h.unmount(); h.unmount()
  assert.equal(h.contexts[0].closes, 1); assert.equal(h.contexts[0].disconnects, 1)
})
test('pending old Mixer work cannot publish into a remounted owner', async () => {
  const old = harness('Mixer', { pendingDecode: true }); await flush(); old.unmount()
  const fresh = harness('Mixer'); await flush(); old.finishDecode(); await flush()
  assert.equal(old.writes.length, 0); assert.equal(old.contexts[0].closes, 1)
  assert.equal(fresh.writes.length, 1); assert.equal(fresh.contexts[0].closes, 0)
  fresh.unmount(); assert.equal(fresh.contexts[0].closes, 1)
})
