import { type ClipTransform } from '../../lib/client'
import InspectorSection from '../../components/inspector/InspectorSection'
import PropertyRow from '../../components/inspector/PropertyRow'
import { useClipTransform } from '../../components/inspector/useClipProp'

export interface TransformSectionProps {
  clipId: string
  stored: ClipTransform | null | undefined
  isOverlay: boolean
}

export default function TransformSection({ clipId, stored, isOverlay }: TransformSectionProps) {
  const { transform, commitField, resetAll, busy } = useClipTransform(clipId, stored)
  const isIdentity = transform.x === 0 && transform.y === 0 && transform.scale === 1 && transform.opacity === 1

  return (
    <InspectorSection
      title="Transform & motion"
      sectionKey="transform"
      summary={isIdentity ? 'Default position and scale' : 'Transform adjusted'}
      summaryTone={isIdentity ? 'neutral' : 'active'}
      bypassed={isIdentity}
      onToggleBypass={() => { if (!isIdentity) void resetAll() }}
      onReset={() => void resetAll()}
    >
      <PropertyRow
        label="Position X" propKey="transform-x"
        value={transform.x} min={0} max={1} step={0.01} default={0}
        disabled={busy}
        onCommit={(v) => void commitField('x', v)}
      />
      <PropertyRow
        label="Position Y" propKey="transform-y"
        value={transform.y} min={0} max={1} step={0.01} default={0}
        disabled={busy}
        onCommit={(v) => void commitField('y', v)}
      />
      <PropertyRow
        label="Scale" propKey="transform-scale" unit="%"
        value={Math.round(transform.scale * 100)} min={1} max={100} step={1} default={100}
        disabled={busy}
        onCommit={(v) => void commitField('scale', v / 100)}
      />
      {isOverlay && (
        <PropertyRow
          label="Opacity" propKey="transform-opacity" unit="%"
          value={Math.round(transform.opacity * 100)} min={0} max={100} step={1} default={100}
          disabled={busy}
          onCommit={(v) => void commitField('opacity', v / 100)}
        />
      )}
    </InspectorSection>
  )
}
