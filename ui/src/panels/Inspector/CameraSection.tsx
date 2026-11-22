import { useEffect, useState } from 'react'
import InspectorSection from '../../components/inspector/InspectorSection'
import PropertyRow from '../../components/inspector/PropertyRow'
import { type ClipTransform } from '../../lib/client'
import { runUserVerb } from '../../lib/userActionFeedback'
import { CAMERA_RING, cameraPreset, cameraTransform, type CameraPosition } from './cameraSectionModel'
import './camera.css'

interface CameraSectionProps {
  clip: string
  stored?: ClipTransform | null
  width: number
  height: number
}
const POSITIONS: Array<[CameraPosition, string]> = [
  ['top_left', 'Top left'], ['top_right', 'Top right'], ['centre', 'Centre'],
  ['bottom_left', 'Bottom left'], ['bottom_right', 'Bottom right'],
]
const SHAPES = [['circle', 'Circle'], ['rounded', 'Rounded'], ['rect', 'Rectangle']] as const

export default function CameraSection({ clip, stored, width, height }: CameraSectionProps) {
  const [transform, setTransform] = useState(() => cameraTransform(stored, {}))
  const [busy, setBusy] = useState(false)
  const [note, setNote] = useState('')
  const ringUnavailable = !transform.shape || transform.shape === 'rect'
  useEffect(() => { setTransform(cameraTransform(stored, {})); setNote('') }, [clip, stored])

  async function write(patch?: Partial<ClipTransform>) {
    if (busy) return
    setBusy(true)
    setNote('')
    try {
      const next = cameraTransform(transform, patch ?? {})
      const result = patch
        ? await runUserVerb('edit.transform', { clip, ...next }, 'Could not change camera.')
        : await runUserVerb('screen_record.camera_reset', { clip }, 'Could not reset camera.')
      if (!result?.ok) setNote(result?.error?.message || 'Could not change camera.')
      else {
        if (patch) setTransform(next)
        document.dispatchEvent(new CustomEvent('cut:show-composed'))
      }
    } catch (error) {
      setNote(error instanceof Error ? error.message : 'Could not change camera.')
    } finally { setBusy(false) }
  }

  return (
    <InspectorSection title="Camera" sectionKey="camera">
      <div className="insp__camera-stack">
        <div className="insp__label">Position</div>
        <div className="insp__camera-positions" role="group" aria-label="Camera position">
          {POSITIONS.map(([position, label]) => (
            <button data-cut-camera-position={position} key={position} className={`insp__btn insp__camera-${position}`} disabled={busy}
              onClick={() => void write(cameraPreset(position, transform.scale, width, height))}>
              {label}
            </button>
          ))}
        </div>
        <PropertyRow label="Size" propKey="camera-size" unit="%"
          value={Math.round(transform.scale * 100)} min={5} max={50} step={1} default={20}
          disabled={busy} onCommit={(value) => void write({ scale: value / 100 })} />
        <div className="insp__label">Shape</div>
        <div className="insp__camera-segments" role="group" aria-label="Camera shape">
          {SHAPES.map(([shape, label]) => (
            <button data-cut-camera-shape={shape} key={shape} className={`insp__btn${(transform.shape ?? 'rect') === shape ? ' insp__btn--on' : ''}`}
              aria-pressed={(transform.shape ?? 'rect') === shape} disabled={busy}
              onClick={() => void write({ shape, radius: shape === 'rounded' ? .18 : undefined })}>
              {label}
            </button>
          ))}
        </div>
        <button data-cut-action="camera-ring" className={`insp__btn${transform.ring ? ' insp__btn--on' : ''}`}
          aria-pressed={!!transform.ring} disabled={busy || ringUnavailable}
          aria-describedby={ringUnavailable ? 'camera-ring-reason' : undefined}
          onClick={() => void write({ ring: transform.ring ? undefined : CAMERA_RING })}>Ring</button>
        {ringUnavailable && <p id="camera-ring-reason" className="insp__hint">A ring needs a circle or rounded shape.</p>}
        <button data-cut-action="camera-reset" className="insp__btn" disabled={busy} onClick={() => void write()}>Back to as recorded</button>
        <p className="insp__hint">Move it on the preview, cut or hide it like any clip.</p>
        {note && <p className="insp__hint insp__hint--error" role="status">{note}</p>}
      </div>
    </InspectorSection>
  )
}
