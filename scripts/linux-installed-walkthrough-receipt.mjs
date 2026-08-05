#!/usr/bin/env node
import { existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

import { artifactInfo } from './lib/ignored-test-rig.mjs'
import {
  buildNativeIntegrityEvidence,
  linuxIntegrityCommand,
} from './lib/native-artifact-integrity.mjs'
import {
  buildInstalledWalkthroughReceipt,
} from './lib/installed-walkthrough-receipt.mjs'
import { sourceContentManifest } from './lib/source-content-manifest.mjs'

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..')

function arg(name, fallback = '') {
  const index = process.argv.indexOf(name)
  return index >= 0 && process.argv[index + 1] ? process.argv[index + 1] : fallback
}

function requiredArg(name) {
  const value = arg(name)
  if (!value) throw new Error(`${name} is required`)
  return value
}

function readJson(path, label) {
  try {
    return JSON.parse(readFileSync(path, 'utf8'))
  } catch (error) {
    throw new Error(`${label} is not readable JSON: ${error?.message || String(error)}`)
  }
}

function writeNew(path, value) {
  mkdirSync(dirname(path), { recursive: true })
  writeFileSync(path, `${JSON.stringify(value, null, 2)}\n`, { encoding: 'utf8', flag: 'wx' })
}

function sourceIdentity() {
  const version = readJson(
    join(ROOT, 'app/desktop/src-tauri/tauri.conf.json'),
    'Tauri config',
  ).version
  const content = sourceContentManifest(ROOT)
  const gitCommit = requiredArg('--source-commit')
  const expectedContent = requiredArg('--source-content-manifest')
  if (!/^[a-f0-9]{40}$/.test(gitCommit)) throw new Error('--source-commit must be a full Git SHA')
  if (content.sha256 !== expectedContent) throw new Error('synchronized source content digest mismatch')
  return { gitCommit, version, contentManifestSha256: content.sha256 }
}

function start() {
  if (process.platform !== 'linux') throw new Error('Linux installed walkthrough must run on native Linux')
  const artifactPath = resolve(requiredArg('--artifact'))
  const packagePath = resolve(requiredArg('--package'))
  const outPath = resolve(requiredArg('--pre-out'))
  if (!existsSync(artifactPath) || !existsSync(packagePath)) throw new Error('installed artifact/package is missing')
  const artifact = artifactInfo(artifactPath)
  const source = sourceIdentity()
  const command = linuxIntegrityCommand(packagePath, 'pre')
  if (command.status !== 0) throw new Error('dpkg-deb --info failed before installed qualification')
  writeNew(outPath, {
    schema: 'shellx-cut/installed-walkthrough-start@1',
    surface: 'linux-control',
    source,
    artifact: { path: artifactPath, ...artifact },
    packagePath,
    commands: [command],
  })
  console.log(`PASS Linux installed walkthrough pre-use seal: ${artifact.sha256}`)
}

async function finish() {
  if (process.platform !== 'linux') throw new Error('Linux installed walkthrough must run on native Linux')
  const pre = readJson(resolve(requiredArg('--pre')), 'pre-use receipt')
  const runtime = readJson(resolve(requiredArg('--runtime')), 'runtime receipt')
  const fullCoverage = readJson(resolve(requiredArg('--full-coverage')), 'full-coverage receipt')
  const outPath = resolve(requiredArg('--out'))
  const integrityOut = resolve(requiredArg('--integrity-out'))
  if (pre.schema !== 'shellx-cut/installed-walkthrough-start@1'
      || pre.surface !== 'linux-control') throw new Error('invalid Linux pre-use receipt')
  const source = sourceIdentity()
  if (JSON.stringify(pre.source) !== JSON.stringify(source)) throw new Error('pre-use source identity mismatch')
  const postArtifact = artifactInfo(pre.artifact.path)
  const postCommand = linuxIntegrityCommand(pre.packagePath, 'post')
  const integrity = await buildNativeIntegrityEvidence({
    source,
    surface: 'linux-control',
    artifactSha256: pre.artifact.sha256,
    preUseSha256: pre.artifact.sha256,
    postUseSha256: postArtifact.sha256,
    commands: [...pre.commands, postCommand],
  })
  const receipt = buildInstalledWalkthroughReceipt({
    source,
    surface: 'linux-control',
    artifact: { sha256: pre.artifact.sha256 },
    runtimeEvidence: runtime,
    fullCoverageReceipt: fullCoverage,
    integrityEvidence: integrity,
  })
  writeNew(integrityOut, integrity)
  writeNew(outPath, receipt)
  console.log(`PASS Linux installed walkthrough: ${outPath}`)
}

function usage() {
  console.log(`Usage:
  node scripts/linux-installed-walkthrough-receipt.mjs --start --source-commit <sha> --source-content-manifest <sha256> --artifact <installed-shell> --package <deb> --pre-out <json>
  node scripts/linux-installed-walkthrough-receipt.mjs --finish --source-commit <sha> --source-content-manifest <sha256> --pre <json> --runtime <json> --full-coverage <json> --integrity-out <json> --out <json>`)
}

async function main() {
  if (process.argv.includes('--help') || process.argv.includes('-h')) return usage()
  if (process.argv.includes('--start') === process.argv.includes('--finish')) {
    throw new Error('choose exactly one of --start or --finish')
  }
  if (process.argv.includes('--start')) start()
  else await finish()
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  main().catch((error) => {
    console.error(error?.stack || error?.message || String(error))
    process.exit(1)
  })
}
