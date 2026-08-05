import assert from 'node:assert/strict'
import { test } from 'node:test'

import {
  dependencyAuditPlan,
  evaluateNpmAudit,
  evaluateRustAudit,
  NPM_TOOLING_EXCEPTION,
  RUST_WARNING_EXCEPTIONS,
  runDependencyAudit,
} from '../release/dependency-audit.mjs'

const cleanReport = JSON.stringify({
  vulnerabilities: { found: false, count: 0, list: [] },
  warnings: {},
})

function warningReport(advisoryId, packageName, version) {
  return JSON.stringify({
    vulnerabilities: { found: false, count: 0, list: [] },
    warnings: {
      unmaintained: [{
        advisory: { id: advisoryId },
        package: { name: packageName, version },
      }],
    },
  })
}

const cleanNpmReport = JSON.stringify({
  vulnerabilities: {},
  metadata: { vulnerabilities: { total: 0 } },
})

const ownedNpmException = Object.freeze({
  advisoryId: 'GHSA-test-test-test',
  source: 999999,
  package: 'test-glob',
  severity: 'high',
  affectedPackages: ['test-glob', 'test-runner'],
  directPackages: ['test-runner'],
  owner: 'Release owner',
  expires: '2026-09-30',
  reason: 'Synthetic policy used to prove the fail-closed exception evaluator.',
})

function npmToolingReport({
  policy = ownedNpmException,
  extra = [],
  severity = policy.severity,
  direct = policy.directPackages,
  source = policy.source,
  advisoryId = policy.advisoryId,
} = {}) {
  const names = [...policy.affectedPackages, ...extra]
  const vulnerabilities = Object.fromEntries(names.map((name) => [
    name,
    {
      severity,
      isDirect: direct.includes(name),
      via: name === policy.package
        ? [{
            source,
            name: policy.package,
            dependency: policy.package,
            title: 'owned test advisory',
            url: `https://github.com/advisories/${advisoryId}`,
            severity,
          }]
        : [policy.package],
      effects: [],
      range: '*',
      fixAvailable: false,
    },
  ]))
  return JSON.stringify({
    vulnerabilities,
    metadata: { vulnerabilities: { total: names.length } },
  })
}

const ownedException = {
  advisoryId: 'RUSTSEC-2099-0001',
  package: 'legacy-crate',
  version: '1.2.3',
  owner: 'Release owner',
  expires: '2099-09-30',
  reason: 'Upstream replacement is scheduled.',
}

test('dependency audit covers both Rust locks plus separate runtime and tooling npm graphs', () => {
  const plan = dependencyAuditPlan('/repo', 'linux')
  assert.deepEqual(plan.map(({ label, command, args, cwd }) => ({ label, command, args, cwd })), [
    {
      label: 'Rust workspace',
      command: 'cargo',
      args: ['audit', '--file', '/repo/app/Cargo.lock', '--json'],
      cwd: '/repo',
    },
    {
      label: 'Rust desktop',
      command: 'cargo',
      args: ['audit', '--file', '/repo/app/desktop/src-tauri/Cargo.lock', '--json'],
      cwd: '/repo',
    },
    {
      label: 'UI production dependencies',
      command: 'npm',
      args: ['audit', '--omit=dev', '--json'],
      cwd: '/repo/ui',
    },
    {
      label: 'UI optional test tooling',
      command: 'npm',
      args: ['audit', '--json'],
      cwd: '/repo/ui',
    },
  ])
})

test('dependency audit uses npm.cmd on Windows', () => {
  const plan = dependencyAuditPlan('/repo', 'win32')
  assert.equal(plan.at(-1).command, 'npm.cmd')
})

test('checked-in warning policy is exact, owned, and expiry-bound', () => {
  const policies = Object.values(RUST_WARNING_EXCEPTIONS).flat()
  assert.equal(policies.length, 19)
  assert.equal(new Set(policies.map((policy) => (
    `${policy.advisoryId}:${policy.package}@${policy.version}`
  ))).size, policies.length)
  for (const policy of policies) {
    assert.match(policy.advisoryId, /^RUSTSEC-\d{4}-\d{4}$/)
    assert.ok(policy.owner)
    assert.match(policy.expires, /^\d{4}-\d{2}-\d{2}$/)
    assert.ok(policy.reason)
  }
  assert.equal(NPM_TOOLING_EXCEPTION, null)
})

test('dependency audit runs every check and fails when any process fails', () => {
  const calls = []
  const runner = (command, args, options) => {
    calls.push({ command, args, options })
    return {
      status: calls.length === 2 ? 1 : 0,
      stdout: command === 'cargo' ? cleanReport : cleanNpmReport,
      stderr: calls.length === 2 ? 'registry timeout' : '',
    }
  }

  const logger = { log() {}, error() {} }
  assert.equal(runDependencyAudit({
    repo: '/repo',
    runner,
    platform: 'linux',
    logger,
    rustWarningExceptions: { workspace: [], desktop: [] },
    npmToolingException: null,
  }), 1)
  assert.equal(calls.length, 4)
  assert.deepEqual(calls.map(({ command }) => command), ['cargo', 'cargo', 'npm', 'npm'])
  assert.deepEqual(calls.map(({ options }) => options.stdio), [
    ['ignore', 'pipe', 'pipe'],
    ['ignore', 'pipe', 'pipe'],
    ['ignore', 'pipe', 'pipe'],
    ['ignore', 'pipe', 'pipe'],
  ])
})

test('npm production audit is zero-tolerance', () => {
  const clean = evaluateNpmAudit({
    label: 'UI production dependencies',
    stdout: cleanNpmReport,
  })
  assert.deepEqual(clean, { accepted: [], failures: [] })

  const vulnerable = evaluateNpmAudit({
    label: 'UI production dependencies',
    stdout: JSON.stringify({
      vulnerabilities: {
        runtime: { severity: 'moderate', isDirect: true, via: [], effects: [] },
      },
      metadata: { vulnerabilities: { total: 1 } },
    }),
  })
  assert.match(vulnerable.failures.join('\n'), /moderate vulnerability in runtime/)
})

test('exact owned npm tooling advisory graph is accepted before review', () => {
  const result = evaluateNpmAudit({
    label: 'UI optional test tooling',
    stdout: npmToolingReport(),
    exception: ownedNpmException,
    now: new Date('2026-09-30T23:59:59Z'),
  })
  assert.deepEqual(result.failures, [])
  assert.deepEqual(result.accepted, [ownedNpmException])
})

test('npm tooling policy fails on advisory, package, directness, severity, or expiry drift', () => {
  const cases = [
    npmToolingReport({ advisoryId: 'GHSA-drift-0000-0000' }),
    npmToolingReport({ extra: ['new-package'] }),
    npmToolingReport({ direct: [] }),
    npmToolingReport({ severity: 'critical' }),
  ]
  for (const stdout of cases) {
    const result = evaluateNpmAudit({
      label: 'UI optional test tooling',
      stdout,
      exception: ownedNpmException,
      now: new Date('2026-01-01T00:00:00Z'),
    })
    assert.notEqual(result.failures.length, 0)
  }

  const expired = evaluateNpmAudit({
    label: 'UI optional test tooling',
    stdout: npmToolingReport(),
    exception: ownedNpmException,
    now: new Date('2026-10-01T00:00:00Z'),
  })
  assert.match(expired.failures.join('\n'), /exception expired/)
})

test('npm tooling policy rejects malformed output, audit errors, count drift, and stale exceptions', () => {
  assert.match(evaluateNpmAudit({
    label: 'UI tooling',
    stdout: 'not json',
    exception: ownedNpmException,
  }).failures.join('\n'), /invalid npm audit JSON/)

  assert.match(evaluateNpmAudit({
    label: 'UI tooling',
    stdout: JSON.stringify({ error: { code: 'ENOLOCK', summary: 'missing lockfile' } }),
    exception: ownedNpmException,
  }).failures.join('\n'), /npm audit error ENOLOCK/)

  assert.match(evaluateNpmAudit({
    label: 'UI tooling',
    stdout: JSON.stringify({
      vulnerabilities: { unexpected: { severity: 'high', isDirect: false, via: [] } },
      metadata: { vulnerabilities: { total: 2 } },
    }),
    exception: ownedNpmException,
  }).failures.join('\n'), /summary\/detail count mismatch/)

  assert.match(evaluateNpmAudit({
    label: 'UI tooling',
    stdout: cleanNpmReport,
    exception: ownedNpmException,
  }).failures.join('\n'), /stale npm tooling exception/)
})

test('exact owned warning exception is accepted before its review date', () => {
  const result = evaluateRustAudit({
    label: 'Rust test',
    stdout: warningReport('RUSTSEC-2099-0001', 'legacy-crate', '1.2.3'),
    exceptions: [ownedException],
    now: new Date('2099-09-30T23:59:59Z'),
  })
  assert.deepEqual(result.failures, [])
  assert.deepEqual(result.accepted, [ownedException])
})

test('warning exception rejects advisory or version drift', () => {
  const result = evaluateRustAudit({
    label: 'Rust test',
    stdout: warningReport('RUSTSEC-2099-0001', 'legacy-crate', '1.2.4'),
    exceptions: [ownedException],
    now: new Date('2099-01-01T00:00:00Z'),
  })
  assert.match(result.failures.join('\n'), /unexpected cargo-audit warning/)
  assert.match(result.failures.join('\n'), /stale warning exception/)
})

test('expired and stale warning exceptions fail closed', () => {
  const expired = evaluateRustAudit({
    label: 'Rust test',
    stdout: warningReport('RUSTSEC-2099-0001', 'legacy-crate', '1.2.3'),
    exceptions: [ownedException],
    now: new Date('2099-10-01T00:00:00Z'),
  })
  assert.match(expired.failures.join('\n'), /warning exception expired/)

  const stale = evaluateRustAudit({
    label: 'Rust test',
    stdout: cleanReport,
    exceptions: [ownedException],
    now: new Date('2099-01-01T00:00:00Z'),
  })
  assert.match(stale.failures.join('\n'), /stale warning exception/)
})

test('vulnerabilities and malformed cargo-audit output fail closed', () => {
  const vulnerable = evaluateRustAudit({
    label: 'Rust test',
    stdout: JSON.stringify({
      vulnerabilities: {
        list: [{
          advisory: { id: 'RUSTSEC-2099-0002' },
          package: { name: 'unsafe-crate', version: '4.5.6' },
        }],
      },
      warnings: {},
    }),
    exceptions: [],
  })
  assert.match(vulnerable.failures.join('\n'), /vulnerability RUSTSEC-2099-0002/)

  const incomplete = evaluateRustAudit({
    label: 'Rust test',
    stdout: JSON.stringify({
      vulnerabilities: { count: 1, list: [] },
      warnings: {},
    }),
    exceptions: [],
  })
  assert.match(incomplete.failures.join('\n'), /without complete details/)

  const malformed = evaluateRustAudit({
    label: 'Rust test',
    stdout: 'not json',
    exceptions: [],
  })
  assert.match(malformed.failures.join('\n'), /invalid cargo-audit JSON/)
})
