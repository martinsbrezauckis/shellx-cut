// Red-proofs that the retained legacy-audit ledger cannot quietly accept drift.
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const uiRoot = resolve(here, '..')
const repoRoot = resolve(uiRoot, '..')
const runner = resolve(here, 'legacy-lib-audit-runner.test.ts')
const tsxCli = resolve(uiRoot, 'node_modules/tsx/dist/cli.mjs')
const ledger = JSON.parse(readFileSync(resolve(here, 'legacy-lib-audit-ledger.json'), 'utf8'))
const scratch = mkdtempSync(join(tmpdir(), 'shellx-cut-legacy-audit-'))

function entry(id: string, value: Record<string, unknown> = ledger) {
  const found = (value.assertions as Array<{ id: string }>).find((candidate) => candidate.id === id)
  assert.ok(found, `fixture is missing ${id}`)
  return found as Record<string, any>
}

function run(name: string, mutate: (value: Record<string, any>) => void = () => {}) {
  const fixture = structuredClone(ledger)
  mutate(fixture)
  const ledgerPath = join(scratch, `${name}.json`)
  writeFileSync(ledgerPath, `${JSON.stringify(fixture)}\n`)
  const result = spawnSync(process.execPath, [tsxCli, runner], {
    cwd: repoRoot,
    encoding: 'utf8',
    timeout: 30_000,
    env: { ...process.env, SHELLX_CUT_LEGACY_AUDIT_LEDGER: ledgerPath },
  })
  assert.equal(result.error, undefined, `${name}: audit runner could not start: ${result.error?.message ?? 'unknown error'}`)
  return { status: result.status, output: `${result.stdout || ''}\n${result.stderr || ''}` }
}

try {
  const accepted = run('accepted')
  assert.equal(accepted.status, 0, accepted.output)
  assert.match(accepted.output, /REGISTERED-NOT-GREEN private control: LLA-010, LLA-011, LLA-017, LLA-018, LLA-020, LLA-021, LLA-022, LLA-027/)

  const staleReplacementLine = run('stale-replacement-line', (fixture) => {
    entry('LLA-028', fixture).replacementEvidence.line -= 1
  })
  assert.notEqual(staleReplacementLine.status, 0)
  assert.match(staleReplacementLine.output, /LLA-028 replacement assertion anchor drifted/)

  const staleSourceLine = run('stale-source-line', (fixture) => {
    entry('LLA-038', fixture).sourceLine += 1
  })
  assert.notEqual(staleSourceLine.status, 0)
  assert.match(staleSourceLine.output, /LLA-038 historical source anchor drifted/)

  const stalePassCount = run('stale-pass-count', (fixture) => {
    fixture.aggregate.complete.pass -= 1
  })
  assert.notEqual(stalePassCount.status, 0)
  assert.match(stalePassCount.output, /legacy aggregate terminated before its complete pass count/)

  const missingEvidence = run('missing-evidence', (fixture) => {
    entry('LLA-001', fixture).replacementEvidence = null
  })
  assert.notEqual(missingEvidence.status, 0)
  assert.match(missingEvidence.output, /LLA-001 needs current replacement evidence/)

  const missingCommand = run('missing-command', (fixture) => {
    delete entry('LLA-001', fixture).replacementCommand
  })
  assert.notEqual(missingCommand.status, 0)
  assert.match(missingCommand.output, /LLA-001 needs an executable replacement command/)

  const orphanedPrivateControl = run('orphaned-private-control', (fixture) => {
    delete entry('LLA-010', fixture).privateControl
  })
  assert.notEqual(orphanedPrivateControl.status, 0)
  assert.match(orphanedPrivateControl.output, /LLA-010 needs one registered private-control authority/)
} finally {
  rmSync(scratch, { recursive: true, force: true })
}

console.log('PASS legacy lib audit integrity red-proofs')
