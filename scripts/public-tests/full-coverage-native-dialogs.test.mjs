import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import test from 'node:test'

const fullCoverageUrl = new URL('../../ui/public-tests/full-coverage-verify.mjs', import.meta.url)
const projectTransitionUrl = new URL('../../ui/public-tests/lib/fullCoverageProjectTransition.mjs', import.meta.url)
const assetsCoverageUrl = new URL('../../ui/public-tests/lib/fullCoverageAssetsActions.mjs', import.meta.url)
const assetsSetupUrl = new URL('../../ui/public-tests/lib/fullCoverageAssetsSetupActions.mjs', import.meta.url)
const assetsPickerUrl = new URL('../../ui/public-tests/lib/fullCoverageAssetsPickerActions.mjs', import.meta.url)
const offlineMediaCoverageUrl = new URL('../../ui/public-tests/lib/fullCoverageOfflineMediaActions.mjs', import.meta.url)
const libraryCoverageUrl = new URL('../../ui/public-tests/lib/fullCoverageLibraryActions.mjs', import.meta.url)
const sequenceCoverageUrl = new URL('../../ui/public-tests/lib/fullCoverageSequenceSwitcherActions.mjs', import.meta.url)

test('native WebView sweeps delegate modal OS pickers instead of poisoning later actions', async () => {
  const source = await readFile(fullCoverageUrl, 'utf8')

  assert.match(source, /const NATIVE_PICKER_CLICK_NA = UI_DRIVER === 'playwright-chromium' \|\| NATIVE_OS_ACTIONS[.]enabled/)
  for (const name of ['caption-import', 'export-choose-folder', 'render-queue-output-picker']) {
    const start = source.indexOf(`name: '${name}'`)
    assert.notEqual(start, -1, `${name} row exists`)
    const probe = source.slice(start, start + 1_000)
    assert.match(probe, /clickNa: NATIVE_PICKER_CLICK_NA/, `${name} delegates its native modal click`)
    assert.match(probe, /nativeAction: \{[\s\S]*mode: 'select'/, `${name} selects a real path when a host controller is paired`)
    assert.match(probe, /verifyResult: true/, `${name} verifies the selected path reached the app`)
  }
  assert.match(
    source,
    /name: 'render-queue-output-picker'[\s\S]+basenameHostPath\(selected\) === basenameHostPath\(queueOutputPath\)/,
    'Render Queue proves the host-selected output landed in the exact row',
  )
  assert.match(
    source,
    /firstQueueOutput = await page[.]locator\('\[data-cut-render-queue-output="0"\]'\)[.]inputValue\(\)[\s\S]+dirnameHostPath\(firstQueueOutput\)[\s\S]+queueSecondOutputName/,
    'Render Queue derives sibling outputs from the host-canonical picker directory',
  )

  const exportLoop = source.indexOf('for (const opt of EXPORT_OPTIONS)')
  assert.notEqual(exportLoop, -1, 'Save As option loop exists')
  const saveAsLoop = source.indexOf('for (const opt of EXPORT_OPTIONS)', exportLoop + 1)
  assert.notEqual(saveAsLoop, -1, 'installed Save As option loop exists')
  assert.match(
    source.slice(saveAsLoop, saveAsLoop + 5_000),
    /name: `export-save-as-\$\{opt\.id\}`[\s\S]*actionId: 'export-saveas-option'/,
    'every installed Save As option has an exact runtime action row',
  )
  assert.match(
    source.slice(saveAsLoop, saveAsLoop + 5_000),
    /nativeAction: \{[\s\S]*mode: 'select'[\s\S]*path: chosenPath[\s\S]*verifyResult: true/,
    'every installed Save As option selects and verifies a real host path',
  )
  assert.match(source, /name: 'library[.]add[(]Browse native picker[)]'[\s\S]+nativeAction: \{ mode: 'select', path: browseFixture/)
  assert.match(source, /name: 'library[.]relink[(]conditional native picker[)]'[\s\S]+nativeAction: \{ mode: 'select', path: pair[.]replacementEngine/)
})

test('destructive confirmations use exact host-owned accept actions', async () => {
  const fullCoverage = await readFile(fullCoverageUrl, 'utf8')
  const assets = await readFile(assetsCoverageUrl, 'utf8')
  const library = await readFile(libraryCoverageUrl, 'utf8')
  const sequences = await readFile(sequenceCoverageUrl, 'utf8')

  for (const [name, source] of [
    ['project delete', fullCoverage],
    ['asset remove', assets],
    ['library bulk remove', library],
    ['sequence delete', sequences],
  ]) {
    assert.match(
      source,
      /nativeAction: \{ mode: 'accept', useDoClick: true, verifyResult: true \}/,
      `${name} accepts and verifies its real host confirmation`,
    )
  }
})

test('project transitions wait for earlier installed background work', async () => {
  const source = await readFile(fullCoverageUrl, 'utf8')
  const transition = await readFile(projectTransitionUrl, 'utf8')
  assert.match(
    source,
    /async function freshProject[\s\S]{0,250}const drained = await drainActiveJobs[(][)][\s\S]{0,180}if [(][!]drained[)][\s\S]{0,400}createProjectWithRetry[(]\{/,
    'every section drains active jobs before attempting its fresh project transition',
  )
  assert.match(
    transition,
    /response = await verb[(]'project[.]create', \{ name, settings \}\)[\s\S]{0,220}response[?][.]error[?][.]code [!]== 'job_cancel_pending'/,
    'fresh project setup retries only the temporary worker-drain refusal',
  )
  assert.match(
    transition,
    /timeoutMs = 180_000[\s\S]{0,800}elapsed >= timeoutMs[\s\S]{0,250}still returned job_cancel_pending/,
    'fresh project retry is bounded and reports exhaustion',
  )
  assert.match(
    source,
    /const projectPath = created[.]result[?][.]path [|][|] ''[\s\S]{0,220}if [(][!]created[.]ok [|][|] [!]projectPath[)][\s\S]{0,350}if [(][!]imported[.]ok [|][|] [!]assetId[)]/,
    'fresh project setup fails fast when create or import did not establish isolation',
  )
  assert.match(
    source,
    /async function secTimelineDialogActions[\s\S]{0,300}await drainActiveJobs[(][)]/,
    'timeline dialog fixtures do not race preceding Inspector jobs',
  )
  assert.match(
    source,
    /async function secComments[\s\S]{0,300}await drainActiveJobs[(][)]/,
    'review handoff render waits for project import work',
  )
  assert.match(
    source,
    /comments-render-failure[.]json[\s\S]{0,500}render_job: handoffRenderJob[\s\S]{0,300}project: await state[(][)]/,
    'review handoff preserves the exact render error and project asset paths before cleanup',
  )
  assert.match(
    source,
    /const fname = 'fcv_forget_'[\s\S]{0,250}await drainActiveJobs[(][)][\s\S]{0,250}const created = await verb[(]'project[.]create'/,
    'residual project.forget proves a successful throwaway project transition',
  )
  assert.match(
    source,
    /async function drainActiveJobs[(]maxMs = 600000[)]/,
    'installed section isolation tolerates slow but still-bounded release enrichment',
  )
  assert.match(
    source,
    /freshProject[(][\s\S]{0,500}activeJobSummary[(][)]/,
    'fresh project failures name the jobs that prevented isolation',
  )
})

test('Assets native WebView sweep delegates every OS-owned picker', async () => {
  const fullCoverage = await readFile(fullCoverageUrl, 'utf8')
  const source = `${await readFile(assetsCoverageUrl, 'utf8')}\n${await readFile(assetsSetupUrl, 'utf8')}`
  const picker = await readFile(assetsPickerUrl, 'utf8')
  const offlineMedia = await readFile(offlineMediaCoverageUrl, 'utf8')

  assert.match(picker, /clickNa: nativePickerClickNa/)
  assert.match(picker, /mode: selectPath && nativeOsActionsEnabled \? 'select' : 'cancel'/)
  assert.match(picker, /verifyResult: !!selectPath && nativeOsActionsEnabled/)
  assert.match(picker, /selectedResponse = await captureVerbResp/)
  assert.match(picker, /await waitForState/)
  assert.match(picker, /selectVerb === 'media[.]relink'/)
  assert.match(picker, /selectedResponse[?][.]result[?][.]asset === selectAsset/)
  for (const selector of [
    '[data-cut-import-cta]',
    '[data-cut-action="import-asset"]',
    '[data-cut-import-otio]',
    '[data-cut-media-health-relink-first="${missing.asset}"]',
    '[data-cut-action="relink-asset"]',
  ]) {
    assert.ok(source.includes(selector), `${selector} is routed through the picker probe`)
  }
  assert.match(
    source,
    /name: 'media-health-relink-first'[\s\S]+selectPath: relinkPair[.]replacementEngine[\s\S]+selectVerb: 'media[.]relink'/,
    'Media Health Relink selects a real replacement through its own native action',
  )
  assert.match(
    source,
    /name: 'relink-asset'[\s\S]+selectPath: relinkPair[.]fourthReplacementEngine[\s\S]+selectVerb: 'media[.]relink'/,
    'Asset-card Relink selects a fourth real replacement through its own native action',
  )
  assert.match(source, /runOfflineMediaRelinkCoverage[(]\{/,
    'Assets coverage installs the shared offline-surface native action chain')
  assert.match(offlineMedia,
    /name: 'preview-relink-offline'[\s\S]+data-cut-preview-relink[\s\S]+selectPath: relinkPair[.]secondReplacementEngine/,
    'Preview Relink selects and verifies a real second replacement')
  assert.match(offlineMedia,
    /name: 'timeline-relink-offline'[\s\S]+data-cut-timeline-relink[\s\S]+selectPath: relinkPair[.]thirdReplacementEngine/,
    'Timeline Relink selects and verifies a real third replacement')
  assert.match(offlineMedia,
    /unlinkSync[(]relinkPair[.]thirdReplacementDriver[)][\s\S]+data-cut-asset-offline/,
    'the chain leaves the fixture offline for the Asset-card Relink action')
  assert.match(source, /selectPath: relinkPair[.]fourthReplacementEngine/,
    'Asset-card Relink owns the final replacement')
  for (const driverPath of [
    'replacementDriver',
    'secondReplacementDriver',
    'thirdReplacementDriver',
  ]) {
    assert.match(fullCoverage, new RegExp(`${driverPath},`),
      `${driverPath} is available to the installed action controller`)
  }
})

test('paired native confirmations have exactly one acceptance owner', async () => {
  const fullCoverage = await readFile(fullCoverageUrl, 'utf8')
  const library = await readFile(libraryCoverageUrl, 'utf8')

  assert.match(
    fullCoverage,
    /nativeOsActionsEnabled: NATIVE_OS_ACTIONS[.]enabled/,
    'the installed controller capability reaches Library action coverage',
  )
  assert.match(
    library,
    /if [(][!]nativeOsActionsEnabled[)] page[.]on[(]'dialog', accept[)]/,
    'CDP auto-accept is disabled when the installed host controller owns the TaskDialog',
  )
  assert.match(
    library,
    /if [(][!]nativeOsActionsEnabled[)] page[.]off[(]'dialog', accept[)]/,
    'the conditional CDP dialog listener is removed symmetrically',
  )
})

test('Library folder deletion waits for its asynchronous reload', async () => {
  const source = await readFile(fullCoverageUrl, 'utf8')
  assert.match(
    source,
    /const removedChip = page[.]locator[\s\S]{0,220}removedChip[.]waitFor[(]\{ state: 'detached', timeout: 8_000 \}[)]/,
    'folder_remove does not race its immediate verb response against the panel reload',
  )
})
