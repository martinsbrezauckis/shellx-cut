import assert from 'node:assert/strict'
import { mkdirSync, mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import test from 'node:test'
import { chromium } from 'playwright'
import { createServer } from 'vite'
import react from '@vitejs/plugin-react'

// The voiceover popover hangs under a trigger that sits in the track-header rail
// at the left edge of the window. It used to be positioned relative to that rail
// and was cut off at the window's left edge (the first letters of "Playhead" and
// "3 s count-in" were missing). It must stay inside the window at any width.
async function openFixture(t, viewport, bottom = false) {
  const cacheDir = mkdtempSync(join(tmpdir(), 'cut-voiceover-layout-'))
  t.after(() => rmSync(cacheDir, { recursive: true, force: true }))
  const server = await createServer({
    root: fileURLToPath(new URL('..', import.meta.url)), cacheDir, configFile: false,
    plugins: [react(), { name: 'voiceover-layout-fixture', configureServer(vite) {
      vite.middlewares.use('/__voiceover_layout__', (_request, response) => {
        response.setHeader('content-type', 'text/html')
        response.end(`<!doctype html><meta name="viewport" content="width=device-width"><body style="margin:0"><div id="root"></div><script type="module">
          import RefreshRuntime from '/@react-refresh'
          RefreshRuntime.injectIntoGlobalHook(window)
          window.$RefreshReg$ = () => {}
          window.$RefreshSig$ = () => type => type
          window.__vite_plugin_react_preamble_installed__ = true
        </script><script type="module" src="/public-tests/fixtures/voiceover-popover-layout-browser.tsx"></script>`)
      })
    } }], server: { host: '127.0.0.1', port: 0 },
  })
  t.after(() => server.close())
  await server.listen()
  const browser = await chromium.launch({ headless: true })
  t.after(() => browser.close())
  const page = await browser.newPage({ viewport })
  const errors = []
  page.on('pageerror', error => errors.push(String(error)))
  await page.route('**/api/verb/screen_record.doctor', route => route.fulfill({
    json: { ok: true, result: { start_allowed: true, microphones: [], microphone_selection: { mode: 'system_default', status: 'ready' } } },
  }))
  await page.goto(new URL(`/__voiceover_layout__${bottom ? '?bottom' : ''}`, server.resolvedUrls.local[0]).href)
  await page.locator('[data-cut-voiceover-track]').waitFor()
  return { page, errors }
}

async function bottomPlacement(page, label) {
  const record = page.locator('[data-cut-action="voiceover-record"]')
  await record.waitFor({ state: 'visible' })
  await page.waitForFunction(() => !document.querySelector('[data-cut-action="voiceover-record"]').disabled)
  // Let resize/scroll and ResizeObserver placement settle before measuring.
  await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))))
  const observed = await record.evaluate(button => {
    const popup = button.closest('.tl-voiceover__popover')
    const trigger = document.querySelector('[data-cut-action="voiceover-controls"]').getBoundingClientRect()
    const p = popup.getBoundingClientRect()
    const b = button.getBoundingClientRect()
    const inside = r => r.width > 0 && r.height > 0 && r.left >= 0 && r.top >= 0 && r.right <= innerWidth && r.bottom <= innerHeight
    const points = [[b.left + b.width / 2, b.top + b.height / 2],
      [b.left + 3, b.top + 3], [b.right - 3, b.top + 3],
      [b.left + 3, b.bottom - 3], [b.right - 3, b.bottom - 3]]
    return { popup: p.toJSON(), button: b.toJSON(), trigger: trigger.toJSON(),
      popupInside: inside(p), buttonInside: inside(b),
      uncovered: points.map(([x, y]) => document.elementFromPoint(x, y)?.closest('button') === button),
      above: p.bottom <= trigger.top, gap: trigger.top - p.bottom }
  })
  console.log(`${label}: ${JSON.stringify(observed)}`)
  if (process.env.CUT_VOICEOVER_LAYOUT_SHOTS) {
    mkdirSync(process.env.CUT_VOICEOVER_LAYOUT_SHOTS, { recursive: true })
    await page.screenshot({ path: join(process.env.CUT_VOICEOVER_LAYOUT_SHOTS, `${label}.png`) })
  }
  return { label, ...observed }
}

for (const viewport of [{ width: 1440, height: 900 }, { width: 1200, height: 800 }, { width: 1000, height: 700 }]) {
  test(`bottom audio track keeps Record voiceover uncovered at ${viewport.width}x${viewport.height}, resize and scroll`, async t => {
    const { page, errors } = await openFixture(t, viewport, true)
    await page.locator('[data-cut-action="voiceover-controls"]').click()
    const label = `bottom-${viewport.width}x${viewport.height}`
    const observations = [await bottomPlacement(page, label)]
    const resized = viewport.width === 1000 ? { width: 1440, height: 900 } : { width: 1000, height: 700 }
    await page.setViewportSize(resized)
    observations.push(await bottomPlacement(page, `${label}-resized`))
    const beforeScroll = await page.locator('[data-cut-action="voiceover-controls"]').boundingBox()
    await page.locator('[data-test-timeline-scroll]').evaluate(el => { el.scrollTop = 36; el.scrollLeft = 40 })
    observations.push(await bottomPlacement(page, `${label}-scrolled`))
    const afterScroll = await page.locator('[data-cut-action="voiceover-controls"]').boundingBox()
    assert.ok(afterScroll.y < beforeScroll.y, 'the timeline really scrolled the trigger')
    for (const observed of observations) {
      assert.ok(observed.popupInside, `${observed.label}: popup outside window`)
      assert.ok(observed.buttonInside, `${observed.label}: Record button outside window`)
      assert.deepEqual(observed.uncovered, [true, true, true, true, true], `${observed.label}: Record centre/corners covered`)
      assert.ok(observed.above && observed.gap <= 6, `${observed.label}: popup must follow the trigger above when below cannot fit`)
    }
    await page.keyboard.press('Escape')
    await page.locator('.tl-voiceover__popover').waitFor({ state: 'detached' })
    await page.waitForFunction(() => document.activeElement?.matches('[data-cut-action="voiceover-controls"]'))
    assert.equal(await page.evaluate(() => document.documentElement.dataset.cutBlockingOverlay), undefined)
    assert.deepEqual(errors, [])
  })
}

for (const viewport of [{ width: 1280, height: 700 }, { width: 390, height: 700 }, { width: 320, height: 420 }]) {
  test(`voiceover popover stays inside a ${viewport.width}x${viewport.height} window`, async t => {
    const { page, errors } = await openFixture(t, viewport)
    await page.locator('[data-cut-action="voiceover-controls"]').click()
    const popover = page.locator('.tl-voiceover__popover')
    await popover.waitFor()
    const box = await popover.evaluate(el => {
      const r = el.getBoundingClientRect()
      return { left: r.left, top: r.top, right: r.right, bottom: r.bottom, width: window.innerWidth, height: window.innerHeight }
    })
    assert.ok(box.left >= 0, `left edge ${box.left} is outside the window`)
    assert.ok(box.top >= 0, `top edge ${box.top} is outside the window`)
    assert.ok(box.right <= box.width, `right edge ${box.right} is outside the ${box.width}px window`)
    assert.ok(box.bottom <= box.height, `bottom edge ${box.bottom} is outside the ${box.height}px window`)
    // Nothing clips it: the point just inside its left edge is the popover itself,
    // and the playhead line that was cut reads in full.
    const hit = await popover.evaluate(el => {
      const r = el.getBoundingClientRect()
      return el.contains(document.elementFromPoint(r.left + 4, r.top + 12))
    })
    assert.equal(hit, true, 'the popover left edge is covered by something else')
    assert.match(await page.locator('[data-cut-voiceover-range]').textContent(), /^Playhead 0 ms$/)
    assert.deepEqual(errors, [])
  })
}
