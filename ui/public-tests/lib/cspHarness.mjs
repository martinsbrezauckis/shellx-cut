import { createHash } from 'node:crypto'
import { readFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const HERE = dirname(fileURLToPath(import.meta.url))
export const DEFAULT_REPO_ROOT = resolve(HERE, '..', '..', '..')
export const CSP_VIOLATION_CONSOLE_PREFIX = 'CUT_CSP_VIOLATION:'
const LOOPBACK_HOSTS = new Set(['127.0.0.1', 'localhost', 'ipc.localhost'])
const SCHEMES_WITH_HOST = /^(?:https?|wss?):\/\//i
const NATIVE_CSP_ERROR_RE = /Content Security Policy|violates the following Content Security Policy|Refused to .* violates/i

const wrappedContexts = new WeakSet()
const pageListeners = new WeakMap()

/** Read `tauri.conf.json` `app.security.csp` and `app/server/src/http.rs` `UI_CSP` from source. */
export function readProductCspPolicy(repoRoot = DEFAULT_REPO_ROOT) {
  const tauriConf = JSON.parse(readFileSync(resolve(repoRoot, 'app/desktop/src-tauri/tauri.conf.json'), 'utf8'))
  const desktopCsp = tauriConf?.app?.security?.csp ?? null
  const httpSource = readFileSync(resolve(repoRoot, 'app/server/src/http.rs'), 'utf8')
  const match = httpSource.match(/const\s+UI_CSP:\s*&str\s*=\s*"((?:[^"\\]|\\[\s\S])*)"\s*;/)
  if (!match) throw new Error('could not extract UI_CSP from app/server/src/http.rs')
  const httpCsp = match[1].replace(/\\\r?\n\s*/g, '')
  return { desktopCsp, httpCsp }
}

/** Parse a CSP string into an ordered map from directive name to source tokens. */
export function parseCspDirectives(csp) {
  const directives = new Map()
  for (const rawPart of String(csp || '').split(';')) {
    const tokens = rawPart.trim().split(/\s+/).filter(Boolean)
    if (tokens.length === 0) continue
    directives.set(tokens[0].toLowerCase(), tokens.slice(1))
  }
  return directives
}

/** Serialize an ordered directive map back into a Content-Security-Policy header value. */
export function serializeCspDirectives(directives) {
  return [...directives.entries()]
    .map(([name, values]) => (values.length ? `${name} ${values.join(' ')}` : name))
    .join('; ')
}

/** Remove directives illegal in `<meta http-equiv="Content-Security-Policy">` (e.g. `frame-ancestors`). */
export function stripMetaUnsupportedDirectives(csp) {
  const directives = parseCspDirectives(csp)
  directives.delete('frame-ancestors')
  return serializeCspDirectives(directives)
}

/** Verify a CSP allows only tightly scoped loopback/local sources and never weakens scripts. */
export function assertCspPolicyTight(csp, { requireFrameAncestors = false, allowStyleUnsafeInline = true } = {}) {
  if (typeof csp !== 'string' || !csp.trim()) throw new Error('CSP policy must be a non-empty string')
  const directives = parseCspDirectives(csp)
  for (const [name, tokens] of directives.entries()) {
    for (const token of tokens) {
      if (token === "'unsafe-eval'") throw new Error(`CSP directive ${name} forbids 'unsafe-eval'`)
      if (token === '*' || token.includes('*.')) throw new Error(`CSP directive ${name} contains wildcard host token ${token}`)
      if (token.toLowerCase().startsWith('asset:')) throw new Error(`CSP directive ${name} unexpectedly allows asset: (${token})`)
      if (SCHEMES_WITH_HOST.test(token)) {
        const host = token.replace(SCHEMES_WITH_HOST, '').split(/[/:]/)[0].toLowerCase()
        if (!LOOPBACK_HOSTS.has(host)) throw new Error(`CSP directive ${name} allows non-loopback host in ${token}`)
      }
    }
  }
  const scriptSrc = directives.get('script-src') || []
  if (!scriptSrc.includes("'self'")) throw new Error(`script-src must include 'self': ${csp}`)
  if (scriptSrc.includes("'unsafe-inline'")) throw new Error(`script-src must not include 'unsafe-inline': ${csp}`)
  const styleSrc = directives.get('style-src') || []
  if (!styleSrc.includes("'self'")) throw new Error(`style-src must include 'self': ${csp}`)
  if (!allowStyleUnsafeInline && styleSrc.includes("'unsafe-inline'")) {
    throw new Error(`style-src must not include 'unsafe-inline': ${csp}`)
  }
  if ((directives.get('object-src') || []).join(' ') !== "'none'") throw new Error(`object-src must be 'none': ${csp}`)
  for (const required of ['ws://127.0.0.1:*', 'ws://localhost:*', 'http://127.0.0.1:*', 'http://localhost:*', 'ipc:', 'http://ipc.localhost']) {
    if (!(directives.get('connect-src') || []).includes(required)) throw new Error(`connect-src missing ${required}`)
  }
  if (!(directives.get('frame-src') || []).includes("'self'")) throw new Error(`frame-src must include 'self': ${csp}`)
  if (requireFrameAncestors && !(directives.get('frame-ancestors') || []).includes("'self'")) {
    throw new Error(`frame-ancestors must include 'self' for same-origin /manual.html framing: ${csp}`)
  }
  return directives
}

/** Extract SHA-256 CSP tokens for inline `<script>` elements in an HTML test fixture. */
export function computeInlineScriptHashes(html) {
  const hashes = []
  const scriptRe = /<script\b([^>]*)>([\s\S]*?)<\/script>/gi
  for (const match of String(html || '').matchAll(scriptRe)) {
    const attrs = match[1] || ''
    const body = match[2] || ''
    if (/\bsrc\s*=/.test(attrs) || !body.trim()) continue
    const digest = createHash('sha256').update(body, 'utf8').digest('base64')
    hashes.push(`'sha256-${digest}'`)
  }
  return [...new Set(hashes)]
}

/** Detect Vite dev-server test-harness HTML (which injects inline preamble scripts and dev styles). */
export function isViteDevFixtureHtml(html) {
  return /\/@react-refresh|\/@vite\/client|<script\b[^>]*type=["']module["'][^>]*\bsrc=["']\/(?:__|src\/|public-tests\/|private-tests\/)/.test(String(html || ''))
}

/** Build the effective CSP header for a served HTML response in tests without weakening production assets. */
export function buildHarnessCspHeader(baseCsp, html = '') {
  const directives = parseCspDirectives(baseCsp)
  if (isViteDevFixtureHtml(html)) {
    const hashes = computeInlineScriptHashes(html)
    if (hashes.length > 0) {
      const scriptSrc = directives.get('script-src') || [...(directives.get('default-src') || ["'self'"])]
      directives.set('script-src', [...new Set([...scriptSrc, ...hashes])])
    }
    const styleSrc = directives.get('style-src') || [...(directives.get('default-src') || ["'self'"])]
    if (!styleSrc.includes("'unsafe-inline'")) directives.set('style-src', [...styleSrc, "'unsafe-inline'"])
    // Vite's dev client (never shipped) pings its server from a blob: SharedWorker when its HMR
    // socket cannot connect; the built UI creates no worker, so the product policy keeps worker-src 'none'.
    directives.set('worker-src', ["'self'", 'blob:'])
  }
  return serializeCspDirectives(directives)
}

function injectMetaCspIntoHtml(html, csp) {
  const meta = `<meta http-equiv="Content-Security-Policy" content="${stripMetaUnsupportedDirectives(csp).replace(/"/g, '&quot;')}">`
  const text = String(html || '')
  if (/<meta\b[^>]*http-equiv=["']Content-Security-Policy["']/i.test(text)) return text
  if (/<head\b[^>]*>/i.test(text)) return text.replace(/<head\b[^>]*>/i, (head) => `${head}${meta}`)
  return `${meta}${text}`
}

/** Attach `securitypolicyviolation` tracking (and optional HTTP header/meta enforcement) to a Playwright page. */
export async function attachPageCspGuard(page, {
  repoRoot = DEFAULT_REPO_ROOT,
  enforceHeaderOnUnprotectedHtml = true,
  allowViteDevFixtureAdaptation = true,
  cspOverride = null,
  onViolation,
} = {}) {
  const baseCsp = cspOverride || readProductCspPolicy(repoRoot).httpCsp
  const violations = []
  const record = (entry) => {
    violations.push(entry)
    onViolation?.(entry)
  }

  let listeners = pageListeners.get(page)
  const firstAttach = !listeners
  if (!listeners) {
    listeners = new Set()
    pageListeners.set(page, listeners)
  }
  listeners.add(record)

  if (firstAttach) {
    const dispatch = (entry) => {
      for (const listener of listeners) listener(entry)
    }
    await page.addInitScript((prefix) => {
      if (window.__cutCspListenerInstalled) return
      window.__cutCspListenerInstalled = true
      window.__cutCspViolations = window.__cutCspViolations || []
      document.addEventListener('securitypolicyviolation', (event) => {
        const detail = {
          violatedDirective: event.violatedDirective || '',
          effectiveDirective: event.effectiveDirective || '',
          blockedURI: event.blockedURI || '',
          sourceFile: event.sourceFile || '',
          lineNumber: event.lineNumber || 0,
          columnNumber: event.columnNumber || 0,
          sample: (event.sample || '').slice(0, 120),
        }
        window.__cutCspViolations.push(detail)
        console.error(`${prefix}${JSON.stringify(detail)}`)
      }, true)
    }, CSP_VIOLATION_CONSOLE_PREFIX)

    page.on('console', (msg) => {
      const text = msg.text()
      if (text.startsWith(CSP_VIOLATION_CONSOLE_PREFIX)) {
        try {
          dispatch(JSON.parse(text.slice(CSP_VIOLATION_CONSOLE_PREFIX.length)))
        } catch {
          dispatch({ violatedDirective: 'unknown', blockedURI: text })
        }
        return
      }
      if (msg.type() === 'error' && NATIVE_CSP_ERROR_RE.test(text)) {
        dispatch({ violatedDirective: 'console-csp', blockedURI: text.slice(0, 300) })
      }
    })

    if (enforceHeaderOnUnprotectedHtml) {
      const context = page.context()
      if (!wrappedContexts.has(context)) {
        wrappedContexts.add(context)
        await context.route(() => true, async (route) => {
          const request = route.request()
          if (request.resourceType() !== 'document' || !/^https?:\/\//i.test(request.url())) {
            return route.fallback()
          }
          let response
          try {
            response = await route.fetch()
          } catch {
            return route.fallback().catch(() => {})
          }
          const headers = { ...response.headers() }
          const existing = Object.keys(headers).find((k) => k.toLowerCase() === 'content-security-policy')
          const contentType = headers['content-type'] || headers['Content-Type'] || ''
          if (!existing && response.status() >= 200 && response.status() < 300 && /text\/html/i.test(contentType)) {
            const body = await response.body()
            const html = body.toString('utf8')
            const headerCsp = allowViteDevFixtureAdaptation ? buildHarnessCspHeader(baseCsp, html) : baseCsp
            headers['content-security-policy'] = headerCsp
            return route.fulfill({ response, body, headers })
          }
          return route.fulfill({ response, headers })
        })
      }

      const originalSetContent = page.setContent.bind(page)
      page.setContent = async (html, options) => {
        const effectiveCsp = allowViteDevFixtureAdaptation ? buildHarnessCspHeader(baseCsp, html) : baseCsp
        return originalSetContent(injectMetaCspIntoHtml(html, effectiveCsp), options)
      }
    }
  }

  return {
    violations,
    drain() { return violations.splice(0, violations.length) },
    assertClean(label = 'Playwright page') {
      if (violations.length > 0) {
        const summary = violations.slice(0, 4).map((v) => `${v.effectiveDirective || v.violatedDirective} -> ${v.blockedURI || v.sample || 'inline'}`).join('; ')
        throw new Error(`${label} observed ${violations.length} CSP violation(s): ${summary}`)
      }
    },
  }
}

/** Attach standard pageerror and securitypolicyviolation failure listeners for full-coverage verification. */
export async function attachFullCoveragePageErrorListeners(page, focusedPageErrors, consoleErrors) {
  page.on('pageerror', (e) => {
    const message = String(e?.message || e).slice(0, 4_000)
    focusedPageErrors.push({ message, stack: String(e?.stack || '').slice(0, 4_000) })
    consoleErrors.push(`PAGEERROR ${String(e).slice(0, 100)}`)
  })
  await attachPageCspGuard(page, {
    enforceHeaderOnUnprotectedHtml: false,
    onViolation: (v) => {
      const detail = `CSP ${v.effectiveDirective || v.violatedDirective} -> ${v.blockedURI || v.sample || 'inline'}`
      focusedPageErrors.push({ message: detail, stack: v.sourceFile ? `${v.sourceFile}:${v.lineNumber || 0}` : '' })
      consoleErrors.push(detail)
    },
  })
}
