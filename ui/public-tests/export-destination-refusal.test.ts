import assert from 'node:assert/strict'
import { clearStoredOutputDirIfAccepted, ensureStoredOutputDirApplied, getStoredOutputDir } from '../src/lib/exportDestination'

const values = new Map([['cut.outputDir', '/chosen/exports']])
Object.assign(globalThis, {
  localStorage: {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => { values.set(key, value) },
    removeItem: (key: string) => { values.delete(key) },
  },
})
assert.equal(await clearStoredOutputDirIfAccepted(async () => false), false)
assert.equal(getStoredOutputDir(), '/chosen/exports', 'rejected clear preserves the selected destination')
await assert.rejects(ensureStoredOutputDirApplied(async () => false), /could not use the selected export folder/)
assert.equal(getStoredOutputDir(), '/chosen/exports', 'rejected default reassertion retains the selected destination')
assert.equal(await clearStoredOutputDirIfAccepted(async () => true), true)
assert.equal(getStoredOutputDir(), null, 'confirmed clear removes the persisted selection')
console.log('export destination: refused clear retains selected folder')
