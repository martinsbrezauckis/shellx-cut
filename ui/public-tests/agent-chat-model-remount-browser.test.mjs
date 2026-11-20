import assert from 'node:assert/strict'
import { fileURLToPath } from 'node:url'
import test from 'node:test'
import { chromium } from 'playwright'
import { createServer } from 'vite'
import react from '@vitejs/plugin-react'

// Everything this test opens is closed in `finally`, not in t.after hooks, and
// a rejection nobody awaits (from Vite or the browser) ends the test at once
// with that error. Before, such a rejection failed the test immediately, the
// hooks registered after that point never ran, and the dev server and browser
// kept the file running until the CI job limit. The dev server needs no file
// watcher (nothing is edited while it runs), so it does not depend on the
// host's free inotify instances either.
test('Codex model override survives leaving Chat for Timeline and reaches agent.chat', { timeout: 120_000 }, async () => {
  let server
  let browser
  let onStrayRejection
  const strayRejection = new Promise((_resolve, reject) => {
    onStrayRejection = (reason) => reject(reason instanceof Error ? reason : new Error(String(reason)))
  })
  process.on('unhandledRejection', onStrayRejection)
  const running = run()
  try {
    await Promise.race([running, strayRejection])
  } finally {
    process.off('unhandledRejection', onStrayRejection)
    // After an early failure the run may still be creating the server or the
    // browser: let it finish that (bounded), then close whatever exists.
    await Promise.race([running.catch(() => {}), new Promise((resolve) => setTimeout(resolve, 20_000).unref())])
    await Promise.allSettled([browser?.close(), server?.close()])
  }

  async function run() {
    server = await createServer({
      root: fileURLToPath(new URL('..', import.meta.url)), configFile: false, plugins: [react(), {
        name: 'chat-model-remount-fixture',
        configureServer(vite) {
          vite.middlewares.use('/__chat_model_remount__', (_request, response) => {
            response.setHeader('content-type', 'text/html')
            response.end(`<div id="root"></div><script type="module">
              import RefreshRuntime from '/@react-refresh'
              RefreshRuntime.injectIntoGlobalHook(window)
              window.$RefreshReg$ = () => {}
              window.$RefreshSig$ = () => type => type
              window.__vite_plugin_react_preamble_installed__ = true
            </script><script type="module" src="/public-tests/fixtures/agent-chat-model-remount.tsx"></script>`)
          })
        },
      }], server: { host: '127.0.0.1', port: 0, watch: null },
    })
    await server.listen()
    browser = await chromium.launch({ headless: true })
    const page = await browser.newPage()
    const errors = []
    page.on('pageerror', error => errors.push(String(error)))
    const requests = []
    await page.route('**/api/verb/**', async route => {
      const name = new URL(route.request().url()).pathname.split('/').at(-1)
      if (name === 'agent.chat') requests.push(route.request().postDataJSON())
      await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({
        ok: true, result: name === 'system.doctor' ? { cards: [] } : name === 'agent.chat' ? { ok: false, reason: 'fixture response', agent: 'codex' } : {},
      }) })
    })
    await page.addInitScript(() => localStorage.setItem('cut.chatAgent', 'codex'))
    await page.goto(new URL('/__chat_model_remount__', server.resolvedUrls.local[0]).href)
    const model = page.locator('[data-cut-chat-model]')
    await model.waitFor({ timeout: 10_000 }).catch(() => { throw new Error(`Chat did not mount: ${errors.join('; ')}`) })
    await model.fill('gpt-6-sol')
    assert.equal(await model.inputValue(), 'gpt-6-sol')
    await page.locator('[data-test-timeline]').click()
    await model.waitFor({ state: 'detached' })
    await page.locator('[data-test-return-chat]').click()
    await model.waitFor({ state: 'visible' })
    assert.equal(await model.inputValue(), 'gpt-6-sol')
    await page.locator('[data-test-other-project]').click()
    assert.equal(await model.inputValue(), '')
    await page.locator('[data-test-original-project]').click()
    assert.equal(await model.inputValue(), 'gpt-6-sol')
    await page.locator('[data-cut-chat-input]').fill('Add a marker at the playhead')
    await page.locator('[data-cut-chat-send]').click()
    await page.waitForFunction(() => document.querySelector('[data-cut-chat-log]')?.textContent?.includes('fixture response'))
    assert.equal(requests.length, 1)
    assert.equal(requests[0].agent, 'codex')
    assert.equal(requests[0].model, 'gpt-6-sol')
    assert.deepEqual(errors, [])
  }
})
