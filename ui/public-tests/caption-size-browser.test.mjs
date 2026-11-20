import assert from 'node:assert/strict'
import { fileURLToPath } from 'node:url'
import test from 'node:test'
import { chromium } from 'playwright'
import { createServer } from 'vite'
import react from '@vitejs/plugin-react'

test('caption size, hex and preset colors send their selected styles through the visible controls', async t => {
  const server = await createServer({
    root: fileURLToPath(new URL('..', import.meta.url)), configFile: false, plugins: [react(), {
      name: 'caption-size-fixture',
      configureServer(vite) {
        vite.middlewares.use('/__caption_size_fixture__', (_request, response) => {
          response.setHeader('content-type', 'text/html')
          response.end(`<div id="root"></div><script type="module">
            import RefreshRuntime from '/@react-refresh'
            RefreshRuntime.injectIntoGlobalHook(window)
            window.$RefreshReg$ = () => {}
            window.$RefreshSig$ = () => type => type
            window.__vite_plugin_react_preamble_installed__ = true
          </script><script type="module" src="/public-tests/fixtures/caption-size-browser.tsx"></script>`)
        })
      },
    }],
    server: { host: '127.0.0.1', port: 0 },
  })
  t.after(() => server.close())
  await server.listen()
  const browser = await chromium.launch({ headless: true })
  t.after(() => browser.close())
  const page = await browser.newPage()
  const errors = []
  page.on('pageerror', error => errors.push(String(error)))
  const requests = []
  await page.route('**/api/verb/**', async route => {
    const request = route.request()
    requests.push({ name: new URL(request.url()).pathname.split('/').at(-1), args: request.postDataJSON() })
    await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ ok: true, result: {} }) })
  })
  const address = server.resolvedUrls.local[0]
  await page.goto(new URL('/__caption_size_fixture__', address).href)
  const input = page.locator('[data-cut-caption-size]')
  await input.waitFor({ timeout: 5_000 }).catch(async error => {
    throw new Error(`${error.message}; page=${(await page.content()).slice(0, 500)}; errors=${errors.join('; ')}`)
  })
  assert.equal(await page.locator('[data-cut-action="caption-color"][value="#ffffff"]').getAttribute('aria-pressed'), 'true')
  assert.equal(await page.locator('[data-cut-action="caption-color"][value="#ffe14d"]').getAttribute('aria-pressed'), 'false')
  await input.click()
  await page.keyboard.press('ControlOrMeta+A')
  for (const [digit, expected] of [['1', '1'], ['0', '10'], ['5', '105']]) {
    await page.keyboard.press(digit)
    assert.equal(await input.inputValue(), expected)
  }
  const hex = page.locator('[data-cut-caption-color-hex]')
  await hex.click()
  await page.keyboard.press('ControlOrMeta+A')
  await page.keyboard.type('#33c')
  assert.equal(await hex.inputValue(), '#33c')
  assert.equal(await page.locator('[data-cut-caption-color]').inputValue(), '#ffffff')
  await page.keyboard.type('c88')
  assert.equal(await hex.inputValue(), '#33cc88')
  assert.equal(await page.locator('[data-cut-caption-color]').inputValue(), '#33cc88')
  await page.locator('[data-cut-caption-style]').click()
  await page.waitForFunction(() => document.querySelector('[data-cut-caption-note]')?.textContent?.includes('Style set'))
  await page.locator('[data-cut-caption-save-name]').fill('typed size')
  await page.locator('[data-cut-action="caption-style-save"]').click()
  await page.waitForFunction(() => document.querySelector('[data-cut-caption-note]')?.textContent?.includes('Saved look'))
  assert.equal(requests.find(request => request.name === 'captions.set_style')?.args.style.size, 105)
  assert.equal(requests.find(request => request.name === 'captions.set_style')?.args.style.color, '#33cc88')
  assert.equal(requests.find(request => request.name === 'captions.save_style')?.args.style.size, 105)
  assert.equal(requests.find(request => request.name === 'captions.save_style')?.args.style.color, '#33cc88')

  const yellow = page.locator('[data-cut-action="caption-color"][value="#ffe14d"]')
  await page.evaluate(() => {
    window.__captionPresetTrusted = false
    document.querySelector('[data-cut-action="caption-color"][value="#ffe14d"]')
      ?.addEventListener('click', event => { window.__captionPresetTrusted = event.isTrusted }, { once: true })
  })
  await yellow.click()
  assert.equal(await page.evaluate(() => window.__captionPresetTrusted), true)
  assert.equal(await yellow.getAttribute('aria-pressed'), 'true')
  assert.equal(await hex.inputValue(), '#ffe14d')
  assert.equal(await page.locator('[data-cut-caption-color]').inputValue(), '#ffe14d')
  await page.locator('[data-cut-caption-style]').click()
  assert.equal(requests.filter(request => request.name === 'captions.set_style').at(-1)?.args.style.color, '#ffe14d')
})
