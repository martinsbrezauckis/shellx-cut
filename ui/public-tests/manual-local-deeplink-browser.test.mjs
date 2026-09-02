import assert from 'node:assert/strict'
import { existsSync } from 'node:fs'
import { readFile } from 'node:fs/promises'
import { createServer } from 'node:http'
import { createRequire } from 'node:module'
import test from 'node:test'
import { extname, join, relative, resolve } from 'node:path'

const uiRoot = resolve(new URL('..', import.meta.url).pathname)
const distRoot = resolve(uiRoot, 'dist')
const requireFromUi = createRequire(resolve(uiRoot, 'package.json'))
const articleIds = [
  'cut.preview.ffmpeg_setup',
  'cut.export.preflight',
  'cut.left.media_health',
]

const contentTypes = {
  '.css': 'text/css; charset=utf-8',
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
}

async function startStaticUi() {
  const server = createServer(async (request, response) => {
    try {
      const pathname = decodeURIComponent(new URL(request.url || '/', 'http://cut.local').pathname)
      const path = pathname.endsWith('/') ? `${pathname}index.html` : pathname
      const target = resolve(distRoot, `.${path}`)
      const escaped = relative(distRoot, target)
      if (escaped === '..' || escaped.startsWith('../')) throw new Error('outside dist')
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
  assert.ok(address && typeof address !== 'string', 'static UI server has a TCP address')
  return { server, url: `http://127.0.0.1:${address.port}/?mock=1` }
}

test('built mock editor opens contextual manual articles locally without revealing Cut', {
  skip: process.env.SHELLX_CUT_RUN_LOCAL_MANUAL_BROWSER_TEST === '1'
    ? false
    : 'set SHELLX_CUT_RUN_LOCAL_MANUAL_BROWSER_TEST=1 on a browser-capable UI build host',
  timeout: 120_000,
}, async () => {
  assert.equal(existsSync(join(distRoot, 'index.html')), true, 'build the UI before the local-manual browser check')
  const { chromium } = requireFromUi('playwright')
  const { server, url } = await startStaticUi()
  const browser = await chromium.launch({ headless: true })
  const page = await browser.newPage({ viewport: { width: 1440, height: 980 } })
  const pageErrors = []
  page.on('pageerror', (error) => pageErrors.push(error.message))

  try {
    await page.goto(url, { waitUntil: 'networkidle' })
    await page.locator('[data-cut-manual-link]').waitFor({ state: 'visible' })
    await page.evaluate(() => {
      window.__cutManualExternalOpenCalls = []
      window.open = (...args) => {
        window.__cutManualExternalOpenCalls.push(args.map((value) => String(value ?? '')))
        return null
      }
    })

    for (const featureId of articleIds) {
      await page.evaluate((requestedFeatureId) => {
        document.dispatchEvent(new CustomEvent('cut:open-manual', {
          detail: { feature: requestedFeatureId },
        }))
      }, featureId)
      const panel = page.locator('[data-cut-local-manual]').first()
      const article = page.locator(`[data-cut-local-manual-feature="${featureId}"]`).first()
      await article.waitFor({ state: 'visible' })
      assert.equal(await article.getAttribute('aria-current'), 'page', `${featureId} is selected in the local panel`)
      assert.equal(await page.locator('[data-cut-highlight]').count(), 0, `${featureId} does not reveal Cut before Show in Cut`)
      assert.equal(await page.locator('[data-cut-environment]').count(), 0, `${featureId} does not open an editor panel before Show in Cut`)
      await page.locator('[data-cut-local-manual-close]').first().click()
      await panel.waitFor({ state: 'detached' })
    }

    await page.locator('[data-cut-manual-link]').first().click()
    await page.locator('[data-cut-local-manual-feature="cut.top.settings"]').first().click()
    const renderedPanel = await page.screenshot({ fullPage: true })
    assert.deepEqual([...renderedPanel.subarray(0, 8)], [137, 80, 78, 71, 13, 10, 26, 10], 'the local manual rendered a PNG-readable editor surface')
    await page.locator('[data-cut-local-manual-reveal]').first().click()
    await page.locator('[data-cut-settings-body="overview"]').first().waitFor({ state: 'visible' })
    await page.locator('[data-cut-highlight]').first().waitFor({ state: 'visible' })
    assert.deepEqual(await page.evaluate(() => window.__cutManualExternalOpenCalls), [], 'no local-manual route opens a browser')
    assert.deepEqual(pageErrors, [], `mock editor had no page errors: ${pageErrors.join(' | ')}`)
  } finally {
    await browser.close()
    await new Promise((resolveClose, rejectClose) => server.close((error) => error ? rejectClose(error) : resolveClose()))
  }
})
