import { memo, useEffect, useLayoutEffect, useRef, useState } from 'react'
import { getWaveform, type Waveform } from '../../lib/client'

interface WaveformCanvasProps {
  /** Asset id to fetch peaks for (audio clips always carry one). */
  asset: string
  /** Clip's SOURCE range inside the asset (ms) - the window of peaks to draw. */
  srcInMs: number
  srcOutMs: number
  /** Clip body size in CSS px (the drawable area; width tracks zoom). */
  width: number
  height: number
  /** Selected clips tint blue (the selection accent), else muted green. */
  selected: boolean
  /** Bottom-strip variant: a slim waveform pinned to the BOTTOM of a VIDEO clip
   *  (over the filmstrip), vs the full-height centered trace on an audio clip.
   *  This is how talking-head audio remains visible when it is muxed into the
   *  video clip and has no separate audio clip to draw on. */
  bottom?: boolean
}

const WaveformCanvas = memo(function WaveformCanvas({
  asset,
  srcInMs,
  srcOutMs,
  width,
  height,
  selected,
  bottom = false,
}: WaveformCanvasProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const [wave, setWave] = useState<Waveform | null>(null)

  useEffect(() => {
    let live = true
    void getWaveform(asset).then((w) => {
      if (live) setWave(w)
    })
    return () => {
      live = false
    }
  }, [asset])

  useLayoutEffect(() => {
    const canvas = canvasRef.current
    if (!canvas) return
    const dpr = typeof devicePixelRatio === 'number' ? devicePixelRatio : 1
    const wCss = Math.max(1, Math.floor(width))
    const hCss = Math.max(1, Math.floor(height))
    canvas.width = Math.round(wCss * dpr)
    canvas.height = Math.round(hCss * dpr)
    const ctx = canvas.getContext('2d')
    if (!ctx) return
    ctx.clearRect(0, 0, canvas.width, canvas.height)
    if (!wave || wave.peaks.length === 0 || wave.source_ms <= 0) return

    const count = wave.peaks.length
    const span = wave.source_ms
    const inMs = Math.max(0, Math.min(srcInMs, span))
    const outMs = Math.max(inMs, Math.min(srcOutMs, span))
    const i0 = Math.floor((inMs / span) * count)
    const i1 = Math.max(i0 + 1, Math.ceil((outMs / span) * count))
    const lo = Math.max(0, Math.min(i0, count - 1))
    const hi = Math.max(lo + 1, Math.min(i1, count))
    const slice = hi - lo

    ctx.save()
    ctx.scale(dpr, dpr)
    const mid = hCss / 2
    const amp = Math.max(1, mid - 2)
    ctx.fillStyle = selected ? 'rgba(96,165,250,0.55)' : 'rgba(74,222,128,0.4)'

    ctx.beginPath()
    for (let x = 0; x < wCss; x++) {
      const b0 = lo + Math.floor((x / wCss) * slice)
      const b1 = lo + Math.floor(((x + 1) / wCss) * slice)
      let peak = 0
      for (let b = b0; b < Math.max(b0 + 1, b1) && b < hi; b++) {
        const v = wave.peaks[b]
        if (v > peak) peak = v
      }
      const h = peak * amp
      const y = mid - h
      if (x === 0) ctx.moveTo(x, y)
      else ctx.lineTo(x, y)
    }
    for (let x = wCss - 1; x >= 0; x--) {
      const b0 = lo + Math.floor((x / wCss) * slice)
      const b1 = lo + Math.floor(((x + 1) / wCss) * slice)
      let peak = 0
      for (let b = b0; b < Math.max(b0 + 1, b1) && b < hi; b++) {
        const v = wave.peaks[b]
        if (v > peak) peak = v
      }
      const h = peak * amp
      ctx.lineTo(x, mid + h)
    }
    ctx.closePath()
    ctx.fill()
    ctx.restore()
  }, [wave, width, height, srcInMs, srcOutMs, selected])

  return (
    <canvas
      ref={canvasRef}
      className={bottom ? 'tl-waveform tl-waveform--bottom' : 'tl-waveform'}
      data-cut-waveform={asset}
      data-wave={wave ? '1' : '0'}
      style={{ width: Math.max(1, Math.floor(width)), height: Math.max(1, Math.floor(height)) }}
      aria-hidden="true"
    />
  )
})

export default WaveformCanvas
