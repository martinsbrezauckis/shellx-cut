import assert from 'node:assert/strict'
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, statSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import test from 'node:test'

import {
  LINUX_SSH_WORKER_HANDLE_SCHEMA,
  controlWorkerBinding,
  inspectLinuxSshWorker,
  makeLinuxSshWorkerHandle,
  validateLinuxSshWorkerHandle,
  writeCreateOnlyWorkerHandle,
} from '../lib/linux-ssh-worker-handle.mjs'

const HASH = 'a'.repeat(64)
const binding = (root) => controlWorkerBinding({
  environment: {
    SHELLX_CUT_TEST_CONTROL_CANDIDATE_ID: 'cut-final-20260816',
    SHELLX_CUT_TEST_CONTROL_WORKSPACE_ROOT: `${root}/candidate`,
    SHELLX_CUT_TEST_CONTROL_SOURCE_COMMIT: '0123456789abcdef0123456789abcdef01234567',
    SHELLX_CUT_TEST_CONTROL_SOURCE_TREE: '89abcdef0123456789abcdef0123456789abcdef',
    SHELLX_CUT_TEST_CONTROL_CONTENT_SHA256: HASH,
    SHELLX_CUT_TEST_CONTROL_ACTION_SHA256: 'b'.repeat(64),
    SHELLX_CUT_TEST_CONTROL_FIXTURE_SHA256: 'c'.repeat(64),
    SHELLX_CUT_TEST_CONTROL_RUN_ID: 'pc2-final',
  },
  outputRoot: `${root}/out`,
  receiptPath: `${root}/out/inner-worker-handle.json`,
  sshPath: '/usr/bin/ssh',
})

function handle(root) {
  return makeLinuxSshWorkerHandle({
    binding: binding(root),
    process: { pid: 4321, startTicks: 9876, pgid: 4321, executablePath: '/usr/bin/ssh', commandSha256: 'd'.repeat(64) },
  })
}

test('inner SSH worker receipt is candidate-bound, PID-reuse-resistant, and exact', () => {
  const value = handle('/tmp/cut-inner-worker-fixture')
  assert.equal(value.schema, LINUX_SSH_WORKER_HANDLE_SCHEMA)
  assert.equal(value.candidate.id, 'cut-final-20260816')
  assert.equal(value.process.pgid, value.process.pid)
  assert.deepEqual(validateLinuxSshWorkerHandle(value, binding('/tmp/cut-inner-worker-fixture')), value)
  assert.throws(() => validateLinuxSshWorkerHandle({ ...value, process: { ...value.process, startTicks: 1 } }, binding('/tmp/cut-inner-worker-fixture')), /hash mismatch/)
  assert.throws(() => makeLinuxSshWorkerHandle({ binding: binding('/tmp/cut-inner-worker-fixture'), process: { ...value.process, pgid: 99 } }), /process group/)
  assert.throws(() => controlWorkerBinding({ environment: {}, outputRoot: '/tmp/out', receiptPath: '/tmp/out/inner-worker-handle.json', sshPath: '/usr/bin/ssh' }), /CANDIDATE_ID/)
})

test('inner SSH worker receipt is atomically create-only and rejects collisions or foreign paths', () => {
  const root = mkdtempSync(join(tmpdir(), 'cut-inner-worker-handle-'))
  try {
    const value = handle(root)
    const path = `${root}/out/inner-worker-handle.json`
    mkdirSync(`${root}/out`, { mode: 0o700 })
    writeCreateOnlyWorkerHandle(path, value)
    assert.equal(statSync(path).mode & 0o777, 0o600)
    assert.deepEqual(JSON.parse(readFileSync(path, 'utf8')), value)
    assert.equal(existsSync(`${path}.${process.pid}.tmp`), false)
    assert.throws(() => writeCreateOnlyWorkerHandle(path, value), /collision/)
    assert.throws(() => writeCreateOnlyWorkerHandle(`${root}/out/other.json`, value), /exact candidate output path/)
  } finally {
    rmSync(root, { recursive: true, force: true })
  }
})

test('inner SSH observation rejects stale, foreign, or non-leader fixture identities', () => {
  const fields = Array(20).fill('0')
  fields[0] = 'S'; fields[2] = '4321'; fields[19] = '9876'
  const reads = (path) => {
    if (path.endsWith('/stat')) return `4321 (ssh) ${fields.join(' ')}`
    if (path.endsWith('/cmdline')) return Buffer.from('/usr/bin/ssh\0-o\0ServerAliveInterval=15\0pc2\0bash -s\0')
    throw new Error(`unexpected read ${path}`)
  }
  const options = { readFile: reads, realpath: () => '/usr/bin/ssh', lstat: () => ({ isSymbolicLink: () => true }) }
  assert.deepEqual(inspectLinuxSshWorker(4321, '/usr/bin/ssh', ['-o', 'ServerAliveInterval=15', 'pc2', 'bash -s'], options), {
    pid: 4321, startTicks: '9876', pgid: 4321, executablePath: '/usr/bin/ssh', commandSha256: '388ba169b2ab2f8d43090fe57e032fc2cbe7c1f1aa279bf0efba7134866adb47',
  })
  fields[2] = '99'
  assert.throws(() => inspectLinuxSshWorker(4321, '/usr/bin/ssh', ['-o', 'ServerAliveInterval=15', 'pc2', 'bash -s'], options), /process observation/)
  fields[2] = '4321'; fields[19] = '0'
  assert.throws(() => inspectLinuxSshWorker(4321, '/usr/bin/ssh', ['-o', 'ServerAliveInterval=15', 'pc2', 'bash -s'], options), /process observation/)
  fields[19] = '9876'
  assert.throws(() => inspectLinuxSshWorker(4321, '/usr/bin/ssh', ['-o', 'ServerAliveInterval=15', 'pc2', 'bash -c'], options), /fixed SSH argv/)
})
