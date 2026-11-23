import type { TrackKind } from '../../lib/client'
import { Availability } from '../../lib/availability'

export interface ClipActionAssetOption {
  id: string
  label: string
}

export interface ClipActionsSectionProps {
  trackKind: TrackKind
  sourceHasAudio?: boolean
  replaceOptions: ClipActionAssetOption[]
  replaceAssetId: string
  note: string | null
  onReplaceAssetChange: (assetId: string) => void
  onReplaceSource: () => void
  onDetachAudio: () => void
}

export default function ClipActionsSection({
  trackKind,
  sourceHasAudio,
  replaceOptions,
  replaceAssetId,
  note,
  onReplaceAssetChange,
  onReplaceSource,
  onDetachAudio,
}: ClipActionsSectionProps) {
  return (
    <div className="insp__group" data-cut-inspector-quick-actions>
      <div className="insp__group-title">Clip actions</div>
      <div className="insp__row">
        <select
          className="insp__select"
          data-cut-inspector-replace-asset
          value={replaceAssetId}
          disabled={replaceOptions.length === 0}
          title={replaceOptions.length === 0 ? 'Import another compatible clip to replace this source' : 'Replacement source'}
          onChange={(e) => onReplaceAssetChange(e.target.value)}
        >
          {replaceOptions.length === 0 ? (
            <option value="">No replacement asset</option>
          ) : (
            replaceOptions.map((asset) => <option key={asset.id} value={asset.id}>{asset.label}</option>)
          )}
        </select>
        <button
          type="button"
          className="insp__btn"
          data-cut-inspector-action="replace-source"
          disabled={!replaceAssetId}
          title={replaceAssetId ? 'Swap this clip source while keeping its slot and timing' : 'Import another compatible clip first'}
          onClick={() => void onReplaceSource()}
        >
          Replace source
        </button>
        {trackKind === 'video' && (
          <Availability rule="editing.detach-audio" subject={{ streams: { audio: sourceHasAudio } }}>
          <button
            type="button"
            className="insp__btn"
            data-cut-inspector-action="detach-audio"
            data-cut-action="inspector-detach-audio"
            title="Move this clip's audio onto its own editable audio track"
            onClick={() => void onDetachAudio()}
          >
            Detach audio
          </button>
          </Availability>
        )}
      </div>
      {note && <p className="insp__hint" data-cut-inspector-quick-actions-note>{note}</p>}
    </div>
  )
}
