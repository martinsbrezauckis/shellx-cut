import assert from 'node:assert/strict'
import { existsSync } from 'node:fs'
import { mkdtemp, readFile, rm } from 'node:fs/promises'
import { spawnSync } from 'node:child_process'
import { createServer } from 'node:http'
import { createRequire } from 'node:module'
import test from 'node:test'
import { tmpdir } from 'node:os'
import { extname, join, relative, resolve } from 'node:path'

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

test('manual packaging source contract selects the real Vite frontend', () => {
  const result = run(['--check'])
  assert.equal(result.status, 0, result.stderr)
  const report = JSON.parse(result.stdout)
  assert.equal(report.result, 'PASS')
  assert.equal(report.architecture, 'vite-real-frontend')
  assert.equal(report.embedUrl, '?manual=embed&mock=1')
  assert.ok(report.legacyPublicationInputs.includes('docs/public/site/manual/cut/index.html'))
})

test('manual packaging refuses to overwrite the historical canonical source', () => {
  const result = run(['--output', join(root, 'docs/public/site/manual/cut')])
  assert.notEqual(result.status, 0)
  assert.match(result.stderr, /refusing to write inside docs\/public\/site|refusing to overwrite the historical/i)
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
    assert.match(index, /(?:src|href)="\.\/assets\//)
    assert.doesNotMatch(index, /(?:src|href)="\/(?!\/)/)
    assert.doesNotMatch(index, /manual-highlight|data-manual-highlight|cut-main-editor-current/i)

    const staticSite = await startStaticServer(scratch)
    server = staticSite.server
    const { chromium } = requireFromUi('playwright')
    browser = await chromium.launch({ headless: true })
    const page = await browser.newPage({ viewport: { width: 1600, height: 1000 } })
    const pageErrors = []
    page.on('pageerror', (error) => pageErrors.push(error.message))
    await page.goto(`${staticSite.url}?feature=cut.top.manual`, { waitUntil: 'networkidle' })
    await page.locator('main[data-cut-manual-shell]').waitFor({ state: 'visible' })
    assert.equal(await page.locator('img').count(), 0, 'the publication renders a real frontend, never a screenshot')

    const editor = page.locator('[data-cut-manual-editor]')
    await editor.waitFor({ state: 'visible' })
    assert.equal(await editor.getAttribute('sandbox'), 'allow-scripts allow-same-origin')
    assert.equal(await editor.contentFrame().locator('#root > *').count() > 0, true, 'the embedded editor frontend rendered')

    await page.locator('[data-cut-manual-search]').fill('settings')
    const settings = page.locator('[data-cut-manual-feature="cut.top.settings"]')
    await settings.waitFor({ state: 'visible' })
    await settings.click()
    await editor.contentFrame().locator('[data-cut-settings-body="overview"]').waitFor({ state: 'visible' })
    await page.locator('[data-cut-manual-explanation] [role="status"]')
      .filter({ hasText: /highlighted|selected|opened/i }).waitFor({ state: 'visible' })
    assert.equal((await page.locator('[data-cut-manual-explanation] h2').textContent())?.trim(), 'Settings')
    assert.equal(await editor.contentFrame().locator('[data-cut-settings-body="overview"]').isVisible(), true, 'Settings subwindow opened in the embedded editor')
    assert.equal(await editor.contentFrame().locator('[data-cut-highlight]').count(), 1, 'the selected Settings surface is highlighted')

    await page.locator('[data-cut-manual-search]').fill('export')
    const exportFeature = page.locator('[data-cut-manual-feature="cut.top.export"]')
    await exportFeature.waitFor({ state: 'visible' })
    await exportFeature.click()
    await editor.contentFrame().locator('[data-cut-export-menu]').waitFor({ state: 'visible' })
    await page.locator('[data-cut-manual-explanation] [role="status"]')
      .filter({ hasText: /highlighted|selected|opened/i }).waitFor({ state: 'visible' })
    assert.equal((await page.locator('[data-cut-manual-explanation] h2').textContent())?.trim(), 'Export menu')
    assert.equal(await editor.contentFrame().locator('[data-cut-export-menu]').isVisible(), true, 'the passive Export menu opened in the embedded editor')
    assert.deepEqual(pageErrors, [], `publication browser errors: ${pageErrors.join(' | ')}`)
  } finally {
    await browser?.close()
    if (server) await new Promise((resolveClose, rejectClose) => server.close((error) => error ? rejectClose(error) : resolveClose()))
    await rm(scratch, { recursive: true, force: true })
  }
})
