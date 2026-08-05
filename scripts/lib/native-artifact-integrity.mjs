import { createHash } from 'node:crypto'
import { spawnSync } from 'node:child_process'

import {
  INSTALLED_INTEGRITY_SCHEMA,
} from './installed-walkthrough-receipt.mjs'
import { probeClosedPort } from './installed-runtime-evidence.mjs'

function outputDigest(stdout, stderr) {
  return createHash('sha256')
    .update(`stdout\0${stdout || ''}\nstderr\0${stderr || ''}`)
    .digest('hex')
}

export function runIntegrityCommand(id, executable, args, { cwd } = {}) {
  const result = spawnSync(executable, args, {
    cwd,
    encoding: 'utf8',
    windowsHide: true,
  })
  return {
    id,
    executable,
    args,
    status: Number.isInteger(result.status) ? result.status : -1,
    signal: result.signal || null,
    errorCode: result.error?.code || null,
    outputSha256: outputDigest(result.stdout, result.stderr),
  }
}

export function macIntegrityCommands(appPath, phase) {
  return [
    runIntegrityCommand(`codesign-${phase}`, 'codesign', ['--verify', '--deep', '--strict', appPath]),
    runIntegrityCommand(`spctl-${phase}`, 'spctl', ['--assess', '--type', 'execute', appPath]),
    runIntegrityCommand(`stapler-${phase}`, 'xcrun', ['stapler', 'validate', appPath]),
  ]
}

export function linuxIntegrityCommand(packagePath, phase) {
  return runIntegrityCommand(`dpkg-info-${phase}`, 'dpkg-deb', ['--info', packagePath])
}

function powerShellSingleQuoted(value) {
  return `'${String(value).replaceAll("'", "''")}'`
}

export function windowsAuthenticodeCommand({ shellPath, cutdPath, phase }) {
  const script = [
    '$ErrorActionPreference="Stop"',
    `$paths=@(${powerShellSingleQuoted(shellPath)},${powerShellSingleQuoted(cutdPath)})`,
    '$rows=@($paths | ForEach-Object {',
    '  $sig=Get-AuthenticodeSignature -LiteralPath $_',
    '  [pscustomobject]@{name=[IO.Path]::GetFileName($_);status=[string]$sig.Status}',
    '})',
    'if(@($rows | Where-Object status -ne "Valid").Count -ne 0){$rows|ConvertTo-Json -Compress;exit 1}',
    '$rows|ConvertTo-Json -Compress',
  ].join(';')
  return runIntegrityCommand(`authenticode-${phase}`, 'powershell.exe', [
    '-NoProfile', '-NonInteractive', '-Command', script,
  ])
}

export async function buildNativeIntegrityEvidence({
  source,
  surface,
  artifactSha256,
  preUseSha256,
  postUseSha256,
  commands,
  signed = false,
  notarized = false,
}) {
  const postUseWebdriverPort = await probeClosedPort()
  const status = artifactSha256 === preUseSha256
    && artifactSha256 === postUseSha256
    && commands.every((command) => command.status === 0)
    && postUseWebdriverPort.closed === true
    ? 'pass'
    : 'fail'
  return {
    schema: INSTALLED_INTEGRITY_SCHEMA,
    generatedAt: new Date().toISOString(),
    status,
    surface,
    source: { ...source },
    artifact: { sha256: artifactSha256, preUseSha256, postUseSha256 },
    webdriverTestFeatureAbsent: postUseWebdriverPort.closed === true,
    webdriverTestPortAfterUse: postUseWebdriverPort,
    signed: signed === true,
    notarized: notarized === true,
    commands,
  }
}
