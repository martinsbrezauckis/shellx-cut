// ui/public-tests/panel-persist-guard.test.ts — crash-safe right-tab restore
// (run: `npm run test:lib`, or `tsx public-tests/panel-persist-guard.test.ts`).
//
// Red-proof for the 2026-08-06 field failure: opening the Color tab under
// software rendering (Xvfb/llvmpipe, WEBKIT_DISABLE_COMPOSITING_MODE=1) killed
// the WebKitGTK web process at paint time, and the persisted layout
// (rightTab:"color") made every subsequent launch boot straight back into the
// killer panel — blank forever until the profile dir was wiped.
//
// These tests drive the REAL loadLayout() + panelPersistGuard against a fake
// localStorage: simulate "panel mounted, session died before a confirmed
// paint" (armed sentinel left behind), then "restart" (fresh loadLayout call),
// and assert the app must NOT restore the killer panel. Written before the
// guard was wired into loadLayout — it failed then (restored 'color'), which
// proves the assertions bite.
//
// No DOM: the guard reads only globalThis.localStorage, stubbed below BEFORE
// the modules are imported (dynamic import keeps the order deterministic).

import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { strict as assert } from 'node:assert'

// ---- localStorage stub (Map-backed, same sync semantics) -------------------
const store = new Map<string, string>()
;(globalThis as { localStorage?: unknown }).localStorage = {
  getItem: (k: string) => (store.has(k) ? store.get(k)! : null),
  setItem: (k: string, v: string) => {
    store.set(k, String(v))
  },
  removeItem: (k: string) => {
    store.delete(k)
  },
  clear: () => store.clear(),
}

const { loadLayout, LAYOUT_DEFAULTS } = await import('../src/layout/useLayout')
const {
  armPanelAttempt,
  disarmPanelAttempt,
  disarmPanelAttemptOnOrderlyUnload,
  confirmPanelPainted,
  recordPanelRenderFailure,
  isPanelRenderBlocked,
  SAFE_RIGHT_TAB,
} = await import('../src/layout/panelPersistGuard')

/** Seed the persisted layout the demo session left behind: Color tab open. */
function persistColorSession(): void {
  store.clear()
  localStorage.setItem(
    'cut.layout.v1',
    JSON.stringify({ ...LAYOUT_DEFAULTS, rightTab: 'color', railCollapsed: false }),
  )
}

// ---- 1. The killer scenario: mount armed, session died, restart ------------
persistColorSession()
armPanelAttempt('color') // AppRightRail arms this before the Color body mounts
// WebView dies here: no disarm, no confirm — the sentinel survives in storage.
const afterCrash = loadLayout() // next launch
assert.equal(
  afterCrash.rightTab,
  SAFE_RIGHT_TAB,
  'a panel that never confirmed a paint must NOT be restored after restart',
)
assert.equal(isPanelRenderBlocked('color'), true, 'the killer panel is blocklisted after the crash')

// ---- 2. The block persists across further restarts (sentinel consumed) -----
const secondBoot = loadLayout()
assert.equal(secondBoot.rightTab, SAFE_RIGHT_TAB, 'later launches still refuse the blocked panel')

// ---- 3. A successful paint self-heals: restore works again -----------------
armPanelAttempt('color') // user opened Color by hand ("load anyway")
confirmPanelPainted('color') // …and this time it actually painted
assert.equal(isPanelRenderBlocked('color'), false, 'a confirmed paint clears the block')
assert.equal(loadLayout().rightTab, 'color', 'a healed panel restores normally again')

// ---- 4. Clean switch-away never poisons anything ---------------------------
persistColorSession()
armPanelAttempt('audio')
disarmPanelAttempt('audio') // clean unmount — JS alive, no crash
assert.equal(isPanelRenderBlocked('audio'), false, 'clean unmount leaves no block')
assert.equal(loadLayout().rightTab, 'color', 'an untouched persisted tab still restores')

// ---- 5. A caught render error (error boundary) also blocks restore ---------
persistColorSession()
recordPanelRenderFailure('color')
assert.equal(loadLayout().rightTab, SAFE_RIGHT_TAB, 'a tab whose render threw is not restored')

// ---- 6. The fallback tab itself can never be refused -----------------------
store.clear()
localStorage.setItem(
  'cut.layout.v1',
  JSON.stringify({ ...LAYOUT_DEFAULTS, rightTab: SAFE_RIGHT_TAB, railCollapsed: false }),
)
armPanelAttempt(SAFE_RIGHT_TAB) // even a death on the safe tab…
assert.equal(loadLayout().rightTab, SAFE_RIGHT_TAB, '…must still restore the safe tab (no dead end)')

// ---- 7. Storage unavailable: guard is inert, app still boots ---------------
delete (globalThis as { localStorage?: unknown }).localStorage
assert.equal(loadLayout().rightTab, LAYOUT_DEFAULTS.rightTab, 'no storage → defaults, no throw')
armPanelAttempt('color') // must not throw either
;(globalThis as { localStorage?: unknown }).localStorage = {
  getItem: (k: string) => (store.has(k) ? store.get(k)! : null),
  setItem: (k: string, v: string) => {
    store.set(k, String(v))
  },
  removeItem: (k: string) => {
    store.delete(k)
  },
  clear: () => store.clear(),
}

// ---- 8. Source wiring: the runtime halves actually use the guard -----------
// (Same source-contract style as theme-toggle-sync.test.ts: a refactor that
// silently drops the arm/confirm calls fails here, not in the field.)
const root = resolve(import.meta.dirname, '..')
const rail = readFileSync(resolve(root, 'src/app/AppRightRail.tsx'), 'utf8')
const boundary = readFileSync(resolve(root, 'src/components/PanelErrorBoundary.tsx'), 'utf8')
assert.match(rail, /armPanelAttempt\(/, 'AppRightRail arms the sentinel before a tab body mounts')
assert.match(rail, /disarmPanelAttempt\(/, 'AppRightRail disarms on clean switch-away')
assert.match(rail, /confirmPanelPainted\(/, 'AppRightRail confirms only after a proven paint')
assert.match(rail, /isPanelRenderBlocked\(/, 'AppRightRail shows the notice for a blocked tab instead of mounting it')
assert.match(rail, /data-cut-panel-render-blocked/, 'blocked notice carries a stable agent selector')
assert.match(rail, /data-cut-panel-render-retry/, 'blocked notice offers a load-anyway action with a stable selector')
assert.match(boundary, /recordPanelRenderFailure\(/, 'the error boundary blocklists a tab whose render threw')
assert.match(rail, /PanelErrorBoundary/, 'the right-rail tab bodies are wrapped in the error boundary')

// ---- 9. Orderly-unload discriminator (2026-08-06 Windows false positive) ---
// A reload/navigation fires pagehide with JS ALIVE — not a paint-crash. The
// armed sentinel must be cleared by the pagehide handler so the next boot does
// NOT blocklist an innocent tab. Red-proof: before the fix this export did not
// exist and a reload inside the arm→confirm window blocklisted Chat on the
// Windows rig while same-run screenshots showed Chat painting fine.
{
  store.clear()
  const handlers: Array<() => void> = []
  const fakeTarget = {
    addEventListener: (type: string, cb: () => void) => {
      assert.equal(type, 'pagehide', 'the discriminator listens to pagehide (crash = no JS = never fires)')
      handlers.push(cb)
    },
  }
  disarmPanelAttemptOnOrderlyUnload(fakeTarget as Pick<Window, 'addEventListener'>)
  assert.equal(handlers.length, 1, 'registers exactly one pagehide handler')
  armPanelAttempt('chat')
  handlers[0]!()
  assert.equal(localStorage.getItem('cut.panelAttempt.v1'), null, 'orderly unload clears the armed sentinel')
  // And the blocklist stays empty at the "next boot": nothing to adopt.
  const { adoptUnconfirmedPanelAttempt } = await import('../src/layout/panelPersistGuard')
  assert.equal(adoptUnconfirmedPanelAttempt(), null, 'next boot adopts nothing after an orderly unload')
  assert.equal(isPanelRenderBlocked('chat'), false, 'no false-positive blocklist entry')
  // Null target (SSR/no window) degrades to a no-op instead of throwing.
  disarmPanelAttemptOnOrderlyUnload(null)
}
assert.match(rail, /disarmPanelAttemptOnOrderlyUnload\(\)/, 'AppRightRail registers the orderly-unload discriminator once per boot')

console.log('PASS panel persist guard (crash-safe right-tab restore)')
