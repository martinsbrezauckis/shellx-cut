import assert from 'node:assert/strict'
import { fileURLToPath } from 'node:url'
import test from 'node:test'
import { chromium } from 'playwright'
import { createServer } from 'vite'
import react from '@vitejs/plugin-react'

test('late Generate reply survives Media unmount but cannot update the replacement project', async t => {
  const server = await createServer({
    root: fileURLToPath(new URL('..', import.meta.url)),
    configFile: false,
    plugins: [react(), {
      name: 'generation-unmount-project-fixture',
      configureServer(vite) {
        vite.middlewares.use('/__generation_unmount__', (_request, response) => {
          response.setHeader('content-type', 'text/html')
          response.end(`<div id="root"></div><script type="module">
            import RefreshRuntime from '/@react-refresh'
            RefreshRuntime.injectIntoGlobalHook(window)
            window.$RefreshReg$ = () => {}
            window.$RefreshSig$ = () => type => type
            window.__vite_plugin_react_preamble_installed__ = true
          </script><script type="module" src="/public-tests/fixtures/generation-unmount-project.tsx"></script>`)
        })
      },
    }],
    server: { host: '127.0.0.1', port: 0 },
  })
  await server.listen()
  t.after(() => server.close())
  const browser = await chromium.launch({ headless: true })
  t.after(() => browser.close())
  const page = await browser.newPage()
  const pageErrors = []
  page.on('pageerror', error => pageErrors.push(String(error)))
  const pending = Promise.withResolvers()
  const statusJobs = []
  await page.route('**/api/verb/**', async route => {
    const name = new URL(route.request().url()).pathname.split('/').at(-1)
    if (name === 'assets.generate') {
      pending.resolve(route)
      return
    }
    if (name === 'jobs.status') statusJobs.push(route.request().postDataJSON()?.job_id)
    await route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({
      ok: true,
      result: name === 'assets.generated_list' ? { items: [] }
        : name === 'jobs.status' ? { state: 'running', progress: 0.2 }
          : {},
    }) })
  })

  await page.goto(new URL('/__generation_unmount__', server.resolvedUrls.local[0]).href)
  await page.locator('[data-cut-generate-prompt]').fill('A generated title image')
  await page.locator('[data-cut-generate-placement-mode="insert"]').click()
  await page.locator('[data-cut-generate-placement-track]').waitFor({ state: 'visible' })
  await page.locator('[data-cut-generate-run]').click()
  await page.locator('[data-cut-generate-run]').click()
  const oldRoute = await pending.promise

  await page.locator('[data-test-unmount]').click()
  await page.locator('[data-test-open-b]').click()
  assert.equal(await page.locator('[data-test-project]').textContent(), 'Project B')
  await oldRoute.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({
    ok: true,
    result: { job_id: 'job-for-A', placement: { mode: 'insert' } },
  }) })
  await page.waitForFunction(() => Object.keys(localStorage).some((key) => key.startsWith('cut.generate.active-job:') && localStorage.getItem(key)?.includes('job-for-A')))
  await page.waitForTimeout(800)
  assert.equal(await page.locator('[data-test-generated-count]').textContent(), '0', 'unmounted A callback cannot refresh B')
  assert.equal(statusJobs.includes('job-for-A'), false, 'unmounted A poll must not run while B is open')
  assert.equal(await page.locator('[data-cut-generate-job-cancel]').count(), 0, 'B must not show A job')

  await page.locator('[data-test-return-a]').click()
  await page.waitForFunction(() => !!document.querySelector('[data-cut-generate-job-cancel="job-for-A"]'))
  await page.waitForFunction(() => !!document.querySelector('[data-cut-generate-job-state]'))
  assert.equal(statusJobs.includes('job-for-A'), true, 'A job resumes polling only when A returns')
  assert.deepEqual(pageErrors, [])
})
