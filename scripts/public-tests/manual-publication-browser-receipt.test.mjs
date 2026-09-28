import assert from 'node:assert/strict'
import { mkdtempSync, readFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import test from 'node:test'

import {
  MANUAL_PUBLICATION_BROWSER_RECEIPT_SCHEMA,
  buildManualPublicationBrowserReceipt,
  manualPublicationBrowserBinding,
  writeManualPublicationBrowserReceipt,
} from '../lib/manual-publication-browser-receipt.mjs'

const hash = (letter) => letter.repeat(64)
const source = {
  schema: 'shellx-cut/manual-publication@1',
  route: '/manual/cut/',
  architecture: 'vite-real-frontend',
  authority: 'ui/manual.html via Vite manifest closure',
  embedUrl: '?manual=embed&mock=1',
  readOnly: true,
  relativeAssetBase: './',
  legacyScreenshotHotspotAuthority: 'rejected',
  excludedLegacyInputs: [],
  source: { gitHead: 'a'.repeat(40) },
  artifacts: [],
  identity: { artifactClosureSha256: hash('b') },
}
const environment = {
  SHELLX_CUT_MANUAL_BROWSER_RECEIPT: '/tmp/manual-browser-receipt.json',
  SHELLX_CUT_MANUAL_BROWSER_RUN_ID: 'manual-browser-r1',
  FCV_SOURCE_GIT_COMMIT: 'a'.repeat(40),
  FCV_SOURCE_GIT_TREE: 'c'.repeat(40),
  FCV_SOURCE_CONTENT_MANIFEST_SHA256: hash('d'),
  FCV_ACTION_MANIFEST_SHA256: hash('e'),
  FCV_SOURCE_VERSION: '0.6.114',
  FCV_PACKAGE_SHA256: hash('f'),
  SHELLX_CUT_MANUAL_BROWSER_PLATFORM: 'macos',
}

function rows() {
  return [
    { id: 'manual-search', interaction: 'native', outcome: 'An empty query result was replaced by the exact Settings-only published Manual result', visible: { emptyQuery: 'no match', emptyResultObserved: true, query: 'settings and cache cleanup', resultCount: 1, featureIds: ['cut.top.settings'], unrelatedFeatureAbsent: true } },
    { id: 'manual-feature', interaction: 'native', outcome: 'Settings replaced Export as the selected explanation and highlighted its embedded target', visible: { previousFeatureId: 'cut.top.export', previousHash: '#cut.top.export', previousHighlightCleared: true, previousTargetClosed: true, featureId: 'cut.top.settings', explanation: 'Settings and editing cache', highlightTarget: '[data-cut-settings-body="overview"]', settingsBodyVisible: true, exportMenuClosed: true } },
  ]
}

test('Manual browser receipt binds the exact staged manual closure and both delegated effects', (t) => {
  const binding = manualPublicationBrowserBinding(environment)
  const receipt = buildManualPublicationBrowserReceipt({
    binding,
    publicationManifest: source,
    publicationManifestBytes: JSON.stringify(source),
    rows: rows().reverse(),
    generatedAt: '2026-09-08T00:00:00.000Z',
  })
  assert.equal(receipt.schema, MANUAL_PUBLICATION_BROWSER_RECEIPT_SCHEMA)
  assert.equal(receipt.status, 'pass')
  assert.deepEqual(receipt.candidate.source, {
    gitCommit: environment.FCV_SOURCE_GIT_COMMIT,
    gitTree: environment.FCV_SOURCE_GIT_TREE,
    contentManifestSha256: environment.FCV_SOURCE_CONTENT_MANIFEST_SHA256,
    actionManifestSha256: environment.FCV_ACTION_MANIFEST_SHA256,
    version: environment.FCV_SOURCE_VERSION,
  })
  assert.equal(receipt.candidate.version, receipt.candidate.source.version)
  assert.deepEqual(receipt.ui.rows.map((row) => row.id), ['manual-feature', 'manual-search'])
  assert.equal(receipt.artifact.manualClosureSha256, source.identity.artifactClosureSha256)
  assert.equal(receipt.artifact.packageSha256, environment.FCV_PACKAGE_SHA256)
  const directory = mkdtempSync(join(tmpdir(), 'cut-manual-browser-receipt-'))
  const output = join(directory, 'receipt.json')
  t.after(() => rmSync(directory, { recursive: true, force: true }))
  const receiptSha256 = writeManualPublicationBrowserReceipt(output, receipt)
  assert.match(receiptSha256, /^[a-f0-9]{64}$/)
  assert.deepEqual(JSON.parse(readFileSync(output, 'utf8')), receipt)
  assert.throws(() => writeManualPublicationBrowserReceipt(output, receipt), /EEXIST/)
})

test('Manual browser receipt refuses incomplete candidate binding and missing action effect rows', () => {
  assert.throws(
    () => manualPublicationBrowserBinding({ ...environment, FCV_PACKAGE_SHA256: '' }),
    /FCV_PACKAGE_SHA256/,
  )
  assert.throws(
    () => buildManualPublicationBrowserReceipt({
      binding: manualPublicationBrowserBinding(environment),
      publicationManifest: source,
      publicationManifestBytes: JSON.stringify(source),
      rows: rows().slice(0, 1),
    }),
    /both delegated action rows/,
  )
  assert.throws(
    () => buildManualPublicationBrowserReceipt({
      binding: {
        ...manualPublicationBrowserBinding(environment),
        candidate: {
          ...manualPublicationBrowserBinding(environment).candidate,
          version: '0.6.115',
        },
      },
      publicationManifest: source,
      publicationManifestBytes: JSON.stringify(source),
      rows: rows(),
    }),
    /candidate versions differ/,
  )
  assert.throws(
    () => buildManualPublicationBrowserReceipt({
      binding: manualPublicationBrowserBinding(environment),
      publicationManifest: source,
      publicationManifestBytes: JSON.stringify({ ...source, route: '/another-manual/' }),
      rows: rows(),
    }),
    /bytes differ from the supplied manifest/,
  )
  assert.throws(
    () => buildManualPublicationBrowserReceipt({
      binding: manualPublicationBrowserBinding(environment),
      publicationManifest: source,
      publicationManifestBytes: 'not JSON',
      rows: rows(),
    }),
    /bytes must contain valid JSON/,
  )
  const genericOutcome = rows()
  genericOutcome[0].outcome = 'pass'
  assert.throws(
    () => buildManualPublicationBrowserReceipt({
      binding: manualPublicationBrowserBinding(environment), publicationManifest: source,
      publicationManifestBytes: JSON.stringify(source), rows: genericOutcome,
    }),
    /descriptive visible outcome/,
  )
  const missingVisibleEffect = rows()
  delete missingVisibleEffect[0].visible.unrelatedFeatureAbsent
  assert.throws(
    () => buildManualPublicationBrowserReceipt({
      binding: manualPublicationBrowserBinding(environment), publicationManifest: source,
      publicationManifestBytes: JSON.stringify(source), rows: missingVisibleEffect,
    }),
    /visible effect keys/,
  )
})
