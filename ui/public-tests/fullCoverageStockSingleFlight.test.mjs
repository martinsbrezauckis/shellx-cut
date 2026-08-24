import assert from 'node:assert/strict'

import { installStockImportSingleFlightHold } from './lib/fullCoverageStockSingleFlight.mjs'

async function browserFetchMode() {
  const previousWindow = globalThis.window
  const previousDocument = globalThis.document
  let upstreamCalls = 0
  const originalFetch = async function () {
    assert.equal(this, window, 'the outer wrapper preserves the prior fetch receiver')
    upstreamCalls += 1
    return new Response(JSON.stringify({ ok: true, result: { asset_id: 'asset_sticker' } }), {
      status: 200,
      headers: { 'content-type': 'application/json' },
    })
  }
  globalThis.document = { baseURI: 'http://127.0.0.1:6161/' }
  globalThis.window = { fetch: originalFetch }
  const page = {
    evaluate: async (callback, value) => callback(value),
    waitForFunction: async (callback, _value, { timeout }) => {
      const deadline = Date.now() + timeout
      while (!callback()) {
        if (Date.now() >= deadline) throw new Error('waitForFunction timed out')
        await new Promise((resolve) => setTimeout(resolve, 1))
      }
    },
  }
  try {
    const hold = await installStockImportSingleFlightHold(page)
    assert.equal(hold.mode, 'browser-fetch')
    const first = window.fetch('/api/verb/assets.fetch', { method: 'POST' })
    await hold.waitForFirstServerResponse(100)
    assert.deepEqual(await hold.snapshot(), {
      fetchRequests: 1,
      firstServerResult: { ok: true, result: { asset_id: 'asset_sticker' } },
    })
    const second = await window.fetch('/api/verb/assets.fetch', { method: 'POST' })
    assert.equal((await second.json()).error.code, 'single_flight_failed')
    assert.equal((await hold.snapshot()).fetchRequests, 2)
    assert.equal(upstreamCalls, 1, 'the synthetic second response must never reach Cut')
    await hold.release()
    assert.equal((await (await first).json()).result.asset_id, 'asset_sticker')
    await hold.dispose()
    assert.equal(window.fetch, originalFetch, 'native fetch instrumentation is restored exactly')
    assert.equal(window.__fcvStockSingleFlight, undefined)
  } finally {
    if (previousWindow === undefined) delete globalThis.window
    else globalThis.window = previousWindow
    if (previousDocument === undefined) delete globalThis.document
    else globalThis.document = previousDocument
  }
}

async function playwrightRouteMode() {
  let installed
  let removed
  const page = {
    route: async (pattern, handler) => { installed = { pattern, handler } },
    unroute: async (pattern, handler) => { removed = { pattern, handler } },
  }
  const hold = await installStockImportSingleFlightHold(page)
  assert.equal(hold.mode, 'playwright-route')
  const fulfillments = []
  const firstHandler = installed.handler({
    fetch: async () => ({
      json: async () => ({ ok: true, result: { asset_id: 'asset_sticker' } }),
      status: () => 200,
    }),
    fulfill: async (value) => { fulfillments.push(value) },
  })
  await hold.waitForFirstServerResponse()
  assert.equal((await hold.snapshot()).fetchRequests, 1)
  await installed.handler({
    fetch: async () => { throw new Error('second route must not reach Cut') },
    fulfill: async (value) => { fulfillments.push(value) },
  })
  assert.equal(JSON.parse(fulfillments[0].body).error.code, 'single_flight_failed')
  await hold.release()
  await firstHandler
  assert.equal(JSON.parse(fulfillments[1].body).result.asset_id, 'asset_sticker')
  await hold.dispose()
  assert.equal(removed.pattern, '**/api/verb/assets.fetch')
  assert.equal(removed.handler, installed.handler)
}

await browserFetchMode()
await playwrightRouteMode()
console.log('full coverage stock single-flight tests passed')
