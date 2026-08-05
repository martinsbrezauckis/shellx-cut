// Playwright-shaped adapter over WebdriverIO's native Tauri session.
//
// The exhaustive Cut verifier predates the embedded cross-platform Tauri
// WebDriver provider and uses a deliberately small subset of Playwright's Page
// and Locator APIs. This adapter implements that subset so the same action
// scenarios can run in WebView2, WebKitGTK, and WKWebView. It does not emulate a
// browser: element interaction still goes through the native WebDriver session.

import { writeFile } from 'node:fs/promises'
import { randomUUID } from 'node:crypto'
import {
  createWebdriverIoLocatorFactory,
  keySequence,
} from './webdriverIoLocator.mjs'

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

function timeoutFrom(options, fallback) {
  const value = Number(options?.timeout)
  return Number.isFinite(value) && value >= 0 ? value : fallback
}

function responseEvent(entry) {
  return {
    url: () => String(entry.url || ''),
    status: () => Number(entry.status || 0),
    json: async () => {
      if (entry.json !== undefined) return entry.json
      return JSON.parse(String(entry.text || ''))
    },
  }
}

function requestEvent(entry) {
  return {
    url: () => String(entry.url || ''),
    postDataJSON: () => entry.requestJson ?? null,
  }
}

function installExecuteConsoleCleanup(wdioBrowser) {
  if (process.env.SHELLX_CUT_WDIO_PROVIDER !== 'external') return
  const marker = Symbol.for('shellx-cut:wdio-execute-console-cleanup')
  if (wdioBrowser[marker]) return
  const serviceExecute = wdioBrowser.execute.bind(wdioBrowser)
  const wrap = (script) => {
    if (typeof script !== 'function') return script
    // Build the wrapper on the Node side. The page receives an ordinary
    // serialized function, so this does not weaken the app's no-unsafe-eval
    // CSP. @wdio/tauri-service 1.2 injects its console forwarder around every
    // external execute() even when captureFrontendLogs=false; shipping apps
    // intentionally do not contain the test-only wdio command/ACL. Remove that
    // service wrapper before and after each adapter evaluation so its rejected
    // plugin:wdio|log_frontend promises cannot masquerade as product errors.
    return Function(`return async function (...args) {
      const cleanup = () => {
        try { globalThis.window?.__wdioConsoleCleanup?.() } catch {}
      }
      cleanup()
      try {
        return await (${script.toString()})(...args)
      } finally {
        cleanup()
      }
    }`)()
  }
  Object.defineProperty(wdioBrowser, 'execute', {
    value: (script, ...args) => serviceExecute(wrap(script), ...args),
    configurable: true,
    writable: true,
  })
  Object.defineProperty(wdioBrowser, marker, { value: true })
}

export function nativeWindowSizeForViewport(requested, metrics, provider = '') {
  const chromeWidth = Math.max(0, Number(metrics?.outerWidth) - Number(metrics?.innerWidth))
  const chromeHeight = Math.max(0, Number(metrics?.outerHeight) - Number(metrics?.innerHeight))
  // Tauri's embedded WKWebView driver accepts backing-pixel window dimensions,
  // while the page exposes CSS-pixel viewport metrics. External WebDrivers use
  // logical window units, so only the embedded provider needs the DPR scale.
  const scale = provider === 'embedded'
    ? Math.max(1, Number(metrics?.devicePixelRatio) || 1)
    : 1
  return {
    width: Math.max(1, Math.round((requested.width + chromeWidth) * scale)),
    height: Math.max(1, Math.round((requested.height + chromeHeight) * scale)),
  }
}

export async function createWebdriverIoPage(wdioBrowser, options = {}) {
  if (!wdioBrowser || typeof wdioBrowser.execute !== 'function') {
    throw new TypeError('createWebdriverIoPage requires an active WebdriverIO browser')
  }
  installExecuteConsoleCleanup(wdioBrowser)

  let defaultTimeout = Number(options.defaultTimeout || 5000)
  const traceEvents = options.traceEvents ?? process.env.FCV_TRACE === '1'
  let closed = false
  let polling = false
  let instrumentedDocument = ''
  const instrumentationId = randomUUID()
  const listeners = new Map()
  const dialogPolicy = { accept: true, prompt: '', persistent: false, dirty: true }

  let viewport = await wdioBrowser.execute(() => ({
    width: Math.max(1, window.innerWidth || document.documentElement.clientWidth || 1600),
    height: Math.max(1, window.innerHeight || document.documentElement.clientHeight || 900),
  })).catch(() => ({ width: 1600, height: 900 }))

  function listenersFor(name) {
    if (!listeners.has(name)) listeners.set(name, new Set())
    return listeners.get(name)
  }

  async function installInstrumentation() {
    const current = await wdioBrowser.execute(() => ({
      href: String(document.location.href),
      bridgeId: String(window.__shellxCutFcvBridge?.bridgeId || ''),
    })).catch(() => ({ href: '', bridgeId: '' }))
    if (
      current.href
      && instrumentedDocument === current.href
      && current.bridgeId === instrumentationId
      && !dialogPolicy.dirty
    ) return

    await wdioBrowser.execute(({ bridgeId, policy }) => {
      const state = window.__shellxCutFcvBridge ||= {
        events: [],
        originalFetch: window.fetch.bind(window),
        originalConfirm: window.confirm.bind(window),
        originalPrompt: window.prompt.bind(window),
      }
      // A new adapter can adopt a still-live bridge without wrapping fetch a
      // second time. A document replacement at the same URL has no bridge, so
      // this identity check forces a fresh installation.
      state.bridgeId = bridgeId
      if (!state.fetchInstalled) {
        window.fetch = (...args) => {
          const input = args[0]
          const requestUrl = typeof input === 'string' ? input : input?.url || ''
          const init = args[1] || {}
          let requestJson = null
          try {
            const body = init.body ?? (typeof input === 'object' ? input?.body : null)
            requestJson = typeof body === 'string' ? JSON.parse(body) : null
          } catch {}
          let observedRequestUrl = String(requestUrl)
          try { observedRequestUrl = new URL(observedRequestUrl, document.baseURI).href } catch {}
          state.events.push({ type: 'request', url: observedRequestUrl, requestJson })
          return state.originalFetch(...args).then(async (response) => {
            const responseUrl = response.url || observedRequestUrl
            let responsePath = ''
            try { responsePath = new URL(responseUrl, document.baseURI).pathname } catch {}
            // The verifier parses bodies only for verb responses. Cloning frame,
            // filmstrip, proxy, source, and export media into JavaScript strings
            // made long installed runs retain gigabytes of binary response data.
            if (!responsePath.startsWith('/api/verb/')) {
              state.events.push({
                type: 'response',
                url: responseUrl,
                status: response.status,
              })
              return response
            }
            try {
              // Consume the instrumentation clone before the application reads
              // the original response. WebKitGTK can abort one branch when a
              // long or larger response body is read concurrently from both
              // clones (observed on screen_record.stop). Sequential reads keep
              // the app response untouched and still preserve exact evidence.
              const text = await response.clone().text()
              let json
              try { json = JSON.parse(text) } catch {}
              const entry = {
                type: 'response',
                url: responseUrl,
                status: response.status,
              }
              // Keep one representation, not both. JSON verb envelopes use the
              // parsed form; a non-JSON failure retains its text fallback.
              if (json === undefined) entry.text = text
              else entry.json = json
              state.events.push(entry)
            } catch (error) {
              state.events.push({
                type: 'pageerror',
                message: `response clone failed: ${String(error?.message || error)}`,
              })
            }
            return response
          })
        }
        window.addEventListener('error', (event) => {
          state.events.push({ type: 'pageerror', message: String(event.error?.stack || event.message || event.error || '') })
        })
        window.addEventListener('unhandledrejection', (event) => {
          state.events.push({ type: 'pageerror', message: String(event.reason?.stack || event.reason || '') })
        })
        state.fetchInstalled = true
      }
      if (!state.actionRecorderInstalled) {
        const candidates = (event) => {
          const origin = event.target instanceof Element
            ? event.target.closest('button, input, select, textarea, summary')
            : null
          if (!origin) return []
          const out = []
          const direct = origin.getAttribute('data-cut-action')
          if (direct) out.push(direct)
          for (const attribute of origin.attributes) {
            if (!attribute.name.startsWith('data-cut-') || attribute.name === 'data-cut-action') continue
            out.push(attribute.name.slice('data-cut-'.length))
          }
          return [...new Set(out.filter(Boolean))]
        }
        const record = (event) => {
          const actions = candidates(event)
          if (actions.length) state.events.push({ type: 'action', actions })
        }
        const recordMouseDownAction = (event) => {
          const origin = event.target instanceof Element
            ? event.target.closest('[data-cut-action]')
            : null
          const action = origin?.getAttribute('data-cut-action')
          if (action) state.events.push({ type: 'action', actions: [action] })
        }
        // Blocking-overlay scrims intentionally close on mousedown and unmount
        // before the browser can emit click. Record their explicit action id at
        // the event that actually drives the product behavior.
        document.addEventListener('mousedown', recordMouseDownAction, true)
        document.addEventListener('click', record, true)
        document.addEventListener('input', record, true)
        document.addEventListener('change', record, true)
        state.actionRecorderInstalled = true
      }
      state.dialogPolicy = policy
      window.confirm = () => {
        state.events.push({ type: 'dialog', dialogType: 'confirm' })
        const accepted = state.dialogPolicy?.accept !== false
        if (!state.dialogPolicy?.persistent) state.dialogPolicy = { accept: true, prompt: '', persistent: true }
        return accepted
      }
      window.prompt = (_message, fallback = '') => {
        state.events.push({ type: 'dialog', dialogType: 'prompt' })
        const accepted = state.dialogPolicy?.accept !== false
        const value = accepted ? String(state.dialogPolicy?.prompt ?? fallback ?? '') : null
        if (!state.dialogPolicy?.persistent) state.dialogPolicy = { accept: true, prompt: '', persistent: true }
        return value
      }
    }, { bridgeId: instrumentationId, policy: { ...dialogPolicy } })
    instrumentedDocument = current.href
    dialogPolicy.dirty = false
  }

  async function drainEvents() {
    if (closed || polling) return
    if (![...listeners.values()].some((callbacks) => callbacks.size > 0)) return
    polling = true
    try {
      // Fetch response clones settle just after the user action resolves.
      await sleep(Number(options.eventSettleMs || 40))
      await installInstrumentation()
      const entries = await wdioBrowser.execute(() => {
        const bridge = window.__shellxCutFcvBridge
        if (!bridge) return []
        return bridge.events.splice(0, bridge.events.length)
      })
      if (traceEvents && entries?.length) {
        const summary = entries.map((entry) =>
          `${entry.type}:${String(
            entry.url
              || entry.message
              || (Array.isArray(entry.actions) ? entry.actions.join(',') : ''),
          ).slice(0, 90)}`,
        )
        process.stderr.write(`[native-adapter-events] ${summary.join(' | ')}\n`)
      }
      for (const entry of entries || []) {
        const name = entry.type
        const callbacks = [...(listeners.get(name) || [])]
        for (const callback of callbacks) {
          try {
            const event = name === 'response'
              ? responseEvent(entry)
              : name === 'request'
                ? requestEvent(entry)
                : name === 'pageerror'
                  ? new Error(String(entry.message || 'page error'))
                  : entry
            await callback(event)
          } catch {}
        }
      }
    } finally {
      polling = false
    }
  }

  async function recordSuccessfulAction(candidates) {
    await installInstrumentation()
    await wdioBrowser.execute((actions) => {
      const bridge = window.__shellxCutFcvBridge
      if (!bridge || !Array.isArray(bridge.events)) return
      const observed = new Set(
        bridge.events
          .filter((entry) => entry?.type === 'action' && Array.isArray(entry.actions))
          .flatMap((entry) => entry.actions.map(String)),
      )
      const missing = [...new Set((actions || []).map(String).filter(Boolean))]
        .filter((action) => !observed.has(action))
      if (missing.length) {
        bridge.events.push({ type: 'action', actions: missing, source: 'webdriver-success-fallback' })
      }
    }, candidates)
  }

  const Locator = createWebdriverIoLocatorFactory({
    browser: wdioBrowser,
    installInstrumentation,
    drainEvents,
    recordSuccessfulAction,
    defaultTimeout: () => defaultTimeout,
  })

  const page = {
    locator: (selector) => new Locator([{ selector }]),
    setDefaultTimeout: (value) => {
      const next = Number(value)
      if (Number.isFinite(next) && next >= 0) defaultTimeout = next
    },
    viewportSize: () => viewport,
    setViewportSize: async ({ width, height }) => {
      const requested = {
        width: Math.max(320, Math.round(Number(width) || 0)),
        height: Math.max(240, Math.round(Number(height) || 0)),
      }
      if (typeof wdioBrowser.setWindowSize !== 'function') {
        throw new Error('the native WebDriver provider does not support window resizing')
      }
      const before = await wdioBrowser.execute(() => ({
        innerWidth: Math.max(1, window.innerWidth || document.documentElement.clientWidth || 1),
        innerHeight: Math.max(1, window.innerHeight || document.documentElement.clientHeight || 1),
        outerWidth: Math.max(1, window.outerWidth || window.innerWidth || 1),
        outerHeight: Math.max(1, window.outerHeight || window.innerHeight || 1),
        devicePixelRatio: Math.max(1, Number(window.devicePixelRatio) || 1),
      }))
      const nativeSize = nativeWindowSizeForViewport(requested, before, options.provider)
      await wdioBrowser.setWindowSize(nativeSize.width, nativeSize.height)
      const deadline = Date.now() + defaultTimeout
      let reached = false
      do {
        viewport = await wdioBrowser.execute(() => ({
          width: Math.max(1, window.innerWidth || document.documentElement.clientWidth || 1),
          height: Math.max(1, window.innerHeight || document.documentElement.clientHeight || 1),
        }))
        if (
          Math.abs(viewport.width - requested.width) <= 24
          && Math.abs(viewport.height - requested.height) <= 48
        ) {
          reached = true
          break
        }
        await sleep(80)
      } while (Date.now() <= deadline)
      if (!reached) {
        throw new Error(
          `native viewport resize did not reach ${requested.width}x${requested.height}; `
          + `actual viewport is ${viewport.width}x${viewport.height}`,
        )
      }
      await drainEvents()
    },
    goto: async (url) => {
      await wdioBrowser.url(url)
      instrumentedDocument = ''
      dialogPolicy.dirty = true
      // The next locator/evaluate call installs the bridge after the new
      // document has settled. Installing synchronously in WKWebView's navigation
      // completion race intermittently times out execute/sync.
    },
    reload: async () => {
      await wdioBrowser.refresh()
      instrumentedDocument = ''
      dialogPolicy.dirty = true
      // Defer bridge installation until the verifier's first post-reload
      // operation. reloadApp already gives React time to mount, while immediate
      // execute/sync here can race WKWebView document replacement.
    },
    waitForSelector: async (selector, options = {}) => {
      await page.locator(selector).first().waitFor({ state: options.state || 'visible', timeout: options.timeout })
    },
    waitForFunction: async (fn, arg, options = {}) => {
      const timeout = timeoutFrom(options, defaultTimeout)
      const deadline = Date.now() + timeout
      let last
      while (Date.now() <= deadline) {
        last = await wdioBrowser.execute(fn, arg).catch(() => false)
        await drainEvents()
        if (last) return last
        await sleep(80)
      }
      throw new Error(`waitForFunction timed out after ${timeout}ms`)
    },
    evaluate: async (fn, arg) => {
      const result = await wdioBrowser.execute(fn, arg)
      await drainEvents()
      return result
    },
    flushEvents: drainEvents,
    screenshot: async ({ path }) => {
      const data = await wdioBrowser.takeScreenshot()
      await writeFile(path, Buffer.from(data, 'base64'))
    },
    keyboard: {
      press: async (value) => {
        await wdioBrowser.keys(keySequence(value))
        await drainEvents()
      },
    },
    mouse: {
      click: async (x, y) => {
        await wdioBrowser.action('pointer', {
          parameters: { pointerType: 'mouse' },
        })
          .move({ x: Math.round(x), y: Math.round(y) })
          .down({ button: 0 })
          .up({ button: 0 })
          .perform()
        await drainEvents()
      },
      drag: async (fromX, fromY, toX, toY) => {
        let action = wdioBrowser.action('pointer', {
          parameters: { pointerType: 'mouse' },
        })
          .move({ x: Math.round(fromX), y: Math.round(fromY) })
          .down({ button: 0 })
        for (let step = 1; step <= 8; step++) {
          const progress = step / 8
          action = action.move({
            x: Math.round(fromX + ((toX - fromX) * progress)),
            y: Math.round(fromY + ((toY - fromY) * progress)),
            duration: 24,
          })
        }
        await action.up({ button: 0 }).perform()
        await drainEvents()
      },
    },
    on: (name, callback) => {
      listenersFor(name).add(callback)
      if (name === 'dialog') {
        const fake = {
          accept: async (value = '') => {
            dialogPolicy.accept = true
            dialogPolicy.prompt = String(value)
            dialogPolicy.persistent = true
            dialogPolicy.dirty = true
          },
          dismiss: async () => {
            dialogPolicy.accept = false
            dialogPolicy.persistent = true
            dialogPolicy.dirty = true
          },
        }
        Promise.resolve(callback(fake)).catch(() => {})
      }
    },
    once: (name, callback) => {
      if (name !== 'dialog') {
        const wrapped = async (event) => {
          listenersFor(name).delete(wrapped)
          await callback(event)
        }
        listenersFor(name).add(wrapped)
        return
      }
      const fake = {
        accept: async (value = '') => {
          dialogPolicy.accept = true
          dialogPolicy.prompt = String(value)
          dialogPolicy.persistent = false
          dialogPolicy.dirty = true
        },
        dismiss: async () => {
          dialogPolicy.accept = false
          dialogPolicy.persistent = false
          dialogPolicy.dirty = true
        },
      }
      Promise.resolve(callback(fake)).catch(() => {})
    },
    off: (name, callback) => {
      listeners.get(name)?.delete(callback)
      if (name === 'dialog') {
        dialogPolicy.persistent = false
        dialogPolicy.dirty = true
      }
    },
  }

  await installInstrumentation()

  return {
    page,
    close: async () => {
      closed = true
      await drainEvents().catch(() => {})
    },
    attestation: {
      ok: true,
      browser: String(wdioBrowser.capabilities?.browserName || 'tauri-webdriver'),
      provider: String(options.provider || 'embedded'),
    },
  }
}
