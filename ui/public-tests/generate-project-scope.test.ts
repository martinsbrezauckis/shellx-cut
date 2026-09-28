import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'

import { GenerateRequestScopeGuard } from '../src/panels/GenerateTemplates/generateRequestScope'

async function staleTemplateCompletionIsIgnored(action: 'preview' | 'insert') {
  const guard = new GenerateRequestScopeGuard(41)
  const deferred = Promise.withResolvers<{ action: string }>()
  const request = guard.begin()
  const projected: string[] = []
  const settle = deferred.promise.then((result) => {
    if (guard.isCurrent(request)) projected.push(result.action)
  })

  guard.setProjectScope(42)
  deferred.resolve({ action })
  await settle
  assert.deepEqual(projected, [], `a stale ${action} completion cannot update the replacement project`)
}

await staleTemplateCompletionIsIgnored('preview')
await staleTemplateCompletionIsIgnored('insert')

const guard = new GenerateRequestScopeGuard(41)
const deferred = Promise.withResolvers<{ status: 'completed' }>()
const first = guard.begin()
const projected: string[] = []
const settle = deferred.promise.then((result) => {
  if (guard.isCurrent(first)) projected.push(result.status)
})

guard.setProjectScope(42)
deferred.resolve({ status: 'completed' })
await settle
assert.deepEqual(projected, [], 'a completed A request cannot project a result after B becomes current')

const current = guard.begin()
assert.equal(guard.isCurrent(current), true, 'the current project owns its new request')
guard.setProjectScope(43)
assert.equal(guard.isCurrent(current), false, 'a switch invalidates an already-issued request synchronously')

const root = resolve(import.meta.dirname, '..')
const leftPanel = readFileSync(resolve(root, 'src/panels/LeftPanel/index.tsx'), 'utf8')
const generate = readFileSync(resolve(root, 'src/panels/GenerateTemplates/index.tsx'), 'utf8')
assert.match(leftPanel, /<GenerateTemplatesWorkspace[\s\S]*projectScope=\{projectScope\}/, 'the persistent Generate panel receives App project scope')
assert.match(generate, /templateRequestGuard = useRef\(new GenerateRequestScopeGuard\(projectScope\)\)/, 'Generate owns a template request guard at its project boundary')
assert.match(generate, /\}, \[projectScope\]\)/, 'Generate clears project-scoped result state on a project switch')
assert.match(generate, /const runPreview[\s\S]*?!templateRequestGuard\.current\.isCurrent\(request\)/, 'a stale preview completion is ignored after its project is replaced')
assert.match(generate, /const runInsert[\s\S]*?!templateRequestGuard\.current\.isCurrent\(request\)/, 'a stale insert completion is ignored after its project is replaced')
assert.match(generate, /setPreview\(null\)/, 'a project switch clears the stale preview')
assert.match(generate, /setInsertResult\(null\)/, 'a project switch clears the stale insert result')
assert.match(generate, /!promptRequestGuard\.current\.isCurrent\(request\)/, 'a prompt completion is ignored after its project is replaced')
assert.match(generate, /!storyboardRequestGuard\.current\.isCurrent\(request\)/, 'a storyboard completion is ignored after its project is replaced')

console.log('PASS Generate project-scope late-completion guard')
