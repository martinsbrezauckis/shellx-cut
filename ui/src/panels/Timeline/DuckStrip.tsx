import { memo } from 'react'
import type { GainWindow } from '../../lib/client'
import { msToPx } from './layout'

/** Duck envelope strip (edit.duck gain_windows): a dimmed scrim over the music
 * lane. The recorded windows are display-only and stay transparent to gestures. */
const DuckStrip = memo(function DuckStrip({ w, zoom }: { w: GainWindow; zoom: number }) {
  const plateauW = Math.max(1, msToPx(w.range_ms[1] - w.range_ms[0], zoom))
  const wing = msToPx(w.attack_ms, zoom)
  const left = msToPx(w.range_ms[0], zoom) - wing
  const total = plateauW + 2 * wing
  const dim = 'rgba(5,5,5,.55)'
  return (
    <div
      className="tl-duck"
      data-cut-duck={`${w.range_ms[0]}-${w.range_ms[1]}`}
      style={{
        left,
        width: total,
        background: `linear-gradient(90deg, rgba(5,5,5,0) 0px, ${dim} ${wing}px, ${dim} ${wing + plateauW}px, rgba(5,5,5,0) ${total}px)`,
      }}
    >
      {plateauW >= 40 && <span className="tl-duck-label">{w.db.toFixed(0)} dB</span>}
    </div>
  )
})

export default DuckStrip
