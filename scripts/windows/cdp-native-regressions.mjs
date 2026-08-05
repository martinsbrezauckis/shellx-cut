#!/usr/bin/env node
// Focused native WebView2 regression gate for timeline interactions.
// Launch the installed app first with scripts/windows/launch-installed-cdp.mjs.

import assert from 'node:assert/strict'
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { spawnSync } from 'node:child_process'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..')

function arg(name, fallback = '') {
  const index = process.argv.indexOf(name)
  return index >= 0 && process.argv[index + 1] ? process.argv[index + 1] : fallback
}

function windowsEnvironmentPath(name, fallback) {
  if (!/^[A-Z_]+$/.test(name)) throw new Error(`Invalid Windows environment name: ${name}`)
  const result = spawnSync(
    'powershell.exe',
    ['-NoProfile', '-Command', `[Environment]::GetEnvironmentVariable('${name}')`],
    { encoding: 'utf8' },
  )
  const value = result.status === 0 ? String(result.stdout || '').trim() : ''
  return value || fallback
}

function psLiteral(value) {
  return `'${String(value).replace(/'/g, "''")}'`
}

function removeWindowsDir(path) {
  return spawnSync('powershell.exe', ['-NoProfile', '-ExecutionPolicy', 'Bypass', '-Command', [
    '$ErrorActionPreference = "Stop"',
    `$p = ${psLiteral(path)}`,
    'if (Test-Path -LiteralPath $p) { Remove-Item -LiteralPath $p -Recurse -Force }',
  ].join('; ')], { encoding: 'utf8' })
}

const CDP = arg('--cdp', process.env.CUT_CDP || 'http://127.0.0.1:9223').replace(/\/$/, '')
const USERPROFILE = windowsEnvironmentPath('USERPROFILE', 'C:\\Users\\Public')
const TEMP = windowsEnvironmentPath('TEMP', 'C:\\Windows\\Temp')
const MEDIA = arg('--media', process.env.CUT_TEST_MEDIA || `${USERPROFILE}\\Downloads\\talkinghead_hq.mp4`)
const OUT = resolve(arg('--out', join('/tmp', `shellx-cut-windows-regressions-${Date.now()}`)))
const VERSION = JSON.parse(readFileSync(join(ROOT, 'app/desktop/src-tauri/tauri.conf.json'), 'utf8')).version
const PROJECT_NAME = `windows-regressions-${Date.now()}`
const PROJECT_DIR = `${TEMP.replace(/[\\/]+$/, '')}\\${PROJECT_NAME}.cutproj`

mkdirSync(OUT, { recursive: true })

const sleep = (ms) => new Promise((done) => setTimeout(done, ms))
const results = []

function pass(name, detail = '') {
  results.push({ name, ok: true, detail })
  console.log(`PASS ${name}${detail ? `: ${detail}` : ''}`)
}

const targets = await (await fetch(`${CDP}/json/list`, { signal: AbortSignal.timeout(5000) })).json()
const target = targets.find((candidate) => candidate.type === 'page' && /127\.0\.0\.1:\d+/.test(candidate.url || ''))
if (!target) throw new Error(`No ShellX Cut WebView2 page exposed at ${CDP}`)

const ws = new WebSocket(target.webSocketDebuggerUrl)
let sequence = 0
const pending = new Map()
ws.addEventListener('message', (event) => {
  let message
  try { message = JSON.parse(String(event.data)) } catch { return }
  if (message?.id && pending.has(message.id)) {
    pending.get(message.id)(message)
    pending.delete(message.id)
  }
})
await new Promise((resolveOpen, rejectOpen) => {
  ws.addEventListener('open', resolveOpen, { once: true })
  ws.addEventListener('error', rejectOpen, { once: true })
})

function command(method, params = {}) {
  return new Promise((resolveCommand, rejectCommand) => {
    const id = ++sequence
    const timer = setTimeout(() => {
      pending.delete(id)
      rejectCommand(new Error(`CDP ${method} timed out`))
    }, 30000)
    pending.set(id, (message) => {
      clearTimeout(timer)
      if (message.error) rejectCommand(new Error(`CDP ${method}: ${message.error.message}`))
      else resolveCommand(message.result || {})
    })
    ws.send(JSON.stringify({ id, method, params }))
  })
}

async function evaluate(expression) {
  const response = await command('Runtime.evaluate', {
    expression,
    returnByValue: true,
    awaitPromise: true,
    userGesture: true,
  })
  if (response.exceptionDetails) {
    throw new Error(response.exceptionDetails.exception?.description || response.exceptionDetails.text || 'page evaluation failed')
  }
  return response.result?.value
}

async function page(fn, ...args) {
  return evaluate(`(${fn.toString()})(...${JSON.stringify(args)})`)
}

async function verb(name, args = {}) {
  return page(async (verbName, verbArgs) => {
    const response = await fetch(`/api/verb/${verbName}`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(verbArgs),
    })
    const text = await response.text()
    try { return JSON.parse(text) } catch { return { ok: false, error: { code: `http_${response.status}`, message: text } } }
  }, name, args)
}

async function state() {
  const response = await verb('project.state', {})
  assert.equal(response.ok, true, `project.state failed: ${JSON.stringify(response.error ?? response)}`)
  return response.result
}

async function waitFor(getValue, predicate, message, timeoutMs = 30000) {
  const started = Date.now()
  let last = null
  while (Date.now() - started < timeoutMs) {
    last = await getValue()
    if (predicate(last)) return last
    await sleep(250)
  }
  throw new Error(`${message}; last=${JSON.stringify(last).slice(0, 1400)}`)
}

async function waitState(predicate, message, timeoutMs = 30000) {
  return waitFor(state, predicate, message, timeoutMs)
}

async function rect(selector) {
  return page((value) => {
    const element = document.querySelector(value)
    if (!element) return null
    element.scrollIntoView({ block: 'center', inline: 'center' })
    const box = element.getBoundingClientRect()
    return { x: box.left, y: box.top, width: box.width, height: box.height }
  }, selector)
}

async function waitRect(selector, timeoutMs = 30000) {
  return waitFor(
    () => rect(selector),
    (value) => value && value.width > 0 && value.height > 0,
    `missing visible selector ${selector}`,
    timeoutMs,
  )
}

async function click(selector) {
  const box = await waitRect(selector)
  const x = box.x + box.width / 2
  const y = box.y + box.height / 2
  await command('Input.dispatchMouseEvent', { type: 'mousePressed', x, y, button: 'left', buttons: 1, clickCount: 1 })
  await command('Input.dispatchMouseEvent', { type: 'mouseReleased', x, y, button: 'left', buttons: 0, clickCount: 1 })
}

async function key(key, code, windowsVirtualKeyCode) {
  await command('Input.dispatchKeyEvent', { type: 'keyDown', key, code, windowsVirtualKeyCode, nativeVirtualKeyCode: windowsVirtualKeyCode })
  await command('Input.dispatchKeyEvent', { type: 'keyUp', key, code, windowsVirtualKeyCode, nativeVirtualKeyCode: windowsVirtualKeyCode })
}

async function drag(sourceSelector, targetSelector, offsetX = 0, targetYFactor = 0.5) {
  const source = await waitRect(sourceSelector)
  const targetBox = await waitRect(targetSelector)
  const start = { x: source.x + source.width / 2, y: source.y + source.height / 2 }
  const end = {
    x: targetBox.x + targetBox.width * 0.62 + offsetX,
    y: targetBox.y + targetBox.height * targetYFactor,
  }
  await command('Input.dispatchMouseEvent', { type: 'mouseMoved', ...start, button: 'none', buttons: 0 })
  await command('Input.dispatchMouseEvent', { type: 'mousePressed', ...start, button: 'left', buttons: 1, clickCount: 1 })
  for (let step = 1; step <= 8; step += 1) {
    const at = {
      x: start.x + ((end.x - start.x) * step) / 8,
      y: start.y + ((end.y - start.y) * step) / 8,
    }
    await command('Input.dispatchMouseEvent', { type: 'mouseMoved', ...at, button: 'left', buttons: 1 })
    await sleep(35)
  }
  await command('Input.dispatchMouseEvent', { type: 'mouseReleased', ...end, button: 'left', buttons: 0, clickCount: 1 })
}

async function dragClipLater(clipId, pixels = 140) {
  const box = await waitRect(`[data-cut-clip="${clipId}"]`)
  const start = { x: box.x + box.width / 2, y: box.y + box.height / 2 }
  const end = { x: start.x + pixels, y: start.y }
  await command('Input.dispatchMouseEvent', { type: 'mouseMoved', ...start, button: 'none', buttons: 0 })
  await command('Input.dispatchMouseEvent', { type: 'mousePressed', ...start, button: 'left', buttons: 1, clickCount: 1 })
  await sleep(80)
  await command('Input.dispatchMouseEvent', { type: 'mouseMoved', x: start.x + pixels * 0.55, y: start.y, button: 'left', buttons: 1 })
  await sleep(120)
  await command('Input.dispatchMouseEvent', { type: 'mouseMoved', ...end, button: 'left', buttons: 1 })
  await sleep(120)
  await command('Input.dispatchMouseEvent', { type: 'mouseReleased', ...end, button: 'left', buttons: 0, clickCount: 1 })
}

function clipDuration(clip) {
  if (clip.kind === 'gap') return Number(clip.duration_ms || 0)
  if (clip.kind === 'caption') return Math.max(0, Number(clip.range_ms?.[1] || 0) - Number(clip.range_ms?.[0] || 0))
  return Math.round((Number(clip.src_out_ms) - Number(clip.src_in_ms)) / Math.max(0.0001, Number(clip.speed || 1)))
}

function clipStart(project, trackId, clipId) {
  const track = project.tracks.find((candidate) => candidate.id === trackId)
  let cursor = 0
  for (const clip of track?.clips || []) {
    if (clip.id === clipId) return cursor
    cursor += clipDuration(clip)
  }
  return null
}

function findClip(project, clipId) {
  for (const track of project.tracks || []) {
    const clip = (track.clips || []).find((candidate) => candidate.id === clipId)
    if (clip) return { track, clip }
  }
  return null
}

function clipCount(project) {
  return (project.tracks || []).reduce((count, track) => count + (track.clips || []).filter((clip) => clip.kind !== 'gap').length, 0)
}

async function installFetchLog() {
  await page(() => {
    window.__cutWindowsVerbLog = []
    if (window.__cutWindowsOriginalFetch) return true
    window.__cutWindowsOriginalFetch = window.fetch.bind(window)
    window.fetch = (...args) => {
      const input = args[0]
      const url = typeof input === 'string' ? input : input?.url || ''
      const match = String(url).match(/\/api\/verb\/([^/?]+)/)
      let requestArgs = null
      if (match) {
        try { requestArgs = JSON.parse(args[1]?.body || '{}') } catch {}
      }
      return window.__cutWindowsOriginalFetch(...args).then((response) => {
        if (match) {
          response.clone().json().then((value) => window.__cutWindowsVerbLog.push({
            verb: decodeURIComponent(match[1]),
            args: requestArgs,
            ok: value?.ok,
            result: value?.result,
            error: value?.error,
          })).catch(() => {})
        }
        return response
      })
    }
    return true
  })
}

async function clearFetchLog() {
  await page(() => { window.__cutWindowsVerbLog = []; return true })
}

async function waitVerb(name, timeoutMs = 30000) {
  return waitFor(
    () => page((verbName) => (window.__cutWindowsVerbLog || []).filter((entry) => entry.verb === verbName).at(-1) || null, name),
    Boolean,
    `no ${name} response observed`,
    timeoutMs,
  )
}

async function previewState() {
  return page(() => {
    const stage = document.querySelector('[data-cut-stage]')
    const video = document.querySelector('[data-cut-video]')
    const poster = document.querySelector('[data-cut-poster]')
    const quality = document.querySelector('[data-cut-quality-toggle]')
    const preview = document.querySelector('[data-cut-panel="preview"]')
    return {
      surface: stage?.getAttribute('data-cut-preview-surface') || '',
      composed: quality?.getAttribute('data-cut-composed') || '',
      videoTime: video instanceof HTMLVideoElement ? video.currentTime : null,
      videoPaused: video instanceof HTMLVideoElement ? video.paused : null,
      videoFilter: video instanceof HTMLVideoElement ? video.style.filter : '',
      posterReady: poster instanceof HTMLImageElement ? poster.complete && poster.naturalWidth > 0 : false,
      posterSrc: poster instanceof HTMLImageElement ? poster.src : '',
      playing: preview?.getAttribute('data-cut-playing') || '',
    }
  })
}

async function seekPlayableVideo(preferredMs, durationMs) {
  const quality = await previewState()
  if (quality.composed === 'true') {
    await click('[data-cut-quality-toggle]')
    await waitFor(previewState, (value) => value.composed !== 'true', 'preview did not switch back to source mode')
  }
  const upper = Math.max(1, Math.round(durationMs))
  const step = Math.max(250, Math.round(upper / 48))
  const candidates = [Math.max(0, Math.min(upper - 1, Math.round(preferredMs)))]
  for (let at = 0; at < upper; at += step) candidates.push(at)
  for (const at of [...new Set(candidates)]) {
    const moved = await verb('ui.playhead', { at_ms: at })
    if (!moved.ok) continue
    await sleep(180)
    const preview = await previewState()
    if (preview.surface === 'live-source' && typeof preview.videoTime === 'number') return at
  }
  throw new Error(`no playable base-video time found across 0..${upper}ms`)
}

let libraryId = null
let failure = null
try {
  const doctor = await verb('system.doctor', {})
  assert.equal(doctor.ok, true, `system.doctor failed: ${JSON.stringify(doctor.error ?? doctor)}`)
  assert.match(String(doctor.result?.app_version || ''), new RegExp(`^${String(VERSION).replace(/\./g, '\\.')}`))
  pass('installed-version', doctor.result.app_version)

  const created = await verb('project.create', {
    name: PROJECT_NAME,
    dir: PROJECT_DIR,
    settings: { width: 1280, height: 720, fps: 30 },
  })
  assert.equal(created.ok, true, `project.create failed: ${JSON.stringify(created.error ?? created)}`)
  await installFetchLog()

  const imported = await verb('media.import', { path: MEDIA, proxy: false, rationale: 'Windows native regression seed' })
  assert.equal(imported.ok, true, `media.import failed: ${JSON.stringify(imported.error ?? imported)}`)
  const seeded = await waitState((project) => {
    const video = (project.tracks || []).find((track) => track.kind === 'video' && (track.clips || []).some((clip) => clip.asset))
    const audio = (project.tracks || []).find((track) => track.kind === 'audio' && (track.clips || []).some((clip) => clip.asset))
    return video && audio
  }, 'import did not create linked video/audio clips', 60000)
  const videoTrack = seeded.tracks.find((track) => track.kind === 'video' && track.clips.some((clip) => clip.asset))
  const audioTrack = seeded.tracks.find((track) => track.kind === 'audio' && track.clips.some((clip) => clip.asset))
  const videoBefore = videoTrack.clips.find((clip) => clip.asset)
  const audioBefore = audioTrack.clips.find((clip) => clip.asset)
  assert.equal(videoBefore.asset, audioBefore.asset)
  assert.deepEqual([videoBefore.src_in_ms, videoBefore.src_out_ms], [audioBefore.src_in_ms, audioBefore.src_out_ms])
  pass('default-import-linked-av', `${videoBefore.id}/${audioBefore.id}`)

  await clearFetchLog()
  await dragClipLater(videoBefore.id)
  const move = await waitVerb('edit.move')
  assert.equal(move.ok, true, `UI edit.move failed: ${JSON.stringify(move.error ?? move)}`)
  assert.equal(move.args?.linked, true, 'timeline drag did not request linked movement')
  const moved = await waitState((project) => {
    const videoStart = clipStart(project, videoTrack.id, videoBefore.id)
    const audioStart = clipStart(project, audioTrack.id, audioBefore.id)
    return videoStart > 0 && videoStart === audioStart
  }, 'timeline drag did not move linked A/V together')
  const movedStart = clipStart(moved, videoTrack.id, videoBefore.id)
  pass('timeline-drag-keeps-av-linked', `${movedStart}ms`)

  const initialVideoTracks = moved.tracks.filter((track) => track.kind === 'video').length
  const initialAudioTracks = moved.tracks.filter((track) => track.kind === 'audio').length
  await click('[data-cut-action="add-video-track"]')
  await waitState((project) => project.tracks.filter((track) => track.kind === 'video').length === initialVideoTracks + 1, 'video track button did not add a track')
  await click('[data-cut-action="add-audio-track"]')
  await waitState((project) => project.tracks.filter((track) => track.kind === 'audio').length === initialAudioTracks + 1, 'audio track button did not add a track')
  pass('discoverable-add-track-controls')

  let current = await state()
  let currentVideo = findClip(current, videoBefore.id).clip
  const qAt = clipStart(current, videoTrack.id, videoBefore.id) + Math.round(clipDuration(currentVideo) * 0.25)
  assert.equal((await verb('ui.playhead', { at_ms: qAt })).ok, true)
  await clearFetchLog()
  await key('q', 'KeyQ', 81)
  const qTrim = await waitVerb('edit.trim')
  assert.equal(qTrim.ok, true, `Q edit.trim failed: ${JSON.stringify(qTrim.error ?? qTrim)}`)
  assert.equal(qTrim.args?.linked, true)
  current = await waitState((project) => {
    const video = findClip(project, videoBefore.id)?.clip
    const audio = findClip(project, audioBefore.id)?.clip
    return Number(video?.src_in_ms) > Number(videoBefore.src_in_ms)
      && Number(audio?.src_in_ms) === Number(video?.src_in_ms)
  }, 'Q did not trim the linked source heads together')
  pass('q-ripple-trim-linked-start')

  currentVideo = findClip(current, videoBefore.id).clip
  const wAt = clipStart(current, videoTrack.id, videoBefore.id) + Math.round(clipDuration(currentVideo) * 0.5)
  assert.equal((await verb('ui.playhead', { at_ms: wAt })).ok, true)
  await clearFetchLog()
  await key('w', 'KeyW', 87)
  const wTrim = await waitVerb('edit.trim')
  assert.equal(wTrim.ok, true, `W edit.trim failed: ${JSON.stringify(wTrim.error ?? wTrim)}`)
  assert.equal(wTrim.args?.linked, true)
  current = await waitState((project) => {
    const video = findClip(project, videoBefore.id)?.clip
    const audio = findClip(project, audioBefore.id)?.clip
    return Number(video?.src_out_ms) < Number(currentVideo.src_out_ms)
      && Number(audio?.src_out_ms) === Number(video?.src_out_ms)
  }, 'W did not trim the linked source tails together')
  assert.equal(current.tracks.filter((track) => track.kind === 'video').length, initialVideoTracks + 1)
  assert.equal(current.tracks.filter((track) => track.kind === 'audio').length, initialAudioTracks + 1)
  pass('w-ripple-trim-linked-end-and-preserve-empty-tracks')

  const assetDuration = Number(current.assets?.[videoBefore.asset]?.probe?.duration_ms || 0)
  assert.ok(assetDuration > 1000, `test asset duration is unavailable: ${assetDuration}`)
  const baseTrackEnd = current.tracks
    .find((track) => track.id === videoTrack.id)
    ?.clips.reduce((sum, clip) => sum + clipDuration(clip), 0) ?? 0
  const playbackSeed = await verb('edit.insert', {
    asset: videoBefore.asset,
    track: videoTrack.id,
    at_ms: baseTrackEnd,
    src_range_ms: [0, assetDuration],
    ripple: false,
    rationale: 'Windows composed playback full-length seed',
  })
  assert.equal(playbackSeed.ok, true, `playback seed insert failed: ${JSON.stringify(playbackSeed.error ?? playbackSeed)}`)
  const playbackClipId = playbackSeed.result?.clip_id
  assert.ok(playbackClipId, 'playback seed insert returned no clip id')
  current = await waitState((project) => !!findClip(project, playbackClipId), 'playback seed clip did not land')
  const graded = await verb('edit.grade', {
    clip: playbackClipId,
    contrast: 1,
    brightness: 0.25,
    saturation: 1,
    gamma: 1,
    rationale: 'Windows composed playback brightness regression',
  })
  assert.equal(graded.ok, true, `edit.grade failed: ${JSON.stringify(graded.error ?? graded)}`)
  const gradedClip = findClip(current, playbackClipId).clip
  const preferredPreviewAt = clipStart(current, videoTrack.id, playbackClipId) + Math.min(1000, Math.max(1, clipDuration(gradedClip) / 3))
  const videoTrackDuration = current.tracks
    .filter((track) => track.kind === 'video')
    .reduce((maximum, track) => Math.max(maximum, (track.clips || []).reduce((sum, clip) => sum + clipDuration(clip), 0)), 0)
  const previewAt = await seekPlayableVideo(preferredPreviewAt, videoTrackDuration)
  await page(() => { document.dispatchEvent(new CustomEvent('cut:show-composed')); return true })
  await waitFor(
    previewState,
    (value) => value.composed === 'true' && value.surface === 'exact-frame' && value.posterReady && value.posterSrc.includes('compose=1'),
    'graded composed preview did not settle on an exact frame',
    60000,
  )
  await click('[data-cut-transport-btn="play"]')
  const started = await waitFor(
    previewState,
    (value) => value.surface === 'live-composite' && value.videoPaused === false && value.videoFilter.includes('brightness(1.25'),
    'graded composed preview did not enter live playback',
    30000,
  )
  await waitFor(previewState, (value) => Number(value.videoTime) > Number(started.videoTime) + 0.3, 'composed video clock did not advance')
  await click('[data-cut-transport-btn="play"]')
  await waitFor(previewState, (value) => value.surface === 'exact-frame' && value.posterReady, 'pause did not restore an exact composed frame', 60000)
  pass('brightness-graded-composed-playback')

  await click('[data-cut-left-tab="assets"]')
  const assetId = Object.keys(current.assets || {})[0]
  const beforeAssetDrop = clipCount(await state())
  await drag(`[data-cut-asset-card="${assetId}"] .assets__icon, [data-cut-asset-card="${assetId}"] .assets__thumb`, '[data-cut-timeline-scroll]', 0, 0.38)
  await waitState((project) => clipCount(project) > beforeAssetDrop, 'Assets drag did not add timeline media', 60000)
  pass('assets-drag-to-timeline')

  const libraryAdded = await verb('library.add', { path: MEDIA, name: `windows-regression-${Date.now()}` })
  assert.equal(libraryAdded.ok, true, `library.add failed: ${JSON.stringify(libraryAdded.error ?? libraryAdded)}`)
  libraryId = libraryAdded.result?.item?.id
  assert.ok(libraryId, 'library.add returned no item id')
  await click('[data-cut-left-tab="library"]')
  const beforeLibraryDrop = clipCount(await state())
  await drag(`[data-cut-library-card="${libraryId}"] .lb-thumb-img, [data-cut-library-card="${libraryId}"] .lb-thumb-glyph`, '[data-cut-timeline-scroll]', 0, 0.38)
  await waitState((project) => clipCount(project) > beforeLibraryDrop, 'Library drag did not add timeline media', 90000)
  pass('library-drag-to-timeline')

  const screenshot = await command('Page.captureScreenshot', { format: 'png', captureBeyondViewport: false })
  writeFileSync(join(OUT, 'windows-native-regressions.png'), Buffer.from(screenshot.data, 'base64'))
} catch (error) {
  failure = error
  results.push({ name: 'run', ok: false, detail: String(error?.stack || error) })
  console.error(`FAIL ${error?.stack || error}`)
} finally {
  await verb('project.close', {}).catch(() => {})
  if (libraryId) await verb('library.remove', { id: libraryId }).catch(() => {})
  await verb('project.delete', { path: PROJECT_DIR }).catch(() => {})
  await verb('project.forget', { path: PROJECT_DIR }).catch(() => {})
  removeWindowsDir(PROJECT_DIR)
  ws.close()
  writeFileSync(join(OUT, 'receipt.json'), `${JSON.stringify({
    schema: 'shellx-cut/windows-native-regressions@1',
    ok: !failure,
    version: VERSION,
    cdp: CDP,
    media: MEDIA,
    project_dir: PROJECT_DIR,
    results,
  }, null, 2)}\n`)
}

console.log(`RECEIPT ${join(OUT, 'receipt.json')}`)
if (failure) process.exit(1)
