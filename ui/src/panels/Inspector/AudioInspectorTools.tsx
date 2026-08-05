import { useState } from 'react'
import InspectorSection from '../../components/inspector/InspectorSection'
import { runUserVerb } from '../../lib/userActionFeedback'
import EffectChainControls from './EffectChainControls'
import {
  AUDIO_EFFECTS,
  CLEANUP_STRENGTHS,
  EQ_PRESETS,
  cleanupStrengthFromInput,
  type CleanupStrength,
  type InspectorMediaSelection,
} from './model'
import { audioCleanupSummary, duckingSummary } from './inspectorTaskModel'

interface AudioInspectorToolsProps {
  selection: InspectorMediaSelection
  speechTrackId: string | null
  onOpenDrawer: (name: string) => void
}

export default function AudioInspectorTools({
  selection,
  speechTrackId,
  onOpenDrawer,
}: AudioInspectorToolsProps) {
  const clip = selection.clip
  const [cleanupStrength, setCleanupStrength] = useState<CleanupStrength>('medium')
  const [cleanupNote, setCleanupNote] = useState<string | null>(null)
  const [cleanupBusy, setCleanupBusy] = useState(false)
  const [effectBusy, setEffectBusy] = useState(false)
  const effectsSummary = audioCleanupSummary(clip)
  const duckSummary = duckingSummary(speechTrackId)

  const cleanupVoice = async () => {
    if (cleanupBusy || effectBusy) return
    setCleanupBusy(true)
    setCleanupNote(null)
    const result = await runUserVerb('audio.cleanup_voice', {
      clip: clip.id,
      strength: cleanupStrength,
      rationale: `inspector: clean voice (${cleanupStrength})`,
    }, 'Could not clean voice.')
    setCleanupBusy(false)
    setCleanupNote(
      result?.ok
        ? `Voice cleaned (${cleanupStrength})`
        : (result?.error?.message ?? result?.error?.code ?? 'Could not clean voice'),
    )
    setTimeout(() => setCleanupNote(null), 4000)
  }

  return (
    <>
      <InspectorSection
        title="Voice cleanup"
        sectionKey="audio-cleanup"
        summary="One-click voice repair"
      >
        <div className="insp__group" data-cut-inspector-group="audio-cleanup">
          <p className="insp__hint">
            Remove steady noise, control room tone, compress peaks, and tune speech EQ in one undoable pass.
          </p>
          <div className="insp__row" data-cut-inspector-cleanup>
            <select
              className="insp__select"
              data-cut-inspector-cleanup-strength
              value={cleanupStrength}
              disabled={cleanupBusy || effectBusy}
              title="How aggressively to denoise, gate, and compress"
              onChange={(event) => setCleanupStrength(
                cleanupStrengthFromInput(event.currentTarget.value, cleanupStrength),
              )}
            >
              {CLEANUP_STRENGTHS.map(({ strength, label }) => (
                <option key={strength} value={strength}>{label}</option>
              ))}
            </select>
            <button
              type="button"
              className="insp__btn insp__btn--accent"
              data-cut-action="audio-cleanup-voice"
              disabled={cleanupBusy || effectBusy}
              onClick={() => void cleanupVoice()}
            >
              {cleanupBusy ? 'Cleaning…' : 'Clean voice'}
            </button>
          </div>
          {cleanupNote && (
            <p className="insp__hint" role="status" data-cut-inspector-cleanup-note>{cleanupNote}</p>
          )}
        </div>
      </InspectorSection>

      <InspectorSection
        title="Audio effects & EQ"
        sectionKey="audio-effects"
        defaultCollapsed
        summary={effectsSummary.label}
        summaryTone={effectsSummary.tone}
      >
        <div className="insp__group" data-cut-inspector-group="audio-effects">
          <EffectChainControls
            clipId={clip.id}
            effects={clip.effects}
            kind="audio"
            options={AUDIO_EFFECTS}
            externallyBusy={cleanupBusy}
            onBusyChange={setEffectBusy}
          />
          <div className="insp__group-title insp__group-title--sub">EQ preset</div>
          <div className="insp__effects" data-cut-inspector-eq>
            {EQ_PRESETS.map(({ preset, label }) => (
              <button
                key={preset}
                type="button"
                className="insp__chip"
                data-cut-inspector-eq-preset={preset}
                onClick={() => void runUserVerb('edit.eq', {
                  clip: clip.id,
                  preset,
                  enabled: true,
                  rationale: `inspector: eq ${preset}`,
                }, `Could not apply the ${label} EQ preset.`)}
              >
                {label}
              </button>
            ))}
            <button
              type="button"
              className="insp__chip"
              data-cut-inspector-eq-preset="clear"
              onClick={() => void runUserVerb('edit.eq', {
                clip: clip.id,
                enabled: false,
                rationale: 'inspector: clear eq',
              }, 'Could not clear the EQ preset.')}
            >
              Clear
            </button>
          </div>
        </div>
      </InspectorSection>

      <InspectorSection
        title="Mix & ducking"
        sectionKey="audio-mix"
        defaultCollapsed
        summary={duckSummary.label}
        summaryTone={duckSummary.tone}
      >
        <div className="insp__group" data-cut-inspector-group="audio-mix">
          <div className="insp__tools">
            <button
              type="button"
              className="insp__tool"
              data-cut-inspector-tool="mixer"
              onClick={() => onOpenDrawer('mixer')}
            >
              Open audio mixer
            </button>
            <button
              type="button"
              className="insp__tool"
              data-cut-inspector-tool="music"
              onClick={() => onOpenDrawer('music')}
            >
              Add or edit music
            </button>
          </div>
          {speechTrackId && speechTrackId !== selection.trackId ? (
            <>
              <div className="insp__group-title insp__group-title--sub">Duck under speech</div>
              <div className="insp__row">
                <button
                  type="button"
                  className="insp__btn"
                  data-cut-inspector-action="duck"
                  onClick={() => void runUserVerb('edit.duck', {
                    music_track: selection.trackId,
                    against_track: speechTrackId,
                    db: -18,
                    attack_ms: 150,
                    rationale: 'inspector: duck music -18dB under speech',
                  }, 'Could not apply automatic ducking.')}
                >
                  Duck −18 dB
                </button>
              </div>
            </>
          ) : (
            <div className="insp__setup-blocker" data-cut-inspector-blocked="duck">
              <span>Add a second audio track with speech before applying automatic ducking.</span>
              <button type="button" data-cut-inspector-open-music onClick={() => onOpenDrawer('music')}>Add music or audio</button>
            </div>
          )}
        </div>
      </InspectorSection>
    </>
  )
}
