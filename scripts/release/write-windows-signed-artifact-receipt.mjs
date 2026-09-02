#!/usr/bin/env node
import { execFileSync, spawnSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { readFileSync, writeFileSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

import {
  WINDOWS_SIGNED_ARTIFACT_RECEIPT_SCHEMA,
  packagedArtifactsFromSigningEvents,
  readWindowsSigningEvents,
  sha256File,
  validateWindowsSignedArtifactReceipt,
} from '../lib/windows-signed-artifact-receipt.mjs'
import { forwardEnvToWindows } from '../lib/wsl-interop-env.mjs'
import { sourceContentManifest } from '../lib/source-content-manifest.mjs'

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..')

function option(name) {
  const index = process.argv.indexOf(name)
  const value = index >= 0 ? process.argv[index + 1] : ''
  if (!value || value.startsWith('--')) throw new Error(`${name} is required`)
  return resolve(value)
}

function git(args) {
  return execFileSync('git', args, { cwd: ROOT, encoding: 'utf8' }).trim()
}

function windowsPath(path) {
  return execFileSync('wslpath', ['-w', resolve(path)], { cwd: ROOT, encoding: 'utf8' }).trim()
}

function inspectAuthenticode(path) {
  const result = spawnSync('powershell.exe', [
    '-NoProfile', '-NonInteractive', '-Command',
    '$ErrorActionPreference="Stop";$sig=Get-AuthenticodeSignature -LiteralPath $env:SHELLX_CUT_ARTIFACT;' +
    'if($sig.Status -ne "Valid"){throw "Authenticode status is $($sig.Status)"};' +
    '[pscustomobject]@{status=[string]$sig.Status;sha256=(Get-FileHash -Algorithm SHA256 -LiteralPath $env:SHELLX_CUT_ARTIFACT).Hash.ToLower()}|ConvertTo-Json -Compress',
  ], {
    cwd: ROOT,
    encoding: 'utf8',
    env: forwardEnvToWindows(process.env, { SHELLX_CUT_ARTIFACT: windowsPath(path) }),
  })
  if (result.status !== 0) throw new Error(`Authenticode inspection failed for ${path}: ${(result.stderr || result.stdout).trim()}`)
  const row = JSON.parse(result.stdout.trim())
  if (row.status !== 'Valid' || row.sha256 !== sha256File(path)) throw new Error(`Authenticode identity is incomplete for ${path}`)
  return { sha256: row.sha256, signatureStatus: row.status }
}

function updaterSignature(path) {
  if (!path) return null
  return { name: path.split(/[\\/]/).at(-1), sha256: sha256File(path) }
}

function main() {
  const out = option('--out')
  const installerPath = option('--installer')
  const shellPath = option('--shell')
  const cutdPath = option('--cutd')
  const eventLog = option('--signing-events')
  const updaterSignaturePath = process.argv.includes('--updater-signature') ? option('--updater-signature') : ''
  const events = readWindowsSigningEvents(eventLog)
  const packaged = packagedArtifactsFromSigningEvents({ events, installerPath })
  const installer = inspectAuthenticode(installerPath)
  const standaloneShell = inspectAuthenticode(shellPath)
  const standaloneCutd = inspectAuthenticode(cutdPath)
  if (packaged.installer.sha256 !== installer.sha256) throw new Error('installer signing event hash differs from final installer')
  if (packaged.cutd.sha256 !== standaloneCutd.sha256) throw new Error('bundled cutd signing event differs from the staged signed cutd')

  const config = JSON.parse(readFileSync(join(ROOT, 'app/desktop/src-tauri/tauri.conf.json'), 'utf8'))
  const receipt = {
    schema: WINDOWS_SIGNED_ARTIFACT_RECEIPT_SCHEMA,
    generatedAt: new Date().toISOString(),
    status: 'pass',
    source: {
      gitCommit: git(['rev-parse', 'HEAD']),
      gitTree: git(['rev-parse', 'HEAD^{tree}']),
      contentManifestSha256: sourceContentManifest(ROOT).sha256,
      version: config.version,
    },
    installer: { name: installerPath.split(/[\\/]/).at(-1), ...installer },
    packaged: {
      shell: { sha256: packaged.shell.sha256, signatureStatus: packaged.shell.signatureStatus },
      cutd: { sha256: packaged.cutd.sha256, signatureStatus: packaged.cutd.signatureStatus },
    },
    standalone: {
      shell: standaloneShell,
      cutd: standaloneCutd,
    },
    updaterSignature: updaterSignature(updaterSignaturePath),
    signingEventsSha256: createHash('sha256').update(readFileSync(eventLog)).digest('hex'),
  }
  validateWindowsSignedArtifactReceipt(receipt, {
    installerPath,
    source: receipt.source,
  })
  writeFileSync(out, `${JSON.stringify(receipt, null, 2)}\n`)
  console.log(`[windows-artifact-receipt] ${out}`)
}

main()
