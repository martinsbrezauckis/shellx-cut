import { useEffect, useMemo, useState } from 'react'
import { createPortal } from 'react-dom'
import { sourceUrl, type GeneratedAssetRecord, type Project } from '../../lib/client'
import { Icon } from '../../icons'
import { useBlockingOverlay } from '../../components/overlay/useBlockingOverlay'

const MAX_REFERENCES = 4

function basename(path: string, fallback: string): string {
  const value = path.replace(/\\/g, '/').split('/').filter(Boolean).pop()
  return value || fallback
}

function visualAssets(project: Project | null) {
  return Object.entries(project?.assets ?? {}).flatMap(([id, asset]) => {
    const kind = (asset.probe as { kind?: unknown } | undefined)?.kind
    if (kind !== 'image' && kind !== 'video') return []
    return [{ id, kind, label: basename(asset.path, id) }]
  })
}

export function GenerationReferences({
  project,
  selected,
  disabled,
  onToggle,
}: {
  project: Project | null
  selected: string[]
  disabled: boolean
  onToggle: (assetId: string) => void
}) {
  const assets = visualAssets(project)
  return (
    <details className="cd-advanced gen-refs" data-cut-generate-references open={selected.length > 0 || undefined}>
      <summary data-cut-generate-references-toggle>
        Visual references
        <span data-cut-generate-reference-count>{selected.length}/{MAX_REFERENCES}</span>
      </summary>
      {assets.length === 0 ? (
        <p className="cd-note">Import or generate an image or video to use as a reference.</p>
      ) : (
        <div className="gen-refs__list">
          {assets.map((asset) => {
            const checked = selected.includes(asset.id)
            const blocked = disabled || (!checked && selected.length >= MAX_REFERENCES)
            return (
              <label className="gen-refs__row" key={asset.id} data-cut-generate-reference={asset.id}>
                <input
                  type="checkbox"
                  data-cut-generate-reference-toggle={asset.id}
                  checked={checked}
                  disabled={blocked}
                  onChange={() => onToggle(asset.id)}
                />
                <Icon name={asset.kind === 'video' ? 'videoClip' : 'image'} size={14} tone="asset" />
                <span title={asset.label}>{asset.label}</span>
                <small>{asset.kind}</small>
              </label>
            )
          })}
        </div>
      )}
      <p className="cd-note">Only registered project media is copied into the isolated provider workspace.</p>
    </details>
  )
}

function generationTime(value: number | null): string | null {
  if (!value) return null
  const date = new Date(value)
  return Number.isNaN(date.getTime()) ? null : date.toLocaleString()
}

export default function GenerationHistory({
  items,
  loading,
  selectedReferences,
  chosenAssetId,
  canInsert,
  canReplace,
  onToggleReference,
  onPrepareVariation,
  onChoose,
  onInsert,
  onReplace,
}: {
  items: GeneratedAssetRecord[]
  loading: boolean
  selectedReferences: string[]
  chosenAssetId: string | null
  canInsert: boolean
  canReplace: boolean
  onToggleReference: (assetId: string) => void
  onPrepareVariation: (record: GeneratedAssetRecord) => void
  onChoose: (record: GeneratedAssetRecord) => void
  onInsert: (record: GeneratedAssetRecord) => void
  onReplace: (record: GeneratedAssetRecord) => void
}) {
  const [compareIds, setCompareIds] = useState<string[]>([])
  const [compareOpen, setCompareOpen] = useState(false)
  const compareItems = useMemo(
    () => compareIds.flatMap((id) => items.find((item) => item.asset_id === id) ?? []),
    [compareIds, items],
  )
  const compareFamily = compareItems[0]?.family_id ?? null
  const closeCompare = () => setCompareOpen(false)
  const compareOverlay = useBlockingOverlay<HTMLDivElement>(closeCompare, compareOpen && compareItems.length === 2)

  useEffect(() => {
    const available = new Set(items.filter((item) => item.integrity === 'verified').map((item) => item.asset_id))
    setCompareIds((current) => current.filter((id) => available.has(id)))
  }, [items])

  const toggleCompare = (item: GeneratedAssetRecord) => {
    setCompareIds((current) => {
      if (current.includes(item.asset_id)) return current.filter((id) => id !== item.asset_id)
      if (current.length >= 2 || (compareFamily && compareFamily !== item.family_id)) return current
      return [...current, item.asset_id]
    })
  }

  const compareDialog = compareOpen && compareItems.length === 2
    ? createPortal(
      <div ref={compareOverlay.dialogRef} className="gen-compare" data-cut-generated-compare-dialog
        data-cut-blocking-overlay role="dialog" aria-modal="true" aria-label="Compare generated takes"
        tabIndex={-1} onKeyDown={compareOverlay.onDialogKeyDown}>
        <button className="gen-compare__backdrop" data-cut-action="generated-compare-backdrop" data-cut-generated-compare-backdrop aria-label="Close comparison"
          onMouseDown={compareOverlay.onScrimMouseDown} />
        <section className="gen-compare__surface">
          <header className="gen-compare__head">
            <div>
              <h2>Compare takes</h2>
              <p>Same request family · inspect both before choosing</p>
            </div>
            <button className="cd-btn cd-btn--ghost" data-cut-generated-compare-close title="Close comparison" onClick={() => setCompareOpen(false)}>
              <Icon name="close" size={16} label="Close comparison" />
            </button>
          </header>
          <div className="gen-compare__grid">
            {compareItems.map((item) => (
              <article className="gen-compare__take" key={item.asset_id} data-cut-generated-compare-take={item.asset_id}>
                <div className="gen-compare__media">
                  {item.kind === 'video' ? (
                    <video src={sourceUrl(item.asset_id)} controls muted preload="metadata" aria-label="Generated video take" />
                  ) : (
                    <img src={sourceUrl(item.asset_id)} alt="" />
                  )}
                </div>
                <div className="gen-compare__details">
                  <p title={item.prompt}>{item.prompt || 'Generated media'}</p>
                  <small>{item.variation ?? 'Base take'} · {item.model ?? item.provider ?? 'provider default'}</small>
                  <button
                    className="cd-btn cd-btn--primary cd-btn--sm"
                    data-cut-generated-choose={item.asset_id}
                    onClick={() => { onChoose(item); setCompareOpen(false) }}
                  >
                    <Icon name="check" size={14} /> Choose this take
                  </button>
                </div>
              </article>
            ))}
          </div>
        </section>
      </div>,
      document.body,
    )
    : null

  return (
    <section className="gen-history" data-cut-generate-history>
      <header className="gen-history__head">
        <span>Generated media</span>
        <div>
          <button
            className="cd-btn cd-btn--ghost cd-btn--sm"
            data-cut-generated-compare
            disabled={compareIds.length !== 2}
            title={compareIds.length === 2 ? 'Compare the two selected takes' : 'Select two verified takes from the same family'}
            onClick={() => setCompareOpen(true)}
          >
            <Icon name="diff" size={14} /> Compare
          </button>
          <small data-cut-generate-history-count>{loading ? 'Loading...' : items.length}</small>
        </div>
      </header>
      {!loading && items.length === 0 ? (
        <p className="cd-note" data-cut-generate-history-empty>No generated media in this project yet.</p>
      ) : (
        <div className="gen-history__list">
          {items.map((item) => {
            const isReference = selectedReferences.includes(item.asset_id)
            const canReference = item.integrity === 'verified'
              && (isReference || selectedReferences.length < MAX_REFERENCES)
            const isCompared = compareIds.includes(item.asset_id)
            const canCompare = item.integrity === 'verified'
              && (isCompared || (compareIds.length < 2 && (!compareFamily || compareFamily === item.family_id)))
            const isChosen = chosenAssetId === item.asset_id
            const when = generationTime(item.created_at_ms)
            return (
              <article
                className={`gen-history__item ${isChosen ? 'gen-history__item--chosen' : ''}`}
                key={item.asset_id}
                data-cut-generated-asset={item.asset_id}
                data-cut-generated-integrity={item.integrity}
                data-cut-generated-chosen={isChosen || undefined}
              >
                <div className="gen-history__media">
                  {item.integrity === 'verified' && item.kind === 'image' ? (
                    <img src={sourceUrl(item.asset_id)} alt="" loading="lazy" />
                  ) : item.integrity === 'verified' && item.kind === 'video' ? (
                    <video src={sourceUrl(item.asset_id)} muted preload="metadata" aria-label="Generated video preview" />
                  ) : (
                    <Icon name="warning" size={18} tone="warn" />
                  )}
                </div>
                <div className="gen-history__main">
                  <div className="gen-history__title">
                    <label title={canCompare ? 'Select this take for side-by-side comparison' : 'Compare only two verified takes from the same family'}>
                      <input
                        type="checkbox"
                        checked={isCompared}
                        disabled={!canCompare}
                        data-cut-generated-compare-select={item.asset_id}
                        onChange={() => toggleCompare(item)}
                      />
                    </label>
                    <p title={item.prompt}>{item.prompt || 'Generated media'}</p>
                    {isChosen && <span><Icon name="check" size={14} /> chosen</span>}
                  </div>
                  <div className="gen-history__facts">
                    <span>{item.provider ?? 'unknown'} · {item.kind ?? 'media'}</span>
                    {item.model && <span title={item.model}>{item.model}</span>}
                    {item.variation && <span>{item.variation}</span>}
                    <span className={`gen-history__integrity gen-history__integrity--${item.integrity}`}>
                      {item.integrity === 'verified' ? <Icon name="verified" size={14} tone="success" /> : <Icon name="warning" size={14} tone="warn" />}
                      {item.integrity.replaceAll('_', ' ')}
                    </span>
                  </div>
                  <small>{when ? `${when} · ` : ''}{item.cost_note}</small>
                  <div className="gen-history__actions">
                    <button
                      className={`cd-btn cd-btn--ghost cd-btn--sm ${isChosen ? 'gen-history__action--on' : ''}`}
                      data-cut-generated-select={item.asset_id}
                      disabled={item.integrity !== 'verified'}
                      title="Choose this take for the next timeline action"
                      onClick={() => onChoose(item)}
                    >
                      <Icon name="check" size={14} /> {isChosen ? 'Chosen' : 'Choose'}
                    </button>
                    <button
                      className="cd-btn cd-btn--ghost cd-btn--sm"
                      data-cut-generated-insert={item.asset_id}
                      disabled={item.integrity !== 'verified' || !canInsert}
                      title={canInsert ? 'Insert this take at the playhead' : 'An unlocked video track is required'}
                      onClick={() => onInsert(item)}
                    >
                      <Icon name="plus" size={14} /> Insert
                    </button>
                    <button
                      className="cd-btn cd-btn--ghost cd-btn--sm"
                      data-cut-generated-replace={item.asset_id}
                      disabled={item.integrity !== 'verified' || !canReplace}
                      title={canReplace ? 'Replace the selected video clip in place' : 'Select an unlocked video media clip first'}
                      onClick={() => onReplace(item)}
                    >
                      <Icon name="reset" size={14} /> Replace
                    </button>
                    <button
                      className={`cd-btn cd-btn--ghost cd-btn--sm ${isReference ? 'gen-history__action--on' : ''}`}
                      data-cut-generated-use-reference={item.asset_id}
                      disabled={!canReference}
                      title={canReference ? 'Use this media as a visual reference' : 'Only verified media can be referenced'}
                      onClick={() => onToggleReference(item.asset_id)}
                    >
                      <Icon name={isReference ? 'check' : 'link'} size={14} /> {isReference ? 'Referenced' : 'Reference'}
                    </button>
                    <button
                      className="cd-btn cd-btn--ghost cd-btn--sm"
                      data-cut-generated-variation={item.asset_id}
                      disabled={item.integrity !== 'verified' || !item.provider || !item.kind}
                      title="Load this request as a new take; generation still requires explicit confirmation"
                      onClick={() => onPrepareVariation(item)}
                    >
                      <Icon name="reset" size={14} /> New variation
                    </button>
                  </div>
                </div>
              </article>
            )
          })}
        </div>
      )}
      {compareDialog}
    </section>
  )
}
