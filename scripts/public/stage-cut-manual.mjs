#!/usr/bin/env node
// Build and atomically stage the Vite-powered Cut manual for a static host.
// This script does not publish and never writes into docs/public/site/manual.

import { randomUUID } from 'node:crypto'
import { existsSync, lstatSync, realpathSync } from 'node:fs'
import { copyFile, mkdtemp, mkdir, rename, rm, writeFile } from 'node:fs/promises'
import { spawn } from 'node:child_process'
import { tmpdir } from 'node:os'
import { basename, dirname, join, resolve } from 'node:path'
import {
  assertMatch,
  assertNoMatch,
  assertRegularFile,
  fail,
  isInside,
  legacyManualRoot,
  manualClosure,
  normalizedRelative,
  outputFiles,
  publicationManifestName,
  readText,
  readViteManifest,
  repoRoot,
  sha256,
  sourceSnapshot,
  uiRoot,
  validateSourceContract,
} from './cut-manual-publication-lib.mjs'

const viteBin = resolve(uiRoot, 'node_modules/.bin/vite')

function parseArgs(argv) {
  let output
  let check = false
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === '--output') {
      output = argv[index + 1]
      if (!output) fail('--output requires a directory ending in /manual/cut')
      index += 1
    } else if (arg === '--check') check = true
    else if (arg === '--help' || arg === '-h') {
      console.log('Usage: node scripts/public/stage-cut-manual.mjs --output <new-candidate>/manual/cut')
      console.log('       node scripts/public/stage-cut-manual.mjs --check')
      process.exit(0)
    } else fail(`unknown argument: ${arg}`)
  }
  if (check && output) fail('--check cannot be combined with --output')
  if (!check && !output) fail('--output <new-candidate>/manual/cut is required; staging never chooses a publication destination')
  return { output: output ? resolve(output) : null, check }
}

function lstatOrNull(path) {
  try {
    return lstatSync(path)
  } catch (error) {
    if (error && typeof error === 'object' && 'code' in error && error.code === 'ENOENT') return null
    throw error
  }
}

function physicalDirectory(path, label) {
  const requested = resolve(path)
  const entry = lstatOrNull(requested)
  if (!entry) fail(`${label} is missing: ${requested}`)
  if (entry.isSymbolicLink()) fail(`${label} must not be a symbolic link (including a dangling link): ${requested}`)
  if (!entry.isDirectory()) fail(`${label} must be a physical directory: ${requested}`)
  const actual = resolve(realpathSync.native(requested))
  if (actual !== requested) fail(`${label} must not traverse a symbolic-link ancestor: ${requested}`)
  return { path: requested, device: entry.dev, inode: entry.ino }
}

function assertSameDirectory(directory, label) {
  const current = physicalDirectory(directory.path, label)
  if (current.device !== directory.device || current.inode !== directory.inode) {
    fail(`${label} was replaced while staging; refusing destination lifecycle drift: ${directory.path}`)
  }
}

function assertMissingOutput(path) {
  const entry = lstatOrNull(path)
  if (!entry) return
  if (entry.isSymbolicLink()) {
    fail(`output candidate directory must be absent, not a symbolic-link or dangling-link replacement: ${path}`)
  }
  fail(`output candidate directory already exists; staging never replaces it: ${path}`)
}

async function prepareOutput(output) {
  if (basename(output) !== 'cut' || basename(dirname(output)) !== 'manual') {
    fail(`output must be the route directory .../manual/cut, got ${output}`)
  }
  if (isInside(resolve(repoRoot, 'docs/public/site'), output)) {
    fail('refusing to write inside docs/public/site; review a generated artifact before any separate publication migration')
  }
  if (isInside(legacyManualRoot, output)) fail('refusing to overwrite the historical screenshot/manual source')
  const candidateRoot = physicalDirectory(dirname(dirname(output)), 'candidate root')
  const manualRoot = dirname(output)
  const manualEntry = lstatOrNull(manualRoot)
  if (!manualEntry) await mkdir(manualRoot, { mode: 0o700 })
  const manual = physicalDirectory(manualRoot, 'manual route root')
  assertMissingOutput(output)
  const stagingDir = join(manual.path, `.${basename(output)}.staging-${process.pid}-${randomUUID()}`)
  await mkdir(stagingDir, { mode: 0o700 })
  return { candidateRoot, manual, output, staging: physicalDirectory(stagingDir, 'candidate staging directory') }
}

function assertOutputLifecycle(lifecycle) {
  assertSameDirectory(lifecycle.candidateRoot, 'candidate root')
  assertSameDirectory(lifecycle.manual, 'manual route root')
  assertSameDirectory(lifecycle.staging, 'candidate staging directory')
  assertMissingOutput(lifecycle.output)
}

async function discardStaging(lifecycle) {
  try {
    assertSameDirectory(lifecycle.candidateRoot, 'candidate root')
    assertSameDirectory(lifecycle.manual, 'manual route root')
    assertSameDirectory(lifecycle.staging, 'candidate staging directory')
    await rm(lifecycle.staging.path, { recursive: true, force: false })
  } catch {
    // A changed parent is deliberately retained for operator inspection.
  }
}

function run(command, args, options) {
  return new Promise((resolveRun, rejectRun) => {
    const child = spawn(command, args, options)
    child.once('error', rejectRun)
    child.once('exit', (code, signal) => {
      if (code === 0) resolveRun()
      else rejectRun(new Error(`${basename(command)} failed (${signal ? `signal ${signal}` : `exit ${code}`})`))
    })
  })
}

async function runVite(buildDir) {
  if (!existsSync(viteBin)) {
    fail('local Vite is unavailable at ui/node_modules; run npm ci in ui on the designated build host before staging')
  }
  await run(viteBin, ['build', '--base=./', '--outDir', buildDir, '--emptyOutDir', '--manifest'], {
    cwd: uiRoot,
    env: { ...process.env, SHELLX_CUT_SOURCEMAPS: '0' },
    stdio: 'inherit',
  })
}

async function copyManualArtifact(buildDir, stagingDir) {
  const files = manualClosure(await readViteManifest(buildDir), buildDir)
  const manualHtml = resolve(buildDir, 'manual.html')
  assertRegularFile(manualHtml, 'built manual HTML')
  await copyFile(manualHtml, join(stagingDir, 'index.html'))
  for (const source of files) {
    const rel = normalizedRelative(buildDir, source)
    const destination = resolve(stagingDir, rel)
    if (!isInside(stagingDir, destination)) fail(`manual artifact escaped staging: ${rel}`)
    await mkdir(dirname(destination), { recursive: true })
    await copyFile(source, destination)
  }
}

async function validateArtifactContract(stagingDir, sourceContract) {
  const html = readText(join(stagingDir, 'index.html'), 'built manual HTML')
  assertMatch(html, /<div id="root"><\/div>/, 'built manual must mount the Vite frontend root')
  assertMatch(html, /(?:src|href)="\.\/assets\//, 'built manual must use relative static asset URLs')
  assertNoMatch(html, /(?:src|href)="\/(?!\/)/, 'built manual must not use root-relative asset URLs')
  assertNoMatch(html, /\/src\/main\.tsx/, 'built manual must not ship a Vite source-module URL')
  assertNoMatch(html, /data-manual-highlight|manual-highlight|cut-main-editor-current|cut-recording-studio-current/i, 'built manual must reject the legacy screenshot/hotspot architecture')
  assertNoMatch(html, /<img\b/i, 'built manual must not be a screenshot manual')

  const artifacts = await outputFiles(stagingDir)
  if (!artifacts.some((artifact) => artifact.path.startsWith('assets/'))) fail('built manual has no Vite asset closure')
  for (const artifact of artifacts) {
    if (artifact.path !== 'index.html' && !artifact.path.startsWith('assets/')) {
      fail(`manual artifact is outside the static asset closure: ${artifact.path}`)
    }
  }
  assertNoMatch(JSON.stringify(artifacts), /(?:^|[\\/])(?:private|\.git)(?:[\\/]|$)|release-studio/i, 'built manual must not include private or control-plane paths')
  return { ...sourceContract, artifacts }
}

function candidateIdentity(source, artifacts) {
  const artifactClosureSha256 = sha256(artifacts
    .map((artifact) => `${artifact.path}\0${artifact.sha256}\n`)
    .sort()
    .join(''))
  const candidateSha256 = sha256([
    source.gitHead,
    source.uiInputTreeSha256,
    artifactClosureSha256,
  ].join('\0'))
  return {
    schema: 'shellx-cut/manual-candidate-identity@1',
    candidateId: `cut-manual-${candidateSha256.slice(0, 20)}`,
    sourceGitHead: source.gitHead,
    uiInputTreeSha256: source.uiInputTreeSha256,
    artifactClosureSha256,
  }
}

async function viteVersion() {
  const pkg = JSON.parse(readText(resolve(uiRoot, 'node_modules/vite/package.json'), 'installed Vite package metadata'))
  if (typeof pkg.version !== 'string') fail('installed Vite package has no version')
  return pkg.version
}

async function gitHead() {
  return new Promise((resolveHead) => {
    const child = spawn('git', ['-C', repoRoot, 'rev-parse', 'HEAD'], { stdio: ['ignore', 'pipe', 'ignore'] })
    let stdout = ''
    child.stdout.on('data', (chunk) => { stdout += chunk })
    child.once('error', () => resolveHead(null))
    child.once('exit', (code) => resolveHead(code === 0 && /^[0-9a-f]{40}\s*$/.test(stdout) ? stdout.trim() : null))
  })
}

async function stage(output) {
  const sourceContract = validateSourceContract()
  const before = await sourceSnapshot()
  let buildDir = null
  let lifecycle = null
  let promoted = false
  try {
    lifecycle = await prepareOutput(output)
    buildDir = await mkdtemp(join(tmpdir(), 'shellx-cut-manual-vite-'))
    await runVite(buildDir)
    const after = await sourceSnapshot()
    if (before.sha256 !== after.sha256) fail('UI build inputs changed while Vite was running; artifact was not staged')
    await copyManualArtifact(buildDir, lifecycle.staging.path)
    const contract = await validateArtifactContract(lifecycle.staging.path, sourceContract)
    const head = await gitHead()
    if (!head) fail('unable to resolve the local Git source identity for this candidate')
    const manifest = {
      schema: 'shellx-cut/manual-publication@1',
      route: '/manual/cut/',
      architecture: contract.architecture,
      authority: 'ui/manual.html via Vite manifest closure',
      embedUrl: contract.embedUrl,
      readOnly: true,
      relativeAssetBase: './',
      legacyScreenshotHotspotAuthority: 'rejected',
      excludedLegacyInputs: contract.legacyPublicationInputs,
      source: {
        gitHead: head,
        uiInputFileCount: before.fileCount,
        uiInputTreeSha256: before.sha256,
        directInputs: before.directInputs,
        viteVersion: await viteVersion(),
      },
      artifacts: contract.artifacts,
    }
    manifest.identity = candidateIdentity(manifest.source, manifest.artifacts)
    await writeFile(join(lifecycle.staging.path, publicationManifestName), `${JSON.stringify(manifest, null, 2)}\n`, 'utf8')
    assertOutputLifecycle(lifecycle)
    await rename(lifecycle.staging.path, lifecycle.output)
    promoted = true
    physicalDirectory(lifecycle.output, 'published candidate directory')
    return manifest
  } finally {
    if (buildDir) await rm(buildDir, { recursive: true, force: true })
    if (lifecycle && !promoted) await discardStaging(lifecycle)
  }
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  const sourceContract = validateSourceContract()
  if (options.check) {
    console.log(JSON.stringify({ result: 'PASS', ...sourceContract }, null, 2))
    return
  }
  const manifest = await stage(options.output)
  console.log(JSON.stringify({
    result: 'PASS',
    output: options.output,
    manifest: join(options.output, publicationManifestName),
    route: manifest.route,
    candidateId: manifest.identity.candidateId,
    artifactCount: manifest.artifacts.length,
  }, null, 2))
}

main().catch((error) => {
  process.stderr.write(`${error instanceof Error ? error.message : String(error)}\n`)
  process.exitCode = 1
})
