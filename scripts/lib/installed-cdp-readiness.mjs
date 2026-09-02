import { existsSync, readFileSync, statSync } from 'node:fs'

export const TEST_OWNED_SLOW_FFMPEG_MARKER_SCHEMA = 'shellx-cut/test-owned-slow-ffmpeg-marker@1'

function assert(condition, message) {
  if (!condition) throw new Error(message)
}

export const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

export async function getJson(url, timeoutMs = 3000) {
  const response = await fetch(url, {
    headers: { connection: 'close' },
    signal: AbortSignal.timeout(timeoutMs),
  })
  if (!response.ok) throw new Error(`${url} returned ${response.status}`)
  return response.json()
}

export function installedCdpPage(targets) {
  assert(Array.isArray(targets), 'installed CDP target list is not an array')
  return targets.find((target) => target?.type === 'page'
    && /127[.]0[.]0[.]1:\d+/.test(target.url || '')) || null
}

export async function waitForInstalledCdpPage({
  cdpBase,
  timeoutMs,
  getJsonImpl = getJson,
  pollMs = 250,
  now = Date.now,
  sleepImpl = sleep,
} = {}) {
  assert(typeof cdpBase === 'string' && cdpBase.length > 0, 'CDP base is required')
  assert(Number.isSafeInteger(timeoutMs) && timeoutMs > 0, 'CDP readiness timeout must be a positive integer')
  assert(Number.isSafeInteger(pollMs) && pollMs > 0, 'CDP readiness poll interval must be a positive integer')
  const startedAt = now()
  let last = ''
  while (now() - startedAt < timeoutMs) {
    try {
      const targets = await getJsonImpl(`${cdpBase}/json/list`)
      const page = installedCdpPage(targets)
      if (page) return { page, attemptsAtMs: now() - startedAt }
      last = `no ShellX Cut page in ${targets.length} target(s)`
    } catch (error) {
      last = error?.message || String(error)
    }
    await sleepImpl(pollMs)
  }
  throw new Error(`Timed out waiting for CDP at ${cdpBase}: ${last}`)
}

function openSocket(url, WebSocketImpl, timeoutMs) {
  return new Promise((resolve, reject) => {
    const socket = new WebSocketImpl(url)
    const timeout = setTimeout(() => {
      socket.close()
      reject(new Error('timed out opening the installed CDP WebSocket'))
    }, timeoutMs)
    const opened = () => {
      clearTimeout(timeout)
      resolve(socket)
    }
    const failed = (event) => {
      clearTimeout(timeout)
      reject(new Error(`installed CDP WebSocket failed: ${event?.message || event?.type || 'error'}`))
    }
    socket.addEventListener('open', opened, { once: true })
    socket.addEventListener('error', failed, { once: true })
  })
}

export async function evaluateInstalledCdpDomRoot({
  webSocketDebuggerUrl,
  timeoutMs = 3000,
  WebSocketImpl = globalThis.WebSocket,
} = {}) {
  assert(typeof webSocketDebuggerUrl === 'string' && webSocketDebuggerUrl.startsWith('ws://'),
    'installed CDP page does not expose a loopback WebSocket debugger URL')
  assert(typeof WebSocketImpl === 'function', 'native Node WebSocket is required for installed CDP DOM readiness')
  const socket = await openSocket(webSocketDebuggerUrl, WebSocketImpl, timeoutMs)
  try {
    return await new Promise((resolve, reject) => {
      const timeout = setTimeout(() => reject(new Error('timed out evaluating installed CDP DOM readiness')), timeoutMs)
      const close = () => {
        clearTimeout(timeout)
        socket.close()
      }
      socket.addEventListener('message', (event) => {
        let message
        try {
          message = JSON.parse(String(event.data))
        } catch (error) {
          close()
          reject(new Error(`installed CDP returned malformed JSON: ${error.message}`))
          return
        }
        if (message.id !== 1) return
        close()
        if (message.error) {
          reject(new Error(`installed CDP Runtime.evaluate failed: ${message.error.message || 'unknown error'}`))
          return
        }
        resolve(message.result?.result?.value === true)
      })
      socket.addEventListener('error', (event) => {
        close()
        reject(new Error(`installed CDP WebSocket error: ${event?.message || event?.type || 'error'}`))
      }, { once: true })
      socket.send(JSON.stringify({
        id: 1,
        method: 'Runtime.evaluate',
        params: {
          expression: "Boolean(document.querySelector('[data-cut-app-root]'))",
          returnByValue: true,
          awaitPromise: true,
        },
      }))
    })
  } finally {
    if (socket.readyState < socket.CLOSING) socket.close()
  }
}

export async function waitForInstalledDomRoot({
  page,
  timeoutMs,
  evaluateImpl = evaluateInstalledCdpDomRoot,
  pollMs = 100,
  now = Date.now,
  sleepImpl = sleep,
} = {}) {
  assert(page?.webSocketDebuggerUrl, 'installed CDP page is missing its WebSocket debugger URL')
  assert(Number.isSafeInteger(timeoutMs) && timeoutMs > 0, 'DOM readiness timeout must be a positive integer')
  const startedAt = now()
  let last = ''
  while (now() - startedAt < timeoutMs) {
    try {
      if (await evaluateImpl({ webSocketDebuggerUrl: page.webSocketDebuggerUrl, timeoutMs: Math.min(3000, timeoutMs) })) {
        return { selector: '[data-cut-app-root]', attemptsAtMs: now() - startedAt }
      }
      last = '[data-cut-app-root] is absent'
    } catch (error) {
      last = error?.message || String(error)
    }
    await sleepImpl(pollMs)
  }
  throw new Error(`Timed out waiting for installed DOM root: ${last}`)
}

export function readTestOwnedSlowFfmpegMarker(path) {
  assert(typeof path === 'string' && path.length > 0, 'slow FFmpeg marker path is required')
  if (!existsSync(path)) return null
  const marker = JSON.parse(readFileSync(path, 'utf8'))
  assert(marker?.schema === TEST_OWNED_SLOW_FFMPEG_MARKER_SCHEMA && marker.testOwned === true,
    'slow FFmpeg marker is not an explicit test-owned marker')
  assert(Number.isSafeInteger(marker.startedAtEpochMs) && marker.startedAtEpochMs > 0,
    'slow FFmpeg marker has an invalid start time')
  return {
    atEpochMs: marker.startedAtEpochMs,
    evidence: {
      markerSchema: marker.schema,
      markerMtimeMs: Math.floor(statSync(path).mtimeMs),
    },
  }
}

export async function waitForTestOwnedSlowFfmpegMarker({
  markerPath,
  timeoutMs,
  readMarker = readTestOwnedSlowFfmpegMarker,
  pollMs = 100,
  now = Date.now,
  sleepImpl = sleep,
} = {}) {
  assert(Number.isSafeInteger(timeoutMs) && timeoutMs > 0, 'slow FFmpeg marker timeout must be a positive integer')
  const startedAt = now()
  while (now() - startedAt < timeoutMs) {
    const marker = readMarker(markerPath)
    if (marker) return marker
    await sleepImpl(pollMs)
  }
  throw new Error(`Timed out waiting for test-owned slow FFmpeg marker: ${markerPath}`)
}
