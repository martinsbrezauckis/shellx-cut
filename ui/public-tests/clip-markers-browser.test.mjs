import assert from 'node:assert/strict'
import { fileURLToPath } from 'node:url'
import test from 'node:test'
import { mkdir } from 'node:fs/promises'
import { chromium } from 'playwright'
import { createServer } from 'vite'
import react from '@vitejs/plugin-react'

test('clip markers render in dark/light themes, follow trim/split, and do not steal clicks or drags', { timeout: 60000 }, async () => {
  let server
  let browser
  try {
    server = await createServer({
      root: fileURLToPath(new URL('..', import.meta.url)),
      configFile: false,
      plugins: [react(), {
        name: 'clip-markers-fixture',
        configureServer(vite) {
          vite.middlewares.use('/__clip_markers__', (_req, res) => {
            res.setHeader('content-type', 'text/html')
            res.end(`<div id="root"></div><script type="module">
              import RefreshRuntime from '/@react-refresh'; RefreshRuntime.injectIntoGlobalHook(window);
              window.$RefreshReg$=()=>{}; window.$RefreshSig$=()=>type=>type; window.__vite_plugin_react_preamble_installed__=true;
            </script><script type="module" src="/public-tests/fixtures/clip-markers.tsx"></script>`)
          })
        },
      }],
      server: { host: '127.0.0.1', port: 0, watch: null },
    })
    await server.listen()
    const port = server.httpServer.address().port
    browser = await chromium.launch({ headless: true, args: ['--no-sandbox'] })
    const page = await browser.newPage({ viewport: { width: 820, height: 260 } })
    const evidence = process.env.CUT_CLIP_MARKERS_EVIDENCE

    for (const theme of ['dark', 'light']) {
      await page.goto(`http://127.0.0.1:${port}/__clip_markers__?theme=${theme}`)
      const m1 = page.locator('[data-cut-clip-marker="cm1"]')
      const m2 = page.locator('[data-cut-clip-marker="cm2"]')
      await m1.waitFor()
      await m2.waitFor()
      assert.equal(await m1.getAttribute('title'), 'Intro highlight')
      assert.equal(await m2.getAttribute('title'), 'Key takeaway')

      // Check tick visibility & contrast color resolved (not transparent).
      const bg = await m1.evaluate((el) => getComputedStyle(el, '::after').backgroundColor)
      assert.ok(bg && bg !== 'rgba(0, 0, 0, 0)', `tick stem color missing in ${theme}: ${bg}`)

      // Clicking and dragging directly ON a marker tick must hit the parent clip!
      const box = await m1.boundingBox()
      assert.ok(box && box.width > 0 && box.height > 0)
      await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2)
      await page.mouse.down()
      await page.mouse.move(box.x + box.width / 2 + 24, box.y + box.height / 2)
      await page.mouse.up()
      assert.equal(await page.locator('[data-cut-test="selected"]').textContent(), 'rec-clip-1')
      assert.ok(Number(await page.locator('[data-cut-test="drags"]').textContent()) >= 1, 'drag starting on marker tick must reach clip handler')

      if (evidence) {
        await mkdir(evidence, { recursive: true })
        await page.screenshot({ path: `${evidence}/clip-markers-${theme}-before.png` })
      }

      // Trim: hides first marker outside visible range.
      await page.locator('[data-cut-test="trim"]').click()
      assert.equal(await page.locator('[data-cut-clip-marker]').count(), 1)
      assert.equal(await page.locator('[data-cut-clip-marker="cm2"]').count(), 1)
      if (evidence) await page.screenshot({ path: `${evidence}/clip-markers-${theme}-trimmed.png` })

      // Split: both halves show only their own range.
      await page.locator('[data-cut-test="split"]').click()
      assert.equal(await page.locator('[data-cut-clip="rec-clip-1"] [data-cut-clip-marker="cm1"]').count(), 1)
      assert.equal(await page.locator('[data-cut-clip="rec-clip-1"] [data-cut-clip-marker="cm2"]').count(), 0)
      assert.equal(await page.locator('[data-cut-clip="rec-clip-2"] [data-cut-clip-marker="cm2"]').count(), 1)
      assert.equal(await page.locator('[data-cut-clip="rec-clip-2"] [data-cut-clip-marker="cm1"]').count(), 0)
      if (evidence) await page.screenshot({ path: `${evidence}/clip-markers-${theme}-split.png` })
    }
  } finally {
    await browser?.close()
    await server?.close()
  }
})
