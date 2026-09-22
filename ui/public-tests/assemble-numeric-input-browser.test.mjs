import assert from 'node:assert/strict'
import { createRequire } from 'node:module'
import { createServer } from 'node:net'
import test from 'node:test'
import { spawn } from 'node:child_process'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const uiRoot = fileURLToPath(new URL('..', import.meta.url))
const requireFromUi = createRequire(resolve(uiRoot, 'package.json'))
const sleep = (ms) => new Promise((resolveSleep) => setTimeout(resolveSleep, ms))

async function reservePort() {
  const reservation = createServer()
  await new Promise((resolveListen, rejectListen) => {
    reservation.once('error', rejectListen)
    reservation.listen(0, '127.0.0.1', resolveListen)
  })
  const address = reservation.address()
  assert.ok(address && typeof address !== 'string', 'Vite reservation has a TCP address')
  await new Promise((resolveClose, rejectClose) => reservation.close((error) => error ? rejectClose(error) : resolveClose()))
  return address.port
}

function startVite(port) {
  return spawn(process.execPath, [resolve(uiRoot, 'node_modules/vite/bin/vite.js'), '--host', '127.0.0.1', '--port', String(port), '--strictPort'], {
    cwd: uiRoot,
    stdio: 'ignore',
  })
}

async function waitForMock(url, vite) {
  const deadline = Date.now() + 20_000
  while (Date.now() < deadline) {
    if (vite.exitCode !== null) throw new Error(`source Vite exited before ready (${vite.exitCode})`)
    try {
      if ((await fetch(url)).ok) return
    } catch {
      // Vite has not bound the reserved port yet.
    }
    await sleep(100)
  }
  throw new Error(`source Vite did not become ready at ${url}`)
}

async function stopVite(vite) {
  if (vite.exitCode !== null) return
  const closed = Promise.race([
    new Promise((resolveClose) => vite.once('close', resolveClose)),
    sleep(4_000).then(() => null),
  ])
  vite.kill('SIGTERM')
  if (await closed === null && vite.exitCode === null) vite.kill('SIGKILL')
}

async function replaceWithTrustedKeys(page, control, text) {
  await control.click()
  await page.keyboard.press('Control+A')
  await page.keyboard.type(text)
}

test('Assemble keeps trusted numeric drafts through parent feedback', {
  skip: process.env.SHELLX_CUT_RUN_ASSEMBLE_NUMERIC_BROWSER_TEST === '1'
    ? false
    : 'set SHELLX_CUT_RUN_ASSEMBLE_NUMERIC_BROWSER_TEST=1 on a browser-capable source host',
  timeout: 120_000,
}, async () => {
  const port = await reservePort()
  const url = `http://127.0.0.1:${port}/?mock=1`
  const vite = startVite(port)
  let browser

  try {
    const { chromium } = requireFromUi('playwright')
    browser = await chromium.launch({ headless: true })
    const page = await browser.newPage({ viewport: { width: 1440, height: 900 } })
    const pageErrors = []
    page.on('pageerror', (error) => pageErrors.push(error.message))
    await waitForMock(url, vite)
    await page.goto(url, { waitUntil: 'domcontentloaded' })
    await page.locator('[data-cut-assemble-btn]').click()
    await page.locator('[data-cut-assemble]').waitFor({ state: 'visible' })

    const target = page.locator('[data-cut-assemble-target]').first()
    await replaceWithTrustedKeys(page, target, '12')
    assert.equal(await target.inputValue(), '12', 'target retains both trusted keys, not the first-key clamp')
    assert.equal(await target.getAttribute('aria-valuenow'), '12', 'parent feedback accepted the exact target')

    const count = page.locator('[data-cut-assemble-count]').first()
    await replaceWithTrustedKeys(page, count, '2.5')
    assert.equal(await count.inputValue(), '2.5', 'an incomplete invalid count remains inspectable while editing')
    assert.equal(await count.getAttribute('aria-valuenow'), '2', 'only its integral prefix reached the controlled parent')
    await count.press('Tab')
    assert.equal(await count.inputValue(), '2', 'blur restores the last valid integer')
    await count.click()
    await count.press('ArrowUp')
    assert.equal(await count.inputValue(), '3', 'integer count increments one whole item')
    await count.press('ArrowDown')
    assert.equal(await count.inputValue(), '2', 'integer count decrements one whole item')

    await page.locator('[data-cut-assemble-mode-opt="from_script"]').click()
    const minScore = page.locator('[data-cut-assemble-minscore]').first()
    await replaceWithTrustedKeys(page, minScore, '0.')
    assert.equal(await minScore.inputValue(), '0.', 'decimal draft survives the zero parent feedback')
    assert.equal(await minScore.getAttribute('aria-valuenow'), '0', 'zero is a valid controlled score')
    await minScore.press('Tab')
    assert.equal(await minScore.inputValue(), '0', 'blur normalizes a completed decimal draft')
    await replaceWithTrustedKeys(page, minScore, '0.4')
    assert.equal(await minScore.inputValue(), '0.4', 'minimum score retains every trusted key')
    assert.equal(await minScore.getAttribute('aria-valuenow'), '0.4', 'parent feedback accepted the exact score')
    assert.deepEqual(pageErrors, [], `mock browser had no page errors: ${pageErrors.join(' | ')}`)
  } finally {
    try {
      await browser?.close()
    } finally {
      await stopVite(vite)
    }
  }
})
