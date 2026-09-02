import { homedir } from 'node:os'

import { controlWorkerBinding, runWorkerProcess } from './linux-ssh-worker-handle.mjs'
import { controlRemoteWorkerBinding } from './linux-remote-worker-handle.mjs'
import { buildSshEnvPayload, SSH_KEEPALIVE_ARGS } from './ssh-stdin-env.mjs'

const SSH_TEST_CONTROL_ARGS = Object.freeze([
  '-o', 'BatchMode=yes', '-o', 'PasswordAuthentication=no', '-o', 'KbdInteractiveAuthentication=no',
  '-o', 'NumberOfPasswordPrompts=0', '-o', 'PreferredAuthentications=publickey', '-o', 'PubkeyAuthentication=yes',
  ...SSH_KEEPALIVE_ARGS,
])

export async function runLinuxSshTestControlTransport({ testControl, host, remoteDir, outputRoot, innerWorkerReceipt, remoteWorkerReceipt, sshBin, remotePath, remoteShell, remoteNode, remoteSetsid, environment, anthropicKey, remoteScript }) {
  const workerBinding = testControl ? controlWorkerBinding({ environment: process.env, outputRoot, receiptPath: innerWorkerReceipt, sshPath: sshBin }) : null
  const desktopPortalBridge = environment.find(([key]) => key === 'DESKTOP_SESSION')?.[1] === '1'
  const remoteWorkerBinding = testControl ? controlRemoteWorkerBinding({ environment: process.env, outputRoot, receiptPath: remoteWorkerReceipt, shellPath: remoteShell, nodePath: remoteNode, setsidPath: remoteSetsid, path: remotePath, desktopPortalBridge }) : null
  if (workerBinding && workerBinding.candidate.workspaceRoot !== remoteDir) throw new Error('test-control inner worker candidate workspace must match the fixed remote directory')
  if (remoteWorkerBinding && remoteWorkerBinding.candidate.workspaceRoot !== remoteDir) throw new Error('test-control remote worker candidate workspace must match the fixed remote directory')
  if (workerBinding && JSON.stringify(workerBinding.candidate) !== JSON.stringify(remoteWorkerBinding.candidate)) throw new Error('test-control remote and inner worker candidate bindings differ')
  if (testControl) environment.push(
    ['SHELLX_CUT_TEST_CONTROL_CANDIDATE_ID', workerBinding.candidate.id],
    ['SHELLX_CUT_TEST_CONTROL_WORKSPACE_ROOT', workerBinding.candidate.workspaceRoot],
    ['SHELLX_CUT_TEST_CONTROL_SOURCE_COMMIT', workerBinding.candidate.sourceCommit],
    ['SHELLX_CUT_TEST_CONTROL_SOURCE_TREE', workerBinding.candidate.sourceTree],
    ['SHELLX_CUT_TEST_CONTROL_CONTENT_SHA256', workerBinding.candidate.contentManifestSha256],
    ['SHELLX_CUT_TEST_CONTROL_ACTION_SHA256', workerBinding.candidate.actionManifestSha256],
    ['SHELLX_CUT_TEST_CONTROL_FIXTURE_SHA256', workerBinding.candidate.fixtureSha256],
    ['SHELLX_CUT_TEST_CONTROL_RUN_ID', workerBinding.runId],
    ['SHELLX_CUT_TEST_CONTROL_REMOTE_HANDLE_OUT', remoteWorkerBinding.receiptPath],
    ['SHELLX_CUT_TEST_CONTROL_REMOTE_SHELL', remoteWorkerBinding.shellPath],
    ['SHELLX_CUT_TEST_CONTROL_REMOTE_PATH', remoteWorkerBinding.path],
    ['SHELLX_CUT_TEST_CONTROL_DESKTOP_PORTAL_BRIDGE', remoteWorkerBinding.mode === 'desktop-bridge' ? '1' : '0'],
  )
  const envPrefix = environment.map(([key, value]) => `${key}=${shellQuote(value)}`).join(' ')
  if (desktopPortalBridge && !environment.some(([key, value]) => key === 'PORTAL_SESSION_TYPE' && value === 'wayland')) {
    throw new Error('test-control desktop portal bridge requires the enrolled Wayland session identity')
  }
  const remoteWorker = desktopPortalBridge
    ? `${shellQuote(remoteShell)} -s <<'SHELLX_CUT_REMOTE_WORKER'\n${remoteScript}\nSHELLX_CUT_REMOTE_WORKER`
    : `${shellQuote(remoteSetsid)} ${shellQuote(remoteShell)} -s <<'SHELLX_CUT_REMOTE_WORKER'\n${remoteScript}\nSHELLX_CUT_REMOTE_WORKER`
  const sshPayload = testControl
    ? { command: `${remoteShell} -s`, input: `${environment.map(([key, value]) => `export ${key}=${shellQuote(value)}`).join('\n')}\nset -eu\n${remoteWorker}` }
    : buildSshEnvPayload(anthropicKey, 'ANTHROPIC_API_KEY', envPrefix, remoteScript)
  await runWorkerProcess(testControl ? sshBin : 'ssh', [...(testControl ? SSH_TEST_CONTROL_ARGS : SSH_KEEPALIVE_ARGS), host, sshPayload.command], { cwd: homedir(), input: sshPayload.input, workerBinding })
}

export function shellQuote(value) {
  return `'${String(value).replaceAll("'", "'\\\"'\\\"'")}'`
}
