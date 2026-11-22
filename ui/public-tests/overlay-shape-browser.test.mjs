import assert from 'node:assert/strict'
import { fileURLToPath } from 'node:url'
import test from 'node:test'
import { chromium } from 'playwright'
import { createServer } from 'vite'
import react from '@vitejs/plugin-react'
import { mkdir } from 'node:fs/promises'

test('shaped overlay preview is square, cover-cropped and draggable', { timeout: 60000 }, async () => {
  let server, browser
  try {
    server = await createServer({ root: fileURLToPath(new URL('..', import.meta.url)), configFile: false, plugins: [react(), {
      name: 'overlay-shape-fixture',
      configureServer(vite) {
        vite.middlewares.use('/__overlay_shape__', (_req, res) => {
          res.setHeader('content-type', 'text/html')
          res.end(`<div id="root"></div><script type="module">
            import RefreshRuntime from '/@react-refresh'; RefreshRuntime.injectIntoGlobalHook(window);
            window.$RefreshReg$=()=>{}; window.$RefreshSig$=()=>type=>type; window.__vite_plugin_react_preamble_installed__=true;
            </script><script type="module" src="/public-tests/fixtures/overlay-shape.tsx"></script>`)
        })
      },
    }], server: { host: '127.0.0.1', port: 0, watch: null } })
    await server.listen()
    browser = await chromium.launch({ headless: true, args: ['--no-sandbox'] })
    const page = await browser.newPage({ viewport: { width: 1000, height: 800 } })
    for (const shape of ['circle', 'rounded']) {
      await page.goto(`http://127.0.0.1:${server.httpServer.address().port}/__overlay_shape__?shape=${shape}`)
      const layer = page.locator('[data-cut-overlay-clip="camera"]')
      await layer.waitFor()
      const style = await layer.evaluate(el => { const s = getComputedStyle(el); return { width: el.clientWidth, height: el.clientHeight, radius: s.borderRadius, fit: s.objectFit } })
      assert.equal(style.width, style.height)
      assert.equal(style.radius, shape === 'circle' ? '50%' : '18%')
      assert.equal(style.fit, 'cover')
      const evidence = process.env.CUT_SHAPE_EVIDENCE
      if (evidence) { await mkdir(evidence, { recursive: true }); await page.screenshot({ path: `${evidence}/${shape}-preview.png` }) }
      const handles = page.locator('[data-cut-xform]')
      const before = await handles.boundingBox()
      await page.mouse.move(before.x + 30, before.y + 30)
      await page.mouse.down(); await page.mouse.move(before.x + 80, before.y + 60); await page.mouse.up()
      assert.ok((await handles.boundingBox()).x > before.x)
      assert.equal(await layer.evaluate(el => getComputedStyle(el).borderRadius), style.radius)
      const grip = page.locator('[data-cut-xform-handle="br"]')
      const box = await grip.boundingBox()
      await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2)
      await page.mouse.down(); await page.mouse.move(box.x + 35, box.y + 35); await page.mouse.up()
      const resized = await handles.boundingBox()
      assert.ok(resized.width > before.width)
      assert.ok(Math.abs(resized.width - resized.height) < 1)
    }
  } finally { await browser?.close(); await server?.close() }
})
