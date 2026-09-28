import assert from 'node:assert/strict'
import { fileURLToPath } from 'node:url'
import test from 'node:test'
import { chromium } from 'playwright'
import { createServer } from 'vite'
import react from '@vitejs/plugin-react'

test('Codex model override survives leaving Chat for Timeline and reaches agent.chat', async t => {
  const server = await createServer({
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
    }], server: { host: '127.0.0.1', port: 0 },
  })
  await server.listen()
  t.after(() => server.close())
  const browser = await chromium.launch({ headless: true })
  t.after(() => browser.close())
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
})
