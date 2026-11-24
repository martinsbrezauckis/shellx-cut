import { createRequire } from 'node:module'
import { attachPageCspGuard, CSP_VIOLATION_CONSOLE_PREFIX } from './cspHarness.mjs'

const require = createRequire(import.meta.url)
const pendingViolations = []
let hooksInstalled = false

function formatViolation(v) {
  return `${v.effectiveDirective || v.violatedDirective || 'csp'} (${v.blockedURI || v.sample || 'inline'})`
}

async function attachGuardNow(page, onViolation) {
  if (!page || page.__cutCspGuarded) return page
  page.__cutCspGuarded = true
  await attachPageCspGuard(page, { onViolation })
  return page
}

function attachGuardQueued(page, onViolation) {
  if (!page || page.__cutCspGuarded) return page
  page.__cutCspGuarded = true
  const ready = attachPageCspGuard(page, { onViolation }).catch((error) => {
    pendingViolations.push({ violatedDirective: 'csp-attach', blockedURI: String(error?.message || error) })
  })
  const origGoto = page.goto.bind(page)
  page.goto = async (...args) => { await ready; return origGoto(...args) }
  const origSetContent = page.setContent.bind(page)
  page.setContent = async (...args) => { await ready; return origSetContent(...args) }
  return page
}

function recordViolation(browserViolations, v) {
  pendingViolations.push(v)
  if (browserViolations) browserViolations.push(v)
}

function wrapContext(context, browserViolations) {
  if (!context || context.__cutCspWrapped) return context
  context.__cutCspWrapped = true
  const origNewPage = context.newPage.bind(context)
  context.newPage = async (...args) => {
    const page = await origNewPage(...args)
    return attachGuardNow(page, (v) => recordViolation(browserViolations, v))
  }
  for (const existing of context.pages()) {
    attachGuardQueued(existing, (v) => recordViolation(browserViolations, v))
  }
  return context
}

function wrapBrowser(browser) {
  if (!browser || browser.__cutCspWrapped) return browser
  browser.__cutCspWrapped = true
  const browserViolations = []
  const origNewContext = browser.newContext.bind(browser)
  browser.newContext = async (...args) => wrapContext(await origNewContext(...args), browserViolations)
  const origNewPage = browser.newPage.bind(browser)
  browser.newPage = async (...args) => {
    const page = await origNewPage(...args)
    return attachGuardNow(page, (v) => recordViolation(browserViolations, v))
  }
  const origClose = browser.close.bind(browser)
  browser.close = async (...args) => {
    await origClose(...args)
    const drained = browserViolations.splice(0, browserViolations.length)
    for (const v of drained) {
      const idx = pendingViolations.indexOf(v)
      if (idx >= 0) pendingViolations.splice(idx, 1)
    }
    if (drained.length > 0 && process.env.SHELLX_CUT_CSP_ALLOW_EXPECTED !== '1') {
      const list = drained.slice(0, 5).map(formatViolation).join('; ')
      throw new Error(`CSP preload guard caught violation(s): ${list}`)
    }
  }
  for (const ctx of browser.contexts()) wrapContext(ctx, browserViolations)
  return browser
}

/** Merge `--disable-features=LocalNetworkAccessChecks` so Playwright `Fetch.fulfillRequest` CSP injection keeps loopback WebSockets working. */
function withLoopbackRouteLaunchOptions(options = {}) {
  const args = Array.isArray(options?.args) ? [...options.args] : []
  const idx = args.findIndex((arg) => typeof arg === 'string' && arg.startsWith('--disable-features='))
  if (idx >= 0) {
    if (!args[idx].split('=')[1]?.split(',').includes('LocalNetworkAccessChecks')) {
      args[idx] = `${args[idx]},LocalNetworkAccessChecks`
    }
  } else {
    args.push('--disable-features=LocalNetworkAccessChecks')
  }
  return { ...options, args }
}

/** Install process-wide Playwright `chromium` wrapping to enforce CSP and fail on `securitypolicyviolation`. */
export function installPlaywrightCspPreload() {
  if (hooksInstalled || process.env.SHELLX_CUT_CSP_PRELOAD === '0') return
  hooksInstalled = true
  const { chromium } = require('playwright')
  const proto = Object.getPrototypeOf(chromium)
  const origLaunch = proto.launch
  const origConnect = proto.connectOverCDP
  const origPersistent = proto.launchPersistentContext
  proto.launch = async function (options, ...rest) {
    return wrapBrowser(await origLaunch.call(this, withLoopbackRouteLaunchOptions(options), ...rest))
  }
  proto.connectOverCDP = async function (...args) {
    return wrapBrowser(await origConnect.apply(this, args))
  }
  proto.launchPersistentContext = async function (userDataDir, options, ...rest) {
    return wrapContext(await origPersistent.call(this, userDataDir, withLoopbackRouteLaunchOptions(options), ...rest))
  }
  process.on('beforeExit', () => {
    if (pendingViolations.length > 0 && process.env.SHELLX_CUT_CSP_ALLOW_EXPECTED !== '1') {
      const list = pendingViolations.slice(0, 5).map(formatViolation).join('; ')
      console.error(`${CSP_VIOLATION_CONSOLE_PREFIX}FAIL ${list}`)
      process.exitCode = 1
    }
  })
}

installPlaywrightCspPreload()
