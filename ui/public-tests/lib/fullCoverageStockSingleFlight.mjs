const FETCH_PATH = '/api/verb/assets.fetch'

async function installPlaywrightHold(page) {
  const routePattern = `**${FETCH_PATH}`
  let fetchRequests = 0
  let firstServerResult = null
  let release
  let released = false
  const releasedPromise = new Promise((resolve) => { release = resolve })
  let firstReached
  const firstReachedPromise = new Promise((resolve) => { firstReached = resolve })
  const handler = async (route) => {
    fetchRequests += 1
    if (fetchRequests !== 1) {
      await route.fulfill({
        contentType: 'application/json',
        body: JSON.stringify({ ok: false, error: { code: 'single_flight_failed' } }),
      })
      return
    }
    const upstream = await route.fetch()
    firstServerResult = await upstream.json()
    firstReached()
    await releasedPromise
    await route.fulfill({
      status: upstream.status(),
      contentType: 'application/json',
      body: JSON.stringify(firstServerResult),
    })
  }
  await page.route(routePattern, handler)
  const releaseResponse = async () => {
    if (released) return
    released = true
    release()
  }
  return {
    mode: 'playwright-route',
    waitForFirstServerResponse: async () => firstReachedPromise,
    snapshot: async () => ({ fetchRequests, firstServerResult }),
    release: releaseResponse,
    dispose: async () => {
      await releaseResponse()
      await page.unroute(routePattern, handler)
    },
  }
}

async function installBrowserFetchHold(page) {
  await page.evaluate(() => {
    const target = window
    if (target.__fcvStockSingleFlight) {
      throw new Error('stock single-flight fetch hold is already installed')
    }
    const state = {
      fetchRequests: 0,
      firstServerResult: null,
      firstServerReached: false,
      released: false,
      previousFetch: window.fetch,
      releaseResponse: null,
      wrapper: null,
    }
    const releasePromise = new Promise((resolve) => { state.releaseResponse = resolve })
    state.wrapper = async (...args) => {
      const input = args[0]
      const url = typeof input === 'string' ? input : input?.url || ''
      let pathname = ''
      try { pathname = new URL(String(url), document.baseURI).pathname } catch {}
      if (pathname !== '/api/verb/assets.fetch') return state.previousFetch.apply(target, args)
      state.fetchRequests += 1
      if (state.fetchRequests !== 1) {
        return new Response(JSON.stringify({ ok: false, error: { code: 'single_flight_failed' } }), {
          status: 200,
          headers: { 'content-type': 'application/json' },
        })
      }
      const response = await state.previousFetch.apply(target, args)
      try { state.firstServerResult = await response.clone().json() } catch {}
      state.firstServerReached = true
      await releasePromise
      return response
    }
    target.__fcvStockSingleFlight = state
    window.fetch = state.wrapper
  })
  const releaseResponse = async () => page.evaluate(() => {
    const state = window.__fcvStockSingleFlight
    if (!state || state.released) return
    state.released = true
    state.releaseResponse()
  })
  return {
    mode: 'browser-fetch',
    waitForFirstServerResponse: async (timeoutMs = 60_000) => page.waitForFunction(
      () => window.__fcvStockSingleFlight?.firstServerReached === true,
      undefined,
      { timeout: timeoutMs },
    ),
    snapshot: async () => page.evaluate(() => {
      const state = window.__fcvStockSingleFlight
      return {
        fetchRequests: state?.fetchRequests || 0,
        firstServerResult: state?.firstServerResult || null,
      }
    }),
    release: releaseResponse,
    dispose: async () => {
      await releaseResponse()
      await page.evaluate(() => {
        const target = window
        const state = target.__fcvStockSingleFlight
        if (!state) return
        if (window.fetch === state.wrapper) window.fetch = state.previousFetch
        delete target.__fcvStockSingleFlight
      })
    },
  }
}

/**
 * Hold the first same-origin sticker import response after it reaches Cut. The
 * Playwright route keeps the browser-run path unchanged; native WebDriver uses
 * an outer fetch wrapper because WebDriverIO intentionally has no request-route
 * API. Both modes expose the same bounded control and always restore ownership.
 */
export async function installStockImportSingleFlightHold(page) {
  if (typeof page.route === 'function' && typeof page.unroute === 'function') {
    return installPlaywrightHold(page)
  }
  return installBrowserFetchHold(page)
}
