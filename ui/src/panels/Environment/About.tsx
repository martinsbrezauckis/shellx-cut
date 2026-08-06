// About.tsx — About + version + the FULL update surface for Settings > About.
//
// The version comes from the engine doctor's `app_version` (system.doctor): the
// Cut UI is SERVED BY cutd (a remote loopback origin) where Tauri 2 blocks
// plugin/command IPC, so the engine — not a Tauri call — is the version source
// of truth. The auto-updater itself runs in the desktop SHELL
// (app/desktop/src-tauri/src/update_state.rs): quiet checks at launch and
// every 6 hours (both governed by the Storage & privacy toggle), results
// surfaced as the topbar button + this panel — never a startup dialog.
//
// This panel shows the whole honest picture over the narrow bridge
// (lib/tauri.ts): current status ("available vX" / "you're on the latest" /
// the exact failure text / the Linux deb-rpm explanation), a "checked N ago"
// timestamp, a manual "Check for updates" button (works even when automatic
// checks are off — an explicit click is its own consent), an "Install &
// restart" action while an update is offered (the shell still runs the native
// confirm + signature-verified install), and a release-notes link. In a
// browser/remote build none of the controls render (isTauri() false) — only
// the disclosure prose and links remain. On Linux the same surface explains
// the deb/rpm flow instead of showing dead buttons.
//
// Callers: SettingsCategoryContent (Settings > About). Deps: lib/doctor,
// lib/tauri (bridge), lib/updateState (pure model).

import { useCallback, useEffect, useRef, useState } from 'react'
import type { DoctorReport } from '../../lib/doctor'
import {
  checkForUpdatesNow,
  getShellUpdateState,
  installUpdateNow,
  isTauri,
  onShellUpdateState,
} from '../../lib/tauri'
import {
  describeUpdateStatus,
  formatCheckedAgo,
  releaseNotesUrl,
  shouldShowUpdateButton,
  type ShellUpdateState,
} from '../../lib/updateState'
import './about-update.css'

export default function About({ report }: { report: DoctorReport | null }) {
  const version = report?.app_version ?? '—'
  const [snapshot, setSnapshot] = useState<ShellUpdateState | null>(null)
  // Local honest feedback when the BRIDGE itself fails (distinct from the
  // shell reporting a failed check inside a valid snapshot).
  const [bridgeError, setBridgeError] = useState<string | null>(null)
  const [requestingCheck, setRequestingCheck] = useState(false)
  const [requestingInstall, setRequestingInstall] = useState(false)
  // Re-render tick so "Checked N ago" stays truthful while the panel is open.
  const [nowMs, setNowMs] = useState(() => Date.now())
  const live = useRef(true)

  const refresh = useCallback(() => {
    // Re-checks isTauri() each call — a test fixture that installs a bridge
    // then dispatches 'cut:refresh-update-state' (the 'cut:refresh-doctor'
    // idiom) gets a read; a removed bridge clears the surface entirely (no
    // bridge ⇒ no controls, matching the browser build).
    if (!isTauri()) {
      setSnapshot(null)
      setBridgeError(null)
      return
    }
    void getShellUpdateState().then((state) => {
      if (live.current && state) setSnapshot(state)
    })
  }, [])

  useEffect(() => {
    live.current = true
    refresh()
    const off = onShellUpdateState((state) => {
      if (live.current) setSnapshot(state)
    })
    const onRefresh = () => refresh()
    document.addEventListener('cut:refresh-update-state', onRefresh)
    const tick = setInterval(() => setNowMs(Date.now()), 30_000)
    return () => {
      live.current = false
      off()
      document.removeEventListener('cut:refresh-update-state', onRefresh)
      clearInterval(tick)
    }
  }, [refresh])

  const checkNow = async () => {
    if (requestingCheck) return
    setRequestingCheck(true)
    setBridgeError(null)
    // Manual check: the shell command ignores the automatic-check preference,
    // so this button keeps working when Storage & privacy turns auto off.
    const state = await checkForUpdatesNow()
    if (!live.current) return
    setRequestingCheck(false)
    if (state) setSnapshot(state)
    else setBridgeError('The desktop shell did not answer the update check request.')
  }

  const installNow = async () => {
    if (requestingInstall) return
    setRequestingInstall(true)
    setBridgeError(null)
    // Shell-owned flow: native confirm → verified install → restart. On
    // success this never resolves; decline/failure resolves honestly and the
    // snapshot broadcast carries the error text for the status line.
    const reply = await installUpdateNow()
    if (!live.current) return
    setRequestingInstall(false)
    if (reply && !reply.ok && !reply.cancelled) refresh()
  }

  // Desktop shell present = the update surface is real. Browser/remote builds
  // render prose only (no dead controls — nothing here can act there).
  const desktop = snapshot !== null
  const status = describeUpdateStatus(snapshot)
  const checkedAgo = snapshot ? formatCheckedAgo(snapshot.checked_at, nowMs) : null
  const installable = shouldShowUpdateButton(snapshot)
  const busy = requestingCheck || requestingInstall || !!snapshot?.checking || !!snapshot?.installing

  return (
    <section className="env-about" data-cut-about data-cut-app-version={version}>
      <div className="env-about-row">
        <span className="env-about-name">ShellX Cut</span>
        <span className="env-about-version" data-cut-about-version>
          v{version}
        </span>
      </div>
      <p className="env-about-tag">
        Agent-first video editor — every edit is a verb, every render ships a measured receipt.
      </p>

      {desktop && (
        <div className="env-about-updates" data-cut-about-update-panel>
          <p
            className={`env-about-status env-about-status--${status.tone}`}
            data-cut-about-update-status={snapshot?.status ?? 'unknown'}
            role="status"
            aria-live="polite"
          >
            {status.text}
            {/* The available state can carry a fresh failure too (e.g. a later
                check or install attempt failed) — show both honestly. */}
            {snapshot?.status === 'available' && snapshot.error && !snapshot.checking && !snapshot.installing && (
              <span className="env-about-status-error"> Last attempt failed: {snapshot.error}</span>
            )}
            {bridgeError && <span className="env-about-status-error"> {bridgeError}</span>}
          </p>
          {checkedAgo && (
            <p className="env-about-checked" data-cut-about-update-checked={snapshot?.checked_at ?? ''}>
              {checkedAgo}
            </p>
          )}
          {snapshot?.supported && (
            <div className="env-about-update-actions">
              <button
                type="button"
                className="env-btn env-btn--secondary env-btn--sm"
                data-cut-about-check-updates
                disabled={busy}
                aria-busy={requestingCheck || !!snapshot?.checking}
                title="Contact GitHub once now to read the signed release feed — works even when automatic checks are off"
                onClick={() => void checkNow()}
              >
                {requestingCheck || snapshot?.checking ? 'Checking…' : 'Check for updates'}
              </button>
              {installable && (
                <button
                  type="button"
                  className="env-btn env-btn--primary env-btn--sm"
                  data-cut-about-install-update
                  disabled={busy}
                  aria-busy={requestingInstall || !!snapshot?.installing}
                  title={`Install ShellX Cut ${snapshot?.version} — asks for confirmation, then restarts the app`}
                  onClick={() => void installNow()}
                >
                  {requestingInstall || snapshot?.installing ? 'Installing…' : 'Install & restart'}
                </button>
              )}
            </div>
          )}
        </div>
      )}

      <p className="env-about-update" data-cut-about-update>
        The installed app checks GitHub for signed releases at launch and every 6 hours while it stays open. You can turn automatic checks off under Storage &amp; privacy; the manual check here still works, and installing always asks before restart. Linux builds update through deb/rpm packages instead and make no update request.
      </p>
      <div className="env-about-links">
        <a href="https://theshellx.com" target="_blank" rel="noopener noreferrer">
          theshellx.com
        </a>
        <span className="env-about-dot">·</span>
        <a
          href="https://github.com/martinsbrezauckis/shellx-cut"
          target="_blank"
          rel="noopener noreferrer"
        >
          GitHub
        </a>
        <span className="env-about-dot">·</span>
        <a
          href={releaseNotesUrl(snapshot)}
          target="_blank"
          rel="noopener noreferrer"
          data-cut-about-release-notes
          title={installable ? `Release notes for ShellX Cut ${snapshot?.version}` : 'Release notes for the latest published version'}
        >
          release notes
        </a>
        <span className="env-about-dot">·</span>
        <span className="env-about-license">MIT</span>
      </div>
    </section>
  )
}
