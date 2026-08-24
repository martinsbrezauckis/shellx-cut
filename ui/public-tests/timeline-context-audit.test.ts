import { strict as assert } from 'node:assert'
import { readFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { createTimelineContextAuditCoverage, TIMELINE_CONTEXT_AUDIT_SCENARIOS } from './lib/fullCoverageTimelineContextAuditScenarios.mjs'

const ids = TIMELINE_CONTEXT_AUDIT_SCENARIOS.map((scenario) => scenario.id)
assert.deepEqual(ids, ['e2e-context-ownership-01', 'e2e-empty-paste-01', 'e2e-gap-boundary-01'])
for (const scenario of TIMELINE_CONTEXT_AUDIT_SCENARIOS) {
  assert.equal(scenario.runner, 'timeline-context-audit')
  assert.equal(scenario.surface, 'browser-ui')
  assert.equal(scenario.receiptSchema, 'shellx-cut/full-coverage-results@1')
  assert.equal(scenario.receiptRoot, 'full-coverage')
  assert.ok(scenario.command.some((part) => part.includes(scenario.id)), `${scenario.id} has a focused command`)
}

for (const id of ids) {
  const emitted: Array<{ name: string; actionId: string }> = []
  const page = { locator: () => ({ first: () => ({}) }) }
  await createTimelineContextAuditCoverage({
    probe: async (_page: unknown, row: { name: string; actionId: string }) => { emitted.push(row) },
  } as never).run(page as never, { only: id })
  assert.deepEqual(emitted.map((row) => [row.name, row.actionId]), [[id, id]], `${id} emits one exact canonical row`)
}

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const read = (relative: string) => readFileSync(resolve(root, relative), 'utf8')
const surfaceMenu = read('src/panels/Timeline/TimelineSurfaceContextMenu.tsx')
const runner = read('public-tests/lib/fullCoverageTimelineContextActions.mjs')
const scenarios = read('public-tests/lib/fullCoverageTimelineContextAuditScenarios.mjs')
for (const hook of ['data-cut-timeline-context-kind', 'data-cut-timeline-context-track', 'data-cut-timeline-context-at-ms']) {
  assert.match(surfaceMenu, new RegExp(hook), `Timeline exposes ${hook} for rendered context-target proof`)
}
assert.match(runner, /createTimelineContextAuditCoverage[\s\S]+FCV_ONLY/, 'the canonical context runner invokes the audit scenarios')
assert.match(runner, /selectClipPair[\s\S]+createTimelineContextAuditCoverage/, 'the canonical context runner injects the shared cross-platform additive-selection helper')
assert.match(scenarios, /pairSelected = await selectClipPair\(page, fixture[.]a, fixture[.]b\)/, 'context ownership uses the shared native Mac modifier fallback')
assert.doesNotMatch(scenarios, /click\(\{ modifiers: \['ControlOrMeta'\] \}\)/, 'context ownership does not duplicate the native modifier path without its Mac fallback')
assert.match(scenarios, /pasteRefusalAlert[\s\S]+data-cut-user-action-feedback[\s\S]+aria-live/, 'unavailable context pastes assert the visible, assertive feedback surface')
assert.match(scenarios, /unavailableAlert = await pasteRefusalAlert[\s\S]+const alert = await pasteRefusalAlert/, 'both empty-lane and gap paste refusals require feedback before the scenario passes')
console.log('PASS timeline-context audit metadata and target hooks')
