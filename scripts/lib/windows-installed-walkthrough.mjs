import { spawnSync } from 'node:child_process'
import { copyFileSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { join, resolve, win32 } from 'node:path'

import { artifactInfo } from './ignored-test-rig.mjs'
import {
  buildNativeIntegrityEvidence,
  windowsAuthenticodeCommand,
} from './native-artifact-integrity.mjs'
import { buildInstalledWalkthroughReceipt } from './installed-walkthrough-receipt.mjs'

function capture(command, args, cwd) {
  const result = spawnSync(command, args, { cwd, encoding: 'utf8', windowsHide: true })
  if (result.status !== 0) throw new Error(`${command} failed: ${result.stderr || result.stdout}`)
  return result.stdout.trim()
}

function windowsToWsl(path, cwd) {
  return capture('wslpath', ['-u', path], cwd)
}

function writeNew(path, value) {
  writeFileSync(path, `${JSON.stringify(value, null, 2)}\n`, { encoding: 'utf8', flag: 'wx' })
}

function readJson(path, label) {
  try {
    return JSON.parse(readFileSync(path, 'utf8'))
  } catch (error) {
    throw new Error(`${label} is not readable JSON: ${error?.message || String(error)}`)
  }
}

export function beginWindowsInstalledWalkthrough({ root, out, source }) {
  const localAppData = capture('powershell.exe', [
    '-NoProfile', '-NonInteractive', '-Command', '$env:LOCALAPPDATA',
  ], root)
  const installRoot = win32.join(localAppData, 'ShellX Cut')
  const shellWin = win32.join(installRoot, 'shellx-cut.exe')
  const cutdWin = win32.join(installRoot, 'cutd.exe')
  const shellWsl = windowsToWsl(shellWin, root)
  const cutdWsl = windowsToWsl(cutdWin, root)
  const shell = artifactInfo(shellWsl)
  const cutd = artifactInfo(cutdWsl)
  if (!shell.exists || !shell.sha256 || !cutd.exists || !cutd.sha256) {
    throw new Error('installed Windows shell/cutd artifacts are missing')
  }
  const command = windowsAuthenticodeCommand({ shellPath: shellWin, cutdPath: cutdWin, phase: 'pre' })
  if (command.status !== 0) throw new Error('installed Windows Authenticode check failed before UI qualification')
  const artifactDir = join(out, 'artifacts')
  mkdirSync(artifactDir, { recursive: true })
  const retainedShell = join(artifactDir, 'installed-shellx-cut.exe')
  const retainedCutd = join(artifactDir, 'installed-cutd.exe')
  copyFileSync(shellWsl, retainedShell)
  copyFileSync(cutdWsl, retainedCutd)
  return {
    source,
    shellWin,
    cutdWin,
    shellWsl,
    cutdWsl,
    retainedShell,
    retainedCutd,
    artifactSha256: shell.sha256,
    preUseSha256: shell.sha256,
    cutdSha256: cutd.sha256,
    commands: [command],
  }
}

export async function finishWindowsInstalledWalkthrough({ root, out, session }) {
  const postShell = artifactInfo(session.shellWsl)
  const postCutd = artifactInfo(session.cutdWsl)
  if (postCutd.sha256 !== session.cutdSha256) throw new Error('installed Windows cutd changed during qualification')
  const command = windowsAuthenticodeCommand({
    shellPath: session.shellWin,
    cutdPath: session.cutdWin,
    phase: 'post',
  })
  const integrity = await buildNativeIntegrityEvidence({
    source: session.source,
    surface: 'windows-installed',
    artifactSha256: session.artifactSha256,
    preUseSha256: session.preUseSha256,
    postUseSha256: postShell.sha256,
    commands: [...session.commands, command],
    signed: true,
  })
  const runtime = readJson(join(out, 'installed-runtime-receipt.json'), 'installed runtime receipt')
  const fullCoverage = readJson(join(out, 'full-coverage-receipt.json'), 'full-coverage receipt')
  const receipt = buildInstalledWalkthroughReceipt({
    source: session.source,
    surface: 'windows-installed',
    artifact: { sha256: session.artifactSha256 },
    runtimeEvidence: runtime,
    fullCoverageReceipt: fullCoverage,
    integrityEvidence: integrity,
  })
  writeNew(join(out, 'installed-artifact-integrity.json'), integrity)
  writeNew(join(out, 'installed-walkthrough-receipt.json'), receipt)
  const retained = artifactInfo(resolve(session.retainedShell))
  if (retained.sha256 !== session.artifactSha256) throw new Error('retained installed Windows artifact digest changed')
  if (artifactInfo(resolve(session.retainedCutd)).sha256 !== session.cutdSha256) {
    throw new Error('retained installed Windows cutd digest changed')
  }
  return receipt
}
