#!/usr/bin/env node
import { appendFileSync } from 'node:fs'

const SCHEMA = 'shellx-cut/github-collected-checks@1'
const allowedOutcomes = new Set(['success', 'failure', 'cancelled', 'skipped'])

function fail(message) {
  throw new Error(`GITHUB-COLLECT-ALL-01: ${message}`)
}

let parsed
try {
  parsed = JSON.parse(process.env.SHELLX_CI_COLLECTED_RESULTS || '')
} catch (error) {
  fail(`SHELLX_CI_COLLECTED_RESULTS must be JSON: ${error.message}`)
}

if (parsed?.schema !== SCHEMA || !Array.isArray(parsed.checks) || parsed.checks.length === 0) {
  fail(`input must use ${SCHEMA} with a non-empty checks array`)
}

const seen = new Set()
const checks = parsed.checks.map((check) => {
  if (!check || typeof check !== 'object' || Array.isArray(check)) fail('each check must be an object')
  if (typeof check.name !== 'string' || !check.name.trim() || seen.has(check.name)) fail(`invalid or duplicate check name ${JSON.stringify(check.name)}`)
  if (!allowedOutcomes.has(check.outcome)) fail(`${check.name} has unsupported outcome ${JSON.stringify(check.outcome)}`)
  if (check.allowSkipped !== undefined && typeof check.allowSkipped !== 'boolean') fail(`${check.name} allowSkipped must be boolean`)
  seen.add(check.name)
  return { name: check.name, outcome: check.outcome, allowSkipped: check.allowSkipped === true }
})

const failed = checks.filter((check) => check.outcome !== 'success' && !(check.outcome === 'skipped' && check.allowSkipped))
const rows = checks.map((check) => `| ${check.outcome === 'success' ? 'PASS' : check.outcome.toUpperCase()} | ${check.name.replaceAll('|', '\\|')} |`)
const summary = [
  '## Collected CI checks',
  '',
  '| Result | Check |',
  '|---|---|',
  ...rows,
  '',
  failed.length === 0 ? `All ${checks.length} collected checks passed.` : `${failed.length} of ${checks.length} collected checks failed.`,
  '',
].join('\n')

process.stdout.write(`${summary}\n`)
if (process.env.GITHUB_STEP_SUMMARY) appendFileSync(process.env.GITHUB_STEP_SUMMARY, summary)
if (failed.length > 0) process.exitCode = 1
