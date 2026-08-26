#!/usr/bin/env node
// Build and atomically stage the Vite-powered Cut manual for a static host.
// This script does not publish and never writes into docs/public/site/manual.

import { randomUUID } from 'node:crypto'
import { existsSync } from 'node:fs'
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
  sourceSnapshot,
  uiRoot,
  validateSourceContract,
} from './cut-manual-publication-lib.mjs'

const viteBin = resolve(uiRoot, 'node_modules/.bin/vite')

function parseArgs(argv) {
  let output
  let scratch = false
  let check = false
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (arg === '--output') {
      output = argv[index + 1]
      if (!output) fail('--output requires a directory ending in /manual/cut')
      index += 1
    } else if (arg === '--scratch') scratch = true
    else if (arg === '--check') check = true
    else if (arg === '--help' || arg === '-h') {
      console.log('Usage: node scripts/public/stage-cut-manual.mjs [--check] [--scratch | --output <dir>/manual/cut]')
      process.exit(0)
    } else fail(`unknown argument: ${arg}`)
  }
  if (check && (scratch || output)) fail('--check cannot be combined with an output option')
  if (scratch && output) fail('choose either --scratch or --output, not both')
  return { output, check }
}

async function resolveOutput({ output }) {
  if (output) return { output: resolve(output), scratchRoot: null }
  const scratchRoot = await mkdtemp(join(tmpdir(), 'shellx-cut-manual-'))
  return { output: join(scratchRoot, 'manual', 'cut'), scratchRoot }
}

function validateOutputPath(output) {
  if (basename(output) !== 'cut' || basename(dirname(output)) !== 'manual') {
    fail(`output must be the route directory .../manual/cut, got ${output}`)
  }
  if (isInside(resolve(repoRoot, 'docs/public/site'), output)) {
    fail('refusing to write inside docs/public/site; review a generated artifact before any separate publication migration')
  }
  if (isInside(legacyManualRoot, output)) fail('refusing to overwrite the historical screenshot/manual source')
  if (existsSync(output)) fail(`output already exists; staging never overwrites it: ${output}`)
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
  await mkdir(stagingDir, { recursive: true })
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
  return { ...sourceContract, artifacts }
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
  validateOutputPath(output)
  const sourceContract = validateSourceContract()
  const before = await sourceSnapshot()
  const buildDir = await mkdtemp(join(tmpdir(), 'shellx-cut-manual-vite-'))
  const stagingDir = join(dirname(output), `.${basename(output)}.staging-${process.pid}-${randomUUID()}`)
  let staged = false
  try {
    await mkdir(dirname(output), { recursive: true })
    await runVite(buildDir)
    const after = await sourceSnapshot()
    if (before.sha256 !== after.sha256) fail('UI build inputs changed while Vite was running; artifact was not staged')
    await copyManualArtifact(buildDir, stagingDir)
    const contract = await validateArtifactContract(stagingDir, sourceContract)
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
        gitHead: await gitHead(),
        uiInputFileCount: before.fileCount,
        uiInputTreeSha256: before.sha256,
        directInputs: before.directInputs,
        viteVersion: await viteVersion(),
      },
      artifacts: contract.artifacts,
    }
    await writeFile(join(stagingDir, publicationManifestName), `${JSON.stringify(manifest, null, 2)}\n`, 'utf8')
    await rename(stagingDir, output)
    staged = true
    return manifest
  } finally {
    await rm(buildDir, { recursive: true, force: true })
    if (!staged) await rm(stagingDir, { recursive: true, force: true })
  }
}

async function main() {
  const options = parseArgs(process.argv.slice(2))
  const sourceContract = validateSourceContract()
  if (options.check) {
    console.log(JSON.stringify({ result: 'PASS', ...sourceContract }, null, 2))
    return
  }
  const destination = await resolveOutput(options)
  let completed = false
  try {
    const manifest = await stage(destination.output)
    completed = true
    console.log(JSON.stringify({
      result: 'PASS',
      output: destination.output,
      scratchRoot: destination.scratchRoot,
      manifest: join(destination.output, publicationManifestName),
      route: manifest.route,
      artifactCount: manifest.artifacts.length,
    }, null, 2))
  } finally {
    if (!completed && destination.scratchRoot) await rm(destination.scratchRoot, { recursive: true, force: true })
  }
}

main().catch((error) => {
  process.stderr.write(`${error instanceof Error ? error.message : String(error)}\n`)
  process.exitCode = 1
})
