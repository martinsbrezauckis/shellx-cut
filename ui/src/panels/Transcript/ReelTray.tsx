import type { ReelSpan } from './model'

interface ReelTrayProps {
  reel: ReelSpan[]
  reelAsset: string | null
  reelBusy: boolean
  reelNote: string
  onClear: () => void
  onAssemble: () => void
  onRemove: (index: number) => void
}

export default function ReelTray({
  reel,
  reelAsset,
  reelBusy,
  reelNote,
  onClear,
  onAssemble,
  onRemove,
}: ReelTrayProps) {
  return (
    <div className="tx__reel" data-cut-reel="">
      <div className="tx__reel-head">
        <span className="tx__reel-title">
          Reel{reelAsset ? <span className="tx__reel-asset"> · {reelAsset}</span> : ''}
        </span>
        <span className="tx__reel-meta">
          {reel.length} span{reel.length === 1 ? '' : 's'}
        </span>
        <span className="tx__reel-actions">
          {reel.length > 0 && (
            <button className="tx__pass-btn" data-cut-action="reel-clear" onClick={onClear} title="Empty the reel tray">
              Clear
            </button>
          )}
          <button
            className="tx__reel-assemble"
            data-cut-action="assemble-reel"
            disabled={reel.length === 0 || reelBusy}
            onClick={onAssemble}
            title="Build the highlight reel from these moments in order"
          >
            {reelBusy ? 'assembling…' : 'Assemble reel'}
          </button>
        </span>
      </div>
      {reel.length === 0 ? (
        <p className="tx__reel-empty" data-cut-reel-empty="">
          Select words, then “Add to reel”. Spans accumulate here in order — that IS the reel order.
        </p>
      ) : (
        <ol className="tx__reel-list">
          {reel.map((s, i) => (
            <li className="tx__reel-row" key={`${s.asset}:${s.range[0]}-${s.range[1]}:${i}`} data-cut-reel-span={`${s.range[0]}-${s.range[1]}`}>
              <span className="tx__reel-idx">{i + 1}</span>
              <span className="tx__reel-snippet" title={s.snippet}>{s.snippet || '—'}</span>
              <span className="tx__reel-range">[{s.range[0]}–{s.range[1]}]</span>
              <button
                className="tx__reel-rm"
                data-cut-action="reel-remove"
                onClick={() => onRemove(i)}
                title="Remove this span from the reel"
                aria-label={`Remove span ${i + 1}`}
              >
                ×
              </button>
            </li>
          ))}
        </ol>
      )}
      {reelNote && <p className="tx__reel-note" data-cut-reel-note="">{reelNote}</p>}
    </div>
  )
}
