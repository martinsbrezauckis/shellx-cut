import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { stripTypeScriptTypes } from 'node:module'
import { resolve } from 'node:path'
import test from 'node:test'
import vm from 'node:vm'

const uiRoot = resolve(import.meta.dirname, '..')
const flush = () => new Promise((done) => setImmediate(done))

function effectFromSource(owner, start, end, context) {
  const source = readFileSync(resolve(uiRoot, 'src/panels/Inspector', owner), 'utf8')
  const first = source.indexOf(start)
  const last = source.indexOf(end, first)
  assert.ok(first >= 0 && last > first, `${owner} catalog effect exists`)
  const code = stripTypeScriptTypes(source.slice(first, last), { mode: 'transform' })
  vm.runInNewContext(code, context)
}

function rig(kind, reply) {
  const calls = []
  const writes = { looks: [], presets: [], selected: '', notes: [] }
  let cleanup
  const clientSource = readFileSync(resolve(uiRoot, 'src/lib/client.ts'), 'utf8')
  const verbStart = clientSource.indexOf('export async function callVerb<')
  const verbEnd = clientSource.indexOf('/** Convenience: fetch full project state', verbStart)
  assert.ok(verbStart >= 0 && verbEnd > verbStart)
  const context = {
    AbortController,
    API_BASE: 'http://127.0.0.1:6161',
    uiActorHeader: () => 'human:test:ui',
    deliveryFeedbackSession: () => 0,
    isHumanDeliveryVerb: () => false,
    reportExportRefusal: () => assert.fail('catalog reads must not publish delivery feedback'),
    project: { name: 'sample' },
    looks: [],
    setLooks: (value) => { writes.looks = value },
    setPresets: (value) => { writes.presets = value },
    setSelPreset: (value) => {
      writes.selected = typeof value === 'function' ? value(writes.selected) : value
    },
    setCapNote: (value) => { writes.notes.push(value) },
    setColorNote: (value) => { writes.notes.push(value) },
    useCallback: (fn) => fn,
    useEffect: (fn) => { cleanup = fn() },
    fetch: (url, init) => {
      calls.push({ url, args: JSON.parse(init.body), signal: init.signal })
      return reply.then((result) => ({ json: async () => result }))
    },
  }
  const clientVerb = stripTypeScriptTypes(clientSource.slice(verbStart, verbEnd).replace('export async function', 'async function'), { mode: 'transform' })
  vm.runInNewContext(clientVerb + '\n globalThis.callVerb = callVerb', context)
  if (kind === 'caption') {
    effectFromSource('ProjectCaptionsSection.tsx', '  const reloadLooks =', '  const saveLook =', context)
  } else {
    effectFromSource('useInspectorColorControls.ts', '  const reloadPresets =', '  const saveLook =', context)
  }
  return { calls, writes, cleanup: () => cleanup?.() }
}

for (const kind of ['caption', 'grade']) {
  const verb = kind === 'caption' ? 'captions.list_styles' : 'grade.list'
  const expectedNote = kind === 'caption' ? 'Could not load caption styles' : 'Could not load saved looks'

  test(`${kind} catalog still loads and publishes a successful read`, async () => {
    const h = rig(kind, Promise.resolve({ ok: true, result: { presets: [{ name: 'A' }] } }))
    await flush()
    assert.equal(h.calls.length, 1)
    assert.match(h.calls[0].url, new RegExp(`/api/verb/${verb.replaceAll('.', '[.]')}$`))
    assert.equal(Object.keys(h.calls[0].args).length, 0)
    assert.equal(h.calls[0].signal.aborted, false)
    assert.equal((kind === 'caption' ? h.writes.looks : h.writes.presets)[0].name, 'A')
    if (kind === 'grade') assert.equal(h.writes.selected, 'A')
    assert.deepEqual(h.writes.notes, [])
    h.cleanup()
  })

  test(`${kind} catalog handles a live transport rejection with visible feedback`, async () => {
    const h = rig(kind, Promise.reject(new Error('Load failed')))
    await flush()
    assert.deepEqual(h.writes.notes, [expectedNote])
    h.cleanup()
  })

  test(`${kind} catalog aborts on cleanup and ignores late response or rejection`, async () => {
    for (const outcome of ['response', 'rejection']) {
      let settle
      const h = rig(kind, new Promise((resolve, reject) => {
        settle = outcome === 'response' ? resolve : reject
      }))
      assert.equal(h.calls.length, 1)
      h.cleanup()
      assert.equal(h.calls[0].signal.aborted, true)
      settle(outcome === 'response'
        ? { ok: true, result: { presets: [{ name: 'stale' }] } }
        : new Error('Load failed'))
      await flush()
      assert.deepEqual(h.writes.looks, [])
      assert.deepEqual(h.writes.presets, [])
      assert.deepEqual(h.writes.notes, [])
    }
  })
}
