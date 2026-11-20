// components/OddProjectSizeNotice.tsx — tells the person, BEFORE an export,
// that an older project's odd frame size must become even, with one button
// that does it. Shown while the open project is odd (so on opening it), and
// shown again with the engine's own sentence when an export is refused with
// `odd_project_size` (any surface: Render, Export, Queue, Save range). After
// the change the notice says the export can be started again.

import { useEffect, useState } from 'react'
import {
  changeProjectToEven,
  changeSizeLabel,
  ODD_PROJECT_SIZE_EVENT,
  oddProjectSize,
  oddSizeMessage,
  type OddSize,
} from '../lib/oddProjectSize'
import { usePublishedUiNotices, type UiNotice } from '../lib/uiNotices'

export interface OddProjectSizeNoticeProps {
  settings: { width?: number; height?: number } | null | undefined
}

/** The `ui.state.notices` entry for what this notice shows right now. */
export function oddSizeNotice(state: 'open' | 'refused' | 'done', size: OddSize): UiNotice {
  return {
    kind: 'odd_project_size',
    state,
    width: size.width,
    height: size.height,
    even_width: size.evenWidth,
    even_height: size.evenHeight,
  }
}

export default function OddProjectSizeNotice({ settings }: OddProjectSizeNoticeProps) {
  const size = oddProjectSize(settings)
  const key = size ? `${size.width}x${size.height}` : null
  const [dismissed, setDismissed] = useState<string | null>(null)
  const [refusal, setRefusal] = useState<string | null>(null)
  const [done, setDone] = useState<string | null>(null)
  const [doneSize, setDoneSize] = useState<OddSize | null>(null)
  const [busy, setBusy] = useState(false)

  useEffect(() => {
    const onRefused = (event: Event) => {
      const message = event instanceof CustomEvent ? event.detail?.message : null
      setDismissed(null)
      setRefusal(typeof message === 'string' && message.trim() ? message.trim() : null)
    }
    document.addEventListener(ODD_PROJECT_SIZE_EVENT, onRefused)
    return () => document.removeEventListener(ODD_PROJECT_SIZE_EVENT, onRefused)
  }, [])

  // A confirmation outlives the odd size by a few seconds, then goes.
  useEffect(() => {
    if (!done) return
    const timer = window.setTimeout(() => setDone(null), 6_000)
    return () => window.clearTimeout(timer)
  }, [done])

  usePublishedUiNotices('odd-project-size',
    done && !size && doneSize ? [oddSizeNotice('done', doneSize)]
      : size && dismissed !== key ? [oddSizeNotice(refusal ? 'refused' : 'open', size)]
        : [])

  if (done && !size) {
    return (
      <div className="user-action-feedback odd-size-notice" data-cut-odd-size-notice="done" role="status">
        <span data-cut-odd-size-message>{done}</span>
      </div>
    )
  }
  if (!size || dismissed === key) return null
  const change = async () => {
    setBusy(true)
    try {
      const result = await changeProjectToEven(size)
      if (result.ok) {
        setRefusal(null)
        setDoneSize(size)
        // Undo does not step back over a format change (it is not a timeline
        // edit), so say that rather than let Undo surprise the person.
        setDone(`Project changed to ${size.evenWidth}x${size.evenHeight}. You can start the export again. Undo does not change the size back.`)
      } else {
        setRefusal(result.error?.message ?? 'The project size could not be changed.')
      }
    } catch {
      setRefusal('The project size could not be changed: the local engine is unreachable.')
    } finally {
      setBusy(false)
    }
  }
  const message = refusal ? `${refusal}.`.replace(/\.\.$/, '.') : oddSizeMessage(size)
  return (
    <div className="user-action-feedback odd-size-notice" data-cut-odd-size-notice={refusal ? 'refused' : 'open'} role="alert">
      <span data-cut-odd-size-message>{message}</span>
      <span className="user-action-feedback__actions">
        <button type="button" data-cut-action="odd-size-change" disabled={busy} onClick={() => void change()}>
          {busy ? 'Changing…' : changeSizeLabel(size)}
        </button>
        <button
          type="button"
          className="user-action-feedback__dismiss"
          data-cut-action="odd-size-dismiss"
          aria-label="Dismiss: the project size stays odd and video exports stay refused"
          onClick={() => { setDismissed(key); setRefusal(null) }}
        >
          ×
        </button>
      </span>
    </div>
  )
}
