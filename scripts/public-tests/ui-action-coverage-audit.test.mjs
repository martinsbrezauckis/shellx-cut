import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import test from 'node:test'

import {
  assessUiActionCoverage,
  inventoryInteractiveSource,
  testSourceReferencesAction,
} from '../ui-action-coverage-audit.mjs'

test('repo roots preserve Windows UNC shares by converting file URLs explicitly', () => {
  for (const script of [
    '../ui-action-coverage-audit.mjs',
    '../update-ui-action-manifest.mjs',
  ]) {
    const source = readFileSync(new URL(script, import.meta.url), 'utf8')
    assert.match(source, /fileURLToPath\(new URL\('\.\.', import\.meta\.url\)\)/)
    assert.doesNotMatch(source, /new URL\('\.\.', import\.meta\.url\)\.pathname/)
  }
})

test('interactive source inventory requires a stable primary data-cut identity', () => {
  const source = `
    // <button onClick={notReal}>Commented out</button>
    <button data-cut-save onClick={save}>Save</button>
    <button data-cut-action="undo" onClick={undo}>Undo</button>
    <select data-cut-quality value={quality}><option>High</option></select>
    <input onChange={(event) => update(event.target.value)} data-cut-after-arrow />
    <button aria-label="Missing identity" onClick={close}>x</button>
  `
  const report = inventoryInteractiveSource(source, 'Fixture.tsx')
  assert.deepEqual(report.identified.map((item) => item.action), ['save', 'undo', 'quality', 'after-arrow'])
  assert.equal(report.unidentified.length, 1)
  assert.equal(report.unidentified[0].line, 7)
})

test('dynamic data-cut-action does not collapse into a fake shared action id', () => {
  const report = inventoryInteractiveSource(
    '<button data-cut-action={actionId} onClick={run}>Run</button>',
    'Dynamic.tsx',
  )
  assert.equal(report.identified.length, 0)
  assert.equal(report.unidentified.length, 1)
})

test('explicit action identities include pointer-driven non-semantic elements', () => {
  const report = inventoryInteractiveSource(
    '<div data-cut-action="timeline-ruler" data-cut-ruler onMouseDown={seek} />',
    'TimelineRuler.tsx',
  )
  assert.deepEqual(report.identified.map((item) => item.action), ['timeline-ruler'])
  assert.equal(report.unidentified.length, 0)
})

test('test ownership recognizes literal data-cut-action values', () => {
  assert.equal(testSourceReferencesAction(
    'page.locator(\'[data-cut-action="undo"]\').click()',
    'undo',
  ), true)
  assert.equal(testSourceReferencesAction(
    'page.locator("[data-cut-save]").click()',
    'save',
  ), true)
  assert.equal(testSourceReferencesAction('page.locator("[data-cut-redo]")', 'undo'), false)
})

test('strict verdict requires zero unidentified and zero unreferenced actions', () => {
  const green = {
    summary: {
      unidentifiedInteractiveElements: 0,
      unreferencedActionIds: 0,
      unreferencedByNativeSweepActionIds: 0,
    },
  }
  assert.equal(assessUiActionCoverage(green, { strict: true }).ok, true)

  const red = {
    summary: {
      unidentifiedInteractiveElements: 2,
      unreferencedActionIds: 3,
      unreferencedByNativeSweepActionIds: 4,
    },
  }
  const verdict = assessUiActionCoverage(red, { strict: true })
  assert.equal(verdict.ok, false)
  assert.equal(verdict.missing.length, 3)
})

test('ratchet rejects growth while the strict inventory is being closed', () => {
  const report = {
    summary: {
      unidentifiedInteractiveElements: 8,
      unreferencedActionIds: 11,
      unreferencedByNativeSweepActionIds: 13,
    },
  }
  assert.equal(assessUiActionCoverage(report, {
    ratchet: {
      unidentifiedInteractiveElements: 8,
      unreferencedActionIds: 11,
      unreferencedByNativeSweepActionIds: 13,
    },
  }).ok, true)
  assert.equal(assessUiActionCoverage(report, {
    ratchet: {
      unidentifiedInteractiveElements: 7,
      unreferencedActionIds: 11,
      unreferencedByNativeSweepActionIds: 13,
    },
  }).ok, false)
  assert.equal(assessUiActionCoverage(report, {
    ratchet: {
      unidentifiedInteractiveElements: 8,
      unreferencedActionIds: 11,
      unreferencedByNativeSweepActionIds: 12,
    },
  }).ok, false)
})
