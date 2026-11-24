import assert from 'node:assert/strict'
import {
  applyOutputFolderStatus,
  clearStoredOutputDirIfAccepted,
  ensureExportDestinationUsable,
  getOutputFolderStatus,
  getStoredOutputDir,
  refreshOutputFolder,
} from '../src/lib/exportDestination'
import { outputFolderLabel, outputFolderStatusFrom, outputFolderTitle } from '../src/lib/outputFolderStatus'

const values = new Map([['cut.outputDir', '/chosen/exports']])
Object.assign(globalThis, {
  localStorage: {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => { values.set(key, value) },
    removeItem: (key: string) => { values.delete(key) },
  },
})
const tail = (dir: string) => dir.split('/').pop() ?? dir

// A refused clear keeps the selected destination; an accepted one removes it.
assert.equal(await clearStoredOutputDirIfAccepted(async () => false), false)
assert.equal(getStoredOutputDir(), '/chosen/exports', 'rejected clear preserves the selected destination')

// The engine's answer is the truth: the browser copy only follows it.
const agentChose = outputFolderStatusFrom({
  dir: '/agent/deliveries', destination: 'chosen', usable: true, problem: null, suggested_action: null,
})!
applyOutputFolderStatus(agentChose)
assert.deepEqual(getOutputFolderStatus(), agentChose)
assert.equal(getStoredOutputDir(), '/agent/deliveries', 'a folder an agent set replaces the stale browser copy')
assert.equal(outputFolderLabel(agentChose, tail), 'deliveries')

// Clearing returns to the project exports folder, and that is visible too.
applyOutputFolderStatus(outputFolderStatusFrom({
  dir: null, destination: 'project_exports', usable: true, problem: null, suggested_action: null, cleared: true,
})!)
assert.equal(getStoredOutputDir(), null)
assert.equal(outputFolderLabel(getOutputFolderStatus(), tail), 'project exports')

// A folder that has vanished is told BEFORE the export, with the way forward.
const gone = outputFolderStatusFrom({
  dir: '/agent/deliveries', destination: 'chosen', usable: false,
  problem: 'export folder not found: /agent/deliveries',
  suggested_action: 'nothing was exported. Choose another export folder',
})!
assert.equal(outputFolderLabel(gone, tail), 'deliveries (unavailable)')
assert.match(outputFolderTitle(gone), /not found: \/agent\/deliveries nothing was exported\. Choose another export folder/)
await assert.rejects(
  ensureExportDestinationUsable(async () => gone),
  /export folder not found: \/agent\/deliveries nothing was exported\. Choose another export folder/,
)
assert.equal(await ensureExportDestinationUsable(async () => agentChose), '/agent/deliveries')
assert.equal(await ensureExportDestinationUsable(async () => null), getStoredOutputDir(), 'an unreachable engine falls back to the cache; the verb then fails by itself')

// Malformed engine answers are not adopted.
assert.equal(outputFolderStatusFrom({ destination: 'chosen', usable: true }), null)
assert.equal(outputFolderStatusFrom({ dir: '/x', destination: 'elsewhere', usable: true }), null)
console.log('export destination: engine value is the truth, a vanished folder is told before export')

// An update carries the old UI-only preference to the engine once. Exercise the
// real verb client and notice publisher; the engine fixture checks every apply.
const notices: string[] = []
const documentEvents = new EventTarget()
documentEvents.addEventListener('cut:user-action-feedback', (event) => {
  notices.push((event as CustomEvent<{ message: string }>).detail.message)
})
Object.assign(globalThis, { document: documentEvents })
const projectExports = outputFolderStatusFrom({ dir: null, destination: 'project_exports', usable: true })!
const chosen = (dir: string) => ({ ...projectExports, dir, destination: 'chosen' as const })
let engine = projectExports
let refused: string | null = null
let beforeRead: (() => Promise<void>) | null = null
let agentBeforeSet: ReturnType<typeof chosen> | null = null
const applies: string[] = []
const conditionalCalls: boolean[] = []
Object.assign(globalThis, {
  fetch: async (url: string, options: RequestInit) => {
    if (url.endsWith('/project.get_output_dir')) {
      const result = engine
      await beforeRead?.()
      return Response.json({ ok: true, result })
    }
    assert.equal(url, '/api/verb/project.set_output_dir')
    const { dir, only_if_empty } = JSON.parse(String(options.body))
    applies.push(dir)
    conditionalCalls.push(only_if_empty === true)
    if (agentBeforeSet) {
      engine = agentBeforeSet
      agentBeforeSet = null
    }
    if (only_if_empty && engine.dir) return Response.json({ ok: true, result: engine })
    if (refused) return Response.json({ ok: false, error: { code: 'invalid_args', message: refused } })
    engine = chosen(dir)
    return Response.json({ ok: true, result: engine })
  },
})
function installation(legacy: string | null, current = projectExports) {
  values.clear()
  if (legacy) values.set('cut.outputDir', legacy)
  engine = current
  refused = null
  beforeRead = null
  agentBeforeSet = null
  applies.length = 0
  conditionalCalls.length = 0
  notices.length = 0
}

const legacy = 'C:\\Users\\Editor\\Deliveries'
installation(legacy)
const firstStarts = await Promise.all([refreshOutputFolder(), refreshOutputFolder()])
assert.deepEqual(firstStarts, [chosen(legacy), chosen(legacy)], 'concurrent startup reads carry the legacy folder over')
assert.deepEqual(applies, [legacy], 'carry-over uses the normal setter exactly once')
assert.deepEqual(conditionalCalls, [true], 'carry-over asks the engine to set only if empty')
assert.equal(getStoredOutputDir(), legacy, 'the accepted folder stays cached')
const restarted = await import(new URL('../src/lib/exportDestination.ts?restart', import.meta.url).href)
assert.equal(restarted.getOutputFolderStatus(), null, 'restart has no in-memory destination state')
await restarted.refreshOutputFolder() // fresh module, same persistent installation storage
assert.deepEqual(applies, [legacy], 'another start does not apply the folder again')

engine = projectExports
applyOutputFolderStatus(engine) // a person or agent clears the engine choice
values.set('cut.outputDir', legacy) // even a stale cache cannot resurrect it
await restarted.refreshOutputFolder()
assert.deepEqual(applies, [legacy], 'a later clear is never undone')
assert.equal(getStoredOutputDir(), null)

installation(legacy)
agentBeforeSet = chosen('/agent/newer')
assert.deepEqual(await refreshOutputFolder(), chosen('/agent/newer'), 'a choice after the read wins at the server')
assert.equal(getStoredOutputDir(), '/agent/newer', 'the newer engine choice replaces the legacy cache')

installation(legacy, agentChose)
await refreshOutputFolder()
engine = projectExports
await refreshOutputFolder()
assert.deepEqual(applies, [], 'an engine choice wins, including after an agent clears it')
assert.equal(getStoredOutputDir(), null)

for (const changed of [agentChose, projectExports]) {
  installation(legacy)
  let finishRead!: () => void
  beforeRead = () => new Promise<void>((resolve) => { finishRead = resolve })
  const reading = refreshOutputFolder()
  // Let fetch parse the initial answer, but deliver the event before it returns.
  await new Promise((resolve) => setImmediate(resolve))
  applyOutputFolderStatus(changed)
  finishRead()
  assert.deepEqual(await reading, changed, 'an agent event wins over an older startup answer')
  assert.deepEqual(applies, [], 'a startup race never reapplies a stale folder')
  assert.equal(getStoredOutputDir(), changed.dir)
}

for (const [reason, expected] of [
  ['export folder not found: /gone', 'Export folder not found: /gone. Choose an existing export folder, then try again.'],
  ['output folder is not writable: /locked', 'Output folder is not writable: /locked. Choose a writable export folder, then try again.'],
]) {
  installation('/old/exports')
  refused = reason
  assert.deepEqual(await refreshOutputFolder(), projectExports)
  assert.deepEqual(applies, ['/old/exports'])
  assert.deepEqual(notices, [expected], 'the engine refusal is shown once in plain words')
  assert.equal(getStoredOutputDir(), null, 'a refused folder stays project exports')
  values.set('cut.outputDir', '/old/exports') // stale storage after another start
  await refreshOutputFolder()
  assert.deepEqual(applies, ['/old/exports'], 'refusal never loops or retries on restart')
  assert.deepEqual(notices, [expected])
}

installation(null)
await refreshOutputFolder()
values.set('cut.outputDir', legacy)
await refreshOutputFolder()
assert.deepEqual(applies, [], 'no old choice consumes carry-over without applying anything')

installation(null) // the 0.6.116 UI already removed its cache when cleared
await refreshOutputFolder()
assert.equal(getStoredOutputDir(), null, 'an already cleared installation stays cleared')
assert.deepEqual(applies, [])
console.log('PASS legacy export folder carry-over is applied once; clearing, refusal and engine choices are preserved')
