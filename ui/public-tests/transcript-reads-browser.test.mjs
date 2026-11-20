import assert from 'node:assert/strict'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import test from 'node:test'
import { chromium } from 'playwright'
import { createServer } from 'vite'
import react from '@vitejs/plugin-react'

test('mounted Transcript recovers failed reads and owns replies by project origin', async t => {
  const cacheDir = mkdtempSync(join(tmpdir(), 'cut-transcript-reads-'))
  t.after(() => rmSync(cacheDir, { recursive: true, force: true }))
  const server = await createServer({
    root: fileURLToPath(new URL('..', import.meta.url)), cacheDir, configFile: false,
    plugins: [react(), { name: 'transcript-reads-fixture', configureServer(vite) {
      vite.middlewares.use('/__transcript_reads__', (_request, response) => {
        response.setHeader('content-type', 'text/html')
        response.end(`<div id="root"></div><script type="module">
          import RefreshRuntime from '/@react-refresh'
          RefreshRuntime.injectIntoGlobalHook(window)
          window.$RefreshReg$ = () => {}
          window.$RefreshSig$ = () => type => type
          window.__vite_plugin_react_preamble_installed__ = true
        </script><script type="module" src="/public-tests/fixtures/transcript-reads-browser.tsx"></script>`)
      })
    } }], server: { host: '127.0.0.1', port: 0 },
  })
  t.after(() => server.close())
  await server.listen()
  const browser = await chromium.launch({ headless: true })
  t.after(() => browser.close())
  const page = await browser.newPage()
  const errors = []
  page.on('pageerror', error => errors.push(String(error)))
  await page.addInitScript(() => {
    window.__transcriptUnhandled = []
    window.addEventListener('unhandledrejection', event => window.__transcriptUnhandled.push(String(event.reason)))
  })
  const requests = { 'system.doctor': [], 'transcript.timeline': [], 'transcript.get': [] }
  await page.route('**/api/verb/**', route => {
    const name = new URL(route.request().url()).pathname.split('/').at(-1)
    if (requests[name]) requests[name].push(route)
    else route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ ok: true, result: {} }) })
  })
  const next = async (name, count) => {
    for (let i = 0; i < 160; i += 1) {
      if (requests[name].length >= count) return requests[name][count - 1]
      await page.waitForTimeout(25)
    }
    assert.fail(`expected ${name} request ${count}, saw ${requests[name].length}; ${errors.join('; ')}`)
  }
  const reply = (route, result) => route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ ok: true, result }) })
  const doctor = ready => ({ cards: [{ id: 'perception', status: 'ok', details: { stt_ready: ready } }] })
  const source = (label, asset = 'shared') => ({ asset, model: 'fixture', words: [{ idx: 0, word: `${label}-source`, start_ms: 0, end_ms: 100 }] })
  const timeline = label => ({ entries: [{ clip_id: 'clip-1', track: 'v1', track_kind: 'video', asset: 'shared',
    word_index: 0, word: `${label}-timeline`, src_start_ms: 0, src_end_ms: 100, timeline_start_ms: 0, timeline_end_ms: 100 }] })
  const replySet = async (counts, label) => {
    await Promise.all([
      reply(await next('system.doctor', counts[0]), doctor(true)),
      reply(await next('transcript.timeline', counts[1]), timeline(label)),
      reply(await next('transcript.get', counts[2]), source(label)),
    ])
  }

  await page.goto(new URL('/__transcript_reads__', server.resolvedUrls.local[0]).href)
  await (await next('system.doctor', 1)).abort('failed')
  await (await next('transcript.timeline', 1)).fulfill({ status: 200, contentType: 'application/json', body: '{invalid json' })
  await (await next('transcript.get', 1)).abort('failed')
  await page.locator('[data-cut-transcript-read-error]').waitFor()
  assert.match(await page.locator('[data-cut-transcript-read-error]').textContent(), /Retry/)
  await page.locator('[data-cut-action="retry-transcript-reads"]').click()
  await replySet([2, 2, 2], 'A')
  await page.locator('[data-cut-transcript-phrase]').filter({ hasText: 'A-timeline' }).waitFor()
  await page.locator('[data-cut-action="view-source"]').click()
  await page.locator('[data-cut-word="shared:0"]').waitFor()
  assert.match(await page.locator('[data-cut-transcript="shared"]').textContent(), /A-source/)
  assert.equal(await page.locator('[data-cut-transcript-read-error]').count(), 0)

  await page.locator('[data-cut-action="view-program"]').click()
  const oldTimeline = await next('transcript.timeline', 3)
  await page.locator('[data-test-b]').click()
  assert.equal(await page.locator('[data-test-origin]').textContent(), 'origin-B')
  assert.equal(await page.locator('[data-cut-transcript-phrase]').count(), 0, 'same-name origin switch clears A timeline immediately')
  await replySet([3, 4, 3], 'B')
  await page.locator('[data-cut-transcript-phrase]').filter({ hasText: 'B-timeline' }).waitFor()
  await reply(oldTimeline, timeline('late-A'))
  await page.waitForTimeout(30)
  assert.doesNotMatch(await page.locator('[data-panel="transcript"]').textContent(), /late-A-timeline/)
  await page.locator('[data-cut-action="view-source"]').click()
  assert.match(await page.locator('[data-cut-transcript="shared"]').textContent(), /B-source/)

  await page.locator('[data-test-a]').click()
  const lateSource = await next('transcript.get', 4)
  await page.locator('[data-test-b]').click()
  await reply(await next('transcript.get', 5), source('new-B'))
  await page.waitForFunction(() => document.querySelector('[data-cut-transcript="shared"]')?.textContent?.includes('new-B-source'))
  await reply(lateSource, source('late-A'))
  await page.waitForTimeout(50)
  assert.match(await page.locator('[data-cut-transcript="shared"]').textContent(), /new-B-source/)
  assert.doesNotMatch(await page.locator('[data-cut-transcript="shared"]').textContent(), /late-A/)

  await page.locator('[data-test-multi]').click()
  const firstMulti = await next('transcript.get', 6)
  const secondMulti = await next('transcript.get', 7)
  assert.deepEqual([firstMulti.request().postDataJSON().asset, secondMulti.request().postDataJSON().asset], ['shared', 'second'])
  await reply(firstMulti, source('multi-first'))
  await page.waitForFunction(() => document.querySelector('[data-cut-transcript="shared"]')?.textContent?.includes('multi-first-source'))
  await page.waitForTimeout(50)
  assert.equal(requests['transcript.get'].length, 7, 'first asset completion does not restart pending sibling')
  await secondMulti.abort('failed')
  await page.locator('[data-cut-transcript-read-error]').waitFor()
  assert.equal(requests['transcript.get'].length, 7, 'sibling failure does not restart successful asset')
  await page.locator('[data-cut-action="retry-transcript-reads"]').click()
  const retriedSecond = await next('transcript.get', 8)
  assert.equal(retriedSecond.request().postDataJSON().asset, 'second')
  await reply(retriedSecond, source('multi-second', 'second'))
  await page.waitForFunction(() => document.querySelector('[data-cut-transcript="second"]')?.textContent?.includes('multi-second-source'))
  assert.equal(requests['transcript.get'].length, 8, 'retry reads only failed asset')

  await page.locator('[data-test-locator-v2]').click()
  const oldLocator = await next('transcript.get', 9)
  assert.equal(oldLocator.request().postDataJSON().asset, 'second')
  await page.locator('[data-test-locator-v3]').click()
  const newLocator = await next('transcript.get', 10)
  await reply(newLocator, source('locator-v3', 'second'))
  await page.waitForFunction(() => document.querySelector('[data-cut-transcript="second"]')?.textContent?.includes('locator-v3-source'))
  await reply(oldLocator, source('locator-v2-late', 'second'))
  await page.waitForTimeout(30)
  assert.doesNotMatch(await page.locator('[data-cut-transcript="second"]').textContent(), /locator-v2-late/)

  await page.locator('[data-test-a]').click()
  const unmounted = await next('transcript.get', 11)
  await page.locator('[data-test-unmount]').click()
  await reply(unmounted, source('after-unmount'))
  await page.waitForTimeout(50)
  assert.equal(await page.locator('[data-panel="transcript"]').count(), 0)
  assert.deepEqual(await page.evaluate(() => window.__transcriptUnhandled), [])
  assert.deepEqual(errors, [])
})
