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
      ? 'Automatic update checks will run from the next launch.'
      : 'Automatic update checks are off from the next launch.')
  }

  const checked = enabled ?? true
  return (
    <section className="settings-network" data-cut-network-activity>
      <div className="settings-network-head">
        <p className="settings-eyebrow">Network activity</p>
        <h4>One quiet check for new releases</h4>
        <p>
          By default, the installed app contacts GitHub once when it opens to read release metadata.
          GitHub receives normal request metadata such as your IP address; Cut sends no project,
          media, edit history, or analytics payload.
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
          <strong>Check for app updates when ShellX Cut opens</strong>
          <small>{desktop ? 'Changes apply on the next launch.' : 'Available in the installed desktop app.'}</small>
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
