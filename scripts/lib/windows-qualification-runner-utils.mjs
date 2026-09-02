import { createHash } from 'node:crypto'
import { spawn, spawnSync } from 'node:child_process'
import { readFileSync } from 'node:fs'
import { homedir } from 'node:os'
import { join, resolve } from 'node:path'

import { forwardEnvToWindows } from './wsl-interop-env.mjs'

export function arg(argv, name, fallback = '') {
  const index = argv.indexOf(name)
  return index >= 0 && argv[index + 1] ? argv[index + 1] : fallback
}

export function values(argv, name) {
  const found = []
  for (let index = 0; index < argv.length; index += 1) {
    if (argv[index] === name && argv[index + 1]) found.push(argv[index + 1])
  }
  return found
}

export const flag = (argv, name) => argv.includes(name)
export const stamp = () => new Date().toISOString().replace(/[:.]/g, '-')
export const expandHome = (path) => path.startsWith('~/') ? join(homedir(), path.slice(2)) : path

export function run(command, args, { env = process.env, cwd } = {}) {
  return new Promise((resolveRun, reject) => {
    const child = spawn(command, args, { cwd, env, stdio: 'inherit' })
    child.on('error', reject)
    child.on('exit', (code, signal) => {
      if (code === 0) resolveRun()
      else reject(new Error(`${command} ${args.join(' ')} failed: code=${code} signal=${signal || 'none'}`))
    })
  })
}

export function captureSync(command, args, { cwd, env = process.env } = {}) {
  const result = spawnSync(command, args, { cwd, env, encoding: 'utf8' })
  if (result.status !== 0) throw new Error(`${command} ${args.join(' ')} failed: ${result.stderr || result.stdout}`)
  return result.stdout.trim()
}

export const windowsPath = (path, cwd) => captureSync('wslpath', ['-w', resolve(path)], { cwd })
export const linuxPath = (path, cwd) => captureSync('wslpath', ['-u', path], { cwd })
export const sha256 = (path) => createHash('sha256').update(readFileSync(path)).digest('hex')

export function inspectWindowsSignedArtifact({ artifactPath, cwd } = {}) {
  const path = resolve(String(artifactPath || ''))
  const script = [
    '$ErrorActionPreference="Stop"',
    '$sig=Get-AuthenticodeSignature -LiteralPath $env:SHELLX_CUT_ARTIFACT',
    'if($sig.Status -ne "Valid"){throw "Authenticode status is $($sig.Status)"}',
    '[pscustomobject]@{sha256=(Get-FileHash -Algorithm SHA256 -LiteralPath $env:SHELLX_CUT_ARTIFACT).Hash.ToLower();signatureStatus=[string]$sig.Status}|ConvertTo-Json -Compress',
  ].join(';')
  const raw = captureSync('powershell.exe', ['-NoProfile', '-NonInteractive', '-Command', script], {
    cwd,
    env: forwardEnvToWindows(
      { ...process.env },
      { SHELLX_CUT_ARTIFACT: windowsPath(path, cwd) },
    ),
  })
  let identity
  try {
    identity = JSON.parse(raw)
  } catch {
    throw new Error('signed Windows artifact probe did not return JSON')
  }
  if (!/^[a-f0-9]{64}$/.test(String(identity?.sha256 || '')) || identity?.signatureStatus !== 'Valid') {
    throw new Error(`signed Windows artifact identity is incomplete: ${path}`)
  }
  return identity
}

export function inspectWindowsInstalledCut({ expectedVersion, cwd, requireSignature = false } = {}) {
  const version = String(expectedVersion || '').trim()
  if (!/^\d+\.\d+\.\d+/.test(version)) throw new Error('installed Cut identity requires an exact expected version')
  const versionPattern = new RegExp(`^${version.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}(?:[.+-]|$)`)
  const script = [
    '$ErrorActionPreference="Stop"',
    `$expected='${version.replaceAll("'", "''")}'`,
    '$root=Join-Path $env:LOCALAPPDATA "ShellX Cut"',
    '$files=[ordered]@{shell="shellx-cut.exe";cutd="cutd.exe"}',
    '$out=[ordered]@{}',
    'foreach($entry in $files.GetEnumerator()) {',
    '  $path=Join-Path $root $entry.Value',
    '  if(-not (Test-Path -LiteralPath $path)){throw "installed $($entry.Key) is missing"}',
    '  $item=Get-Item -LiteralPath $path',
    '  $product=[string]$item.VersionInfo.ProductVersion',
    '  if(-not ($product -match ("^"+[regex]::Escape($expected)+"(?:[.+-]|$)"))){throw "installed $($entry.Key) version $product differs from $expected"}',
    '  $signature=Get-AuthenticodeSignature -LiteralPath $path',
    `  if(${requireSignature ? '$true' : '$false'} -and $signature.Status -ne "Valid"){throw "installed $($entry.Key) Authenticode status is $($signature.Status)"}`,
    '  $out[$entry.Key]=[ordered]@{path=$path;sha256=(Get-FileHash -Algorithm SHA256 -LiteralPath $path).Hash.ToLower();productVersion=$product;signatureStatus=[string]$signature.Status}',
    '}',
    '$out|ConvertTo-Json -Compress',
  ].join(';')
  const raw = captureSync('powershell.exe', ['-NoProfile', '-NonInteractive', '-Command', script], { cwd })
  let identity
  try {
    identity = JSON.parse(raw)
  } catch {
    throw new Error('installed Cut identity probe did not return JSON')
  }
  for (const name of ['shell', 'cutd']) {
    const entry = identity?.[name]
    if (!/^[a-f0-9]{64}$/.test(String(entry?.sha256 || ''))
      || (requireSignature && entry?.signatureStatus !== 'Valid')
      || !versionPattern.test(String(entry?.productVersion || ''))) {
      throw new Error(`installed ${name} identity probe is incomplete`)
    }
  }
  return identity
}
