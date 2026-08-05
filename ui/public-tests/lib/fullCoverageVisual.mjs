// fullCoverageVisual.mjs - visual proof helpers for the exhaustive verifier.
//
// The full-coverage gate records two kinds of visual evidence: composed frames
// for SSIM before/after checks and screenshots for rendered UI groups. Keep the
// screenshot cache and ffmpeg SSIM command here so result-proof behavior is not
// spread through the runner.

import { spawnSync } from 'node:child_process'
import { copyFileSync, mkdirSync, writeFileSync } from 'node:fs'
import { extname, join } from 'node:path'
import { base64ToBuffer } from '../../../scripts/lib/safe-data.mjs'

export function parseSsimAll(stderr) {
  const match = String(stderr || '').match(
    /All:\s*(-?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][+-]?\d+)?)/,
  )
  return match ? Number.parseFloat(match[1]) : null
}

export function createVisualProof({ verb, tmpDir, screensDir, ffmpegBin = 'ffmpeg' }) {
  if (typeof verb !== 'function') throw new TypeError('createVisualProof requires verb')
  if (!tmpDir) throw new TypeError('createVisualProof requires tmpDir')
  if (!screensDir) throw new TypeError('createVisualProof requires screensDir')

  let frameSeq = 0
  const shotCache = new Map()

  function frameExt(mime) {
    if (String(mime || '').includes('png')) return 'png'
    if (String(mime || '').includes('jpeg') || String(mime || '').includes('jpg')) return 'jpg'
    return 'bin'
  }

  async function frame(at) {
    const seq = frameSeq++
    const attempts = []
    for (let attempt = 0; attempt < 2; attempt += 1) {
      // Composed software frames can exceed the general verb timeout on a
      // cold installed Windows run. This is evidence-only and read-only, so a
      // single bounded retry is safe; it never repeats a user mutation.
      const r = await verb(
        'render.frame',
        { at_ms: at, compose: true, inline: true },
        { timeoutMs: 120_000 },
      )
      const ext = frameExt(r.result?.mime)
      const dst = join(tmpDir, `f${seq}.${ext}`)
      let copyError = null
      try {
        const b64 = r.result?.base64
        if (b64) {
          writeFileSync(dst, base64ToBuffer(b64, { expectPng: ext === 'png' }))
          return dst
        }
        const src = r.result?.path
        if (src) {
          copyFileSync(src, dst)
          return dst
        }
      } catch (error) {
        copyError = String(error)
      }
      attempts.push({
        attempt: attempt + 1,
        ok: r?.ok === true,
        error: r?.error || null,
        resultKeys: r?.result && typeof r.result === 'object' ? Object.keys(r.result).sort() : [],
        mime: r?.result?.mime || null,
        hasBase64: typeof r?.result?.base64 === 'string' && r.result.base64.length > 0,
        hasPath: typeof r?.result?.path === 'string' && r.result.path.length > 0,
        copyError,
      })
    }
    // The installed runner removes its exact temp directory after the sweep.
    // Retain response-shape and copy/decode diagnostics beside private screen
    // evidence so a missing frame never degrades into an opaque `ssim=n/a`.
    try {
      const failureDir = join(screensDir, '_frame-failures')
      mkdirSync(failureDir, { recursive: true })
      writeFileSync(join(failureDir, `frame-${seq}.json`), `${JSON.stringify({
        atMs: at,
        attempts,
      }, null, 2)}\n`)
    } catch {
      // Evidence retention must never mask the original no-frame result.
    }
    return null
  }

  function ssim(a, b) {
    // Windows can transiently refuse/lock an ffmpeg child immediately after
    // render.frame closes its own encoder. A single missing process result made
    // a valid composed effect fail even though both retained JPEGs differed.
    // Retry only the evidence calculation; never reclick or mutate the product.
    let lastResult = null
    for (let attempt = 0; attempt < 3; attempt += 1) {
      const r = spawnSync(ffmpegBin,
        [
          '-i', a,
          '-i', b,
          '-filter_complex',
          '[0:v]scale=320:180,format=yuv420p[x];[1:v]scale=320:180,format=yuv420p[y];[x][y]ssim',
          '-f', 'null', '-',
        ],
        { encoding: 'utf8', timeout: 30_000 })
      lastResult = r
      const value = parseSsimAll(r.stderr)
      if (value != null) return value
    }
    // Preserve failure-only inputs and process diagnostics beside the private
    // screenshot evidence. Without these, the outer installed runner removes
    // its exact temp directory and an FFmpeg incompatibility becomes an opaque
    // `ssim=n/a` that can only be investigated by repeating a long UI section.
    const failureDir = join(screensDir, '_ssim-failures', `comparison-${frameSeq}`)
    try {
      mkdirSync(failureDir, { recursive: true })
      copyFileSync(a, join(failureDir, `before${extname(a) || '.bin'}`))
      copyFileSync(b, join(failureDir, `after${extname(b) || '.bin'}`))
      writeFileSync(join(failureDir, 'ffmpeg.json'), `${JSON.stringify({
        status: lastResult?.status ?? null,
        signal: lastResult?.signal ?? null,
        error: lastResult?.error ? String(lastResult.error) : null,
        stdout: lastResult?.stdout || '',
        stderr: lastResult?.stderr || '',
      }, null, 2)}\n`)
    } catch {
      // Evidence retention must never mask the original no-score result.
    }
    return null
  }

  function shotPath(surface, name) {
    const dir = join(screensDir, surface)
    mkdirSync(dir, { recursive: true })
    return join(dir, `${name.replace(/[^\w.-]+/g, '_')}.png`)
  }

  async function renderGroup(page, surface, groupName, locator) {
    const key = `${surface}/${groupName}`
    if (shotCache.has(key)) return shotCache.get(key)
    const path = shotPath(surface, groupName)
    let out
    // The caller may already have narrowed this locator with nth(). Calling
    // first() again is not neutral in the native adapter: it replaces that
    // index and can make visual proof inspect a different historical row.
    const el = locator
    if (!(await el.count())) {
      out = { ok: false, detail: 'group absent', shot: '' }
    } else {
      const visible = await el.isVisible().catch(() => false)
      const box = await el.boundingBox().catch(() => null)
      const nonZero = !!box && box.width > 1 && box.height > 1
      // Native WebViews can resize independently of the adapter's last
      // requested viewport. Judge geometry against the live CSS viewport so a
      // visibly rendered right-rail control is not rejected by stale bounds.
      const vp = await page.evaluate(() => ({
        width: Math.max(1, window.innerWidth || document.documentElement.clientWidth || 1),
        height: Math.max(1, window.innerHeight || document.documentElement.clientHeight || 1),
      })).catch(() => page.viewportSize() || { width: 1600, height: 900 })
      const onScreen = !!box && box.x + box.width > 0 && box.y + box.height > 0 && box.x < vp.width && box.y < vp.height
      try { await el.screenshot({ path }) } catch { await page.screenshot({ path }).catch(() => {}) }
      out = { ok: visible && nonZero && onScreen, detail: `vis=${visible} box=${box ? `${Math.round(box.width)}x${Math.round(box.height)}@${Math.round(box.x)},${Math.round(box.y)}` : 'none'} viewport=${Math.round(vp.width)}x${Math.round(vp.height)} onScreen=${onScreen}`, shot: path }
    }
    shotCache.set(key, out)
    return out
  }

  return { frame, ssim, renderGroup }
}
