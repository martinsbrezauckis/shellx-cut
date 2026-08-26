import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { chmodSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { createServer } from 'node:http'
import { tmpdir } from 'node:os'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const uiRoot = resolve(here, '..')
const repoRoot = resolve(uiRoot, '..')
const packageJson = JSON.parse(readFileSync(join(uiRoot, 'package.json'), 'utf8'))
const contracts = [
  {
    id: 'LLA-008',
    command: 'verify-generated-lifecycle',
    script: 'node public-tests/verify-generated-asset-lifecycle.mjs',
    diagnostic: /project\.create failed/,
  },
  {
    id: 'LLA-009',
    command: 'verify-publish-package',
    script: 'node public-tests/verify-publish-package.mjs',
    diagnostic: /project\.create failed/,
  },
  {
    id: 'LLA-032',
    command: 'verify-layout-contract',
    script: 'node public-tests/layout-contract-verify.mjs',
    diagnostic: /data-cut-panel="topbar"/,
  },
]

function listen(server) {
  return new Promise((resolve, reject) => {
    server.once('error', reject)
    server.listen(0, '127.0.0.1', () => resolve(server.address().port))
  })
}

function run(command, env) {
  return new Promise((resolve, reject) => {
    const child = spawn('npm', ['--prefix', uiRoot, 'run', command], {
      cwd: repoRoot,
      env,
    })
    let stdout = ''
    let stderr = ''
    const timer = setTimeout(() => {
      child.kill('SIGTERM')
      reject(new Error(`${command} timed out`))
    }, 30_000)
    child.stdout.on('data', (chunk) => { stdout += chunk })
    child.stderr.on('data', (chunk) => { stderr += chunk })
    child.once('error', (error) => {
      clearTimeout(timer)
      reject(error)
    })
    child.once('close', (status, signal) => {
      clearTimeout(timer)
      resolve({ status, signal, stdout, stderr })
    })
  })
}

const scratch = mkdtempSync(join(tmpdir(), 'cut-package-command-contract-'))
const ffmpegFixture = join(scratch, 'ffmpeg-fixture.mjs')
writeFileSync(ffmpegFixture, '#!/usr/bin/env node\nprocess.exit(0)\n')
chmodSync(ffmpegFixture, 0o755)

const server = createServer((request, response) => {
  if (request.url?.startsWith('/api/verb/project.create')) {
    response.writeHead(200, { 'content-type': 'application/json' })
    response.end(JSON.stringify({ ok: false, error: { code: 'contract_probe', message: 'controlled command-contract failure' } }))
    return
  }
  response.writeHead(200, { 'content-type': 'text/html' })
  response.end('<!doctype html><title>controlled incomplete Cut surface</title>')
})

try {
  const port = await listen(server)
  const endpoint = `http://127.0.0.1:${port}`
  for (const contract of contracts) {
    assert.equal(
      packageJson.scripts?.[contract.command],
      contract.script,
      `${contract.id} must expose its direct verifier command`,
    )
    const result = await run(contract.command, {
      ...process.env,
      SWEEP_CUTD: endpoint,
      SWEEP_APP: endpoint,
      CUTD_ADDR: `127.0.0.1:${port}`,
      FFMPEG_BIN: ffmpegFixture,
      CUT_LAYOUT_VERIFY_TIMEOUT_MS: '100',
    })
    const output = `${result.stdout || ''}\n${result.stderr || ''}`
    assert.equal(result.signal, null, `${contract.id} command was terminated by ${result.signal}`)
    assert.equal(result.status, 1, `${contract.id} must fail non-zero against an incomplete stack`)
    assert.match(output, contract.diagnostic, `${contract.id} must report its verifier-owned failed result`)
    assert.doesNotMatch(output, /layout-contract-verify: PASS/, `${contract.id} incomplete-stack run must not report a green result`)
  }
} finally {
  await new Promise((resolve) => server.close(resolve))
  rmSync(scratch, { recursive: true, force: true })
}

console.log('package command contract tests passed')
