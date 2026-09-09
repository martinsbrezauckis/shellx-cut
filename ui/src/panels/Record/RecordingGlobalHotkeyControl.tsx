import type { RecordHotkeyCapability } from '../../lib/tauri'

interface RecordingGlobalHotkeyControlProps {
  capability: RecordHotkeyCapability
  disabled: boolean
  onEnable: () => void
  onDisable: () => void
}

function statusCopy(capability: RecordHotkeyCapability): string {
  if (capability.state === 'observed' || capability.state === 'registered') {
    return 'F9 works globally, including while another app is focused.'
  }
  if (capability.state === 'configured') {
    return 'GNOME is configured. Press F9 once to confirm the desktop callback.'
  }
  return capability.reason || 'F9 works while ShellX Cut is focused.'
}

/**
 * A small Record-settings row for GNOME's user-visible custom shortcut.
 * The configured state deliberately stays separate from the observed callback:
 * a saved binding cannot prove that GNOME has delivered it to this app yet.
 */
export function RecordingGlobalHotkeyControl({
  capability,
  disabled,
  onEnable,
  onDisable,
}: RecordingGlobalHotkeyControlProps) {
  if (!capability.can_enable) return null
  const configured = capability.enabled
  return (
    <div
      className="rec__field rec__field--global-hotkey"
      data-cut-rec-global-hotkey-state={capability.state}
      data-cut-rec-global-hotkey-scope={capability.scope}
    >
      <span className="rec__label">Record shortcut</span>
      <div className="rec__global-hotkey-controls">
        <button
          type="button"
          className="rec__export-btn rec__export-btn--small"
          data-cut-action="record-global-f9-enable"
          disabled={disabled || configured}
          onClick={onEnable}
        >
          Enable global F9
        </button>
        <button
          type="button"
          className="rec__export-btn rec__export-btn--ghost rec__export-btn--small"
          data-cut-action="record-global-f9-disable"
          disabled={disabled || !configured}
          onClick={onDisable}
        >
          Disable global F9
        </button>
      </div>
      <p className="rec__source-note" data-cut-rec-global-hotkey-detail>{statusCopy(capability)}</p>
    </div>
  )
}
