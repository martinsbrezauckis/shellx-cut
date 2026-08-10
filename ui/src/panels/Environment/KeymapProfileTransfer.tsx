import { useState } from 'react'
import { writeClipboardText } from '../../lib/clipboard'
import {
  KEYMAP_PRESETS,
  applyKeymapPreset,
  importKeymapProfile,
  serializeKeymapProfile,
  type KeymapPresetId,
} from '../../lib/keymapProfile'

interface KeymapProfileTransferProps {
  onInfo: (text: string) => void
  onWarning: (text: string) => void
  onBegin: () => void
}

export default function KeymapProfileTransfer({
  onInfo,
  onWarning,
  onBegin,
}: KeymapProfileTransferProps) {
  const [open, setOpen] = useState(false)
  const [text, setText] = useState('')
  const [presetId, setPresetId] = useState<KeymapPresetId | ''>('')
  const preset = KEYMAP_PRESETS.find((candidate) => candidate.id === presetId)

  const applyPreset = () => {
    onBegin()
    if (!preset) return
    const result = applyKeymapPreset(preset.id)
    if (!result.ok) {
      onWarning(result.reason)
      return
    }
    onInfo(`${preset.label} applied: ${result.changed} custom binding${result.changed === 1 ? '' : 's'}.`)
    setPresetId('')
  }

  const copy = async () => {
    onBegin()
    try {
      await writeClipboardText(serializeKeymapProfile())
      onInfo('Shortcut profile copied as portable JSON.')
    } catch {
      onWarning('Clipboard unavailable. Open Import profile and paste JSON from another trusted editor.')
    }
  }

  const apply = () => {
    onBegin()
    const result = importKeymapProfile(text)
    if (!result.ok) {
      onWarning(result.reason)
      return
    }
    const ignored = result.ignored > 0 ? ` ${result.ignored} unknown command${result.ignored === 1 ? '' : 's'} ignored.` : ''
    onInfo(`Shortcut profile applied: ${result.changed} custom binding${result.changed === 1 ? '' : 's'}.${ignored}`)
    setText('')
    setOpen(false)
  }

  return (
    <div className="env-keymap-profile" data-cut-keymap-profile>
      <div className="env-keymap-profile__preset">
        <label>
          <span>Shortcut style</span>
          <select
            data-cut-action="keymap-profile-preset"
            value={presetId}
            onChange={(event) => setPresetId(event.currentTarget.value as KeymapPresetId)}
          >
            <option value="" disabled>Choose a style…</option>
            {KEYMAP_PRESETS.map((option) => (
              <option key={option.id} value={option.id}>{option.label}</option>
            ))}
          </select>
        </label>
        <button
          type="button"
          className="env-btn env-btn--ghost"
          data-cut-action="keymap-profile-preset-apply"
          disabled={!preset}
          onClick={applyPreset}
        >
          Apply preset
        </button>
      </div>
      <p className="env-keymap-profile__description" data-cut-keymap-profile-description>
        {preset
          ? `${preset.description} Only commands available in Cut are mapped.`
          : 'Start from a familiar shortcut style, then customise individual commands or copy the profile.'}
      </p>
      <div className="env-keymap-profile__actions">
        <button type="button" className="env-btn env-btn--ghost" data-cut-action="keymap-profile-copy" onClick={() => void copy()}>
          Copy profile
        </button>
        <button
          type="button"
          className="env-btn env-btn--ghost"
          data-cut-action="keymap-profile-import-toggle"
          aria-expanded={open}
          onClick={() => {
            onBegin()
            setOpen((value) => !value)
          }}
        >
          {open ? 'Close import' : 'Import profile…'}
        </button>
      </div>
      {open ? (
        <div className="env-keymap-profile__import">
          <label htmlFor="cut-keymap-profile-json">Paste shortcut profile JSON</label>
          <textarea
            id="cut-keymap-profile-json"
            data-cut-action="keymap-profile-json"
            data-cut-keymap-profile-json
            value={text}
            spellCheck={false}
            placeholder={`{ "schema": "shellx-cut/keymap@1", "bindings": { … } }`}
            onChange={(event) => setText(event.currentTarget.value)}
          />
          <div>
            <span>Only shortcut IDs and keys are included—never project or media data.</span>
            <button
              type="button"
              className="env-btn"
              data-cut-action="keymap-profile-apply"
              disabled={!text.trim()}
              onClick={apply}
            >Apply profile</button>
          </div>
        </div>
      ) : null}
    </div>
  )
}
