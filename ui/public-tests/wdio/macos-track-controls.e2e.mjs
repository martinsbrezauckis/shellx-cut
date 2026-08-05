import assert from 'node:assert/strict'
import { mkdir } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'

const CLIP = process.env.SHELLX_CUT_WDIO_CLIP
const OUT_DIR = process.env.SHELLX_CUT_WDIO_OUT || join(tmpdir(), `shellx-cut-wdio-${Date.now()}`)

if (!CLIP) {
  throw new Error('SHELLX_CUT_WDIO_CLIP must point to a real video clip on the Mac')
}

function sleep(ms) {
  return new Promise((r) => setTimeout(r, ms))
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
      .then((res) => res.text().then((text) => ({ res, text })))
      .then(({ res, text }) => {
        try {
          done({ ok: true, value: JSON.parse(text) })
        } catch {
          done({ ok: true, value: { ok: false, error: { code: `http_${res.status}`, message: text } } })
        }
      })
      .catch((err) => done({ ok: false, error: String(err?.stack || err?.message || err) }))
  }, name, args)
}

async function projectState() {
  const res = await verb('project.state', {})
  assert.equal(res.ok, true, `project.state failed: ${JSON.stringify(res.error ?? res)}`)
  return res.result
}

async function uiState() {
  const res = await verb('ui.state', {})
  assert.equal(res.ok, true, `ui.state failed: ${JSON.stringify(res.error ?? res)}`)
  return res.result
}

async function opsLen() {
  const res = await verb('project.ops', {})
  assert.equal(res.ok, true, `project.ops failed: ${JSON.stringify(res.error ?? res)}`)
  return res.result?.ops?.length ?? 0
}

async function waitForState(predicate, timeoutMs = 30000) {
  const start = Date.now()
  let last = null
  while (Date.now() - start < timeoutMs) {
    const state = await projectState()
    last = state
    if (predicate(state)) return state
    await sleep(350)
  }
  const summary = {
    name: last?.name,
    tracks: (last?.tracks || []).map((track) => ({
      id: track.id,
      kind: track.kind,
      muted: track.muted,
      solo: track.solo,
      visible: track.visible,
      locked: track.locked,
      pan: track.pan,
      gain_db: track.gain_db,
      clips: (track.clips || []).length,
    })),
    op_tail: (last?.ops || []).slice(-5).map((op) => ({ id: op.id, verb: op.verb, args: op.args })),
  }
  throw new Error(`timed out waiting for project state; last=${JSON.stringify(summary)}`)
}

async function installVerbLog() {
  await pageAsync((done) => {
    const w = window
    w.__cutWdioVerbLog = []
    if (!w.__cutWdioFetchOriginal) {
      w.__cutWdioFetchOriginal = w.fetch.bind(w)
      w.fetch = (...args) => {
        const input = args[0]
        const url = typeof input === 'string' ? input : input?.url || ''
        const match = String(url).match(/\/api\/verb\/([^/?]+)/)
        let requestArgs = null
        if (match) {
          try {
            const body = args[1]?.body ?? (typeof input === 'object' ? input?.body : null)
            requestArgs = typeof body === 'string' ? JSON.parse(body) : null
          } catch {}
        }
        return w.__cutWdioFetchOriginal(...args).then((res) => {
          if (match) {
            const verbName = decodeURIComponent(match[1])
            res.clone().text()
              .then((text) => {
                let json = null
                try { json = JSON.parse(text) } catch {}
                w.__cutWdioVerbLog.push({ verb: verbName, args: requestArgs, status: res.status, ok: json?.ok, result: json?.result, error: json?.error })
              })
              .catch((err) => {
                w.__cutWdioVerbLog.push({ verb: verbName, args: requestArgs, status: res.status, error: String(err?.message || err) })
              })
          }
          return res
        })
      }
    }
    done({ ok: true, value: true })
  })
}

async function waitForVerbLog(verbName, timeoutMs = 60000) {
  let last = []
  await browser.waitUntil(async () => {
    last = await pageAsync((name, done) => {
      const entries = (window.__cutWdioVerbLog || []).filter((entry) => entry.verb === name)
      done({ ok: true, value: entries })
    }, verbName)
    return last.length > 0
  }, { timeout: timeoutMs, timeoutMsg: `no ${verbName} response observed; last=${JSON.stringify(last).slice(0, 900)}` })
  return last[last.length - 1]
}

async function waitForSelector(selector, timeoutMs = 30000) {
  await browser.waitUntil(async () => {
    return pageAsync((sel, done) => {
      done({ ok: true, value: !!document.querySelector(sel) })
    }, selector)
  }, { timeout: timeoutMs, timeoutMsg: `missing selector ${selector}` })
}

async function waitForEnabled(selector, timeoutMs = 30000) {
  await waitForSelector(selector, timeoutMs)
  await browser.waitUntil(async () => {
    return pageAsync((sel, done) => {
      const el = document.querySelector(sel)
      done({ ok: true, value: !(el instanceof HTMLButtonElement || el instanceof HTMLInputElement || el instanceof HTMLSelectElement) || !el.disabled })
    }, selector)
  }, { timeout: timeoutMs, timeoutMsg: `selector stayed disabled ${selector}` })
}

async function clickOnly(selector, timeoutMs = 30000) {
  await waitForEnabled(selector, timeoutMs)
  await pageAsync((sel, done) => {
    const el = document.querySelector(sel)
    if (!el) {
      done({ ok: false, error: `missing selector ${sel}` })
      return
    }
    el.scrollIntoView({ block: 'center', inline: 'center' })
    el.dispatchEvent(new MouseEvent('mousedown', { bubbles: true, cancelable: true, view: window }))
    el.dispatchEvent(new MouseEvent('mouseup', { bubbles: true, cancelable: true, view: window }))
    el.dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true, view: window }))
    done({ ok: true, value: true })
  }, selector)
}

async function clickAndWait(selector, predicate, timeoutMs = 30000) {
  await clickOnly(selector, timeoutMs)
  return waitForState(predicate, timeoutMs)
}

async function seekUi(atMs) {
  const moved = await verb('ui.playhead', { at_ms: Math.round(atMs) })
  assert.equal(moved.ok, true, `ui.playhead failed: ${JSON.stringify(moved.error ?? moved)}`)
  await browser.waitUntil(async () => Math.abs(Number((await uiState())?.playhead_ms ?? -1) - Math.round(atMs)) <= 1, {
    timeout: 10000,
    timeoutMsg: `playhead did not move to ${Math.round(atMs)}ms`,
  })
}

async function pressEditorKey(key) {
  await pageAsync((value, done) => {
    document.body.dispatchEvent(new KeyboardEvent('keydown', {
      key: value,
      code: `Key${String(value).toUpperCase()}`,
      bubbles: true,
      cancelable: true,
    }))
    done({ ok: true, value: true })
  }, key)
}

async function setSelectValue(selector, value) {
  await waitForSelector(selector)
  await pageAsync((sel, nextValue, done) => {
    const el = document.querySelector(sel)
    if (!(el instanceof HTMLSelectElement)) {
      done({ ok: false, error: `selector is not a select: ${sel}` })
      return
    }
    el.scrollIntoView({ block: 'center', inline: 'center' })
    el.value = String(nextValue)
    el.dispatchEvent(new Event('input', { bubbles: true }))
    el.dispatchEvent(new Event('change', { bubbles: true }))
    done({ ok: true, value: true })
  }, selector, value)
}

async function getAttribute(selector, name) {
  return pageAsync((sel, attr, done) => {
    done({ ok: true, value: document.querySelector(sel)?.getAttribute(attr) || '' })
  }, selector, name)
}

async function waitForAttribute(selector, name, expected, timeoutMs = 10000) {
  await browser.waitUntil(async () => (await getAttribute(selector, name)) === expected, {
    timeout: timeoutMs,
    timeoutMsg: `${selector} ${name} did not become ${JSON.stringify(expected)}`,
  })
}

describe('ShellX Cut macOS track controls', () => {
  it('drives add-track, Q/W linked trims, visibility, lock, mute, solo, listen, and pan in the native WKWebView', async () => {
    await mkdir(OUT_DIR, { recursive: true })
    await waitForSelector('[data-cut-panel="topbar"]')
    await waitForSelector('[data-cut-panel="timeline"]')

    const projectName = `wdio-track-controls-${Date.now()}`
    const projectDir = join(OUT_DIR, `${projectName}.cutproj`)
    const created = await verb('project.create', {
      name: projectName,
      dir: projectDir,
      settings: { width: 1280, height: 720, fps: 30 },
    })
    assert.equal(created.ok, true, `project.create failed: ${JSON.stringify(created.error ?? created)}`)

    const imported = await verb('media.import', {
      path: resolve(CLIP),
      proxy: false,
      rationale: 'macOS WDIO track controls seed media',
    })
    assert.equal(imported.ok, true, `media.import failed: ${JSON.stringify(imported.error ?? imported)}`)

    const seeded = await waitForState((state) =>
      (state.tracks || []).some((track) => track.kind === 'video' && (track.clips || []).some((clip) => clip.asset)) &&
      (state.tracks || []).some((track) => track.kind === 'audio' && (track.clips || []).some((clip) => clip.asset)),
    )
    const videoTrack = seeded.tracks.find((track) => track.kind === 'video' && (track.clips || []).some((clip) => clip.asset))
    const audioTrack = seeded.tracks.find((track) => track.kind === 'audio' && (track.clips || []).some((clip) => clip.asset))
    assert.ok(videoTrack?.id, 'seeded project has a video track')
    assert.ok(audioTrack?.id, 'seeded project has an audio track')

    const videoCount = seeded.tracks.filter((track) => track.kind === 'video').length
    const audioCount = seeded.tracks.filter((track) => track.kind === 'audio').length
    await clickAndWait(
      '[data-cut-action="add-video-track"]',
      (state) => state.tracks.filter((track) => track.kind === 'video').length === videoCount + 1,
    )
    await clickAndWait(
      '[data-cut-action="add-audio-track"]',
      (state) => state.tracks.filter((track) => track.kind === 'audio').length === audioCount + 1,
    )

    const videoClipBefore = videoTrack.clips.find((clip) => clip.asset)
    const audioClipBefore = audioTrack.clips.find((clip) => clip.asset)
    assert.ok(videoClipBefore?.id && audioClipBefore?.id, 'seeded linked clips have ids')
    const durationMs = Number(videoClipBefore.src_out_ms) - Number(videoClipBefore.src_in_ms)
    assert.ok(durationMs >= 1000, `test clip is long enough for Q/W trims (${durationMs}ms)`)

    const qAtMs = Math.max(1, Math.round(durationMs * 0.25))
    await seekUi(qAtMs)
    await installVerbLog()
    await pressEditorKey('q')
    const qTrim = await waitForVerbLog('edit.trim')
    assert.equal(qTrim.ok, true, `Q edit.trim failed: ${JSON.stringify(qTrim.error ?? qTrim)}`)
    assert.equal(qTrim.args?.clip, videoClipBefore.id, 'Q targets the active program video clip')
    assert.equal(qTrim.args?.linked, true, 'Q explicitly requests linked-media trim')
    assert.equal(qTrim.result?.linked, true, 'Q resolves an exact linked counterpart')
    assert.equal(qTrim.result?.linked_clip, audioClipBefore.id, 'Q trims the imported audio counterpart')
    const afterQ = await waitForState((state) => {
      const video = state.tracks.flatMap((track) => track.clips || []).find((clip) => clip.id === videoClipBefore.id)
      const audio = state.tracks.flatMap((track) => track.clips || []).find((clip) => clip.id === audioClipBefore.id)
      return Number(video?.src_in_ms) > Number(videoClipBefore.src_in_ms)
        && Number(audio?.src_in_ms) === Number(video?.src_in_ms)
    })
    const videoAfterQ = afterQ.tracks.flatMap((track) => track.clips || []).find((clip) => clip.id === videoClipBefore.id)
    const audioAfterQ = afterQ.tracks.flatMap((track) => track.clips || []).find((clip) => clip.id === audioClipBefore.id)
    assert.deepEqual(
      [videoAfterQ.src_in_ms, videoAfterQ.src_out_ms],
      [audioAfterQ.src_in_ms, audioAfterQ.src_out_ms],
      'Q keeps linked video/audio source windows identical',
    )

    const remainingMs = Number(videoAfterQ.src_out_ms) - Number(videoAfterQ.src_in_ms)
    const wAtMs = Math.max(1, Math.round(remainingMs * 0.5))
    await seekUi(wAtMs)
    await installVerbLog()
    await pressEditorKey('w')
    const wTrim = await waitForVerbLog('edit.trim')
    assert.equal(wTrim.ok, true, `W edit.trim failed: ${JSON.stringify(wTrim.error ?? wTrim)}`)
    assert.equal(wTrim.args?.clip, videoClipBefore.id, 'W targets the active program video clip')
    assert.equal(wTrim.result?.linked, true, 'W resolves an exact linked counterpart')
    const afterW = await waitForState((state) => {
      const video = state.tracks.flatMap((track) => track.clips || []).find((clip) => clip.id === videoClipBefore.id)
      const audio = state.tracks.flatMap((track) => track.clips || []).find((clip) => clip.id === audioClipBefore.id)
      return Number(video?.src_out_ms) < Number(videoAfterQ.src_out_ms)
        && Number(audio?.src_out_ms) === Number(video?.src_out_ms)
    })
    const videoAfterW = afterW.tracks.flatMap((track) => track.clips || []).find((clip) => clip.id === videoClipBefore.id)
    const audioAfterW = afterW.tracks.flatMap((track) => track.clips || []).find((clip) => clip.id === audioClipBefore.id)
    assert.deepEqual(
      [videoAfterW.src_in_ms, videoAfterW.src_out_ms],
      [audioAfterW.src_in_ms, audioAfterW.src_out_ms],
      'W keeps linked video/audio source windows identical',
    )
    assert.equal(afterW.tracks.filter((track) => track.kind === 'video').length, videoCount + 1, 'Q/W preserve the user-created empty video track')
    assert.equal(afterW.tracks.filter((track) => track.kind === 'audio').length, audioCount + 1, 'Q/W preserve the user-created empty audio track')

    await waitForSelector(`[data-cut-track="${videoTrack.id}"] [data-cut-action="toggle-track-visibility"]`)
    await waitForSelector(`[data-cut-track="${videoTrack.id}"] [data-cut-action="toggle-track-lock"]`)
    await waitForSelector(`[data-cut-track="${audioTrack.id}"] [data-cut-action="toggle-mute"]`)
    await waitForSelector(`[data-cut-track="${audioTrack.id}"] [data-cut-action="toggle-solo"]`)
    await waitForSelector(`[data-cut-track="${audioTrack.id}"] [data-cut-action="track-listen"]`)
    await waitForSelector(`[data-cut-track="${audioTrack.id}"] [data-cut-action="set-pan"]`)

    await clickAndWait(
      `[data-cut-track="${videoTrack.id}"] [data-cut-action="toggle-track-visibility"]`,
      (state) => state.tracks.find((track) => track.id === videoTrack.id)?.visible === false,
    )
    await waitForAttribute(`[data-cut-track="${videoTrack.id}"]`, 'data-cut-track-visible', 'false')

    await clickAndWait(
      `[data-cut-track="${videoTrack.id}"] [data-cut-action="toggle-track-lock"]`,
      (state) => state.tracks.find((track) => track.id === videoTrack.id)?.locked === true,
    )
    await waitForAttribute(`[data-cut-track="${videoTrack.id}"]`, 'data-cut-track-locked', 'true')

    const muteButton = `[data-cut-track="${audioTrack.id}"] [data-cut-action="toggle-mute"]`
    const gainBeforeMute = seeded.tracks.find((track) => track.id === audioTrack.id)?.gain_db ?? 0
    await clickAndWait(
      muteButton,
      (state) => state.tracks.find((track) => track.id === audioTrack.id)?.muted === true,
    )
    const muted = await projectState()
    const mutedTrack = muted.tracks.find((track) => track.id === audioTrack.id)
    assert.equal(mutedTrack?.muted, true, 'audio track mute flag is set')
    assert.equal(mutedTrack?.gain_db ?? 0, gainBeforeMute, 'mute does not rewrite track gain')
    await waitForAttribute(muteButton, 'data-cut-muted', 'true')
    await installVerbLog()
    await clickOnly(muteButton)
    const muteOff = await waitForVerbLog('edit.mute')
    assert.equal(muteOff.args?.on, false, `second mute click sent wrong args: ${JSON.stringify(muteOff)}`)
    assert.equal(muteOff.ok, true, `second mute click edit.mute failed: ${JSON.stringify(muteOff.error ?? muteOff)}`)
    assert.equal(muteOff.result?.muted, false, `second mute click did not clear mute: ${JSON.stringify(muteOff)}`)
    await waitForState(
      (state) => state.tracks.find((track) => track.id === audioTrack.id)?.muted !== true,
    )
    const unmuted = await projectState()
    const unmutedTrack = unmuted.tracks.find((track) => track.id === audioTrack.id)
    assert.equal(unmutedTrack?.muted === true, false, 'audio track mute flag clears on second click')
    assert.equal(unmutedTrack?.gain_db ?? 0, gainBeforeMute, 'unmute preserves track gain')
    await waitForAttribute(muteButton, 'data-cut-muted', '')

    const soloButton = `[data-cut-track="${audioTrack.id}"] [data-cut-action="toggle-solo"]`
    await clickAndWait(
      soloButton,
      (state) => state.tracks.find((track) => track.id === audioTrack.id)?.solo === true,
    )
    const soloed = await projectState()
    assert.equal(soloed.tracks.find((track) => track.id === audioTrack.id)?.solo, true, 'audio track solo flag is set')
    await waitForAttribute(soloButton, 'data-cut-soloed', 'true')

    await setSelectValue(`[data-cut-track="${audioTrack.id}"] [data-cut-action="set-pan"]`, -0.5)
    const panned = await waitForState((state) => {
      const track = state.tracks.find((candidate) => candidate.id === audioTrack.id)
      return Math.abs(Number(track?.pan ?? 0) - -0.5) < 0.01
    })
    assert.equal(Math.round((panned.tracks.find((track) => track.id === audioTrack.id)?.pan ?? 0) * 10), -5)

    await installVerbLog()
    const listenButton = `[data-cut-track="${audioTrack.id}"] [data-cut-action="track-listen"]`
    await waitForSelector(listenButton)
    const opsBeforeListen = await opsLen()
    await pageAsync((sel, done) => {
      const el = document.querySelector(sel)
      if (!el) {
        done({ ok: false, error: `missing selector ${sel}` })
        return
      }
      el.scrollIntoView({ block: 'center', inline: 'center' })
      el.dispatchEvent(new MouseEvent('mousedown', { bubbles: true, cancelable: true, view: window }))
      el.dispatchEvent(new MouseEvent('mouseup', { bubbles: true, cancelable: true, view: window }))
      el.dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true, view: window }))
      done({ ok: true, value: true })
    }, listenButton)
    const exportAudio = await waitForVerbLog('export.audio')
    assert.equal(exportAudio.ok, true, `track listen export.audio failed: ${JSON.stringify(exportAudio.error ?? exportAudio)}`)
    assert.ok(exportAudio.result?.path, 'track listen produced an exported audio stem path')
    assert.equal(await opsLen(), opsBeforeListen, 'track listen does not mutate the timeline op log')

    const screenshot = join(OUT_DIR, 'macos-track-controls.png')
    await browser.saveScreenshot(screenshot)
  })
})
