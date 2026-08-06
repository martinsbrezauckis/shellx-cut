// topbar/UpdateButton.tsx — the ONE quiet update affordance in the topbar.
//
// Renders exactly one pill button ("Update to vX") when — and only when — the
// desktop shell's update snapshot says a newer signed release is available
// (found by the automatic check at (re)launch or the 6-hourly re-check, or by
// a manual check in Settings > About). Everything else renders nothing:
// browser/remote builds (isTauri() false), up-to-date, error, unsupported
// (Linux deb/rpm), and idle states all stay invisible here — their honest
// detail lives in Settings > About. Never a modal: clicking asks the SHELL to
// run its install flow (native confirm → signature-verified install →
// restart); a decline resets quietly, a failure shows the shell's honest
// error as the button title + About status.
//
// State arrives over the narrow bridge (lib/tauri.ts): an initial
// get_update_state read plus the `cut:update-state` event stream. The DOM
// CustomEvent 'cut:refresh-update-state' forces a re-read — the same
// re-sync idiom as 'cut:refresh-doctor', used by tests/fixtures after they
// swap the bridge under the mounted app.
//
// Callers: topbar/index.tsx. Deps: lib/tauri (bridge), lib/updateState (pure
// model), topbar.css (.tb-update).

import { useCallback, useEffect, useRef, useState } from 'react'
import {
  getShellUpdateState,
  installUpdateNow,
  isTauri,
  onShellUpdateState,
} from '../lib/tauri'
import {
  shouldShowUpdateButton,
  updateButtonLabel,
  type ShellUpdateState,
} from '../lib/updateState'

export default function UpdateButton() {
  const [state, setState] = useState<ShellUpdateState | null>(null)
  // Local in-flight flag: covers the gap between the click and the shell's
  // `installing:true` broadcast so double-clicks can't queue two installs.
  const [requesting, setRequesting] = useState(false)
  const live = useRef(true)

  const refresh = useCallback(() => {
    // Re-checks isTauri() each time: a test fixture that installs a bridge
    // then dispatches 'cut:refresh-update-state' gets a real read, and a
    // removed bridge clears the state (no bridge ⇒ no update surface — the
    // button must never keep showing a stale offer).
    if (!isTauri()) {
      setState(null)
      return
    }
    void getShellUpdateState().then((snapshot) => {
      if (live.current && snapshot) setState(snapshot)
    })
  }, [])

  useEffect(() => {
    live.current = true
    refresh()
    const off = onShellUpdateState((snapshot) => {
      if (live.current) setState(snapshot)
    })
    const onRefresh = () => refresh()
    document.addEventListener('cut:refresh-update-state', onRefresh)
    return () => {
      live.current = false
      off()
      document.removeEventListener('cut:refresh-update-state', onRefresh)
    }
  }, [refresh])

  if (!shouldShowUpdateButton(state) || !state) return null

  const busy = requesting || state.installing
  const install = async () => {
    if (busy) return
    setRequesting(true)
    // The shell owns confirm + install + restart. On success this promise
    // never resolves (the app restarts); a decline or failure resolves with
    // the honest reply and the broadcast snapshot carries any error text.
    const reply = await installUpdateNow()
    if (!live.current) return
    setRequesting(false)
    if (reply && !reply.ok && !reply.cancelled) refresh()
  }

  return (
    <button
      type="button"
      className="tb-btn tb-update"
      data-cut-update-btn
      data-cut-update-version={state.version ?? ''}
      aria-label={`Update to version ${state.version}`}
      aria-busy={busy}
      disabled={busy}
      title={state.error
        ? `Last attempt failed: ${state.error} — click to try again`
        : `ShellX Cut ${state.version} is ready — installs and restarts after you confirm`}
      onClick={(e) => { e.currentTarget.blur(); void install() }}
    >
      <span className="tb-update-dot" aria-hidden="true" />
      {updateButtonLabel(state)}
    </button>
  )
}
