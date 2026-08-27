import {
  type KeyboardEvent,
  type PointerEvent,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from 'react'
import { getWaveform, type Waveform } from '../../lib/client'

const SEEK_STEP_MS = 1_000
const SEEK_LARGE_STEP_MS = 5_000

interface SourceWaveformProps {
  /** Registered source whose existing media.waveform peaks are projected. */
  asset: string
  /** Measured source duration, in the Source Monitor's canonical milliseconds. */
  durationMs: number
  currentMs: number
  inMs: number
  outMs: number
  /** Update the real Source Monitor transport; this component owns no media state. */
  onSeek: (atMs: number) => void
}

function clamp(value: number, max: number): number {
  return Math.max(0, Math.min(max, Math.round(value)))
}

function formatTime(ms: number): string {
  const total = Math.max(0, Math.round(ms))
  const minutes = Math.floor(total / 60_000)
  const seconds = Math.floor((total % 60_000) / 1_000)
  const millis = total % 1_000
  return `${minutes}:${String(seconds).padStart(2, '0')}.${String(millis).padStart(3, '0')}`
}

/**
 * A display-only projection of the server-owned audio peaks. Seeking changes
 * the actual Source Monitor media element through the parent callback; it does
 * not create a timeline lane, mixer state, or local edit mutation.
 */
export default function SourceWaveform({ asset, durationMs, currentMs, inMs, outMs, onSeek }: SourceWaveformProps) {
  const canvasRef = useRef<HTMLCanvasElement | null>(null)
  const [wave, setWave] = useState<Waveform | null | undefined>(undefined)
  const [width, setWidth] = useState(0)
  const sourceMs = Math.max(0, wave?.source_ms ?? durationMs)
  const isSeekable = sourceMs > 0
  const state = wave === undefined ? 'loading' : wave ? 'ready' : 'unavailable'

  useEffect(() => {
    let live = true
    setWave(undefined)
    // Reuse the timeline's default-resolution promise/cache for this asset;
    // opening Source Monitor must not decode the same audio a second time.
    void getWaveform(asset).then((next) => {
      if (live) setWave(next)
    })
    return () => {
      live = false
    }
  }, [asset])

  useLayoutEffect(() => {
    const canvas = canvasRef.current
    if (!canvas) return
    const measure = () => setWidth(Math.max(0, Math.round(canvas.getBoundingClientRect().width)))
    measure()
    const observer = new ResizeObserver(measure)
    observer.observe(canvas)
    return () => observer.disconnect()
  }, [])

  useLayoutEffect(() => {
    const canvas = canvasRef.current
    if (!canvas || width <= 0) return
    const dpr = typeof devicePixelRatio === 'number' ? devicePixelRatio : 1
    const height = 54
    canvas.width = Math.round(width * dpr)
    canvas.height = Math.round(height * dpr)
    const ctx = canvas.getContext('2d')
    if (!ctx) return
    ctx.clearRect(0, 0, canvas.width, canvas.height)
    if (!wave || wave.peaks.length === 0 || sourceMs <= 0) return

    ctx.save()
    ctx.scale(dpr, dpr)
    const position = (ms: number) => clamp((clamp(ms, sourceMs) / sourceMs) * width, width)
    const inX = position(inMs)
    const outX = position(Math.max(inMs, outMs))
    const mid = height / 2
    const amplitude = Math.max(1, mid - 4)

    ctx.fillStyle = 'rgba(94, 234, 212, 0.09)'
    ctx.fillRect(inX, 0, Math.max(1, outX - inX), height)
    ctx.strokeStyle = 'rgba(94, 234, 212, 0.62)'
    ctx.lineWidth = 1
    ctx.beginPath()
    for (let x = 0; x < width; x += 1) {
      const from = Math.floor((x / width) * wave.peaks.length)
      const to = Math.max(from + 1, Math.ceil(((x + 1) / width) * wave.peaks.length))
      let peak = 0
      for (let index = from; index < to && index < wave.peaks.length; index += 1) {
        peak = Math.max(peak, wave.peaks[index] ?? 0)
      }
      const size = peak * amplitude
      ctx.moveTo(x + 0.5, mid - size)
      ctx.lineTo(x + 0.5, mid + size)
    }
    ctx.stroke()

    ctx.lineWidth = 1
    ctx.strokeStyle = 'rgba(245, 158, 11, 0.95)'
    for (const mark of [inX, outX]) {
      ctx.beginPath()
      ctx.moveTo(mark + 0.5, 0)
      ctx.lineTo(mark + 0.5, height)
      ctx.stroke()
    }
    const currentX = position(currentMs)
    ctx.strokeStyle = 'rgba(255, 255, 255, 0.95)'
    ctx.lineWidth = 2
    ctx.beginPath()
    ctx.moveTo(currentX + 0.5, 0)
    ctx.lineTo(currentX + 0.5, height)
    ctx.stroke()
    ctx.restore()
  }, [currentMs, inMs, outMs, sourceMs, wave, width])

  const seekAtPointer = (event: PointerEvent<HTMLDivElement>) => {
    if (!isSeekable) return
    const bounds = event.currentTarget.getBoundingClientRect()
    if (bounds.width <= 0) return
    onSeek(((event.clientX - bounds.left) / bounds.width) * sourceMs)
  }

  const seekWithKeyboard = (event: KeyboardEvent<HTMLDivElement>) => {
    if (!isSeekable) return
    let next: number | null = null
    if (event.key === 'Home') next = 0
    if (event.key === 'End') next = sourceMs
    if (event.key === 'ArrowLeft') next = currentMs - (event.shiftKey ? SEEK_LARGE_STEP_MS : SEEK_STEP_MS)
    if (event.key === 'ArrowRight') next = currentMs + (event.shiftKey ? SEEK_LARGE_STEP_MS : SEEK_STEP_MS)
    if (next === null) return
    event.preventDefault()
    onSeek(clamp(next, sourceMs))
  }

  const status = state === 'loading'
    ? 'Loading audio waveform…'
    : state === 'unavailable'
      ? 'Audio waveform unavailable for this source.'
      : 'Click to seek · Arrow keys move 1s · Shift + Arrow moves 5s'

  return (
    <section
      className="source-waveform"
      data-cut-source-waveform={asset}
      data-cut-source-waveform-state={state}
      aria-label="Source audio waveform"
    >
      <div
        className="source-waveform__control"
        data-cut-source-waveform-control
        role="slider"
        tabIndex={isSeekable ? 0 : -1}
        aria-label="Source waveform position"
        aria-describedby="source-waveform-help"
        aria-valuemin={0}
        aria-valuemax={sourceMs}
        aria-valuenow={clamp(currentMs, sourceMs)}
        aria-valuetext={formatTime(currentMs)}
        aria-disabled={!isSeekable}
        onPointerDown={seekAtPointer}
        onKeyDown={seekWithKeyboard}
      >
        <canvas ref={canvasRef} className="source-waveform__canvas" aria-hidden="true" />
      </div>
      <div className="source-waveform__legend" id="source-waveform-help">
        <span>Audio waveform</span>
        <span className="source-waveform__range">Amber marks show In and Out · White line is current</span>
        <span className="source-waveform__status" data-cut-source-waveform-status aria-live="polite">{status}</span>
      </div>
    </section>
  )
}
