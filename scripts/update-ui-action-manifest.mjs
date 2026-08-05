#!/usr/bin/env node
import { readFileSync, writeFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

import {
  assessUiActionCoverage,
  buildUiActionCoverageAudit,
} from './ui-action-coverage-audit.mjs'

const REPO_ROOT = resolve(fileURLToPath(new URL('..', import.meta.url)))
export const UI_ACTION_MANIFEST_PATH = resolve(
  REPO_ROOT,
  'ui/public-tests/full-ui-action-manifest.json',
)

export function buildUiSourceActionManifest(options = {}) {
  const report = buildUiActionCoverageAudit(options)
  const verdict = assessUiActionCoverage(report, { strict: true })
  if (!verdict.ok) {
    throw new Error(`strict UI action coverage is incomplete: ${verdict.missing.join('; ')}`)
  }
  const actions = report.actions.map((action) => action.id).sort()
  return {
    schema: 'shellx-cut/ui-source-action-manifest@1',
    actionCount: actions.length,
    actions,
  }
}

export function serializeUiSourceActionManifest(manifest) {
  return `${JSON.stringify(manifest, null, 2)}\n`
}

function main() {
  const next = serializeUiSourceActionManifest(buildUiSourceActionManifest())
  if (process.argv.includes('--write')) {
    writeFileSync(UI_ACTION_MANIFEST_PATH, next, 'utf8')
    console.log(`wrote ${UI_ACTION_MANIFEST_PATH}`)
    return
  }
  let current = ''
  try {
    current = readFileSync(UI_ACTION_MANIFEST_PATH, 'utf8')
  } catch {}
  if (current !== next) {
    console.error(
      `UI source-action manifest is stale or missing: ${UI_ACTION_MANIFEST_PATH}\n` +
      'After reviewing the strict inventory, refresh it with: node scripts/update-ui-action-manifest.mjs --write',
    )
    process.exitCode = 1
    return
  }
  console.log(`PASS UI source-action manifest (${JSON.parse(current).actionCount} actions)`)
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  main()
}
