import { runUserVerb } from '../../lib/userActionFeedback'
import InspectorSection from '../../components/inspector/InspectorSection'
import PropertyRow from '../../components/inspector/PropertyRow'

export interface VolumeSectionProps {
  clipId: string
  gainDb: number
}

export default function VolumeSection({ clipId, gainDb }: VolumeSectionProps) {
  return (
    <InspectorSection
      title="Volume"
      sectionKey="volume"
      summary={gainDb === 0 ? '0 dB' : `${gainDb > 0 ? '+' : ''}${gainDb} dB`}
      summaryTone={gainDb === 0 ? 'neutral' : 'active'}
      bypassed={gainDb === 0}
      onToggleBypass={() => { if (gainDb !== 0) void runUserVerb('edit.gain', { clip: clipId, db: 0, rationale: 'inspector: clear gain' }, 'Could not clear the clip level.') }}
      onReset={() => void runUserVerb('edit.gain', { clip: clipId, db: 0, rationale: 'inspector: reset gain' }, 'Could not reset the clip level.')}
    >
      <PropertyRow
        label="Gain" propKey="gain" unit="dB"
        value={gainDb} min={-60} max={12} step={0.5} default={0}
        onCommit={(v) => void runUserVerb('edit.gain', { clip: clipId, db: v, rationale: `inspector: gain ${v} dB` }, 'Could not change the clip level.')}
      />
    </InspectorSection>
  )
}
