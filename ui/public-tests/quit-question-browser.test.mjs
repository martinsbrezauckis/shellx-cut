// Closing the app while an export runs: the shell sends `cut:quit-jobs-running`
// and this window asks. Both buttons are pressed for real and each answer is
// read back the way the shell reads it (`answer_quit_question{stop}`).
import assert from 'node:assert/strict'
import { fileURLToPath } from 'node:url'
import test from 'node:test'
import { chromium } from 'playwright'
import { createServer } from 'vite'
import react from '@vitejs/plugin-react'

const QUESTION = {
  message: 'quitting ShellX Cut would cancel 1 running job(s) that were asked for (exports, renders, reviews, analysis, downloads) and nothing has been stopped',
  cause: 'running: job_004 (render, 42% done, rendering)',
  running: [{ job_id: 'job_004', kind: 'render', progress: 0.42, message: 'rendering' }],
  suggested_action: 'wait for them to finish, or cancel them with jobs.cancel, or repeat the call with cancel_running_jobs:true to stop them cleanly before the app closes',
  question: '',
}

test('the close-time export question: Keep working, Esc and a click outside keep the work; Quit and stop it stops it', async t => {
  const server = await createServer({
    root: fileURLToPath(new URL('..', import.meta.url)), configFile: false, plugins: [react(), {
      name: 'quit-question-fixture',
      configureServer(vite) {
        vite.middlewares.use('/__quit_question_fixture__', (_request, response) => {
          response.setHeader('content-type', 'text/html')
          response.end(`<div id="root"></div><script type="module">
            import RefreshRuntime from '/@react-refresh'
            RefreshRuntime.injectIntoGlobalHook(window)
            window.$RefreshReg$ = () => {}
            window.$RefreshSig$ = () => type => type
            window.__vite_plugin_react_preamble_installed__ = true
          </script><script type="module" src="/public-tests/fixtures/quit-question-browser.tsx"></script>`)
        })
      },
    }],
    server: { host: '127.0.0.1', port: 0 },
  })
  t.after(() => server.close())
  await server.listen()
  const browser = await chromium.launch({ headless: true })
  t.after(() => browser.close())
  const page = await browser.newPage({ viewport: { width: 1000, height: 700 } })
  const errors = []
  page.on('pageerror', error => errors.push(String(error)))
  await page.goto(new URL('/__quit_question_fixture__', server.resolvedUrls.local[0]).href)
  await page.locator('[data-test-under-question]').waitFor({ timeout: 10_000 })
  for (let i = 0; i < 100 && await page.evaluate(() => window.__quitListeners('cut:quit-jobs-running')) < 1; i += 1) {
    await page.waitForTimeout(20)
  }
  assert.equal(await page.evaluate(() => window.__quitListeners('cut:quit-jobs-running')), 1, 'the window listens for the shell question')

  const question = page.locator('[data-cut-quit-question]')
  const keep = page.locator('[data-cut-quit-keep]')
  const stop = page.locator('[data-cut-quit-stop]')
  const quitting = page.locator('[data-cut-quitting]')
  const ask = async () => {
    await page.evaluate(payload => window.__shellEmit('cut:quit-jobs-running', payload), QUESTION)
    await question.waitFor({ state: 'visible', timeout: 5_000 })
  }
  const answers = () => page.evaluate(() => window.__quitCalls
    .filter(call => call.command === 'answer_quit_question')
    .map(call => call.args?.stop))
  const acks = () => page.evaluate(() => window.__quitCalls.filter(call => call.command === 'acknowledge_quit_question').length)

  // The question names the work in plain words and focuses Keep working.
  await ask()
  assert.equal(await acks(), 1, 'the window tells the shell it shows the question')
  assert.match(await question.textContent(), /An export is still running/)
  assert.match(await question.textContent(), /Render, 42% done/)
  assert.doesNotMatch(await question.textContent(), /job_004/, 'no job id for the person')
  assert.equal(await page.evaluate(() => document.activeElement?.hasAttribute('data-cut-quit-keep')), true, 'Keep working is the default')

  // The work keeps running while the person reads: the number follows it, and
  // a number that is no longer current is not shown at all.
  let progress = 0.68
  let engineAnswers = true
  await page.route('**/api/verb/jobs.status', route => (engineAnswers
    ? route.fulfill({ status: 200, contentType: 'application/json', body: JSON.stringify({ ok: true, result: { job_id: 'job_004', kind: 'render', state: 'running', progress } }) })
    : route.fulfill({ status: 503, contentType: 'application/json', body: JSON.stringify({ ok: false, error: { code: 'io', message: 'unavailable' } }) })))
  // ui.state reads the running lines from data-cut-quit-question-running: it
  // must carry the same live value as the visible text, never the opening one.
  const listedForAgents = () => page.evaluate(() => document.querySelector('[data-cut-quit-question]')?.getAttribute('data-cut-quit-question-running')).then((value) => JSON.parse(value ?? '[]'))
  await page.waitForFunction(() => /Render, 68% done/.test(document.querySelector('[data-cut-quit-question]')?.textContent || ''), null, { timeout: 5_000 })
  assert.deepEqual(await listedForAgents(), ['job_004 (render, 68% done)'], 'ui.state follows the live progress')
  progress = 0.91
  await page.waitForFunction(() => /Render, 91% done/.test(document.querySelector('[data-cut-quit-question]')?.textContent || ''), null, { timeout: 5_000 })
  assert.deepEqual(await listedForAgents(), ['job_004 (render, 91% done)'])
  engineAnswers = false
  await page.waitForFunction(() => {
    const text = document.querySelector('[data-cut-quit-question] li')?.textContent || ''
    return text === 'Render'
  }, null, { timeout: 8_000 })
  assert.deepEqual(await listedForAgents(), ['job_004 (render)'], 'no stale number for agents either')
  engineAnswers = true
  progress = 0.95

  // Keep working: answers no and closes the question.
  await keep.click()
  await question.waitFor({ state: 'detached', timeout: 5_000 })
  assert.deepEqual(await answers(), [false])
  assert.equal(await quitting.count(), 0)

  // Esc keeps working, also with the focus outside the question.
  await ask()
  await page.evaluate(() => document.activeElement?.blur())
  await page.keyboard.press('Escape')
  await question.waitFor({ state: 'detached', timeout: 5_000 })
  assert.deepEqual(await answers(), [false, false])

  // A click outside the question keeps working.
  await ask()
  await page.mouse.click(8, 8)
  await question.waitFor({ state: 'detached', timeout: 5_000 })
  assert.deepEqual(await answers(), [false, false, false])

  // Quit and stop it: answers yes and says it is stopping.
  await ask()
  await stop.click()
  await quitting.waitFor({ state: 'visible', timeout: 5_000 })
  assert.deepEqual(await answers(), [false, false, false, true])
  assert.match(await quitting.textContent(), /Stopping the work, then ShellX Cut closes/)
  assert.equal(await question.count(), 0)

  // The shell ends the question when the work finished meanwhile.
  await page.evaluate(() => window.__shellEmit('cut:quit-jobs-settled', {}))
  await quitting.waitFor({ state: 'detached', timeout: 5_000 })
  await ask()
  await page.evaluate(() => window.__shellEmit('cut:quit-jobs-settled', {}))
  await question.waitFor({ state: 'detached', timeout: 5_000 })
  assert.deepEqual(await answers(), [false, false, false, true], 'the shell ending the question is not an answer')
  assert.deepEqual(errors, [])
})
