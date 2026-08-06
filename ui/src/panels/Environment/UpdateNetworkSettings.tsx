// UpdateNetworkSettings.tsx — the Storage & privacy control for AUTOMATIC
// update checks (Settings > Storage & privacy > Network activity).
//
// One switch governs ALL automatic release-feed checks the desktop shell
// performs: the launch check and the 6-hourly re-check while the app stays
// open (update_state.rs re-reads this preference before every automatic
// check, so turning it off stops them immediately — no restart needed). The
// manual "Check for updates" button in Settings > About is deliberately NOT
// governed by this switch: an explicit click is its own consent.
//
// Callers: SettingsCategoryContent (storage-privacy). Deps: lib/tauri
// (get/set of the persisted native-shell preference).

import { useEffect, useState } from 'react'
import {
  getLaunchUpdatePreference,
  isTauri,
  setLaunchUpdatePreference,
} from '../../lib/tauri'

export default function UpdateNetworkSettings() {
  const desktop = isTauri()
  const [enabled, setEnabled] = useState<boolean | null>(null)
  const [saving, setSaving] = useState(false)
  const [message, setMessage] = useState('')

  useEffect(() => {
    let current = true
    void getLaunchUpdatePreference().then((preference) => {
      if (!current) return
      setEnabled(preference?.check_on_launch ?? true)
      if (desktop && !preference) {
        setMessage('The installed app could not read this preference. Update checks remain on by default.')
      }
    })
    return () => { current = false }
  }, [desktop])

  const changePreference = async (next: boolean) => {
    if (!desktop || saving) return
    setSaving(true)
    setMessage('')
    const saved = await setLaunchUpdatePreference(next)
    setSaving(false)
    if (!saved) {
      setMessage('Could not save this preference. The previous setting is unchanged.')
      return
    }
    setEnabled(saved.check_on_launch)
    setMessage(saved.check_on_launch
      ? 'Automatic update checks are on: at launch and every 6 hours while the app stays open.'
      : 'Automatic update checks are off. The manual Check for updates button in About still works.')
  }

  const checked = enabled ?? true
  return (
    <section className="settings-network" data-cut-network-activity>
      <div className="settings-network-head">
        <p className="settings-eyebrow">Network activity</p>
        <h4>Quiet checks for new releases</h4>
        <p>
          By default, the installed app contacts GitHub when it opens, and then every 6 hours while
          it stays open, to read release metadata. GitHub receives normal request metadata such as
          your IP address; Cut sends no project, media, edit history, or analytics payload. Finding
          an update never interrupts you — it only shows a topbar button and the status in About.
        </p>
      </div>
      <label className={`settings-network-toggle${desktop ? '' : ' settings-network-toggle--disabled'}`}>
        <input
          type="checkbox"
          role="switch"
          checked={checked}
          disabled={!desktop || enabled === null || saving}
          data-cut-action="update-check-on-launch"
          data-cut-update-check-on-launch
          onChange={(event) => { void changePreference(event.currentTarget.checked) }}
        />
        <span>
          <strong>Check for app updates automatically</strong>
          <small>{desktop ? 'Covers the launch check and the 6-hour re-check; changes apply immediately. Checking from About stays available.' : 'Available in the installed desktop app.'}</small>
        </span>
      </label>
      <p
        className={message.startsWith('Could not') ? 'settings-network-message settings-network-message--error' : 'settings-network-message'}
        data-cut-update-check-status
        aria-live="polite"
      >
        {message}
      </p>
    </section>
  )
}
