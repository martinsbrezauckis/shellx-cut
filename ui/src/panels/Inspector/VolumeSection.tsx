import { useCallback, useRef, useState } from 'react'
import { runUserVerb } from '../../lib/userActionFeedback'
import { mediaClipTimelineDurationMs } from '../../lib/client'
import InspectorSection from '../../components/inspector/InspectorSection'
import PropertyRow from '../../components/inspector/PropertyRow'
import VolumeAutomationEditor from './VolumeAutomationEditor'
import type { InspectorMediaClip } from './model'
import {
  createVolumeAutomationStaticGainGuard,
  type VolumeAutomationTransientState,
} from './volumeAutomationModel'

export interface VolumeSectionProps {
  clip: InspectorMediaClip
  isAudioClip: boolean
  /** Ephemeral project.state revision used by controlled automation mutations. */
  projectRevision?: string | null
}

export default function VolumeSection({ clip, isAudioClip, projectRevision }: VolumeSectionProps) {
  const clipId = clip.id
  const gainDb = typeof clip.gain_db === 'number' ? clip.gain_db : 0
  const durationMs = mediaClipTimelineDurationMs(clip)
  const keyframes = clip.keyframes
  const automationPoints = (keyframes ?? []).find((track) => track.param === 'volume')?.points.length ?? 0
  const staticGainGuard = useRef(createVolumeAutomationStaticGainGuard())
  staticGainGuard.current.observeAuthoritative(clipId, automationPoints, projectRevision)
  const currentClipId = useRef(clipId)
  currentClipId.current = clipId
  const [, setEditorAutomation] = useState<VolumeAutomationTransientState | null>(null)
  const reportAutomationState = useCallback((state: VolumeAutomationTransientState) => {
    if (state.clipId !== currentClipId.current) return
    staticGainGuard.current.report(state)
    setEditorAutomation(state)
  }, [])
  const gainState = staticGainGuard.current.state()
  const effectiveAutomationPoints = gainState.effectivePointCount
  const gainUnavailableReason = gainState.refreshRequired
    ? 'Refresh project state before editing static Gain.'
    : !gainState.projectRevision
      ? 'Volume controls are waiting for the current project revision.'
    : gainState.inFlight
      ? 'Volume automation is updating. Wait before editing static Gain.'
      : 'Clear automation to edit static Gain'
  const gainSummary = gainDb === 0 ? '0 dB' : `${gainDb > 0 ? '+' : ''}${gainDb} dB`
  const isIdentity = gainDb === 0 && effectiveAutomationPoints === 0
  const resetVolume = () => {
    staticGainGuard.current.runIfAllowed(clipId, (controls) => {
      if (gainDb === 0) return
      void runUserVerb('edit.gain', { clip: clipId, db: 0, rationale: 'inspector: reset gain', ...controls }, 'Could not reset the clip level.')
    })
  }
  return (
    <InspectorSection
      title="Volume"
      sectionKey="volume"
      summary={effectiveAutomationPoints ? `${effectiveAutomationPoints} automation point${effectiveAutomationPoints === 1 ? '' : 's'} · ${gainSummary}` : gainSummary}
      summaryTone={isIdentity ? 'neutral' : 'active'}
      bypassed={isIdentity}
      onToggleBypass={gainState.blocked ? undefined : () => { if (!isIdentity) resetVolume() }}
      onReset={gainState.blocked ? undefined : resetVolume}
    >
      <PropertyRow
        label="Gain" propKey="gain" unit="dB"
        value={gainDb} min={-60} max={12} step={0.5} default={0}
        disabled={gainState.blocked}
        onCommit={(v) => staticGainGuard.current.runIfAllowed(clipId, (controls) => {
          void runUserVerb('edit.gain', { clip: clipId, db: v, rationale: `inspector: gain ${v} dB`, ...controls }, 'Could not change the clip level.')
        })}
      />
      {gainState.blocked && <p className="insp__hint" data-cut-volume-gain-unavailable>{gainUnavailableReason}</p>}
      {isAudioClip ? (
        <VolumeAutomationEditor
          key={clipId}
          clipId={clipId}
          durationMs={durationMs}
          keyframes={keyframes}
          projectRevision={projectRevision}
          hasSpeedRamp={clip.speed_ramp != null}
          onAutomationStateChange={reportAutomationState}
        />
      ) : (
        <p className="insp__hint insp__hint--error" data-cut-volume-automation-unavailable>Volume automation is available only for audio clips.</p>
      )}
    </InspectorSection>
  )
}
