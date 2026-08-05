import { type StudioBackground } from './studioTypes'

interface StudioPreviewProps {
  background: StudioBackground
  phase: 'idle' | 'recording' | 'finalizing' | 'done' | 'error'
  elapsed: string
}

export function StudioPreview({ background, phase, elapsed }: StudioPreviewProps) {
  return (
    <div
      className={`rec-studio-preview rec-studio-preview--${background}`}
      data-cut-studio-preview
      data-cut-studio-background={background}
      data-cut-studio-camera-available="false"
    >
      <div className="rec-studio-preview__screen" aria-hidden="true">
        <div className="rec-studio-preview__bar" />
        <div className="rec-studio-preview__rows">
          <span />
          <span />
          <span />
        </div>
      </div>
      <div className="rec-studio-preview__status">
        <span data-cut-rec-preview-phase={phase}>{phase === 'recording' ? 'REC' : 'Ready'}</span>
        <span>{elapsed}</span>
      </div>
    </div>
  )
}
