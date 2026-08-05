#!/usr/bin/env node
import { readdirSync, readFileSync } from 'node:fs'
import { relative, resolve } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

const REPO_ROOT = resolve(fileURLToPath(new URL('..', import.meta.url)))
const ELEMENT_START = /<([a-z][a-z0-9.-]*)\b/g
const SEMANTIC_INTERACTIVE = new Set(['button', 'input', 'select', 'textarea', 'summary'])
const STATIC_ACTION_VALUE = /\bdata-cut-action\s*=\s*(["'])([a-z0-9][a-z0-9._:-]*)\1/
const PRIMARY_ACTION_ID = /\bdata-cut-(?!action\b)([a-z0-9-]+)/

// Initial ratchet from the fresh inventory. These are gaps to close,
// not acceptable release totals. --strict requires both to reach zero.
export const ACTION_COVERAGE_RATCHET = {
  unidentifiedInteractiveElements: 0,
  unreferencedActionIds: 154,
  unreferencedByNativeSweepActionIds: 261,
}

function filesUnder(root, extensions) {
  const out = []
  const visit = (dir) => {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      const path = resolve(dir, entry.name)
      if (entry.isDirectory()) visit(path)
      else if (extensions.some((extension) => entry.name.endsWith(extension))) out.push(path)
    }
  }
  visit(root)
  return out.sort()
}

function lineAt(source, index) {
  return source.slice(0, index).split('\n').length
}

function compactTag(tag) {
  return tag.replace(/\s+/g, ' ').slice(0, 180)
}

function withoutComments(source) {
  const preserveLines = (match) => match.replace(/[^\n]/g, ' ')
  return source
    .replace(/\/\*[\s\S]*?\*\//g, preserveLines)
    .replace(/(^|[^:])\/\/[^\n]*/gm, preserveLines)
}

function interactiveTags(source) {
  const tags = []
  for (const match of source.matchAll(ELEMENT_START)) {
    let braces = 0
    let quote = ''
    let escaped = false
    let end = -1
    for (let index = (match.index || 0) + match[0].length; index < source.length; index += 1) {
      const char = source[index]
      if (quote) {
        if (escaped) escaped = false
        else if (char === '\\') escaped = true
        else if (char === quote) quote = ''
        continue
      }
      if (char === '"' || char === "'" || char === '`') {
        quote = char
      } else if (char === '{') {
        braces += 1
      } else if (char === '}') {
        braces = Math.max(0, braces - 1)
      } else if (char === '>' && braces === 0) {
        end = index + 1
        break
      }
    }
    if (end > 0) {
      const tagSource = source.slice(match.index || 0, end)
      // Pointer-driven surfaces such as timeline clips, seams, ruler ranges,
      // transcript words, and placement pads are real UI actions even though
      // their rendered element is not a native button/input. Include an
      // explicit stable data-cut-action on any element while retaining the
      // semantic-control inventory for legacy data-cut-* identities.
      if (!SEMANTIC_INTERACTIVE.has(match[1]) && !STATIC_ACTION_VALUE.test(tagSource)) continue
      tags.push({
        tag: match[1],
        index: match.index || 0,
        source: tagSource,
      })
    }
  }
  return tags
}

function primaryActionId(tag) {
  // data-cut-action is a namespace; its literal VALUE is the actual stable
  // identity (`data-cut-action="undo"` means `undo`, not the meaningless
  // shared id `action`). Other data-cut-* attributes carry identity in the
  // attribute name and may use their value to distinguish repeated instances.
  return tag.match(STATIC_ACTION_VALUE)?.[2] ||
    tag.match(PRIMARY_ACTION_ID)?.[1] ||
    ''
}

export function inventoryInteractiveSource(source, file = '<source>') {
  const searchable = withoutComments(source)
  const identified = []
  const unidentified = []
  for (const match of interactiveTags(searchable)) {
    const tag = match.source
    const action = primaryActionId(tag)
    const item = {
      file,
      line: lineAt(source, match.index),
      tag: match.tag,
      snippet: compactTag(tag),
    }
    if (action) identified.push({ ...item, action })
    else unidentified.push(item)
  }
  return { identified, unidentified }
}

export function testSourceReferencesAction(source, actionId) {
  return source.includes(`data-cut-${actionId}`) ||
    source.includes(`data-cut-action="${actionId}"`) ||
    source.includes(`data-cut-action='${actionId}'`)
}

export function buildUiActionCoverageAudit({
  repoRoot = REPO_ROOT,
  sourceFiles,
  testFiles,
  nativeSweepFiles,
} = {}) {
  const sources = sourceFiles || filesUnder(resolve(repoRoot, 'ui/src'), ['.tsx'])
  const tests = testFiles || filesUnder(resolve(repoRoot, 'ui/public-tests'), ['.mjs', '.ts'])
  const nativeTests = nativeSweepFiles || tests.filter((file) => {
    const rel = relative(repoRoot, file).replaceAll('\\', '/')
    return rel === 'ui/public-tests/full-coverage-verify.mjs' ||
      /^ui\/public-tests\/lib\/fullCoverage[^/]*\.mjs$/.test(rel)
  })
  const identified = []
  const unidentified = []
  for (const file of sources) {
    const result = inventoryInteractiveSource(
      readFileSync(file, 'utf8'),
      relative(repoRoot, file),
    )
    identified.push(...result.identified)
    unidentified.push(...result.unidentified)
  }
  const testSource = tests.map((file) => readFileSync(file, 'utf8')).join('\n')
  const nativeSweepSource = nativeTests.map((file) => readFileSync(file, 'utf8')).join('\n')
  const actionOwners = new Map()
  for (const item of identified) {
    const owners = actionOwners.get(item.action) || new Set()
    owners.add(`${item.file}:${item.line}`)
    actionOwners.set(item.action, owners)
  }
  const actions = [...actionOwners]
    .map(([id, owners]) => ({
      id,
      owners: [...owners].sort(),
      referencedByUiTests: testSourceReferencesAction(testSource, id),
      referencedByNativeSweep: testSourceReferencesAction(nativeSweepSource, id),
    }))
    .sort((a, b) => a.id.localeCompare(b.id))
  const unreferenced = actions.filter((action) => !action.referencedByUiTests)
  const unreferencedByNativeSweep = actions.filter((action) => !action.referencedByNativeSweep)
  return {
    schema: 'shellx-cut/ui-action-source-coverage@1',
    generatedAt: new Date().toISOString(),
    summary: {
      interactiveElements: identified.length + unidentified.length,
      identifiedInteractiveElements: identified.length,
      uniqueActionIds: actions.length,
      unidentifiedInteractiveElements: unidentified.length,
      unreferencedActionIds: unreferenced.length,
      unreferencedByNativeSweepActionIds: unreferencedByNativeSweep.length,
    },
    actions,
    unidentified,
    unreferenced,
    unreferencedByNativeSweep,
  }
}

export function assessUiActionCoverage(report, {
  strict = false,
  ratchet = ACTION_COVERAGE_RATCHET,
} = {}) {
  const unidentified = report.summary.unidentifiedInteractiveElements
  const unreferenced = report.summary.unreferencedActionIds
  const nativeUnreferenced = report.summary.unreferencedByNativeSweepActionIds ?? 0
  const missing = []
  if (strict) {
    if (unidentified > 0) missing.push(`${unidentified} interactive elements lack a stable data-cut action id`)
    if (unreferenced > 0) missing.push(`${unreferenced} stable action ids are absent from UI tests`)
    if (nativeUnreferenced > 0) missing.push(`${nativeUnreferenced} stable action ids are absent from the native full sweep`)
  } else {
    if (unidentified > ratchet.unidentifiedInteractiveElements) {
      missing.push(`unidentified interactive elements grew ${ratchet.unidentifiedInteractiveElements} -> ${unidentified}`)
    }
    if (unreferenced > ratchet.unreferencedActionIds) {
      missing.push(`unreferenced action ids grew ${ratchet.unreferencedActionIds} -> ${unreferenced}`)
    }
    if (nativeUnreferenced > (ratchet.unreferencedByNativeSweepActionIds ?? Number.POSITIVE_INFINITY)) {
      missing.push(`native-sweep action gaps grew ${ratchet.unreferencedByNativeSweepActionIds} -> ${nativeUnreferenced}`)
    }
  }
  return { ok: missing.length === 0, strict, missing }
}

function printHuman(report, verdict) {
  const s = report.summary
  console.log(
    `UI action source coverage: ${s.interactiveElements} interactive elements, ` +
    `${s.uniqueActionIds} stable ids, ${s.unidentifiedInteractiveElements} unidentified, ` +
    `${s.unreferencedActionIds} absent from UI tests, ` +
    `${s.unreferencedByNativeSweepActionIds} absent from the native full sweep`,
  )
  for (const item of report.unidentified) {
    console.log(`  NO-ID ${item.file}:${item.line} ${item.snippet}`)
  }
  for (const item of report.unreferenced) {
    console.log(`  NO-TEST data-cut-${item.id} (${item.owners.join(', ')})`)
  }
  for (const item of report.unreferencedByNativeSweep) {
    console.log(`  NO-NATIVE-SWEEP data-cut-${item.id} (${item.owners.join(', ')})`)
  }
  if (verdict.ok) {
    console.log(verdict.strict ? 'PASS strict UI action source coverage' : 'PASS UI action coverage ratchet')
  } else {
    for (const miss of verdict.missing) console.error(`FAIL ${miss}`)
  }
}

function main() {
  const strict = process.argv.includes('--strict')
  const report = buildUiActionCoverageAudit()
  const verdict = assessUiActionCoverage(report, { strict })
  if (process.argv.includes('--json')) console.log(JSON.stringify({ ...report, verdict }, null, 2))
  else printHuman(report, verdict)
  if (!verdict.ok) process.exitCode = 1
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  main()
}
