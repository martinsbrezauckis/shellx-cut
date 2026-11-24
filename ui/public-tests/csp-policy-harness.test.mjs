import assert from 'node:assert/strict'
import { createServer } from 'node:http'
import { existsSync } from 'node:fs'
import { readFile } from 'node:fs/promises'
import { dirname, extname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import test from 'node:test'
import { chromium } from 'playwright'
import {
  assertCspPolicyTight,
  attachPageCspGuard,
  buildHarnessCspHeader,
  computeInlineScriptHashes,
  isViteDevFixtureHtml,
  readProductCspPolicy,
  stripMetaUnsupportedDirectives,
} from './lib/cspHarness.mjs'
import { sealedPlaywrightChromiumLaunchOptions } from './lib/sealedPlaywrightChromium.mjs'

const UI_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const REPO_ROOT = resolve(UI_ROOT, '..')
const MIME = {
  '.css': 'text/css; charset=utf-8',
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.json': 'application/json; charset=utf-8',
  '.svg': 'image/svg+xml',
}

/** Poll until the guard has seen a violation (up to 15 s: a loaded box can report a blocked frame late). */
async function waitForViolation(guard, timeoutMs = 15_000) {
  const deadline = Date.now() + timeoutMs
  while (guard.violations.length === 0 && Date.now() < deadline) {
    await new Promise((done) => setTimeout(done, 50))
  }
}

test('desktop tauri.conf.json CSP and cutd UI_CSP are tight, aligned, and cover all desktop IPC schemes', () => {
  const { desktopCsp, httpCsp } = readProductCspPolicy(REPO_ROOT)
  assertCspPolicyTight(desktopCsp, { requireFrameAncestors: false })
  assertCspPolicyTight(httpCsp, { requireFrameAncestors: true })
  assert.equal(
    desktopCsp,
    stripMetaUnsupportedDirectives(httpCsp),
    'tauri.conf.json CSP should stay aligned with cutd UI_CSP (without frame-ancestors, which <meta> rejects)',
  )

  const viteHtml = '<body style="margin:0"><script type="module">import RefreshRuntime from "/@react-refresh";</script><script type="module" src="/__fixture.js"></script></body>'
  assert.equal(isViteDevFixtureHtml(viteHtml), true)
  const hashes = computeInlineScriptHashes(viteHtml)
  assert.equal(hashes.length, 1)
  const viteCsp = buildHarnessCspHeader(httpCsp, viteHtml)
  assert.ok(viteCsp.includes(hashes[0]), 'Vite harness header must hash only the fixture inline preamble script')
  assert.ok(!/script-src[^;]*unsafe-inline/.test(viteCsp), 'Vite harness must never allow unsafe-inline scripts')
  assert.ok(viteCsp.includes("style-src 'self' 'unsafe-inline'"), 'Vite dev harness adapts dev style injection')
  assert.ok(viteCsp.includes("worker-src 'self' blob:"), 'Vite dev harness allows only its own blob: ping worker')
  assert.ok(httpCsp.includes("worker-src 'none'"), 'the product policy keeps worker-src none')

  const builtHtml = '<script type="module" crossorigin src="/assets/index-abc.js"></script><link rel="stylesheet" href="/assets/index-abc.css">'
  assert.equal(isViteDevFixtureHtml(builtHtml), false)
  assert.equal(buildHarnessCspHeader(httpCsp, builtHtml), httpCsp, 'built production HTML keeps exact UI_CSP unmodified')
})

test('CSP harness rejects securitypolicyviolation in Chromium and built ui/dist + /manual.html iframe pass cleanly', { timeout: 120_000 }, async (t) => {
  const { desktopCsp, httpCsp } = readProductCspPolicy(REPO_ROOT)
  assertCspPolicyTight(desktopCsp, { requireFrameAncestors: false })
  assertCspPolicyTight(httpCsp, { requireFrameAncestors: true })

  const distRoot = resolve(UI_ROOT, 'dist')
  const server = createServer(async (req, res) => {
    const pathname = new URL(req.url || '/', 'http://127.0.0.1').pathname
    if (pathname === '/bad-csp') {
      res.writeHead(200, { 'content-type': 'text/html; charset=utf-8', 'content-security-policy': httpCsp })
      res.end('<!doctype html><html><body><img src="https://example.invalid/tracker.png"></body></html>')
      return
    }
    if (pathname === '/frame-host') {
      res.writeHead(200, { 'content-type': 'text/html; charset=utf-8', 'content-security-policy': httpCsp })
      res.end('<!doctype html><html><body><iframe src="/frame-child-none"></iframe></body></html>')
      return
    }
    if (pathname === '/frame-child-none') {
      const legacyNoneCsp = httpCsp.replace("frame-ancestors 'self'", "frame-ancestors 'none'")
      res.writeHead(200, { 'content-type': 'text/html; charset=utf-8', 'content-security-policy': legacyNoneCsp })
      res.end('<!doctype html><html><body>blocked child</body></html>')
      return
    }
    if (pathname === '/fallback') {
      res.writeHead(200, { 'content-type': 'text/html; charset=utf-8' })
      res.end(await readFile(resolve(REPO_ROOT, 'app/desktop/fallback/index.html'), 'utf8'))
      return
    }
    const target = existsSync(resolve(distRoot, 'manual.html'))
      ? resolve(distRoot, `.${pathname === '/' ? '/index.html' : pathname}`)
      : resolve(REPO_ROOT, 'app/desktop/fallback/index.html')
    try {
      const body = await readFile(target)
      res.writeHead(200, {
        'content-type': MIME[extname(target)] || 'application/octet-stream',
        'content-security-policy': httpCsp,
      })
      res.end(body)
    } catch {
      res.writeHead(404)
      res.end('not found')
    }
  })
  await new Promise((done) => server.listen(0, '127.0.0.1', done))
  t.after(() => server.close())
  const { port } = server.address()
  const base = `http://127.0.0.1:${port}`

  // Red/negative proof: a blocked remote image and legacy `frame-ancestors 'none'` must trigger CSP violations.
  const prevAllow = process.env.SHELLX_CUT_CSP_ALLOW_EXPECTED
  process.env.SHELLX_CUT_CSP_ALLOW_EXPECTED = '1'
  const badBrowser = await chromium.launch({ headless: true, ...sealedPlaywrightChromiumLaunchOptions() })
  try {
    const badPage = await badBrowser.newPage()
    const badGuard = await attachPageCspGuard(badPage, {
      repoRoot: REPO_ROOT,
      enforceHeaderOnUnprotectedHtml: false,
      cspOverride: httpCsp,
    })
    await badPage.goto(`${base}/bad-csp`, { waitUntil: 'load' })
    await badPage.waitForFunction(() => (window.__cutCspViolations?.length || 0) > 0)
    await waitForViolation(badGuard)
    assert.throws(() => badGuard.assertClean('negative-csp'), /CSP violation/)
    badGuard.drain()

    await badPage.goto(`${base}/frame-host`, { waitUntil: 'load' })
    await waitForViolation(badGuard)
    assert.throws(() => badGuard.assertClean('legacy-frame-ancestors-none'), /CSP violation/)
    badGuard.drain()
    await badPage.close()
  } finally {
    await badBrowser.close()
    if (prevAllow === undefined) delete process.env.SHELLX_CUT_CSP_ALLOW_EXPECTED
    else process.env.SHELLX_CUT_CSP_ALLOW_EXPECTED = prevAllow
  }

  // Built manual + same-origin embedded live iframe (/manual.html?manual=embed&mock=1) when dist is built.
  if (existsSync(resolve(distRoot, 'manual.html'))) {
    const manualBrowser = await chromium.launch({ headless: true, ...sealedPlaywrightChromiumLaunchOptions() })
    t.after(() => manualBrowser.close())
    const manualPage = await manualBrowser.newPage({ viewport: { width: 1280, height: 800 } })
    const manualGuard = await attachPageCspGuard(manualPage, {
      repoRoot: REPO_ROOT,
      enforceHeaderOnUnprotectedHtml: false,
      cspOverride: httpCsp,
    })
    await manualPage.goto(`${base}/manual.html?mock=1`, { waitUntil: 'networkidle' })
    await manualPage.waitForSelector('iframe[data-cut-manual-editor]')
    await manualPage.frameLocator('iframe[data-cut-manual-editor]').locator('body *').first().waitFor()
    manualGuard.assertClean('manual-with-iframe')
    await manualPage.close()
  }
})
