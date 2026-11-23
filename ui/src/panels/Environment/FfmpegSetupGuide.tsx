// FfmpegSetupGuide.tsx — step-by-step video-processing setup where Cut has no
// built-in download (macOS today). The steps, commands and the requirement for
// a self-chosen build come from the doctor's ffmpeg card (`details.setup`), the
// same guide the engine's `tool_missing` refusal points agents to.
// "Check again" re-scans without restarting the app; "Choose ffmpeg…" saves a
// build the person already has (system.set_ffmpeg validates it and its ffprobe).

import { Availability, availabilitySentence, useAvailability } from '../../lib/availability'
import { useState } from 'react'
import { callVerb } from '../../lib/client'
import { writeClipboardText } from '../../lib/clipboard'
import type { FfmpegDownloadOffer, FfmpegSetupGuide as Guide } from '../../lib/doctor'
import { pickFfmpeg } from '../../lib/tauri'

/** What the Download button fetches, shown before the person chooses it: the
 *  publisher, version, size, licence and source (the engine's consent text),
 *  plus what this build cannot do. Choosing Download is the agreement. */
export function FfmpegDownloadConsent({ offer }: { offer: FfmpegDownloadOffer }) {
  return (
    <div className="env-row-detail env-ffsetup" data-cut-ffmpeg-consent>
      <p className="env-ffsetup-consent" data-cut-ffmpeg-consent-text>{offer.consent}</p>
      {offer.limitation && (
        <p className="env-ffsetup-custom" data-cut-ffmpeg-consent-limitation>{offer.limitation}</p>
      )}
    </div>
  )
}

export default function FfmpegSetupGuide({
  guide,
  onChanged,
  otherWays = false,
  copy = writeClipboardText,
  pick = pickFfmpeg,
}: {
  guide: Guide
  onChanged: () => void
  /** The steps follow a Download offer shown above them. */
  otherWays?: boolean
  copy?: (text: string) => Promise<void>
  pick?: () => Promise<string | null>
}) {
  const chooser = useAvailability('shell.desktop')
  const [copied, setCopied] = useState<number | null>(null)
  const [note, setNote] = useState<string | null>(null)
  const [busy, setBusy] = useState(false)

  const onCopy = async (index: number, command: string) => {
    try {
      await copy(command)
      setCopied(index)
      setNote(null)
    } catch {
      setCopied(null)
      setNote('Could not copy. Select the command and copy it by hand.')
    }
  }

  const onChoose = async () => {
    if (chooser.state === 'absent') { setNote(availabilitySentence('shell.desktop')); return }
    const path = await pick()
    if (!path) return
    setBusy(true)
    setNote(null)
    const r = await callVerb('system.set_ffmpeg', { path })
    setBusy(false)
    if (!r.ok) {
      setNote([r.error?.message, r.error?.suggested_action].filter(Boolean).join('. ') || 'Could not use that ffmpeg.')
      return
    }
    onChanged()
  }

  return (
    <div className="env-row-detail env-ffsetup" data-cut-ffmpeg-setup={guide.route}>
      {otherWays && <p className="env-ffsetup-heading" data-cut-ffmpeg-setup-other>Other ways</p>}
      <ol className="env-ffsetup-steps">
        {guide.steps.map((step, i) => (
          <li key={step} data-cut-ffmpeg-setup-step={i + 1}>{step}</li>
        ))}
      </ol>
      {guide.commands.map((c, i) => (
        <div className="env-ffsetup-command" key={c.command} data-cut-ffmpeg-setup-command={i + 1}>
          <span className="env-ffsetup-command-label">{c.label}</span>
          <code className="env-ffsetup-code" title={c.command}>{c.command}</code>
          <button
            type="button"
            className="env-btn env-btn--sm"
            data-cut-ffmpeg-setup-copy={i + 1}
            onClick={() => void onCopy(i, c.command)}
          >
            {copied === i ? 'Copied' : 'Copy'}
          </button>
        </div>
      ))}
      {guide.help_url && (
        <p className="env-ffsetup-help" data-cut-ffmpeg-setup-help>
          More about this step: <span className="env-ffsetup-url">{guide.help_url}</span>
        </p>
      )}
      <div className="env-ffsetup-actions">
        <button
          type="button"
          className="env-btn env-btn--primary env-btn--sm"
          data-cut-ffmpeg-setup-check
          onClick={onChanged}
          disabled={busy}
          title="Look for video processing again; no restart needed"
        >
          Check again
        </button>
        <Availability rule="shell.desktop">
        <button
          type="button"
          className="env-btn env-btn--sm"
          data-cut-action="settings-video-tool-choose"
          data-cut-ffmpeg-setup-choose
          onClick={() => void onChoose()}
          disabled={busy}
          title="Use an ffmpeg that is already on this computer"
        >
          Choose ffmpeg…
        </button>
        </Availability>
      </div>
      <p className="env-ffsetup-custom" data-cut-ffmpeg-setup-custom>{guide.custom_build}</p>
      {note && <div className="env-card-err" data-cut-ffmpeg-setup-note>{note}</div>}
    </div>
  )
}
