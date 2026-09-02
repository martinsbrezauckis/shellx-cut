import { createHash } from 'node:crypto'
import { isAbsolute } from 'node:path'

export const LINUX_REMOTE_WORKER_HANDLE_SCHEMA = 'shellx-cut.linux-remote-worker-handle@1'

const ID = /^[a-z0-9][a-z0-9._-]*$/
const GIT_OBJECT = /^[a-f0-9]{40}$/
const SHA256 = /^[a-f0-9]{64}$/
const POSITIVE = /^[1-9][0-9]*$/

export function controlRemoteWorkerBinding({ environment, outputRoot, receiptPath, shellPath, nodePath, setsidPath, path, desktopPortalBridge = false }) {
  if (!isAbsolute(outputRoot) || !isAbsolute(receiptPath) || receiptPath !== `${outputRoot}/remote-worker-handle.json`) {
    throw new Error('test-control remote worker receipt must be the exact private output-root handle path')
  }
  for (const [name, value] of Object.entries({ shellPath, nodePath, setsidPath })) {
    if (!isAbsolute(value)) throw new Error(`test-control remote worker requires an absolute enrolled ${name}`)
  }
  if (typeof path !== 'string' || path.split(':').length === 0 || path.split(':').some((entry) => !isAbsolute(entry))) {
    throw new Error('test-control remote worker requires an absolute enrolled PATH')
  }
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
    throw new Error('test-control remote worker candidate binding is invalid')
  }
  const runId = required(environment, 'SHELLX_CUT_TEST_CONTROL_RUN_ID')
  if (!ID.test(runId)) throw new Error('test-control remote worker run id is invalid')
  if (typeof desktopPortalBridge !== 'boolean') throw new Error('test-control remote worker desktop portal bridge mode is invalid')
  return { candidate, outputRoot, receiptPath, runId, shellPath, nodePath, setsidPath, path, mode: desktopPortalBridge ? 'desktop-bridge' : 'setsid' }
}

export function makeLinuxRemoteWorkerHandle({ binding, process }) {
  validateBinding(binding)
  const mode = remoteWorkerMode(binding)
  exactKeys(process, 'remote worker process', ['pid', 'startTicks', 'pgid', 'executablePath', 'commandSha256'])
  for (const key of ['pid', 'startTicks', 'pgid']) if (!POSITIVE.test(String(process[key]))) throw new Error(`remote worker process ${key} must be positive`)
  if (mode === 'setsid' && String(process.pid) !== String(process.pgid)) throw new Error('remote worker process group must be led by its exact shell PID')
  if (process.executablePath !== binding.shellPath || !SHA256.test(process.commandSha256)) throw new Error('remote worker process identity is invalid')
  const unsigned = {
    schema: LINUX_REMOTE_WORKER_HANDLE_SCHEMA,
    runId: binding.runId,
    candidate: structuredClone(binding.candidate),
    outputRoot: binding.outputRoot,
    mode,
    process: structuredClone(process),
  }
  return { ...unsigned, receiptSha256: digest(stable(unsigned)) }
}

export function validateLinuxRemoteWorkerHandle(handle, binding = null) {
  exactKeys(handle, 'remote worker receipt', ['schema', 'runId', 'candidate', 'outputRoot', 'mode', 'process', 'receiptSha256'])
  if (handle.schema !== LINUX_REMOTE_WORKER_HANDLE_SCHEMA) throw new Error('remote worker receipt schema is invalid')
  if (!ID.test(handle.runId) || !isAbsolute(handle.outputRoot) || !['setsid', 'desktop-bridge'].includes(handle.mode)) throw new Error('remote worker receipt identity is invalid')
  exactKeys(handle.candidate, 'remote worker receipt candidate', ['id', 'workspaceRoot', 'sourceCommit', 'sourceTree', 'contentManifestSha256', 'actionManifestSha256', 'fixtureSha256'])
  exactKeys(handle.process, 'remote worker receipt process', ['pid', 'startTicks', 'pgid', 'executablePath', 'commandSha256'])
  for (const key of ['pid', 'startTicks', 'pgid']) if (!POSITIVE.test(String(handle.process[key]))) throw new Error(`remote worker receipt process ${key} is invalid`)
  if ((handle.mode === 'setsid' && String(handle.process.pid) !== String(handle.process.pgid)) || !isAbsolute(handle.process.executablePath) || !SHA256.test(handle.process.commandSha256) || !SHA256.test(handle.receiptSha256)) throw new Error('remote worker receipt process is invalid')
  const unsigned = { ...handle }; delete unsigned.receiptSha256
  if (handle.receiptSha256 !== digest(stable(unsigned))) throw new Error('remote worker receipt hash mismatch')
  if (binding && (stable(binding.candidate) !== stable(handle.candidate) || binding.runId !== handle.runId || binding.outputRoot !== handle.outputRoot || binding.shellPath !== handle.process.executablePath || remoteWorkerMode(binding) !== handle.mode)) throw new Error('remote worker receipt does not match its controller binding')
  return structuredClone(handle)
}

export const REMOTE_WORKER_RECEIPT_NODE_PROGRAM = String.raw`import { createHash } from 'node:crypto';
import { closeSync, existsSync, fsyncSync, linkSync, openSync, readFileSync, realpathSync, unlinkSync, writeFileSync } from 'node:fs';
const fail = (text) => { throw new Error(text); };
const required = (name) => { const value = process.env[name]; if (typeof value !== 'string' || !value || value.includes('\n') || value.includes('\r')) fail('missing remote-worker ' + name); return value; };
const digest = (value) => createHash('sha256').update(value).digest('hex');
const stable = (value) => Array.isArray(value) ? '[' + value.map(stable).join(',') + ']' : value && typeof value === 'object' ? '{' + Object.keys(value).sort().map((key) => JSON.stringify(key) + ':' + stable(value[key])).join(',') + '}' : JSON.stringify(value);
const path = required('SHELLX_CUT_TEST_CONTROL_REMOTE_HANDLE_OUT');
const outputRoot = required('WDIO_OUT');
if (!path.startsWith('/') || path !== outputRoot + '/remote-worker-handle.json') fail('remote-worker receipt path is not exact');
if (existsSync(path)) fail('remote-worker receipt collision');
const shell = required('SHELLX_CUT_TEST_CONTROL_REMOTE_SHELL');
const mode = process.env.SHELLX_CUT_TEST_CONTROL_DESKTOP_PORTAL_BRIDGE === '1' ? 'desktop-bridge' : 'setsid';
const pid = process.ppid;
if (!Number.isInteger(pid) || pid < 2) fail('remote-worker parent PID is invalid');
const stat = String(readFileSync('/proc/' + pid + '/stat', 'utf8')); const end = stat.lastIndexOf(')'); const fields = end >= 0 ? stat.slice(end + 2).trim().split(/\s+/) : [];
const pgid = fields[2]; const startTicks = fields[19]; const executablePath = realpathSync('/proc/' + pid + '/exe'); const cmdline = readFileSync('/proc/' + pid + '/cmdline');
if (!/^[1-9][0-9]*$/.test(String(pgid)) || !/^[1-9][0-9]*$/.test(String(startTicks)) || (mode === 'setsid' && String(pid) !== String(pgid)) || executablePath !== shell) fail('remote-worker process is not the exact registered shell');
if (!Buffer.from(cmdline).equals(Buffer.from(shell + '\0-s\0'))) fail('remote-worker shell argv is not fixed');
const candidate = { id: required('SHELLX_CUT_TEST_CONTROL_CANDIDATE_ID'), workspaceRoot: required('SHELLX_CUT_TEST_CONTROL_WORKSPACE_ROOT'), sourceCommit: required('SHELLX_CUT_TEST_CONTROL_SOURCE_COMMIT'), sourceTree: required('SHELLX_CUT_TEST_CONTROL_SOURCE_TREE'), contentManifestSha256: required('SHELLX_CUT_TEST_CONTROL_CONTENT_SHA256'), actionManifestSha256: required('SHELLX_CUT_TEST_CONTROL_ACTION_SHA256'), fixtureSha256: required('SHELLX_CUT_TEST_CONTROL_FIXTURE_SHA256') };
const unsigned = { schema: 'shellx-cut.linux-remote-worker-handle@1', runId: required('SHELLX_CUT_TEST_CONTROL_RUN_ID'), candidate, outputRoot, mode, process: { pid, startTicks, pgid: Number(pgid), executablePath, commandSha256: digest(cmdline) } };
const receipt = { ...unsigned, receiptSha256: digest(stable(unsigned)) }; const temporary = path + '.' + process.pid + '.tmp'; let fd = null;
try { fd = openSync(temporary, 'wx', 0o600); writeFileSync(fd, JSON.stringify(receipt) + '\n', { encoding: 'utf8' }); fsyncSync(fd); closeSync(fd); fd = null; linkSync(temporary, path); unlinkSync(temporary); }
catch (error) { if (fd !== null) closeSync(fd); try { unlinkSync(temporary); } catch {} throw error; }`;

function validateBinding(binding) {
  controlRemoteWorkerBinding({
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
    outputRoot: binding?.outputRoot, receiptPath: binding?.receiptPath, shellPath: binding?.shellPath,
    nodePath: binding?.nodePath, setsidPath: binding?.setsidPath, path: binding?.path,
    desktopPortalBridge: binding?.mode === 'desktop-bridge',
  })
}
function remoteWorkerMode(binding) {
  const mode = binding?.mode ?? 'setsid'
  if (!['setsid', 'desktop-bridge'].includes(mode)) throw new Error('test-control remote worker mode is invalid')
  return mode
}

function required(environment, name) {
  const value = environment?.[name]
  if (typeof value !== 'string' || value.length === 0 || value.includes('\n') || value.includes('\r')) throw new Error(`test-control remote worker ${name} is required`)
  return value
}
function digest(value) { return createHash('sha256').update(value).digest('hex') }
function stable(value) { if (Array.isArray(value)) return `[${value.map(stable).join(',')}]`; if (value && typeof value === 'object') return `{${Object.keys(value).sort().map((key) => `${JSON.stringify(key)}:${stable(value[key])}`).join(',')}}`; return JSON.stringify(value) }
function exactKeys(value, label, keys) { if (!value || typeof value !== 'object' || Array.isArray(value) || Object.keys(value).sort().join('|') !== [...keys].sort().join('|')) throw new Error(`${label} has unsupported or missing fields`) }
