import { useEffect, useState } from 'react'
import InspectorSection from '../../components/inspector/InspectorSection'
import { callVerb } from '../../lib/client'
import { userVerbFailureMessage } from '../../lib/userActionFeedback'

export function OldRecordingZoomSection({
  clip,
  revision,
}: {
  clip: string
  revision?: string | null
}) {
  const [busy, setBusy] = useState(false)
  const [errorText, setErrorText] = useState<string | null>(null)

  useEffect(() => {
    setErrorText(null)
  }, [clip, revision])

  const makeEditable = () => {
    setBusy(true)
    setErrorText(null)
    callVerb('screen_record.make_editable', { clip })
      .then((reply) => {
        setBusy(false)
        if (!reply.ok) {
          setErrorText(
            userVerbFailureMessage(reply, 'Could not make this recording editable.')
              ?? 'Could not make this recording editable.',
          )
        }
      })
      .catch(() => {
        setBusy(false)
        setErrorText('Could not reach the editor engine. Try again.')
      })
  }

  return (
    <InspectorSection title="Zoom" sectionKey="recording-zoom">
      <div className="insp__group">
        <p className="insp__hint">
          Turn this earlier recording into editable zooms and camera. The original video stays.
        </p>
        <div className="insp__row">
          <button
            type="button"
            className="insp__btn"
            data-cut-action="recording-make-editable"
            disabled={busy}
            onClick={makeEditable}
          >
            {busy ? 'Making editable…' : 'Make editable'}
          </button>
        </div>
        {errorText && (
          <p
            role="alert"
            className="insp__hint insp__hint--error"
            data-cut-status="recording-make-editable-error"
          >
            {errorText}
          </p>
        )}
      </div>
    </InspectorSection>
  )
}
