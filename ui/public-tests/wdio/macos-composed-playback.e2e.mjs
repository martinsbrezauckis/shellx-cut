import assert from 'node:assert/strict'
import { mkdir } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'

const CLIP = process.env.SHELLX_CUT_WDIO_CLIP
const OUT_DIR = process.env.SHELLX_CUT_WDIO_OUT || join(tmpdir(), `shellx-cut-composed-playback-${Date.now()}`)

if (!CLIP) throw new Error('SHELLX_CUT_WDIO_CLIP must point to a real video clip on the Mac')

function sleep(ms) {
  return new Promise((done) => setTimeout(done, ms))
}

async function pageAsync(fn, ...args) {
  const result = await browser.executeAsync(fn, ...args)
  if (!result?.ok) throw new Error(result?.error || 'page eval failed')
  return result.value
}

async function verb(name, args = {}) {
  return pageAsync((verbName, verbArgs, done) => {
    fetch(`/api/verb/${verbName}`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(verbArgs),
    })
      .then((response) => response.text().then((text) => ({ response, text })))
      .then(({ response, text }) => {
        try { done({ ok: true, value: JSON.parse(text) }) }
        catch { done({ ok: true, value: { ok: false, error: { code: `http_${response.status}`, message: text } } }) }
      })
      .catch((error) => done({ ok: false, error: String(error?.stack || error?.message || error) }))
  }, name, args)
}

async function projectState() {
  const result = await verb('project.state', {})
  assert.equal(result.ok, true, `project.state failed: ${JSON.stringify(result.error ?? result)}`)
  return result.result
}

async function waitForState(predicate, timeoutMs = 30000) {
  const started = Date.now()
  let last = null
  while (Date.now() - started < timeoutMs) {
    last = await projectState()
    if (predicate(last)) return last
    await sleep(300)
  }
  throw new Error(`timed out waiting for project state: ${JSON.stringify(last)}`)
}

async function domState() {
  return pageAsync((done) => {
    const stage = document.querySelector('[data-cut-stage]')
    const video = document.querySelector('[data-cut-video]')
    const poster = document.querySelector('[data-cut-poster]')
    const composed = document.querySelector('[data-cut-quality-toggle]')
    const preview = document.querySelector('[data-cut-panel="preview"]')
    let posterLuma = null
    if (poster instanceof HTMLImageElement && poster.complete && poster.naturalWidth > 0) {
      try {
        const canvas = document.createElement('canvas')
        canvas.width = 32
        canvas.height = 18
        const context = canvas.getContext('2d', { willReadFrequently: true })
        context?.drawImage(poster, 0, 0, canvas.width, canvas.height)
        const pixels = context?.getImageData(0, 0, canvas.width, canvas.height).data
        if (pixels) {
          let total = 0
          for (let i = 0; i < pixels.length; i += 4) total += 0.2126 * pixels[i] + 0.7152 * pixels[i + 1] + 0.0722 * pixels[i + 2]
          posterLuma = total / (pixels.length / 4)
        }
      } catch { /* same-origin frame should be readable; null is reported if it is not */ }
    }
    done({
      ok: true,
      value: {
        surface: stage?.getAttribute('data-cut-preview-surface') || '',
        videoTime: video instanceof HTMLVideoElement ? video.currentTime : null,
        videoPaused: video instanceof HTMLVideoElement ? video.paused : null,
        videoFilter: video instanceof HTMLVideoElement ? video.style.filter : '',
        posterSrc: poster instanceof HTMLImageElement ? poster.src : '',
        posterReady: poster instanceof HTMLImageElement ? poster.complete && poster.naturalWidth > 0 : false,
        posterLuma,
        composed: composed?.getAttribute('data-cut-composed') || '',
        playing: preview?.getAttribute('data-cut-playing') || '',
      },
    })
  })
}

async function waitForDom(predicate, message, timeoutMs = 20000) {
  const started = Date.now()
  let last = null
  while (Date.now() - started < timeoutMs) {
    last = await domState()
    if (predicate(last)) return last
    await sleep(200)
  }
  throw new Error(`${message}; last=${JSON.stringify(last)}`)
}

describe('ShellX Cut macOS composed playback', () => {
  it('keeps a brightness-graded composed preview playing and returns to an exact frame on pause', async () => {
    await mkdir(OUT_DIR, { recursive: true })
    const projectName = `wdio-composed-playback-${Date.now()}`
    const projectDir = join(OUT_DIR, `${projectName}.cutproj`)
    try {
      const created = await verb('project.create', {
        name: projectName,
        dir: projectDir,
        settings: { width: 1280, height: 720, fps: 30 },
      })
      assert.equal(created.ok, true, `project.create failed: ${JSON.stringify(created.error ?? created)}`)

      const imported = await verb('media.import', {
        path: resolve(CLIP),
        proxy: false,
        rationale: 'macOS composed playback regression seed',
      })
      assert.equal(imported.ok, true, `media.import failed: ${JSON.stringify(imported.error ?? imported)}`)
      const state = await waitForState((project) =>
        (project.tracks || []).some((track) => track.kind === 'video' && (track.clips || []).some((clip) => clip.asset)),
      )
      const clip = state.tracks
        .find((track) => track.kind === 'video' && (track.clips || []).some((candidate) => candidate.asset))
        ?.clips.find((candidate) => candidate.asset)
      assert.ok(clip?.id, 'import created a video clip')

      await verb('ui.playhead', { at_ms: 500 })
      await waitForDom(
        (value) => value.surface === 'live-source' && typeof value.videoTime === 'number',
        'imported source was not playable before grading',
      )
      const graded = await verb('edit.grade', {
        clip: clip.id,
        contrast: 1,
        brightness: 0.25,
        saturation: 1,
        gamma: 1,
        rationale: 'macOS composed playback brightness regression',
      })
      assert.equal(graded.ok, true, `edit.grade failed: ${JSON.stringify(graded.error ?? graded)}`)
      await waitForState((project) => project.tracks
        .flatMap((track) => track.clips || [])
        .some((candidate) => candidate.id === clip.id && candidate.grade?.brightness === 0.25))
      await pageAsync((done) => {
        document.dispatchEvent(new CustomEvent('cut:show-composed'))
        done({ ok: true, value: true })
      })

      await waitForDom(
        (value) => value.composed === 'true' && value.surface === 'exact-frame' && value.posterSrc.includes('compose=1') && value.posterReady && value.posterLuma > 20,
        'paused graded preview did not settle on an exact composed frame',
        60000,
      )

      await $('[data-cut-transport-btn="play"]').click()
      const started = await waitForDom(
        (value) => value.surface === 'live-composite' && value.videoPaused === false && value.videoFilter.includes('brightness(1.25'),
        'graded composed preview did not switch to live playback',
      )
      const startTime = started.videoTime ?? 0
      const advanced = await waitForDom(
        (value) => typeof value.videoTime === 'number' && value.videoTime > startTime + 0.35,
        'graded composed video clock did not advance',
      )
      assert.ok(advanced.videoTime > startTime, `video time did not advance: ${startTime} -> ${advanced.videoTime}`)

      await $('[data-cut-transport-btn="play"]').click()
      await waitForDom(
        (value) => value.surface === 'exact-frame' && value.posterSrc.includes('compose=1') && value.posterReady && value.posterLuma > 20,
        'pausing did not restore the exact composed frame',
        60000,
      )
      await browser.saveScreenshot(join(OUT_DIR, 'macos-composed-playback.png'))
    } finally {
      await verb('project.close', {}).catch(() => {})
      await verb('project.delete', { path: projectDir }).catch(() => {})
      await verb('project.forget', { path: projectDir }).catch(() => {})
    }
  })
})
