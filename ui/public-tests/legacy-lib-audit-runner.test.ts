// Keeps the retained legacy aggregate auditable without treating its known
// historical string assertions as a green product gate.
import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { dirname, isAbsolute, resolve } from 'node:path'

type LedgerEntry = {
  id: string
  sourceLine: number
  label: string
  classification: string
  owner: string
  modernTest: string | null
  replacementEvidence: ReplacementEvidence | null
  replacementCommand?: string
  privateControl?: string
  sourceAnchor?: string
  sourceContext?: string
  legacyExpectation?: 'fails' | 'passes' | 'removed'
}

type ReplacementEvidence = {
  testPath: string
  line: number
  assertion: string
  caseAnchor?: string
}

type Ledger = {
  schema: string
  aggregate: {
    path: string
    baseline: { commit: string; sha256: string; command: string; pass: number; fail: number; exitCode: number }
    complete: { reviewedCommit: string; sha256: string; pass: number; fail: number; exitCode: number }
  }
  assertions: LedgerEntry[]
  resolvedAssertions?: LedgerEntry[]
  terminal: { classification: string; owner: string; modernTest: string }
  control: {
    id: string
    packageScript: string
    integrityPackageScript: string
    command: string
    document: string
    documentAnchor: string
  }
  privateControls: Array<{
    id: string
    project: string
    commit: string
    profilePath: string
    procedure: string
    testCommand: string[]
    status: string
    rows: Array<{ id: string; control: string }>
  }>
}

const here = dirname(fileURLToPath(import.meta.url))
const uiRoot = resolve(here, '..')
const repoRoot = resolve(uiRoot, '..')
const configuredLedgerPath = process.env.SHELLX_CUT_LEGACY_AUDIT_LEDGER
const ledgerPath = configuredLedgerPath ? resolve(configuredLedgerPath) : resolve(here, 'legacy-lib-audit-ledger.json')
const ledger = JSON.parse(readFileSync(ledgerPath, 'utf8')) as Ledger
const legacyPath = resolve(repoRoot, ledger.aggregate.path)
const tsxCli = resolve(uiRoot, 'node_modules/tsx/dist/cli.mjs')
const allowedClassifications = new Set([
  'real current defect',
  'stale/renamed contract',
  'duplicate of owned current test',
  'current replacement coverage',
  'fixture/environment limitation',
  'harness bug',
  'missing current replacement coverage',
  'private procedure/control registered-not-green',
])

const hash = (value: string) => createHash('sha256').update(value).digest('hex')
const repoPath = (relativePath: string, description: string) => {
  assert.ok(relativePath && !isAbsolute(relativePath), `${description} must be a repository-relative path`)
  const path = resolve(repoRoot, relativePath)
  assert.ok(path.startsWith(`${repoRoot}/`), `${description} escapes the repository: ${relativePath}`)
  return path
}
const sourceAtCommit = (commit: string, relativePath: string, description: string) => {
  assert.match(commit, /^[0-9a-f]{40}$/, `${description} must use an exact commit`)
  repoPath(relativePath, description)
  const result = spawnSync('git', ['show', `${commit}:${relativePath}`], {
    cwd: repoRoot,
    encoding: 'utf8',
  })
  assert.equal(result.error, undefined, `${description} could not read the reviewed source: ${result.error?.message ?? 'unknown error'}`)
  assert.equal(result.status, 0, `${description} reviewed source is unavailable: ${result.stderr || ''}`)
  return result.stdout || ''
}

assert.equal(ledger.schema, 'shellx-cut/legacy-lib-audit-ledger/2')
const reviewedBaseline = sourceAtCommit(ledger.aggregate.baseline.commit, ledger.aggregate.path, 'legacy baseline')
assert.equal(hash(reviewedBaseline), ledger.aggregate.baseline.sha256, 'legacy baseline hash drifted')
const reviewedComplete = sourceAtCommit(ledger.aggregate.complete.reviewedCommit, ledger.aggregate.path, 'legacy complete candidate')
assert.equal(hash(reviewedComplete), ledger.aggregate.complete.sha256, 'reviewed complete aggregate hash drifted')
assert.equal(hash(readFileSync(legacyPath, 'utf8')), ledger.aggregate.complete.sha256, 'legacy aggregate source differs from its reviewed complete candidate')
const expectedFailures = ledger.assertions.filter((entry) => !entry.legacyExpectation || entry.legacyExpectation === 'fails')
const resolvedLegacyAssertions = ledger.assertions.filter((entry) => entry.legacyExpectation === 'passes' || entry.legacyExpectation === 'removed')
assert.equal(expectedFailures.length, ledger.aggregate.complete.fail, 'ledger must classify every expected legacy failure')
assert.equal(ledger.terminal.classification, 'harness bug', 'the terminal crash must have a harness disposition')
assert.ok(ledger.terminal.owner && ledger.terminal.modernTest, 'the terminal crash must have an owner and regression test')

const packageJson = JSON.parse(readFileSync(resolve(uiRoot, 'package.json'), 'utf8')) as { scripts?: Record<string, string> }
assert.equal(packageJson.scripts?.[ledger.control.packageScript], 'tsx public-tests/legacy-lib-audit-runner.test.ts', 'legacy audit command must invoke only the audit runner')
assert.equal(packageJson.scripts?.[ledger.control.integrityPackageScript], 'tsx public-tests/legacy-lib-audit-integrity.test.ts', 'legacy integrity command must invoke only its red-proof suite')
assert.equal(ledger.control.command, 'npm --prefix ui run test:legacy-audit')
assert.match(readFileSync(resolve(repoRoot, ledger.control.document), 'utf8'), new RegExp(ledger.control.documentAnchor), 'legacy audit command must remain discoverable in the public test guide')
assert.match(packageJson.scripts?.['test:lib'] || '', /capture-error-contract\.test\.ts/, 'the migrated current capture contract must stay in test:lib')
assert.doesNotMatch(packageJson.scripts?.['test:lib'] || '', /legacy-lib-audit-runner|lib\.test\.ts/, 'routine test:lib must not replay the legacy aggregate')

const ids = new Set<string>()
const labels = new Set<string>()
const resolvedIds = new Set((ledger.resolvedAssertions || []).map((entry) => entry.id))
const readEvidence = (relativePath: string) => {
  return readFileSync(repoPath(relativePath, 'evidence path'), 'utf8')
}
const baselineLines = reviewedBaseline.split('\n')
const privateControls = new Map(ledger.privateControls.map((control) => [control.id, control]))
assert.equal(privateControls.size, ledger.privateControls.length, 'duplicate legacy private-control id')
const registeredControlEntries = ledger.assertions.filter((entry) => entry.classification === 'private procedure/control registered-not-green')
assert.ok(registeredControlEntries.length > 0, 'registered private-control rows must stay explicit while legacy replacement coverage is pending')
for (const control of privateControls.values()) {
  assert.equal(control.project, 'release-studio', `${control.id} must identify its authority project`)
  assert.match(control.commit, /^[0-9a-f]{40}$/, `${control.id} must pin the inspected authority revision`)
  assert.equal(control.profilePath, 'projects/shellx-cut/profile.json', `${control.id} must identify the Cut profile`)
  assert.equal(control.procedure, 'projects/shellx-cut/private/procedures/SHELLX_CUT_FEATURE_CHANGE_CONTROL.md', `${control.id} must identify the private procedure`)
  assert.deepEqual(control.testCommand, ['node', '--test', 'tools/cut-private-feature-control.test.mjs'], `${control.id} must name the executable private-control test`)
  assert.equal(control.status, 'registered-not-green', `${control.id} must not green the legacy audit before replacement evidence exists`)
  assert.ok(control.rows.length > 0, `private control ${control.id} has no owned ledger rows`)
  assert.equal(new Set(control.rows.map((row) => row.id)).size, control.rows.length, `private control ${control.id} repeats a ledger row`)
  for (const row of control.rows) assert.ok(row.control, `${control.id}.${row.id} lacks a concrete control mapping`)
}
assert.deepEqual(
  [...registeredControlEntries.map((entry) => entry.id)].sort(),
  [...privateControls.values()].flatMap((control) => control.rows.map((row) => row.id)).sort(),
  'every registered private-control legacy row must map one-to-one to its authority control',
)

const assertReplacementCommand = (entry: LedgerEntry, evidence: ReplacementEvidence) => {
  assert.ok(entry.replacementCommand, `${entry.id} needs an executable replacement command`)
  const uiRelativeTestPath = evidence.testPath.startsWith('ui/') ? evidence.testPath.slice('ui/'.length) : null
  const npmCommand = entry.replacementCommand.match(/^npm --prefix ui run ([a-z0-9:_-]+)$/)
  if (npmCommand) {
    const script = packageJson.scripts?.[npmCommand[1]]
    assert.ok(script, `${entry.id} replacement package command is not registered`)
    assert.ok(uiRelativeTestPath && script.includes(uiRelativeTestPath), `${entry.id} replacement package command does not execute ${evidence.testPath}`)
    return
  }
  assert.equal(entry.replacementCommand, `node --test ${evidence.testPath}`, `${entry.id} replacement command must directly execute its named test`)
}

for (const entry of [...ledger.assertions, ...(ledger.resolvedAssertions || [])]) {
  assert.ok(!ids.has(entry.id), `duplicate ledger id ${entry.id}`)
  assert.ok(!labels.has(entry.label), `duplicate ledger label ${entry.label}`)
  assert.ok(allowedClassifications.has(entry.classification), `unknown classification for ${entry.id}`)
  assert.ok(entry.sourceLine > 0 && entry.owner, `incomplete ledger entry ${entry.id}`)
  assert.ok(!entry.legacyExpectation || entry.legacyExpectation === 'passes' || entry.legacyExpectation === 'removed', `${entry.id} has an invalid legacy expectation`)
  const sourceAnchor = entry.sourceAnchor || entry.label
  assert.ok(baselineLines[entry.sourceLine - 1]?.includes(sourceAnchor), `${entry.id} historical source anchor drifted at line ${entry.sourceLine}`)
  if (entry.sourceContext) {
    const contextStart = Math.max(0, entry.sourceLine - 20)
    assert.ok(baselineLines.slice(contextStart, entry.sourceLine).some((line) => line.includes(entry.sourceContext!)), `${entry.id} historical source context drifted`)
  }
  if (entry.classification === 'stale/renamed contract' || entry.classification === 'current replacement coverage') {
    assert.ok(entry.replacementEvidence, `${entry.id} needs current replacement evidence`)
    const evidence = entry.replacementEvidence
    const evidenceLines = readEvidence(evidence.testPath).split('\n')
    assert.equal(entry.modernTest, evidence.testPath, `${entry.id} must name the replacement test, not its owner`)
    assert.ok(evidence.line > 0 && evidenceLines[evidence.line - 1]?.includes(evidence.assertion), `${entry.id} replacement assertion anchor drifted`)
    if (evidence.caseAnchor) assert.ok(evidenceLines.some((line) => line.includes(evidence.caseAnchor!)), `${entry.id} replacement case anchor drifted`)
    assertReplacementCommand(entry, evidence)
    if (entry.classification === 'current replacement coverage') {
      assert.ok(resolvedIds.has(entry.id), `${entry.id} must leave the active legacy-failure ledger once its replacement passes`)
    }
  } else if (entry.classification === 'missing current replacement coverage' || entry.classification === 'private procedure/control registered-not-green') {
    assert.equal(entry.modernTest, null, `${entry.id} cannot name a nonexistent replacement test`)
    assert.equal(entry.replacementEvidence, null, `${entry.id} cannot call missing coverage a replacement`)
    assert.equal(entry.replacementCommand, undefined, `${entry.id} cannot call a registered-not-green control an executable replacement`)
    assert.ok(entry.privateControl && privateControls.has(entry.privateControl), `${entry.id} needs one registered private-control authority`)
    assert.ok(privateControls.get(entry.privateControl!)?.rows.some((row) => row.id === entry.id), `${entry.id} is not registered by its declared private control`)
  }
  ids.add(entry.id)
  labels.add(entry.label)
}

const result = spawnSync(process.execPath, [tsxCli, legacyPath], {
  cwd: uiRoot,
  encoding: 'utf8',
  timeout: 30_000,
})
assert.equal(result.error, undefined, `legacy aggregate could not run: ${result.error?.message ?? 'unknown error'}`)
assert.equal(result.signal, null, `legacy aggregate terminated by ${result.signal}`)
assert.equal(result.status, ledger.aggregate.complete.exitCode, 'legacy aggregate must reach its classified assertion outcome')

const stdout = result.stdout || ''
const stderr = result.stderr || ''
const output = `${stdout}\n${stderr}`
assert.doesNotMatch(output, /(?:ReferenceError|TypeError|SyntaxError):/, 'legacy aggregate terminated with a harness exception')
assert.match(stderr, new RegExp(`\\b${ledger.aggregate.complete.fail} unit check\\(s\\) FAILED\\s*$`), 'legacy aggregate did not emit its terminal summary')

const observedFailures = [...stdout.matchAll(/^FAIL  (.+)$/gm)].map((match) => match[1])
const observedPasses = [...stdout.matchAll(/^PASS  /gm)].length
assert.equal(observedPasses, ledger.aggregate.complete.pass, 'legacy aggregate terminated before its complete pass count')
assert.equal(observedFailures.length, ledger.aggregate.complete.fail, 'legacy aggregate has an unexpected failure count')
assert.deepEqual(
  [...new Set(observedFailures)].sort(),
  [...new Set(expectedFailures.map((entry) => entry.label))].sort(),
  'legacy aggregate has unclassified or missing failures',
)
for (const entry of resolvedLegacyAssertions) {
  assert.ok(!observedFailures.includes(entry.label), `${entry.id} is expected to pass in the retained aggregate`)
}

console.log(
  `AUDIT legacy lib aggregate integrity verified: ${observedPasses} PASS, ${observedFailures.length} classified historical failures; ` +
  `REGISTERED-NOT-GREEN private control: ${registeredControlEntries.map((entry) => entry.id).join(', ')}`,
)
