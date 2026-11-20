#!/usr/bin/env node

import { spawnSync } from 'node:child_process'
import { dirname, resolve } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

import { macosScreenCaptureKitBridgeContract } from './screencapturekit-bridge-contract.mjs'

const HERE = dirname(fileURLToPath(import.meta.url))
const REPO = resolve(HERE, '..', '..')

// Reviewed 2026-11-05 against the exact locked graph. These are warning-only
// maintenance/compatibility dispositions; vulnerability entries still fail.
const REVIEW_BY = '2026-12-05'
const RELEASE_OWNER = 'ShellX Cut release'

const exception = (advisoryId, packageName, version, reason, kind = 'unmaintained') => Object.freeze({
  advisoryId,
  package: packageName,
  version,
  kind,
  owner: RELEASE_OWNER,
  expires: REVIEW_BY,
  reason,
})

/**
 * The optional installed-UI test harness is currently zero-tolerance too. Keep
 * the exception evaluator for a future explicitly owned graph, but never leave
 * a stale policy after compatible patched transitive releases become available.
 */
export const NPM_TOOLING_EXCEPTION = null

export const RUST_WARNING_EXCEPTIONS = Object.freeze({
  workspace: Object.freeze([
    exception(
      'RUSTSEC-2026-0206',
      'rustybuzz',
      '0.20.1',
      'Unmaintained through locked usvg 0.47; resvg/usvg 0.48+ require coordinated harfrust/fontdb migration and title-rendering qualification.',
    ),
    exception(
      'RUSTSEC-2026-0192',
      'ttf-parser',
      '0.25.1',
      'Unmaintained through fontdb 0.23/usvg 0.47; replacing with fontdb 0.24 and resvg/usvg 0.48+ requires font/title-rendering qualification.',
    ),
    exception(
      'RUSTSEC-2026-0371',
      'zbus_names',
      '4.3.2',
      'Standalone crate merged into zbus 6; capture-linux uses ashpd 0.13/zbus 5. ashpd 0.14 migration needs portal/capture qualification.',
    ),
    exception(
      'RUSTSEC-2026-0370',
      'zvariant',
      '5.12.0',
      'Standalone crate merged into zbus 6; locked ashpd 0.13 requires zbus 5 wire types. Qualify portal/capture before migrating.',
    ),
    exception(
      'RUSTSEC-2026-0372',
      'zvariant_derive',
      '5.12.0',
      'Build-time derive for the same ashpd 0.13/zbus 5 wire types; upstream replacement is part of the coordinated zbus 6 migration.',
    ),
    exception(
      'RUSTSEC-2026-0543',
      'chrono',
      '0.4.45',
      'Unmaintained notice without a known vulnerability, and no successor release exists. The engine uses it for RFC 3339 timestamps; replacement with the time crate is planned and this entry is reviewed until then.',
    ),
  ]),
  desktop: Object.freeze([
    exception(
      'RUSTSEC-2024-0370',
      'proc-macro-error',
      '1.0.4',
      'The inherited GTK3 macro stack still uses this build-time helper.',
    ),
    exception(
      'RUSTSEC-2024-0429',
      'glib',
      '0.18.5',
      'Existing GLib compatibility deferral: Linux Tauri/GTK selects 0.18.5. Cut and the inspected normal dependency graph contain no VariantStrIter calls; GNOME Settings::strv uses g_settings_get_strv. Other feature/target paths remain unproven; upstream fixed >=0.20 needs GTK-stack migration.',
      'unsound',
    ),
    exception(
      'RUSTSEC-2026-0371',
      'zbus_names',
      '4.3.4',
      'Standalone crate merged into zbus 6; Tauri opener 2.5.5 selects zbus 5 and stable opener 2.7.2 still requires zbus 5. Revisit with upstream migration.',
    ),
    exception(
      'RUSTSEC-2026-0370',
      'zvariant',
      '5.15.0',
      'Standalone crate merged into zbus 6; the maintained Tauri opener line still requires zbus 5 wire types. No compatible replacement edge is available.',
    ),
    exception(
      'RUSTSEC-2026-0372',
      'zvariant_derive',
      '5.15.0',
      'Build-time derive for the Tauri opener/zbus 5 wire types; replacement follows upstream opener migration to zbus 6.',
    ),
    exception(
      'RUSTSEC-2026-0543',
      'chrono',
      '0.4.45',
      'Lockfile-only entry: optional dependency of serde_with (through tauri-utils) that no target enables; chrono is not compiled into this graph.',
    ),
  ]),
  // The optional installed-UI harness lock: only the desktop entries for the
  // same crate and version, with the same reason text.
  harness: Object.freeze([
    exception(
      'RUSTSEC-2024-0370',
      'proc-macro-error',
      '1.0.4',
      'The inherited GTK3 macro stack still uses this build-time helper.',
    ),
    exception(
      'RUSTSEC-2024-0429',
      'glib',
      '0.18.5',
      'Existing GLib compatibility deferral: Linux Tauri/GTK selects 0.18.5. Cut and the inspected normal dependency graph contain no VariantStrIter calls; GNOME Settings::strv uses g_settings_get_strv. Other feature/target paths remain unproven; upstream fixed >=0.20 needs GTK-stack migration.',
      'unsound',
    ),
    exception(
      'RUSTSEC-2026-0543',
      'chrono',
      '0.4.45',
      'Lockfile-only entry: optional dependency of serde_with (through tauri-utils) that no target enables; chrono is not compiled into this graph.',
    ),
  ]),
})

export function dependencyAuditPlan(repo = REPO, platform = process.platform) {
  const npm = platform === 'win32' ? 'npm.cmd' : 'npm'
  return [
    {
      kind: 'rust',
      scope: 'workspace',
      label: 'Rust workspace',
      command: 'cargo',
      args: ['audit', '--file', resolve(repo, 'app', 'Cargo.lock'), '--json'],
      cwd: repo,
    },
    {
      kind: 'rust',
      scope: 'desktop',
      label: 'Rust desktop',
      command: 'cargo',
      args: ['audit', '--file', resolve(repo, 'app', 'desktop', 'src-tauri', 'Cargo.lock'), '--json'],
      cwd: repo,
    },
    {
      kind: 'rust',
      scope: 'harness',
      label: 'Rust test harness',
      command: 'cargo',
      args: ['audit', '--file', resolve(repo, 'app', 'desktop', 'src-tauri', 'test-support', 'tauri-plugin-wdio-webdriver', 'Cargo.lock'), '--json'],
      cwd: repo,
    },
    {
      kind: 'npm',
      scope: 'runtime',
      label: 'UI production dependencies',
      command: npm,
      args: ['audit', '--omit=dev', '--json'],
      cwd: resolve(repo, 'ui'),
    },
    {
      kind: 'npm',
      scope: 'tooling',
      label: 'UI optional test tooling',
      command: npm,
      args: ['audit', '--json'],
      cwd: resolve(repo, 'ui'),
    },
  ]
}

function warningKey({ advisoryId, package: packageName, version }) {
  return `${advisoryId}:${packageName}@${version}`
}

function reportWarningKey(warning) {
  return warningKey({
    advisoryId: warning.advisory?.id,
    package: warning.package?.name,
    version: warning.package?.version,
  })
}

function reportWarnings(report) {
  return Object.entries(report.warnings || {}).flatMap(([kind, warnings]) => (
    Array.isArray(warnings) ? warnings.map((warning) => ({ ...warning, kind })) : []
  ))
}

function validException(policy) {
  return policy
    && typeof policy.advisoryId === 'string'
    && typeof policy.package === 'string'
    && typeof policy.version === 'string'
    && ['unmaintained', 'unsound'].includes(policy.kind)
    && typeof policy.owner === 'string'
    && policy.owner.trim() !== ''
    && /^\d{4}-\d{2}-\d{2}$/.test(policy.expires)
    && typeof policy.reason === 'string'
    && policy.reason.trim() !== ''
}

function validNpmException(policy) {
  return policy
    && typeof policy.advisoryId === 'string'
    && Number.isInteger(policy.source)
    && typeof policy.package === 'string'
    && typeof policy.severity === 'string'
    && Array.isArray(policy.affectedPackages)
    && policy.affectedPackages.length > 0
    && new Set(policy.affectedPackages).size === policy.affectedPackages.length
    && Array.isArray(policy.directPackages)
    && new Set(policy.directPackages).size === policy.directPackages.length
    && policy.directPackages.every((name) => policy.affectedPackages.includes(name))
    && typeof policy.owner === 'string'
    && policy.owner.trim() !== ''
    && /^\d{4}-\d{2}-\d{2}$/.test(policy.expires)
    && typeof policy.reason === 'string'
    && policy.reason.trim() !== ''
}

export function evaluateRustAudit({
  label,
  stdout,
  exceptions,
  now = new Date(),
}) {
  const failures = []
  let report
  try {
    report = JSON.parse(stdout)
  } catch (error) {
    return {
      accepted: [],
      failures: [`${label}: invalid cargo-audit JSON (${error.message})`],
    }
  }

  const vulnerabilities = report.vulnerabilities?.list || []
  if ((report.vulnerabilities?.count || 0) > vulnerabilities.length) {
    failures.push(`${label}: cargo-audit reported vulnerabilities without complete details`)
  }
  for (const vulnerability of vulnerabilities) {
    failures.push(
      `${label}: vulnerability ${vulnerability.advisory?.id || 'unknown'} `
      + `in ${vulnerability.package?.name || 'unknown'}@${vulnerability.package?.version || 'unknown'}`,
    )
  }

  const policyByKey = new Map()
  for (const policy of exceptions) {
    if (!validException(policy)) {
      failures.push(`${label}: malformed warning exception ${JSON.stringify(policy)}`)
      continue
    }
    const key = warningKey(policy)
    if (policyByKey.has(key)) failures.push(`${label}: duplicate warning exception ${key}`)
    policyByKey.set(key, policy)
  }

  const accepted = []
  const seen = new Set()
  const today = now.toISOString().slice(0, 10)
  for (const warning of reportWarnings(report)) {
    const key = reportWarningKey(warning)
    const policy = policyByKey.get(key)
    if (!policy) {
      failures.push(`${label}: unexpected cargo-audit warning ${key}`)
      continue
    }
    seen.add(key)
    if (warning.kind !== policy.kind) {
      failures.push(`${label}: warning kind changed ${key} (expected ${policy.kind}, observed ${warning.kind})`)
      continue
    }
    if (today > policy.expires) {
      failures.push(`${label}: warning exception expired ${key} (review was due ${policy.expires})`)
      continue
    }
    accepted.push(policy)
  }

  for (const key of policyByKey.keys()) {
    if (!seen.has(key)) failures.push(`${label}: stale warning exception ${key}`)
  }

  return { accepted, failures }
}

function sorted(values) {
  return [...values].sort((a, b) => a.localeCompare(b))
}

function sameStrings(actual, expected) {
  const a = sorted(actual)
  const b = sorted(expected)
  return a.length === b.length && a.every((value, index) => value === b[index])
}

/**
 * Evaluate npm's JSON without trusting its exit status: npm intentionally exits
 * non-zero when it reports vulnerabilities. Runtime is always zero-tolerance.
 * The optional tooling audit may accept exactly one owned advisory graph.
 */
export function evaluateNpmAudit({
  label,
  stdout,
  exception: policy = null,
  now = new Date(),
}) {
  const failures = []
  let report
  try {
    report = JSON.parse(stdout)
  } catch (error) {
    return {
      accepted: [],
      failures: [`${label}: invalid npm audit JSON (${error.message})`],
    }
  }

  if (report.error) {
    return {
      accepted: [],
      failures: [`${label}: npm audit error ${report.error.code || 'unknown'}: ${report.error.summary || report.error.message || 'unknown error'}`],
    }
  }

  const vulnerabilities = report.vulnerabilities && typeof report.vulnerabilities === 'object'
    ? report.vulnerabilities
    : {}
  const names = Object.keys(vulnerabilities)
  const reportedTotal = report.metadata?.vulnerabilities?.total
  if (Number.isInteger(reportedTotal) && reportedTotal !== names.length) {
    failures.push(`${label}: npm audit summary/detail count mismatch (${reportedTotal} vs ${names.length})`)
  }

  if (!policy) {
    for (const name of names) {
      const vulnerability = vulnerabilities[name]
      failures.push(`${label}: ${vulnerability?.severity || 'unknown'} vulnerability in ${name}`)
    }
    return { accepted: [], failures }
  }

  if (!validNpmException(policy)) {
    return {
      accepted: [],
      failures: [`${label}: malformed npm tooling exception ${JSON.stringify(policy)}`],
    }
  }
  if (names.length === 0) {
    return {
      accepted: [],
      failures: [`${label}: stale npm tooling exception ${policy.advisoryId}`],
    }
  }
  if (!sameStrings(names, policy.affectedPackages)) {
    failures.push(
      `${label}: affected package set drifted (expected ${policy.affectedPackages.length}, got ${names.length})`,
    )
  }

  const direct = names.filter((name) => vulnerabilities[name]?.isDirect)
  if (!sameStrings(direct, policy.directPackages)) {
    failures.push(`${label}: direct affected package set drifted`)
  }

  const rootAdvisories = names.flatMap((name) => {
    const via = vulnerabilities[name]?.via
    return Array.isArray(via)
      ? via.filter((entry) => entry && typeof entry === 'object')
      : []
  })
  if (rootAdvisories.length !== 1) {
    failures.push(`${label}: expected exactly one root advisory, got ${rootAdvisories.length}`)
  } else {
    const advisory = rootAdvisories[0]
    const advisoryId = typeof advisory.url === 'string'
      ? advisory.url.split('/').filter(Boolean).at(-1)
      : ''
    if (
      advisory.source !== policy.source
      || advisoryId !== policy.advisoryId
      || advisory.name !== policy.package
      || advisory.severity !== policy.severity
    ) {
      failures.push(`${label}: root advisory identity drifted`)
    }
  }

  const severityDrift = names.filter((name) => vulnerabilities[name]?.severity !== policy.severity)
  if (severityDrift.length > 0) {
    failures.push(`${label}: severity drifted for ${severityDrift.join(', ')}`)
  }

  const today = now.toISOString().slice(0, 10)
  if (today > policy.expires) {
    failures.push(`${label}: npm tooling exception expired (review was due ${policy.expires})`)
  }

  return {
    accepted: failures.length === 0 ? [policy] : [],
    failures,
  }
}

function processFailureDetail(result) {
  if (result.error) return result.error.message
  const stderr = String(result.stderr || '').trim().split('\n').filter(Boolean).at(-1)
  return stderr ? `exit ${result.status ?? 'unknown'} (${stderr})` : `exit ${result.status ?? 'unknown'}`
}

export function runDependencyAudit({
  repo = REPO,
  runner = spawnSync,
  platform = process.platform,
  logger = console,
  now = new Date(),
  rustWarningExceptions = RUST_WARNING_EXCEPTIONS,
  npmToolingException = NPM_TOOLING_EXCEPTION,
} = {}) {
  const failures = []

  const bridgeContract = macosScreenCaptureKitBridgeContract({ repo })
  failures.push(...bridgeContract.failures.map((detail) => ({
    label: 'macOS ScreenCaptureKit bridge',
    detail,
  })))

  for (const check of dependencyAuditPlan(repo, platform)) {
    let acceptedNpmNonzero = false
    logger.log(`[dependency-audit] ${check.label}`)
    const result = runner(check.command, check.args, {
      cwd: check.cwd,
      encoding: 'utf8',
      stdio: ['ignore', 'pipe', 'pipe'],
    })

    if (check.kind === 'rust' && !result.error && result.stdout) {
      const evaluated = evaluateRustAudit({
        label: check.label,
        stdout: result.stdout,
        exceptions: rustWarningExceptions[check.scope] || [],
        now,
      })
      failures.push(...evaluated.failures.map((detail) => ({
        label: check.label,
        detail: detail.startsWith(`${check.label}: `)
          ? detail.slice(check.label.length + 2)
          : detail,
      })))
      if (evaluated.accepted.length > 0) {
        const nextReview = evaluated.accepted
          .map(({ expires }) => expires)
          .sort()[0]
        logger.log(
          `[dependency-audit] ${check.label}: ${evaluated.accepted.length} `
          + `owned warning exception(s), next review ${nextReview}`,
        )
      }
    } else if (check.kind === 'rust' && !result.error) {
      failures.push({ label: check.label, detail: 'cargo audit returned no JSON' })
    }

    if (check.kind === 'npm' && !result.error && result.stdout) {
      const evaluated = evaluateNpmAudit({
        label: check.label,
        stdout: result.stdout,
        exception: check.scope === 'tooling' ? npmToolingException : null,
        now,
      })
      failures.push(...evaluated.failures.map((detail) => ({
        label: check.label,
        detail: detail.startsWith(`${check.label}: `)
          ? detail.slice(check.label.length + 2)
          : detail,
      })))
      if (evaluated.accepted.length > 0) {
        acceptedNpmNonzero = true
        logger.log(
          `[dependency-audit] ${check.label}: exact ${evaluated.accepted[0].advisoryId} `
          + `exception accepted, review by ${evaluated.accepted[0].expires}`,
        )
      }
    } else if (check.kind === 'npm' && !result.error) {
      failures.push({ label: check.label, detail: 'npm audit returned no JSON' })
    }

    // cargo-audit exits zero when its JSON contains allowed warnings; npm audit
    // exits non-zero for the exact accepted vulnerability graph. Parsed audit
    // content is authoritative for both; only launch errors need a second error.
    if (result.error || (result.status !== 0 && !acceptedNpmNonzero)) {
      failures.push({ label: check.label, detail: processFailureDetail(result) })
    }
  }

  if (failures.length > 0) {
    logger.error('[dependency-audit] FAIL')
    for (const failure of failures) logger.error(`- ${failure.label}: ${failure.detail}`)
    return 1
  }

  logger.log('[dependency-audit] PASS')
  return 0
}

const isMain = process.argv[1]
  && pathToFileURL(resolve(process.argv[1])).href === import.meta.url
if (isMain) process.exitCode = runDependencyAudit()
