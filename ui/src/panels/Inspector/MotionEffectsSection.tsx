import type { MotionClipLink, MotionEffectLayerSummary } from '../../lib/motionLinkModel'

export interface MotionEffectsSectionProps {
  link: MotionClipLink
}

const MAX_VISIBLE_LAYERS = 8

function countLabel(count: number, singular: string): string {
  return `${count} ${singular}${count === 1 ? '' : 's'}`
}

function layerName(layer: MotionEffectLayerSummary): string {
  return layer.name || layer.id || 'Unnamed layer'
}

function percent(value: number | null | undefined): string | null {
  return typeof value === 'number' && Number.isFinite(value)
    ? `${Math.round(value * 100)}%`
    : null
}

export default function MotionEffectsSection({ link }: MotionEffectsSectionProps) {
  const effects = link.effects
  if (!effects) return null
  if (!effects.available) {
    return (
      <section className="insp__motion-effects" data-cut-motion-effects data-cut-motion-effects-state="unavailable">
        <div className="insp__subhead">Key & roto</div>
        <p className="insp__motion-warning">The linked package exists, but its authored Motion document cannot be inspected safely. The last rendered fallback remains usable.</p>
      </section>
    )
  }
  const keyed = effects.keyedLayerCount ?? 0
  const roto = effects.rotoLayerCount ?? 0
  const tracked = effects.trackedRotoLayerCount ?? 0
  const layers = (effects.layers ?? []).slice(0, MAX_VISIBLE_LAYERS)
  const hidden = Math.max(0, (effects.layers?.length ?? 0) - layers.length)
  return (
    <section className="insp__motion-effects" data-cut-motion-effects data-cut-motion-effects-state={link.state === 'source-dirty' ? 'render-stale' : 'current'}>
      <div className="insp__subhead">Key & roto</div>
      {keyed === 0 && roto === 0 ? (
        <p className="insp__hint">No bounded chroma key or animated roto is authored on this package.</p>
      ) : (
        <>
          <div className="insp__motion-effect-counts" aria-label="Motion keying and rotoscope summary">
            <span>{countLabel(keyed, 'keyed layer')}</span>
            <span>{countLabel(roto, 'roto layer')}</span>
            {tracked > 0 && <span>{tracked} tracked</span>}
          </div>
          <ul className="insp__motion-effect-list">
            {layers.map((layer, index) => {
              const spill = percent(layer.keying?.spillSuppression)
              return (
                <li key={`${layer.id || 'layer'}-${index}`} data-cut-motion-effect-layer={layer.id || index}>
                  <strong title={layer.id || undefined}>{layerName(layer)}</strong>
                  <span className="insp__motion-effect-tags">
                    {layer.keying && <span>Chroma{layer.keying.keyColor ? ` · ${layer.keying.keyColor}` : ''}</span>}
                    {spill && <span>Spill {spill}</span>}
                    {layer.keying?.matteCleanup && <span>Matte cleanup</span>}
                    {layer.roto && <span>Roto · {layer.roto.frameCount} frame{layer.roto.frameCount === 1 ? '' : 's'}</span>}
                    {layer.roto?.tracked && <span>Tracked{layer.roto.model ? ` · ${layer.roto.model}` : ''}</span>}
                  </span>
                </li>
              )
            })}
          </ul>
          {(effects.truncated || hidden > 0) && <p className="insp__hint">Additional affected layers are summarized by the counts above. Open Motion for the full stack.</p>}
        </>
      )}
      <p className={link.state === 'source-dirty' ? 'insp__motion-warning' : 'insp__hint'}>
        {link.state === 'source-dirty'
          ? 'The source effects changed after this render. Refresh render to update Cut’s pixels.'
          : 'Motion owns key color, spill, matte cleanup, and roto geometry; Cut keeps a stable rendered fallback.'}
      </p>
    </section>
  )
}
