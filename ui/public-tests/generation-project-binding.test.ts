import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import type { Project } from '../src/lib/client'
import { GenerateRequestScopeGuard } from '../src/panels/GenerateTemplates/generateRequestScope'
import {
  generationHistoryMatchesScope,
  generationJobStorageKey,
  generationProjectKey,
  readStoredGenerationJob,
} from '../src/panels/Generate/generationProjectScope'

// Same visible name and colliding asset ID; only the origin identity distinguishes these projects.
const project = (origin: string) => ({
  name: 'Campaign',
  project_identity: { origin_path_sha256: origin, project_name: 'Campaign' },
  assets: { 'same-asset-id': { hash: 'same-hash' } },
}) as unknown as Project
const projectA = project('origin-A')
const projectB = project('origin-B')
const keyA = generationProjectKey(projectA, 10)
const keyB = generationProjectKey(projectB, 11)
assert.ok(keyA && keyB)
assert.notEqual(keyA, keyB)
assert.notEqual(generationJobStorageKey(keyA), generationJobStorageKey(keyB))

const storage = new Map<string, string>()
const guard = new GenerateRequestScopeGuard(10)
const oldRequest = guard.begin()
const lateJob = Promise.withResolvers<{ job_id: string }>()
const settle = lateJob.promise.then((result) => {
  storage.set(generationJobStorageKey(keyA), JSON.stringify({ ...result, project_key: keyA }))
  return guard.isCurrent(oldRequest)
})
guard.setProjectScope(11)
lateJob.resolve({ job_id: 'job-for-A' })
assert.equal(await settle, false, 'late A result cannot become visible in B')
assert.equal(readStoredGenerationJob(storage.get(generationJobStorageKey(keyB)) ?? null, keyB), null, 'B cannot resume A job')
assert.equal(readStoredGenerationJob(storage.get(generationJobStorageKey(keyA)) ?? null, keyA)?.job_id, 'job-for-A', 'A job remains resumable when A returns')
assert.equal(readStoredGenerationJob(JSON.stringify({ job_id: 'unbound-legacy' }), keyB), null, 'legacy unbound job cannot be assigned to B')

const historyGuard = new GenerateRequestScopeGuard(10)
const oldHistoryRequest = historyGuard.begin()
const lateHistory = Promise.withResolvers<{ asset_id: string }[]>()
let visibleHistory: { asset_id: string }[] = []
const settleHistory = lateHistory.promise.then((items) => {
  if (historyGuard.isCurrent(oldHistoryRequest)) visibleHistory = items
})
historyGuard.setProjectScope(11)
lateHistory.resolve([{ asset_id: 'same-asset-id' }])
await settleHistory
assert.deepEqual(visibleHistory, [], 'late A history cannot replace B history even when asset IDs collide')
assert.equal(generationHistoryMatchesScope(10, 11), false, 'a displayed A record cannot initiate placement in B')
assert.equal(generationHistoryMatchesScope(11, 11), true, 'a current B record can be placed in B')

const generateSource = readFileSync(resolve(import.meta.dirname, '../src/panels/Generate/index.tsx'), 'utf8')
const workspaceSource = readFileSync(resolve(import.meta.dirname, '../src/panels/GenerateTemplates/index.tsx'), 'utf8')
assert.match(workspaceSource, /<GenerateAssetSurface[\s\S]*?projectScope=\{projectScope\}/, 'embedded media Generate receives the project transition scope')
assert.match(generateSource, /const historyRequestGuard = useRef\(new GenerateRequestScopeGuard\(projectScope\)\)/, 'history responses use the project scope guard')
assert.match(generateSource, /generationHistoryMatchesScope\(historyOwnerScopeRef\.current, projectScope\)/, 'history actions enforce their owner scope')
assert.match(generateSource, /generationJobStorageKey\(ownerKey\)/, 'queued jobs persist under their request project key')

console.log('Generate media project binding checks passed')
