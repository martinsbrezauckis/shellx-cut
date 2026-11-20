import assert from 'node:assert/strict'
import {
  applyOutputFolderStatus,
  clearStoredOutputDirIfAccepted,
  ensureExportDestinationUsable,
  getOutputFolderStatus,
  getStoredOutputDir,
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
