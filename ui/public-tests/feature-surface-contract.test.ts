// Executable public-surface contract; private change procedures stay outside it.
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const uiRoot = resolve(here, '..')
const repoRoot = resolve(uiRoot, '..')
const surfaceContract = readFileSync(resolve(repoRoot, 'docs/public/FEATURE_SURFACE_CONTRACT.md'), 'utf8')
const agentHandoff = readFileSync(resolve(repoRoot, 'START_HERE_FOR_AGENT.txt'), 'utf8')
const packageJson = JSON.parse(readFileSync(resolve(uiRoot, 'package.json'), 'utf8')) as { scripts?: Record<string, string> }
const libraryRunner = readFileSync(resolve(uiRoot, 'public-tests/lib/runLibraryTests.mjs'), 'utf8')
const schema = JSON.parse(readFileSync(resolve(repoRoot, 'schema/verbs.json'), 'utf8'))
const tauriConfig = JSON.parse(readFileSync(resolve(repoRoot, 'app/desktop/src-tauri/tauri.conf.json'), 'utf8')) as {
  bundle?: { resources?: Record<string, string> }
}
const agentDocs = readFileSync(resolve(repoRoot, 'scripts/lib/agent-docs.mjs'), 'utf8')
const agentApi = readFileSync(resolve(repoRoot, 'app/server/src/http.rs'), 'utf8')
const stockPanel = [
  'src/panels/Stock/index.tsx',
  'src/panels/Stock/StockResults.tsx',
  'src/components/NativeFolderPicker.tsx',
].map((path) => readFileSync(resolve(uiRoot, path), 'utf8')).join('\n')
const assemblePanel = [
  'src/panels/Assemble/index.tsx',
  'src/components/NativeFolderPicker.tsx',
].map((path) => readFileSync(resolve(uiRoot, path), 'utf8')).join('\n')
const portableCopy = [
  'src/panels/Projects/PortableCopy.tsx',
  'src/components/NativeFolderPicker.tsx',
].map((path) => readFileSync(resolve(uiRoot, path), 'utf8')).join('\n')
const gradePanel = readFileSync(resolve(uiRoot, 'src/panels/Grade/index.tsx'), 'utf8')
const renderQueue = readFileSync(resolve(uiRoot, 'src/topbar/RenderQueueModal.tsx'), 'utf8')
const readme = readFileSync(resolve(repoRoot, 'README.md'), 'utf8')
const publicFeatures = readFileSync(resolve(repoRoot, 'docs/public/FEATURES.md'), 'utf8')
const manual = readFileSync(resolve(repoRoot, 'docs/public/site/manual/manual.js'), 'utf8')
const retiredWorkflow = ['FEATURE', 'CHANGE', 'WORKFLOW'].join('_')

const surfaceRow = (exposure: string) => {
  const match = surfaceContract.match(new RegExp('^\\| `' + exposure + '` \\| (.+) \\| (.+) \\|$', 'm'))
  assert.ok(match, `surface contract is missing ${exposure}`)
  return match[0]
}

const assertSurface = (id: string, phrases: string[]) => {
  for (const phrase of phrases) assert.ok(surfaceContract.includes(phrase), `${id}: public surface contract must include ${phrase}`)
}

const assertTaxonomy = (id: string, exposure: string, phrases: string[]) => {
  const row = surfaceRow(exposure)
  for (const phrase of phrases) assert.ok(row.includes(phrase), `${id}: ${exposure} must include ${phrase}`)
}

assertSurface('LLA-012', ['discoverable UI path', 'data-cut-*', 'ui.open', 'ui.state'])
assertSurface('LLA-013', ['Optional capabilities and installables', 'user outcome', 'one primary next action', 'Advanced details'])
assertSurface('LLA-014', ['ui.open', 'ui.state', 'ui.screenshot', 'Debug API or MCP surface'])
assertSurface('LLA-015', ['skill/shellx-cut/SKILL.md', 'skill/shellx-cut/reference.md', 'GET /api/agent'])
assertSurface('LLA-016', ['README.md', 'docs/public/FEATURES.md', 'user manual'])
assertSurface('LLA-019', ['ui.screenshot', 'visual inspection'])
assertTaxonomy('LLA-023', 'human', ['discoverable UI path', 'ui.open', 'ui.state'])
assertTaxonomy('LLA-024', 'agent_only', ['intentionally callable by an agent', 'schema/verbs.json', 'skill/shellx-cut/SKILL.md'])
assertTaxonomy('LLA-025', 'internal', ['implementation detail', 'not an advertised human or agent feature'])
assertTaxonomy('LLA-026', 'rig_only', ['deterministic verification rig or fixture', 'not represent it as product availability'])

assert.match(JSON.stringify(schema), /"human"[\s\S]*"agent_only"[\s\S]*"internal"[\s\S]*"rig_only"/)
assert.equal(
  tauriConfig.bundle?.resources?.['../../../docs/public/FEATURE_SURFACE_CONTRACT.md'],
  'agent-docs/docs/public/FEATURE_SURFACE_CONTRACT.md',
)
assert.equal(tauriConfig.bundle?.resources?.[`../../../docs/public/${retiredWorkflow}.md`], undefined)
assert.match(agentDocs, /id: "feature-surfaces", path: "docs\/public\/FEATURE_SURFACE_CONTRACT\.md", advertised: true/)
assert.match(agentApi, /"id": "feature-surfaces", "path": "docs\/public\/FEATURE_SURFACE_CONTRACT\.md"/)
assert.doesNotMatch(surfaceContract, /(?:change summary|Before editing|package\.json|release operator)/i)
assert.match(agentHandoff, /ShellX Cut agent handoff/)
assert.match(agentHandoff, /docs\/public\/FEATURE_SURFACE_CONTRACT\.md/)
assert.match(agentHandoff, /This is not a public export checklist/)
assert.doesNotMatch(agentHandoff, new RegExp(retiredWorkflow))
assert.doesNotMatch(agentDocs, new RegExp(retiredWorkflow))
assert.doesNotMatch(agentApi, new RegExp(retiredWorkflow))
assert.equal(packageJson.scripts?.['test:feature-surface-contract'], 'tsx public-tests/feature-surface-contract.test.ts')
assert.equal(packageJson.scripts?.['test:lib'], 'node public-tests/lib/runLibraryTests.mjs')
assert.match(libraryRunner, /discoverLibraryTests\(\)/, 'the canonical library runner discovers this test instead of pinning a manual list')

for (const name of ['assets.providers', 'assets.search', 'assets.fetch']) {
  assert.equal(schema.verbs.find((verb: { name: string; behavior?: { ui_exposure?: string } }) => verb.name === name)?.behavior?.ui_exposure, 'human',
    `${name} is human-visible only when Find media can exercise it`)
}
assert.match(stockPanel, /callVerb\('assets\.providers', \{\}\)/, 'Find media must discover its provider catalog from the matching server')
assert.doesNotMatch(stockPanel, /const PROVIDERS\s*=/, 'Find media must not hardcode a provider availability list')
for (const selector of [
  'data-cut-stock-providers-status',
  'data-cut-stock-provider-note',
  'data-cut-stock-hit-license',
  'data-cut-stock-hit-attribution',
]) {
  assert.match(stockPanel, new RegExp(selector), `Find media must expose ${selector} for inspection`)
}
const stockImportCoordinator = readFileSync(new URL('../src/panels/Stock/importCoordinator.ts', import.meta.url), 'utf8')
const appSource = readFileSync(new URL('../src/App.tsx', import.meta.url), 'utf8')
assert.match(stockPanel, /importCoordinator\.begin\(projectScope, hit\.id\)/, 'Find media synchronously admits every import through its app-owned coordinator before dispatch')
assert.match(stockPanel, /if \(!requestStateAfterStart\) return/, 'a queued second-hit handler must not dispatch assets.fetch after admission is refused')
assert.match(stockPanel, /disabled=\{importDisabled\}/, 'every Import button is disabled while any import is in flight')
assert.match(stockPanel, /data-cut-stock-import-status/, 'Find media exposes an accessible all-import busy reason')
assert.match(appSource, /createStockImportCoordinator\(\)/, 'App creates exactly one lifetime-owned Find-media import coordinator')
assert.match(appSource, /stockImportCoordinator\.setProjectScope\(projectSessionRef\.current\)/, 'a project switch clears old Find-media presentation state without opening an old in-flight request')
assert.match(appSource, /projectScope=\{projectSession\}/, 'App forwards the monotonic project identity to the workspace')
assert.match(stockPanel, /projectScope: number/, 'Find media receives the App project identity instead of inferring one from project name')
assert.match(stockImportCoordinator, /activeRequestScope/, 'the coordinator retains an old request lock until it settles')
assert.match(stockImportCoordinator, /scope === snapshot\.projectScope/, 'old-project imports may never publish Added state into the new project')
assert.match(stockImportCoordinator, /scope !== snapshot\.projectScope\) return null/, 'a stale project handler cannot admit a request after a project switch')
assert.match(stockPanel, /pickFolder\(\{ title: dialogTitle/, 'Find media local-folder search uses the native folder picker')
assert.match(stockPanel, /data-cut-stock-dir-choose/, 'Find media exposes a stable folder-picker control')
assert.doesNotMatch(stockPanel, /data-cut-stock-dir\s+type=["']text/, 'Find media must never ask users to type a folder path')
assert.match(assemblePanel, /pickFolder\(\{ title: dialogTitle/, 'AI B-roll uses the native folder picker')
assert.match(assemblePanel, /data-cut-assemble-dir-choose/, 'AI B-roll exposes a stable folder-picker control')
assert.doesNotMatch(assemblePanel, /data-cut-assemble-dir[^-][\s\S]{0,120}(?:<input|onChange)/, 'AI B-roll must never ask users to type a folder path')
for (const name of ['project.package_plan', 'project.package_create']) {
  assert.equal(schema.verbs.find((verb: { name: string; behavior?: { ui_exposure?: string } }) => verb.name === name)?.behavior?.ui_exposure, 'human',
    `${name} is human-visible only when Projects can drive its real preview-first flow`)
}
assert.match(portableCopy, /pickFolder\(\{ title: dialogTitle/, 'Portable Copy uses the native folder picker')
assert.match(portableCopy, /data-cut-portable-dir-choose/, 'Portable Copy exposes a stable native destination control')
assert.doesNotMatch(portableCopy, /data-cut-portable-dir[^-][\s\S]{0,120}(?:<input|onChange)/, 'Portable Copy must never ask users to type a destination path')
assert.match(portableCopy, /callVerb\('project\.package_plan'/, 'Portable Copy gets counts and collision truth from the server plan')
assert.match(portableCopy, /callVerb\('project\.package_create'/, 'Portable Copy creates only after the explicit confirmation route')
assert.equal([...portableCopy.matchAll(/b5_receipt: b5Receipt/g)].length, 2,
  'Portable Copy carries current durable B5 relink evidence into both package calls')
assert.match(portableCopy, /callVerb\('jobs\.status'/, 'Portable Copy displays only actual job state and progress')
assert.match(portableCopy, /data-cut-portable-review/, 'Portable Copy has a deliberate review-before-create step')
assert.match(gradePanel, /data-cut-grade-lut-pick/, 'Color grading selects LUT files through the native picker')
assert.doesNotMatch(gradePanel, /data-cut-grade-lut(?:\s|>)[\s\S]{0,120}<input|Paste \.cube path|Advanced path/, 'Color grading must not expose an editable LUT path')
assert.match(renderQueue, /data-cut-render-queue-output-pick/, 'Render queue rows select output files through the native save picker')
assert.doesNotMatch(renderQueue, /<input[\s\S]{0,180}data-cut-render-queue-output/, 'Render queue must not expose editable output paths')
for (const source of [stockPanel, readme, publicFeatures, manual]) {
  assert.match(source, /search or import a\s+result/, 'network-provider copy must disclose both search and import/download contact')
  assert.doesNotMatch(source, /only when you search/, 'network-provider copy must not falsely promise search-only contact')
}

console.log('PASS public feature-surface contract')
