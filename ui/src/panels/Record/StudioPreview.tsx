import { studioBackgroundPreset, type StudioState } from './studioTypes'
import type { RecordingSourcePreviewPresentation } from './recordingNativeSourcePreview'
import './recordingSourcePreview.css'

interface StudioPreviewProps {
  studio: StudioState
  phase: 'idle' | 'countdown' | 'starting' | 'recording' | 'finalizing' | 'done' | 'error'
  elapsed: string
  sceneName: string
  sceneState: string
  sourcePreview: RecordingSourcePreviewPresentation
}

export function StudioPreview({ studio, phase, elapsed, sceneName, sceneState, sourcePreview }: StudioPreviewProps) {
  const preset = studioBackgroundPreset(studio.background)
  const offlineFixture = typeof window !== 'undefined' && new URLSearchParams(window.location.search).get('mock') === '1'
  const showsNativeFrame = Boolean(sourcePreview.frameUrl)
  const sourceLabel = showsNativeFrame
    ? offlineFixture ? 'Offline fixture · not captured' : 'Native source'
    : 'Composition fallback · not source capture'
  return (
    <div
      className={`rec-studio-preview rec-studio-preview--${studio.background}`}
      data-cut-studio-preview
      data-cut-studio-background={studio.background}
      data-cut-studio-preset={preset.id}
      data-cut-studio-camera-enabled={studio.camera.enabled ? 'true' : 'false'}
      aria-label={`Studio preview: ${preset.description}`}
    >
      <div className="rec-studio-preview__heading">
        <span>Composition preview</span>
        <small>Controls stay outside the recorded frame</small>
      </div>
      <div
        className="rec-studio-preview__screen"
        data-cut-rec-native-preview-state={sourcePreview.state}
        data-cut-rec-native-preview-kind={showsNativeFrame ? (offlineFixture ? 'fixture' : 'native') : 'fallback'}
      >
        {sourcePreview.frameUrl ? (
          <img
            className="rec-studio-preview__native-frame"
            data-cut-rec-native-preview-frame
            src={sourcePreview.frameUrl}
            alt={offlineFixture ? 'Offline source preview fixture, not desktop capture' : 'Native selected-source preview'}
          />
        ) : (
          <>
            <div className="rec-studio-preview__bar" />
            <div className="rec-studio-preview__rows">
              <span />
              <span />
              <span />
            </div>
          </>
        )}
        <span className="rec-studio-preview__screen-label">{sourceLabel}</span>
      </div>
      {studio.camera.enabled && (
        <div
          className={`rec-studio-preview__camera rec-studio-preview__camera--${studio.camera.shape}`}
          data-cut-rec-camera-layout-preview
          aria-label="Camera layout preview"
          style={{
            left: `${studio.camera.x * 100}%`,
            top: `${studio.camera.y * 100}%`,
            width: `${studio.camera.size * 56.25}%`,
            height: `${studio.camera.size * 100}%`,
          }}
        >
          Camera
        </div>
      )}
      <div className="rec-studio-preview__status" role="status" aria-live="polite">
        <span data-cut-rec-preview-phase={phase}>
          {phase === 'recording' ? 'REC' : phase === 'countdown' || phase === 'starting' ? 'Starting' : 'Ready'}
        </span>
        <span data-cut-rec-scene-preview={sceneState}>{sceneName}</span>
        <span>{elapsed}</span>
      </div>
    </div>
  )
}
