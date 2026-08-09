import { useState } from 'react'
import {
  setSttModel,
  STT_DEFAULT_MODEL,
  STT_MODELS,
  STT_V3_LANGUAGES,
  type DoctorCard,
} from '../../lib/doctor'

export function SttModelControl({ card, onChanged }: { card: DoctorCard; onChanged: () => void }) {
  const [sttBusy, setSttBusy] = useState(false)
  const [sttNote, setSttNote] = useState<string | null>(null)
  if (card.id !== 'perception') return null

  const sttModel = card.details?.stt_model as string | undefined
  const sttIsDefault = card.details?.stt_model_default === true
  const sttIsRecommended = !!sttModel && STT_MODELS.some((m) => m.id === sttModel)
  const changeStt = async (model: string | null) => {
    setSttBusy(true)
    setSttNote(null)
    const ok = await setSttModel(model)
    setSttBusy(false)
    if (ok) {
      const label = model
        ? STT_MODELS.find((entry) => entry.id === model)?.label ?? model
        : STT_MODELS.find((entry) => entry.id === STT_DEFAULT_MODEL)?.label ?? STT_DEFAULT_MODEL
      setSttNote(
        `${model ? 'Caption model set to' : 'Caption model reset to'} ${label}. ` +
        'Applies to the next transcription — re-run it on a clip to re-transcribe.',
      )
      onChanged()
    } else {
      setSttNote('Could not change the transcription model.')
    }
  }

  return (
    <div
      className="env-row-detail env-ff"
      data-cut-env-stt-control
      data-cut-env-stt-busy={sttBusy ? 'true' : 'false'}
    >
      <div className="env-ff-row">
        <span className="env-ff-label">Caption model:</span>
        {sttIsRecommended ? (
          <select
            className="env-btn env-btn--sm"
            data-cut-env-stt-model
            value={sttModel}
            disabled={sttBusy}
            title="Speech-to-text model used for transcription and captions. Applies to the next transcription."
            onChange={(e) => void changeStt(e.target.value)}
          >
            {STT_MODELS.map((m) => (
              <option key={m.id} value={m.id}>{m.label}</option>
            ))}
          </select>
        ) : (
          <>
            {/* Active model is a custom/advanced id — show it read-only (honest:
                we don't pretend the two-item switch represents it) + offer reset. */}
            <span className="env-ff-label" data-cut-env-stt-custom>Custom model</span>
            <button
              className="env-btn env-btn--sm"
              data-cut-env-stt-reset
              disabled={sttBusy}
              onClick={() => void changeStt(null)}
              title="Return to the recommended caption model"
            >
              Use default
            </button>
          </>
        )}
        {sttIsRecommended && !sttIsDefault && (
          <button
            className="env-btn env-btn--sm"
            data-cut-env-stt-reset
            disabled={sttBusy}
            onClick={() => void changeStt(null)}
            title="Return to the recommended caption model"
          >
            Default
          </button>
        )}
      </div>
      <div className="env-ff-note" data-cut-env-stt-langs>
        {sttModel?.startsWith('nemo-canary')
          ? 'Best for smaller European languages. First run downloads the speech model and word aligner.'
          : sttModel?.startsWith('whisper')
            ? 'Fallback for broad language coverage. Larger download and slower first run.'
            : 'Recommended for most caption and transcript edits. Switch when the transcript language needs it.'}
      </div>
      <details className="env-advanced env-advanced--nested" data-cut-env-stt-advanced>
        <summary className="env-advanced-summary" data-cut-env-stt-advanced-toggle>Model details</summary>
        <div className="env-advanced-note">
          {!sttIsRecommended && sttModel && <>Current custom model: <code>{sttModel}</code>. </>}
          Fast captions uses {STT_DEFAULT_MODEL}. Smaller languages uses nemo-canary-1b-v2 with MMS_FA word
          alignment. Compatibility fallback uses whisperx-large-v3. Fast captions covers: {STT_V3_LANGUAGES}.
        </div>
      </details>
      {sttNote && <div className="env-ff-note" data-cut-env-stt-note>{sttNote}</div>}
    </div>
  )
}
