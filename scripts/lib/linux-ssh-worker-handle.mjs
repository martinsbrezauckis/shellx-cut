import { createHash } from 'node:crypto'
import { spawn } from 'node:child_process'
import { closeSync, fsyncSync, linkSync, lstatSync, openSync, readFileSync, realpathSync, unlinkSync, writeFileSync } from 'node:fs'
import { isAbsolute } from 'node:path'

export const LINUX_SSH_WORKER_HANDLE_SCHEMA = 'shellx-cut.linux-ssh-worker-handle@1'

const ID = /^[a-z0-9][a-z0-9._-]*$/
const GIT_OBJECT = /^[a-f0-9]{40}$/
const SHA256 = /^[a-f0-9]{64}$/
const POSITIVE = /^[1-9][0-9]*$/

export function controlWorkerBinding({ environment, outputRoot, receiptPath, sshPath }) {
  if (!isAbsolute(outputRoot) || !isAbsolute(receiptPath) || receiptPath !== `${outputRoot}/inner-worker-handle.json`) {
    throw new Error('test-control inner worker receipt must be the exact private output-root handle path')
  }
  if (!isAbsolute(sshPath)) throw new Error('test-control inner worker requires an absolute enrolled SSH executable')
  const candidate = {
    id: required(environment, 'SHELLX_CUT_TEST_CONTROL_CANDIDATE_ID'),
    workspaceRoot: required(environment, 'SHELLX_CUT_TEST_CONTROL_WORKSPACE_ROOT'),
    sourceCommit: required(environment, 'SHELLX_CUT_TEST_CONTROL_SOURCE_COMMIT'),
    sourceTree: required(environment, 'SHELLX_CUT_TEST_CONTROL_SOURCE_TREE'),
    contentManifestSha256: required(environment, 'SHELLX_CUT_TEST_CONTROL_CONTENT_SHA256'),
    actionManifestSha256: required(environment, 'SHELLX_CUT_TEST_CONTROL_ACTION_SHA256'),
    fixtureSha256: required(environment, 'SHELLX_CUT_TEST_CONTROL_FIXTURE_SHA256'),
  }
  if (!ID.test(candidate.id) || !isAbsolute(candidate.workspaceRoot) || !GIT_OBJECT.test(candidate.sourceCommit) || !GIT_OBJECT.test(candidate.sourceTree)
    || !SHA256.test(candidate.contentManifestSha256) || !SHA256.test(candidate.actionManifestSha256) || !SHA256.test(candidate.fixtureSha256)) {
    throw new Error('test-control inner worker candidate binding is invalid')
  }
  const runId = required(environment, 'SHELLX_CUT_TEST_CONTROL_RUN_ID')
  if (!ID.test(runId)) throw new Error('test-control inner worker run id is invalid')
  return { candidate, outputRoot, receiptPath, runId, sshPath }
}

export function runWorkerProcess(command, args, { input, cwd, workerBinding = null } = {}) {
  return new Promise((resolveRun, reject) => {
    const child = spawn(command, args, {
      cwd,
      env: process.env,
      stdio: input ? ['pipe', 'inherit', 'inherit'] : 'inherit',
      detached: workerBinding !== null,
    })
    let settled = false
    const removeSignalForwarders = () => {
      process.removeListener('SIGTERM', forwardTerm)
      process.removeListener('SIGINT', forwardInterrupt)
    }
    const rejectRun = (error) => {
      if (settled) return
      settled = true
      removeSignalForwarders()
      if (workerBinding && child.pid) {
        try { process.kill(-child.pid, 'SIGTERM') } catch { try { child.kill('SIGTERM') } catch { /* exact child already stopped */ } }
      }
      reject(error)
    }
    const forwardTerm = () => rejectRun(new Error('registered SSH worker received SIGTERM; exact worker group was terminated'))
    const forwardInterrupt = () => rejectRun(new Error('registered SSH worker received SIGINT; exact worker group was terminated'))
    if (workerBinding) {
      process.once('SIGTERM', forwardTerm)
      process.once('SIGINT', forwardInterrupt)
    }
    child.on('error', rejectRun)
    child.on('spawn', () => {
      if (!workerBinding) return
      try {
        const observed = inspectLinuxSshWorker(child.pid, workerBinding.sshPath, args)
        writeCreateOnlyWorkerHandle(workerBinding.receiptPath, makeLinuxSshWorkerHandle({ binding: workerBinding, process: observed }))
      } catch (error) {
        rejectRun(error)
      }
    })
    child.on('exit', (code, signal) => {
      if (settled) return
      settled = true
      removeSignalForwarders()
      if (code === 0) resolveRun()
      else reject(new Error(`${command} ${args.join(' ')} failed: code=${code} signal=${signal || 'none'}`))
    })
    if (input) child.stdin.end(input)
  })
}

export function makeLinuxSshWorkerHandle({ binding, process }) {
  validateBinding(binding)
  exactKeys(process, 'inner worker process', ['pid', 'startTicks', 'pgid', 'executablePath', 'commandSha256'])
  for (const key of ['pid', 'startTicks', 'pgid']) if (!POSITIVE.test(String(process[key]))) throw new Error(`inner worker process ${key} must be positive`)
  if (String(process.pid) !== String(process.pgid)) throw new Error('inner worker process group must be led by its exact SSH PID')
  if (process.executablePath !== binding.sshPath || !SHA256.test(process.commandSha256)) throw new Error('inner worker process identity is invalid')
  const receipt = {
    schema: LINUX_SSH_WORKER_HANDLE_SCHEMA,
    runId: binding.runId,
    candidate: structuredClone(binding.candidate),
    outputRoot: binding.outputRoot,
    process: structuredClone(process),
  }
  return { ...receipt, receiptSha256: digest(stable(receipt)) }
}

export function inspectLinuxSshWorker(pid, sshPath, argv, { readFile = readFileSync, lstat = lstatSync, realpath = realpathSync } = {}) {
  if (!Number.isInteger(pid) || pid < 2) throw new Error('inner worker PID is invalid')
  const stat = String(readFile(`/proc/${pid}/stat`, 'utf8'))
  const end = stat.lastIndexOf(')')
  const fields = end >= 0 ? stat.slice(end + 2).trim().split(/\s+/) : []
  const startTicks = fields[19]
  const pgid = fields[2]
  const executablePath = String(realpath(`/proc/${pid}/exe`))
  const cmdline = readFile(`/proc/${pid}/cmdline`)
  if (!POSITIVE.test(startTicks || '') || !POSITIVE.test(pgid || '') || String(pid) !== pgid || executablePath !== sshPath) throw new Error('inner worker process observation is incomplete or foreign')
  const expected = Buffer.from(`${[sshPath, ...argv].join('\0')}\0`)
  if (!Buffer.from(cmdline).equals(expected)) throw new Error('inner worker command does not match the fixed SSH argv')
  try {
    const mode = lstat(`/proc/${pid}/exe`)
    if (!mode.isSymbolicLink()) throw new Error('inner worker executable observation is not a proc symlink')
  } catch (error) {
    throw new Error(`inner worker executable observation failed: ${error instanceof Error ? error.message : String(error)}`)
  }
  return { pid, startTicks, pgid: Number(pgid), executablePath, commandSha256: digest(cmdline) }
}

export function writeCreateOnlyWorkerHandle(path, handle, { open = openSync, write = writeFileSync, fsync = fsyncSync, close = closeSync, link = linkSync, unlink = unlinkSync } = {}) {
  validateLinuxSshWorkerHandle(handle)
  if (!isAbsolute(path) || path !== `${handle.outputRoot}/inner-worker-handle.json`) throw new Error('inner worker receipt path must be the exact candidate output path')
  const temporary = `${path}.${process.pid}.tmp`
  let fd = null
  try {
    fd = open(temporary, 'wx', 0o600)
    write(fd, `${JSON.stringify(handle)}\n`, { encoding: 'utf8' })
    fsync(fd)
    close(fd)
    fd = null
    link(temporary, path)
    unlink(temporary)
  } catch (error) {
    if (fd !== null) close(fd)
    try { unlink(temporary) } catch { /* preserve the original failure */ }
    if (error && typeof error === 'object' && error.code === 'EEXIST') throw new Error('inner worker receipt collision')
    throw error
  }
}

export function validateLinuxSshWorkerHandle(handle, binding = null) {
  exactKeys(handle, 'inner worker receipt', ['schema', 'runId', 'candidate', 'outputRoot', 'process', 'receiptSha256'])
  if (handle.schema !== LINUX_SSH_WORKER_HANDLE_SCHEMA) throw new Error('inner worker receipt schema is invalid')
  if (!ID.test(handle.runId) || !isAbsolute(handle.outputRoot)) throw new Error('inner worker receipt identity is invalid')
  exactKeys(handle.candidate, 'inner worker receipt candidate', ['id', 'workspaceRoot', 'sourceCommit', 'sourceTree', 'contentManifestSha256', 'actionManifestSha256', 'fixtureSha256'])
  controlWorkerBinding({
    environment: {
      SHELLX_CUT_TEST_CONTROL_CANDIDATE_ID: handle.candidate.id,
      SHELLX_CUT_TEST_CONTROL_WORKSPACE_ROOT: handle.candidate.workspaceRoot,
      SHELLX_CUT_TEST_CONTROL_SOURCE_COMMIT: handle.candidate.sourceCommit,
      SHELLX_CUT_TEST_CONTROL_SOURCE_TREE: handle.candidate.sourceTree,
      SHELLX_CUT_TEST_CONTROL_CONTENT_SHA256: handle.candidate.contentManifestSha256,
      SHELLX_CUT_TEST_CONTROL_ACTION_SHA256: handle.candidate.actionManifestSha256,
      SHELLX_CUT_TEST_CONTROL_FIXTURE_SHA256: handle.candidate.fixtureSha256,
      SHELLX_CUT_TEST_CONTROL_RUN_ID: handle.runId,
    },
    outputRoot: handle.outputRoot,
    receiptPath: `${handle.outputRoot}/inner-worker-handle.json`,
    sshPath: binding?.sshPath ?? handle.process?.executablePath,
  })
  exactKeys(handle.process, 'inner worker receipt process', ['pid', 'startTicks', 'pgid', 'executablePath', 'commandSha256'])
  for (const key of ['pid', 'startTicks', 'pgid']) if (!POSITIVE.test(String(handle.process[key]))) throw new Error(`inner worker receipt process ${key} is invalid`)
  if (String(handle.process.pid) !== String(handle.process.pgid) || !isAbsolute(handle.process.executablePath) || !SHA256.test(handle.process.commandSha256) || !SHA256.test(handle.receiptSha256)) throw new Error('inner worker receipt process is invalid')
  const unsigned = { ...handle }; delete unsigned.receiptSha256
  if (handle.receiptSha256 !== digest(stable(unsigned))) throw new Error('inner worker receipt hash mismatch')
  if (binding && (stable(binding.candidate) !== stable(handle.candidate) || binding.runId !== handle.runId || binding.outputRoot !== handle.outputRoot || binding.sshPath !== handle.process.executablePath)) throw new Error('inner worker receipt does not match its controller binding')
  return structuredClone(handle)
}

function validateBinding(binding) {
  controlWorkerBinding({
    environment: {
      SHELLX_CUT_TEST_CONTROL_CANDIDATE_ID: binding?.candidate?.id,
      SHELLX_CUT_TEST_CONTROL_WORKSPACE_ROOT: binding?.candidate?.workspaceRoot,
      SHELLX_CUT_TEST_CONTROL_SOURCE_COMMIT: binding?.candidate?.sourceCommit,
      SHELLX_CUT_TEST_CONTROL_SOURCE_TREE: binding?.candidate?.sourceTree,
      SHELLX_CUT_TEST_CONTROL_CONTENT_SHA256: binding?.candidate?.contentManifestSha256,
      SHELLX_CUT_TEST_CONTROL_ACTION_SHA256: binding?.candidate?.actionManifestSha256,
      SHELLX_CUT_TEST_CONTROL_FIXTURE_SHA256: binding?.candidate?.fixtureSha256,
      SHELLX_CUT_TEST_CONTROL_RUN_ID: binding?.runId,
    },
    outputRoot: binding?.outputRoot,
    receiptPath: binding?.receiptPath,
    sshPath: binding?.sshPath,
  })
}

function required(environment, name) {
  const value = environment?.[name]
  if (typeof value !== 'string' || value.length === 0 || value.includes('\n') || value.includes('\r')) throw new Error(`test-control inner worker ${name} is required`)
  return value
}

function digest(value) { return createHash('sha256').update(value).digest('hex') }
function stable(value) { if (Array.isArray(value)) return `[${value.map(stable).join(',')}]`; if (value && typeof value === 'object') return `{${Object.keys(value).sort().map((key) => `${JSON.stringify(key)}:${stable(value[key])}`).join(',')}}`; return JSON.stringify(value) }
function exactKeys(value, label, keys) { if (!value || typeof value !== 'object' || Array.isArray(value) || Object.keys(value).sort().join('|') !== [...keys].sort().join('|')) throw new Error(`${label} has unsupported or missing fields`) }
