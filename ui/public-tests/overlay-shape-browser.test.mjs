import assert from 'node:assert/strict'
import { fileURLToPath } from 'node:url'
import test from 'node:test'
import { chromium } from 'playwright'
import { createServer } from 'vite'
import react from '@vitejs/plugin-react'
import { mkdir, readFile } from 'node:fs/promises'

test('rectangle source shape matches export; circle and rounded stay square and draggable', { timeout: 240000 }, async () => {
  let server, browser
  try {
    server = await createServer({ root: fileURLToPath(new URL('..', import.meta.url)), configFile: false, plugins: [react(), {
      name: 'overlay-shape-fixture',
      configureServer(vite) {
        vite.middlewares.use('/rect-evidence', async (req, res) => {
          const name = req.url.slice(1)
          if (!/^(camera|export)-[0-9x-]+(?:false)?\.png$/.test(name)) { res.statusCode = 404; res.end(); return }
          if (process.env.CUT_RECT_EVIDENCE) {
            try { res.setHeader('content-type', 'image/png'); res.end(await readFile(`${process.env.CUT_RECT_EVIDENCE}/${name}`)); return } catch { res.statusCode = 404; res.end(); return }
          }
          const [, sw, sh] = name.match(/camera-(\d+)x(\d+)/) ?? []
          res.setHeader('content-type', 'image/svg+xml')
          res.end(`<svg xmlns="http://www.w3.org/2000/svg" width="${sw}" height="${sh}"><path fill="red" d="M0 0h${sw}v${sh}H0z"/></svg>`)
        })
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
    page.setDefaultNavigationTimeout(120000)
    for (const [pw, ph] of [[640, 360], [360, 640]]) {
      for (const [sw, sh] of [[320, 240], [320, 180], [180, 320]]) {
        await page.setViewportSize({ width: pw * 2 + 40, height: ph + 100 })
        await page.goto(`http://127.0.0.1:${server.httpServer.address().port}/__overlay_shape__?shape=rect&sw=${sw}&sh=${sh}&pw=${pw}&ph=${ph}`)
        const layer = page.locator('[data-cut-overlay-clip="camera"]')
        await layer.evaluate(async el => { await el.decode() })
        const stage = page.locator('[data-rect-stage]')
        const box = await layer.boundingBox(), canvas = await stage.boundingBox()
        assert.ok(Math.abs(box.width - pw * 0.25) < 1)
        assert.ok(Math.abs(box.height - pw * 0.25 * sh / sw) < 1)
        assert.ok(Math.abs(box.x - canvas.x - pw * 0.6) < 1)
        assert.ok(Math.abs(box.y - canvas.y - ph * 0.1) < 1)
        if (process.env.CUT_RECT_EVIDENCE) {
          await page.locator('[data-rect-export]').evaluate(async el => { await el.decode() })
          const shot = await stage.screenshot()
          const error = await page.evaluate(async data => {
            const live = new Image(); live.src = data; await live.decode()
            const exported = document.querySelector('[data-rect-export]')
            const c = document.createElement('canvas'); c.width = live.width; c.height = live.height
            const ctx = c.getContext('2d'); ctx.drawImage(live, 0, 0)
            const a = ctx.getImageData(0, 0, c.width, c.height).data
            ctx.drawImage(exported, 0, 0); const b = ctx.getImageData(0, 0, c.width, c.height).data
            let sum = 0; for (let i = 0; i < a.length; i++) sum += Math.abs(a[i] - b[i])
            return sum / a.length
          }, `data:image/png;base64,${shot.toString('base64')}`)
          assert.ok(error < 4, `live preview/export mean channel error ${error}`)
          console.log(`rectangle ${sw}x${sh} -> ${pw}x${ph}: live/export mean channel error ${error.toFixed(3)}`)
          if (process.env.CUT_SHAPE_EVIDENCE) {
            await mkdir(process.env.CUT_SHAPE_EVIDENCE, { recursive: true })
            await page.screenshot({ path: `${process.env.CUT_SHAPE_EVIDENCE}/rect-${sw}x${sh}-${pw}x${ph}.png` })
          }
        }
        await page.goto(page.url() + '&handles=1')
        const handles = page.locator('[data-cut-xform]')
        const before = await handles.boundingBox()
        assert.ok(Math.abs(before.width / before.height - sw / sh) < 0.02)
        const grip = await page.locator('[data-cut-xform-handle="br"]').boundingBox()
        await page.mouse.move(grip.x + grip.width / 2, grip.y + grip.height / 2)
        await page.mouse.down(); await page.mouse.move(grip.x + 12, grip.y + 12); await page.mouse.up()
        const resized = await handles.boundingBox()
        assert.ok(resized.width > before.width)
        assert.ok(Math.abs(resized.width / resized.height - sw / sh) < 0.02)
      }
    }
    await page.setViewportSize({ width: 1000, height: 800 })
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
