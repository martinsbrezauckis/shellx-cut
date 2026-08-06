// layout/panelPersistGuard.ts — crash-safe persistence for the right-rail tab.
//
// Role: makes the persisted right-sidebar tab (useLayout `rightTab`) safe to
// restore after a render crash. Field failure this guards against (observed
// 2026-08-06, Linux Xvfb/llvmpipe + WEBKIT_DISABLE_COMPOSITING_MODE=1):
// opening the Color tab hard-blanked the whole WebView (WebKitGTK web-process
// death at paint time), and because `rightTab:"color"` + `railCollapsed:false`
// were already persisted, EVERY subsequent launch restored the killer panel and
// booted blank until ~/.local/share/lv.shellx.cut was wiped by hand.
//
// A web-process crash runs no JS cleanup, so the guard is a TWO-PHASE COMMIT
// against localStorage (which survives the crash):
//   1. armPanelAttempt(tab)    — written synchronously BEFORE a tab body mounts.
//   2. confirmPanelPainted(tab) — after the mounted panel provably painted
//      (double requestAnimationFrame + a short settle delay); clears the
//      sentinel and un-blocks the tab.
//   3. disarmPanelAttempt(tab) — clean switch-away/unmount (JS still alive).
// If the WebView dies between 1 and 2, the sentinel is still armed at the next
// boot. adoptUnconfirmedPanelAttempt() then moves that tab onto a BLOCKLIST,
// and safeRightTab() refuses to restore a blocklisted tab (falls back to the
// safe default). The tab stays openable BY HAND — AppRightRail shows an honest
// notice with a "load anyway" action first — and a later successful paint
// (step 2) self-heals the blocklist, so a healthy machine restores normally.
//
// Honest limitation: a crash AFTER the confirmed first paint (e.g. seconds into
// interaction) is not caught by the sentinel — the observed field failure was a
// blank at open/paint time, which this covers. recordPanelRenderFailure() also
// lets the React error boundary blocklist a tab whose render THREW (the
// JS-catchable class).
//
// Storage may be unavailable (private mode / quota): every helper degrades to a
// no-op, matching useLayout's own persistence behavior — the app always runs.
//
// Callers: layout/useLayout.ts (boot-time adopt + safe restore),
// app/AppRightRail.tsx (arm/disarm/confirm + blocked notice),
// components/PanelErrorBoundary.tsx (recordPanelRenderFailure).
// Dependencies: none (localStorage only) — kept pure for node-level tests
// (ui/public-tests/panel-persist-guard.test.ts).

/** Sentinel: the right-tab body that is mounting/mounted but has not yet
 *  confirmed a successful paint. Shape: {"tab":string,"at":number}. */
const ATTEMPT_KEY = 'cut.panelAttempt.v1'
/** Blocklist: tabs whose last mount never confirmed a paint (previous session
 *  died) or whose render threw. Shape: {[tab:string]: number} (epoch ms). */
const BLOCKED_KEY = 'cut.panelBlocked.v1'

/** The always-safe right tab. Restore falls back here; it is never refused
 *  (otherwise a corrupt blocklist could leave the rail with no restorable tab). */
export const SAFE_RIGHT_TAB = 'properties'

/** Read a JSON object from storage; null on absence/corruption/no storage. */
function readJson(key: string): Record<string, unknown> | null {
  try {
    const raw = localStorage.getItem(key)
    if (!raw) return null
    const parsed: unknown = JSON.parse(raw)
    return parsed !== null && typeof parsed === 'object' ? (parsed as Record<string, unknown>) : null
  } catch {
    return null
  }
}

/** Best-effort write; storage failures are swallowed (guard becomes inert). */
function writeJson(key: string, value: unknown): void {
  try {
    localStorage.setItem(key, JSON.stringify(value))
  } catch {
    // storage unavailable — persistence safety degrades to session-only
  }
}

function removeKey(key: string): void {
  try {
    localStorage.removeItem(key)
  } catch {
    // storage unavailable — nothing to remove
  }
}

/** Arm the sentinel for `tab`. MUST run synchronously before the tab body is
 *  allowed to mount (AppRightRail does this in a layout effect, which commits
 *  before the browser paints the panel). Overwrites any previous sentinel —
 *  only one right-tab body exists at a time. */
export function armPanelAttempt(tab: string): void {
  writeJson(ATTEMPT_KEY, { tab, at: Date.now() })
}

/** Clean switch-away/unmount: JS is alive, so the mount did not kill the
 *  WebView. Clears the sentinel only if it still belongs to `tab`. */
export function disarmPanelAttempt(tab: string): void {
  const current = readJson(ATTEMPT_KEY)
  if (current && current.tab === tab) removeKey(ATTEMPT_KEY)
}

/** The mounted tab provably painted: clear the sentinel AND self-heal the
 *  blocklist for that tab, so a machine where the panel works again restores
 *  it normally on the next launch. */
export function confirmPanelPainted(tab: string): void {
  disarmPanelAttempt(tab)
  const blocked = readJson(BLOCKED_KEY)
  if (blocked && tab in blocked) {
    delete blocked[tab]
    writeJson(BLOCKED_KEY, blocked)
  }
}

/** A tab body's render threw (React error boundary). Blocklist it so restore
 *  never boots into it, and clear its sentinel — the boundary caught the
 *  failure, so this session is still alive and showing the honest notice. */
export function recordPanelRenderFailure(tab: string): void {
  const blocked = readJson(BLOCKED_KEY) ?? {}
  blocked[tab] = Date.now()
  writeJson(BLOCKED_KEY, blocked)
  disarmPanelAttempt(tab)
}

/** True when `tab` previously failed to render (crash-before-paint in an
 *  earlier session, or a caught render error) and has not painted since. */
export function isPanelRenderBlocked(tab: string): boolean {
  const blocked = readJson(BLOCKED_KEY)
  return !!blocked && tab in blocked
}

/** Boot-time adoption: a sentinel that survived into a NEW session means the
 *  previous session ended while that tab was mounting/mounted without a
 *  confirmed paint — treat it as the killer and blocklist it. Returns the
 *  adopted tab (for logging/tests) or null. Idempotent: the sentinel is
 *  consumed, so a second call in the same boot is a no-op.
 *
 *  Called from useLayout's load(), which runs once per WebView boot BEFORE
 *  AppRightRail can arm a new sentinel — so any sentinel seen here is
 *  necessarily from a previous session, never from this one. */
export function adoptUnconfirmedPanelAttempt(): string | null {
  const attempt = readJson(ATTEMPT_KEY)
  if (!attempt || typeof attempt.tab !== 'string' || !attempt.tab) return null
  const tab = attempt.tab
  removeKey(ATTEMPT_KEY)
  // A dead SAFE_RIGHT_TAB session is recorded too (honest data), but
  // safeRightTab() never refuses the fallback, so it cannot brick restore.
  const blocked = readJson(BLOCKED_KEY) ?? {}
  blocked[tab] = Date.now()
  writeJson(BLOCKED_KEY, blocked)
  return tab
}

/** Orderly unload discriminator. A real paint-crash kills the web process, so
 *  it can never run JS — while an intentional reload/navigation/quit always
 *  fires `pagehide` with JS alive. An armed-but-unconfirmed sentinel at
 *  pagehide is therefore NOT crash evidence and must not survive into the next
 *  boot: without this, a reload landing inside the arm→confirm window
 *  blocklists an innocent tab. Field failure (2026-08-06, Windows strict
 *  qualification): a scripted fresh-project reload right after opening the
 *  Chat tab blocklisted Chat for the rest of the run while the same run's
 *  screenshots proved Chat painting fine.
 *
 *  Register ONCE per WebView boot (AppRightRail does this in a mount effect).
 *  `target` is injectable so node-level tests can drive it without a DOM. */
export function disarmPanelAttemptOnOrderlyUnload(
  target: Pick<Window, 'addEventListener'> | null = typeof window === 'undefined' ? null : window,
): void {
  if (!target) return
  target.addEventListener('pagehide', () => removeKey(ATTEMPT_KEY))
}

/** Restore-time gate: never restore a tab that failed to render. The fallback
 *  itself is always allowed, so restore can never dead-end. */
export function safeRightTab<T extends string>(restored: T, fallback: T): T {
  if (restored === fallback) return restored
  return isPanelRenderBlocked(restored) ? fallback : restored
}
