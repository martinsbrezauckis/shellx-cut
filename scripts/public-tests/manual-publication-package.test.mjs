import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { existsSync } from 'node:fs'
import { mkdir, mkdtemp, readFile, readdir, rm, symlink } from 'node:fs/promises'
import { spawnSync } from 'node:child_process'
import { createServer } from 'node:http'
import { createRequire } from 'node:module'
import test from 'node:test'
import { tmpdir } from 'node:os'
import { dirname, extname, join, relative, resolve } from 'node:path'
import { sealedPlaywrightChromiumLaunchOptions } from '../../ui/public-tests/lib/sealedPlaywrightChromium.mjs'

const root = resolve(new URL('../..', import.meta.url).pathname)
const stage = resolve(root, 'scripts/public/stage-cut-manual.mjs')
const vite = resolve(root, 'ui/node_modules/.bin/vite')
const playwrightPackage = resolve(root, 'ui/node_modules/playwright/package.json')
const requireFromUi = createRequire(resolve(root, 'ui/package.json'))

const contentTypes = {
  '.css': 'text/css; charset=utf-8',
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
}

async function startStaticServer(siteRoot) {
  const server = createServer(async (request, response) => {
    try {
      const pathname = decodeURIComponent(new URL(request.url || '/', 'http://manual.local').pathname)
      const relativePath = pathname.endsWith('/') ? `${pathname}index.html` : pathname
      const target = resolve(siteRoot, `.${relativePath}`)
      const escaped = relative(siteRoot, target)
      if (escaped === '..' || escaped.startsWith('../')) {
        response.writeHead(403)
        response.end('forbidden')
        return
      }
      const bytes = await readFile(target)
      response.writeHead(200, { 'content-type': contentTypes[extname(target)] || 'application/octet-stream' })
      response.end(bytes)
    } catch {
      response.writeHead(404)
      response.end('not found')
    }
  })
  await new Promise((resolveListen, rejectListen) => {
    server.once('error', rejectListen)
    server.listen(0, '127.0.0.1', resolveListen)
  })
  const address = server.address()
  assert.ok(address && typeof address !== 'string')
  return { server, url: `http://127.0.0.1:${address.port}/manual/cut/` }
}

function run(args) {
  return spawnSync(process.execPath, [stage, ...args], {
    cwd: root,
    encoding: 'utf8',
    timeout: 120_000,
  })
}

async function outputPaths(root, prefix = '') {
  const entries = await readdir(join(root, prefix), { withFileTypes: true })
  const paths = []
  for (const entry of entries.sort((left, right) => left.name.localeCompare(right.name))) {
    const relativePath = join(prefix, entry.name).replaceAll('\\', '/')
    if (entry.isDirectory()) paths.push(...await outputPaths(root, relativePath))
    else if (entry.isFile()) paths.push(relativePath)
    else assert.fail(`candidate output contains a non-regular entry: ${relativePath}`)
  }
  return paths
}

function sha256(value) {
  return createHash('sha256').update(value).digest('hex')
}

async function viteBuildTempRoots() {
  return (await readdir(tmpdir(), { withFileTypes: true }))
    .filter((entry) => entry.isDirectory() && entry.name.startsWith('shellx-cut-manual-vite-'))
    .map((entry) => entry.name)
    .sort((left, right) => left.localeCompare(right))
}

test('manual packaging source contract selects the real Vite frontend', () => {
  const result = run(['--check'])
  assert.equal(result.status, 0, result.stderr)
  const report = JSON.parse(result.stdout)
  assert.equal(report.result, 'PASS')
  assert.equal(report.architecture, 'vite-real-frontend')
  assert.equal(report.embedUrl, '?manual=embed&mock=1')
  assert.ok(report.legacyPublicationInputs.includes('docs/public/site/manual/cut/index.html'))
})

test('manual packaging requires one explicit new candidate destination', () => {
  const result = run([])
  assert.notEqual(result.status, 0)
  assert.match(result.stderr, /--output <new-candidate>\/manual\/cut is required/i)
})

test('manual packaging refuses to overwrite the historical canonical source', () => {
  const result = run(['--output', join(root, 'docs/public/site/manual/cut')])
  assert.notEqual(result.status, 0)
  assert.match(result.stderr, /refusing to write inside docs\/public\/site|refusing to overwrite the historical/i)
})

test('manual packaging refuses output replacement and symbolic-link destination paths', async () => {
  const scratch = await mkdtemp(join(tmpdir(), 'shellx-cut-manual-destination-test-'))
  try {
    const existingOutput = join(scratch, 'existing', 'manual', 'cut')
    await mkdir(existingOutput, { recursive: true })
    const buildRootsBeforeExistingDestination = await viteBuildTempRoots()
    const existingResult = run(['--output', existingOutput])
    assert.notEqual(existingResult.status, 0)
    assert.match(existingResult.stderr, /output candidate directory already exists/i)
    assert.deepEqual(await viteBuildTempRoots(), buildRootsBeforeExistingDestination, 'existing destinations leave no new Vite build temporary root')

    const danglingManualRoot = join(scratch, 'dangling-parent', 'manual')
    await mkdir(dirname(danglingManualRoot), { recursive: true })
    await symlink('missing-manual-root', danglingManualRoot)
    const buildRootsBeforeSymlinkDestination = await viteBuildTempRoots()
    const danglingParentResult = run(['--output', join(danglingManualRoot, 'cut')])
    assert.notEqual(danglingParentResult.status, 0)
    assert.match(danglingParentResult.stderr, /manual route root must not be a symbolic link/i)
    assert.deepEqual(await viteBuildTempRoots(), buildRootsBeforeSymlinkDestination, 'symbolic-link destinations leave no new Vite build temporary root')

    const danglingOutput = join(scratch, 'dangling-output', 'manual', 'cut')
    await mkdir(dirname(danglingOutput), { recursive: true })
    await symlink('missing-candidate-root', danglingOutput)
    const danglingOutputResult = run(['--output', danglingOutput])
    assert.notEqual(danglingOutputResult.status, 0)
    assert.match(danglingOutputResult.stderr, /output candidate directory must be absent, not a symbolic-link or dangling-link replacement/i)
  } finally {
    await rm(scratch, { recursive: true, force: true })
  }
})

test('manual packaging writes an interactive Vite closure at the /manual/cut route', {
  skip: process.env.SHELLX_CUT_RUN_VITE_MANUAL_PACKAGE_TEST === '1' && existsSync(vite) && existsSync(playwrightPackage)
    ? false
    : 'set SHELLX_CUT_RUN_VITE_MANUAL_PACKAGE_TEST=1 on a designated build host with Vite and Playwright installed',
  timeout: 180_000,
}, async () => {
  const scratch = await mkdtemp(join(tmpdir(), 'shellx-cut-manual-package-test-'))
  const output = join(scratch, 'manual', 'cut')
  let browser
  let server
  try {
    const result = run(['--output', output])
    assert.equal(result.status, 0, result.stderr)
    const manifest = JSON.parse(await readFile(join(output, 'publication-manifest.json'), 'utf8'))
    const index = await readFile(join(output, 'index.html'), 'utf8')
    assert.equal(manifest.route, '/manual/cut/')
    assert.equal(manifest.architecture, 'vite-real-frontend')
    assert.equal(manifest.legacyScreenshotHotspotAuthority, 'rejected')
    assert.ok(manifest.artifacts.some((artifact) => artifact.path.startsWith('assets/')))
    assert.equal(manifest.identity.schema, 'shellx-cut/manual-candidate-identity@1')
    const closure = manifest.artifacts
      .map((artifact) => `${artifact.path}\0${artifact.sha256}\n`)
      .sort()
      .join('')
    assert.equal(manifest.identity.artifactClosureSha256, sha256(closure), 'candidate identity binds the exact manifest closure')
    assert.equal(
      manifest.identity.candidateId,
      `cut-manual-${sha256([manifest.source.gitHead, manifest.source.uiInputTreeSha256, manifest.identity.artifactClosureSha256].join('\0')).slice(0, 20)}`,
      'candidate identity is deterministic from source and artifact identities',
    )
    assert.deepEqual(
      await outputPaths(output),
      [...manifest.artifacts.map((artifact) => artifact.path), 'publication-manifest.json'].sort((left, right) => left.localeCompare(right)),
      'candidate emits only the manifest closure and its identity manifest',
    )
    assert.ok(manifest.artifacts.every((artifact) => artifact.path === 'index.html' || artifact.path.startsWith('assets/')))
    assert.ok(manifest.source.directInputs.every((input) => !/^(?:private|\.git)\//.test(input.path)))
    assert.match(index, /(?:src|href)="\.\/assets\//)
    assert.doesNotMatch(index, /(?:src|href)="\/(?!\/)/)
    assert.doesNotMatch(index, /manual-highlight|data-manual-highlight|cut-main-editor-current/i)

    const staticSite = await startStaticServer(scratch)
    server = staticSite.server
    const { chromium } = requireFromUi('playwright')
    browser = await chromium.launch({ headless: true, ...sealedPlaywrightChromiumLaunchOptions() })
    const page = await browser.newPage({ viewport: { width: 1600, height: 1000 } })
    const pageErrors = []
    const consoleErrors = []
    page.on('pageerror', (error) => pageErrors.push(error.message))
    page.on('console', (message) => {
      if (message.type() === 'error') consoleErrors.push(message.text())
    })
    await page.goto(`${staticSite.url}?feature=cut.top.settings`, { waitUntil: 'networkidle' })
    await page.locator('main[data-cut-manual-shell]').waitFor({ state: 'visible' })
    assert.equal(await page.locator('img').count(), 0, 'the publication renders a real frontend, never a screenshot')

    const editor = page.locator('[data-cut-manual-editor]')
    await editor.waitFor({ state: 'visible' })
    assert.equal(await editor.getAttribute('sandbox'), 'allow-scripts allow-same-origin')
    const embedded = editor.contentFrame()
    assert.equal(await embedded.locator('#root > *').count() > 0, true, 'the embedded editor frontend rendered')
    await embedded.locator('[data-cut-settings-body="overview"]').waitFor({ state: 'visible' })
    assert.equal((await page.locator('[data-cut-manual-explanation] h2').textContent())?.trim(), 'Settings and editing cache')
    assert.equal(new URL(page.url()).searchParams.get('feature'), 'cut.top.settings', 'established query deep links select and reveal the real subwindow')

    await embedded.locator('[data-cut-environment-close]').click()
    await embedded.locator('[data-cut-environment-scrim]').waitFor({ state: 'detached' })
    await embedded.locator('[data-cut-export-btn]').click()
    await embedded.locator('[data-cut-export-menu]').waitFor({ state: 'visible' })
    await page.locator('[data-cut-manual-explanation] h2').filter({ hasText: 'Export menu' }).waitFor({ state: 'visible' })
    await page.waitForFunction(() => window.location.hash === '#cut.top.export')
    assert.equal((await page.locator('[data-cut-manual-explanation] h2').textContent())?.trim(), 'Export menu')
    assert.equal(await embedded.locator('[data-cut-export-menu]').isVisible(), true, 'a real embedded menu click selects its matching explanation')

    await embedded.locator('[data-cut-settings-btn]').click()
    await embedded.locator('[data-cut-settings-body="overview"]').waitFor({ state: 'visible' })
    await page.locator('[data-cut-manual-explanation] h2').filter({ hasText: 'Settings' }).waitFor({ state: 'visible' })
    await page.waitForFunction(() => window.location.hash === '#cut.top.settings')
    assert.equal(await embedded.locator('[data-cut-highlight]').count(), 1, 'the embedded subwindow route retains the real highlight overlay')

    const readOnly = await embedded.locator('body').evaluate(async () => {
      const verb = async (name) => fetch(`/api/verb/${name}`, { method: 'POST', body: '{}' }).then((response) => response.json())
      const before = await verb('project.state')
      const mutation = await verb('project.delete')
      const after = await verb('project.state')
      return { before: before.result, mutation, after: after.result }
    })
    assert.equal(readOnly.mutation.ok, false, 'embedded manual refuses a project mutation')
    assert.equal(readOnly.mutation.error?.code, 'manual_read_only')
    assert.deepEqual(readOnly.after, readOnly.before, 'read-only refusal leaves the embedded project byte-equivalent')
    assert.deepEqual(pageErrors, [], `publication browser errors: ${pageErrors.join(' | ')}`)
    assert.deepEqual(consoleErrors, [], `publication browser console errors: ${consoleErrors.join(' | ')}`)
  } finally {
    await browser?.close()
    if (server) await new Promise((resolveClose, rejectClose) => server.close((error) => error ? rejectClose(error) : resolveClose()))
    await rm(scratch, { recursive: true, force: true })
  }
})
