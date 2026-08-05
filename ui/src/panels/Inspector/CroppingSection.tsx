import { type ClipCrop } from '../../lib/client'
import InspectorSection from '../../components/inspector/InspectorSection'
import PropertyRow from '../../components/inspector/PropertyRow'
import { useClipCrop, type SourceDims } from '../../components/inspector/useClipProp'

export interface CroppingSectionProps {
  clipId: string
  stored: ClipCrop | null | undefined
  dims: SourceDims | null
}

export default function CroppingSection({ clipId, stored, dims }: CroppingSectionProps) {
  const { crop, commitField, resetAll, busy } = useClipCrop(clipId, stored, dims)
  const isIdentity = !dims || (crop.x === 0 && crop.y === 0 && crop.w === dims.w && crop.h === dims.h)

  return (
    <InspectorSection
      title="Cropping"
      sectionKey="cropping"
      defaultCollapsed
      summary={!dims ? 'Waiting for source dimensions' : isIdentity ? 'No crop' : `${crop.w}×${crop.h} crop`}
      summaryTone={!dims ? 'warning' : isIdentity ? 'neutral' : 'active'}
      bypassed={isIdentity}
      onToggleBypass={() => { if (!isIdentity) void resetAll() }}
      onReset={() => void resetAll()}
    >
      {!dims ? (
        <p className="insp__hint" data-cut-crop-pending>
          Waiting for the asset probe - cropping needs the source dimensions.
        </p>
      ) : (
        <>
          <PropertyRow
            label="Crop X" propKey="crop-x" unit="px"
            value={crop.x} min={0} max={dims.w} step={1} default={0}
            disabled={busy}
            onCommit={(v) => void commitField('x', v)}
          />
          <PropertyRow
            label="Crop Y" propKey="crop-y" unit="px"
            value={crop.y} min={0} max={dims.h} step={1} default={0}
            disabled={busy}
            onCommit={(v) => void commitField('y', v)}
          />
          <PropertyRow
            label="Crop W" propKey="crop-w" unit="px"
            value={crop.w} min={1} max={dims.w} step={1} default={dims.w}
            disabled={busy}
            onCommit={(v) => void commitField('w', v)}
          />
          <PropertyRow
            label="Crop H" propKey="crop-h" unit="px"
            value={crop.h} min={1} max={dims.h} step={1} default={dims.h}
            disabled={busy}
            onCommit={(v) => void commitField('h', v)}
          />
        </>
      )}
    </InspectorSection>
  )
}
