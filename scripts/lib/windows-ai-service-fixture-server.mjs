import { createHash } from 'node:crypto'
import { createServer } from 'node:http'
import {
  WINDOWS_AI_SERVICE_FIXTURE_DIARIZE_MODEL,
  WINDOWS_AI_SERVICE_FIXTURE_DUB_MODEL,
  WINDOWS_AI_SERVICE_FIXTURE_PROTOCOL,
  WINDOWS_AI_SERVICE_FIXTURE_PROVENANCE,
  wavDurationMs,
} from './windows-ai-service-fixture-protocol.mjs'

const MAX_WAV_BYTES = 64 * 1024 * 1024
const MAX_JSON_BYTES = 1024 * 1024
const SAMPLE_RATE = 24_000

function invariant(condition, message) {
  if (!condition) throw new Error(message)
}

function counters() {
  return {
    diarize: { health: 0, request: 0, rejected: 0, control: 0 },
    dub: { health: 0, request: 0, rejected: 0, control: 0 },
  }
}

function snapshotCounters(value) {
  return JSON.parse(JSON.stringify(value))
}

function json(response, status, value) {
  response.writeHead(status, {
    'content-type': 'application/json; charset=utf-8',
    'cache-control': 'no-store',
  })
  response.end(JSON.stringify(value))
}

function binary(response, value) {
  response.writeHead(200, {
    'content-type': 'application/octet-stream',
    'cache-control': 'no-store',
    'x-shellx-cut-provenance': WINDOWS_AI_SERVICE_FIXTURE_PROVENANCE,
  })
  response.end(value)
}

async function readBody(request, limit) {
  const chunks = []
  let total = 0
  for await (const chunk of request) {
    total += chunk.length
    if (total > limit) throw new Error('request body exceeds fixture limit')
    chunks.push(chunk)
  }
  return Buffer.concat(chunks)
}

export function deterministicTurns(durationMs, maxSpeakers = '') {
  const duration = Math.max(1, Math.round(Number(durationMs) || 0))
  const requested = Number(maxSpeakers)
  if (Number.isFinite(requested) && requested === 1) {
    return [{ start_ms: 0, end_ms: duration, speaker: 'S1' }]
  }
  const split = Math.max(1, Math.floor(duration / 2))
  if (split >= duration) {
    return [{ start_ms: 0, end_ms: duration, speaker: 'S1' }]
  }
  return [
    { start_ms: 0, end_ms: split, speaker: 'S1' },
    { start_ms: split, end_ms: duration, speaker: 'S2' },
  ]
}

export function deterministicPcm({ text, voice, duration } = {}) {
  invariant(
    typeof text === 'string' && typeof voice === 'string',
    'fixture dub request must contain string text and voice',
  )
  const requested = Number(duration)
  const seconds = Number.isFinite(requested) && requested > 0
    ? Math.min(10, Math.max(0.02, requested))
    : Math.min(1, 0.08 + Math.min(180, text.length) * 0.004)
  const samples = Math.max(1, Math.round(seconds * SAMPLE_RATE))
  const seed = createHash('sha256').update(`${voice}\u0000${text}`).digest()
  const pcm = Buffer.alloc(samples * 2)
  for (let index = 0; index < samples; index += 1) {
    const value = ((seed[index % seed.length] - 128) * 128) + ((index % 97) - 48) * 4
    pcm.writeInt16LE(Math.max(-32_768, Math.min(32_767, value)), index * 2)
  }
  return pcm
}

function endpointFor(server) {
  const address = server.address()
  invariant(
    address
      && typeof address === 'object'
      && address.address === '127.0.0.1'
      && Number.isInteger(address.port),
    'fixture server did not bind an IPv4 loopback port',
  )
  return `http://127.0.0.1:${address.port}`
}

function listen(server) {
  return new Promise((resolve, reject) => {
    server.once('error', reject)
    server.listen(0, '127.0.0.1', () => {
      server.off('error', reject)
      resolve()
    })
  })
}

function close(server) {
  return new Promise((resolve, reject) => {
    server.close((error) => error ? reject(error) : resolve())
  })
}

function requestUrl(request) {
  return new URL(request.url || '/', 'http://127.0.0.1')
}

function contentType(request) {
  return String(request.headers['content-type'] || '')
    .split(';', 1)[0]
    .trim()
    .toLowerCase()
}

function controlRequest(request, response, counter, runToken, signalShutdown) {
  const url = requestUrl(request)
  if (request.method !== 'POST' || url.pathname !== '/__shellx_cut_fixture_shutdown') return false
  counter.control += 1
  if (url.searchParams.get('run_token') !== runToken) {
    counter.rejected += 1
    json(response, 403, { ok: false })
    return true
  }
  json(response, 202, { ok: true })
  signalShutdown()
  return true
}

export async function createAiServiceFixtureServers({ runToken } = {}) {
  invariant(/^[a-f0-9]{32,64}$/.test(String(runToken || '')), 'fixture host requires a bounded run token')
  let finishShutdown
  const shutdownRequested = new Promise((resolve) => { finishShutdown = resolve })
  const counts = counters()
  const diarize = createServer(async (request, response) => {
    if (controlRequest(request, response, counts.diarize, runToken, finishShutdown)) return
    const url = requestUrl(request)
    if (request.method === 'GET' && url.pathname === '/health') {
      counts.diarize.health += 1
      return json(response, 200, {
        status: 'ok',
        loaded: true,
        model: WINDOWS_AI_SERVICE_FIXTURE_DIARIZE_MODEL,
        provenance: WINDOWS_AI_SERVICE_FIXTURE_PROVENANCE,
      })
    }
    if (request.method !== 'POST' || url.pathname !== '/diarize') {
      counts.diarize.rejected += 1
      return json(response, 404, { ok: false })
    }
    if (contentType(request) !== 'audio/wav') {
      counts.diarize.rejected += 1
      request.resume()
      return json(response, 415, { ok: false })
    }
    try {
      const durationMs = wavDurationMs(await readBody(request, MAX_WAV_BYTES))
      const turns = deterministicTurns(durationMs, url.searchParams.get('max_speakers'))
      counts.diarize.request += 1
      return json(response, 200, {
        turns,
        n_speakers: new Set(turns.map((turn) => turn.speaker)).size,
        model: WINDOWS_AI_SERVICE_FIXTURE_DIARIZE_MODEL,
        provenance: WINDOWS_AI_SERVICE_FIXTURE_PROVENANCE,
        device: 'fixture-loopback',
        audio_s: durationMs / 1000,
        infer_s: 0,
      })
    } catch {
      counts.diarize.rejected += 1
      return json(response, 400, { ok: false })
    }
  })
  const dub = createServer(async (request, response) => {
    if (controlRequest(request, response, counts.dub, runToken, finishShutdown)) return
    const url = requestUrl(request)
    if (request.method === 'GET' && url.pathname === '/health') {
      counts.dub.health += 1
      return json(response, 200, {
        status: 'ok',
        loaded: true,
        model: WINDOWS_AI_SERVICE_FIXTURE_DUB_MODEL,
        provenance: WINDOWS_AI_SERVICE_FIXTURE_PROVENANCE,
      })
    }
    if (request.method !== 'POST' || url.pathname !== '/synthesize') {
      counts.dub.rejected += 1
      return json(response, 404, { ok: false })
    }
    if (contentType(request) !== 'application/json') {
      counts.dub.rejected += 1
      request.resume()
      return json(response, 415, { ok: false })
    }
    try {
      const payload = JSON.parse((await readBody(request, MAX_JSON_BYTES)).toString('utf8'))
      const pcm = deterministicPcm(payload)
      counts.dub.request += 1
      return binary(response, pcm)
    } catch {
      counts.dub.rejected += 1
      return json(response, 400, { ok: false })
    }
  })
  await Promise.all([listen(diarize), listen(dub)])
  const endpoints = { diarize: endpointFor(diarize), dub: endpointFor(dub) }
  return {
    endpoints,
    requestCounters: counts,
    shutdownRequested,
    manifest: () => ({
      protocol: WINDOWS_AI_SERVICE_FIXTURE_PROTOCOL,
      pid: process.pid,
      endpoints,
      requestCounters: snapshotCounters(counts),
    }),
    close: async () => { await Promise.all([close(diarize), close(dub)]) },
  }
}
