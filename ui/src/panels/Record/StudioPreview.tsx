import { useRef, useState, type KeyboardEvent, type MouseEvent } from 'react'
import { studioBackgroundPreset, type StudioState } from './studioTypes'
import type { RecordingSourcePreviewPresentation } from './recordingNativeSourcePreview'
import type { RecordingLiveFramePresentation } from './recordingLiveFramePresentation'
import { useRecordingLiveFrame } from './useRecordingLiveFrame'
import { RecordingCompositionMenu, type RecordingCompositionActions, type RecordingCompositionMenuState } from './RecordingCompositionMenu'
import './recordingSourcePreview.css'
import './recordingCompositionMenu.css'

interface StudioPreviewProps {
  studio: StudioState
  phase: 'idle' | 'countdown' | 'starting' | 'recording' | 'finalizing' | 'done' | 'error'
  elapsed: string
  sceneName: string
  sceneState: string
  sourcePreview: RecordingSourcePreviewPresentation
  sourcePreviewCanStart?: boolean
  onSourcePreviewStart?: () => void
  activeCaptureId?: string | null
  livePresentation?: RecordingLiveFramePresentation | null
  actions?: RecordingCompositionActions
}

export function StudioPreview({ studio, phase, elapsed, sceneName, sceneState, sourcePreview, sourcePreviewCanStart = false, onSourcePreviewStart, activeCaptureId, livePresentation, actions }: StudioPreviewProps) {
  const [menu, setMenu] = useState<RecordingCompositionMenuState | null>(null)
  const triggerRef = useRef<HTMLElement | null>(null)
  const previewRef = useRef<HTMLDivElement | null>(null)
  const rawCapture = actions?.rawCapture ?? false
  const openMenu = (event: MouseEvent<HTMLElement> | KeyboardEvent<HTMLElement>, target: RecordingCompositionMenuState['target']) => {
    if (!actions) return
    event.preventDefault()
    event.stopPropagation()
    triggerRef.current = event.currentTarget
    const rect = event.currentTarget.getBoundingClientRect()
    const pointer = 'clientX' in event && event.type === 'contextmenu'
    setMenu({ x: pointer ? event.clientX : rect.left + 12, y: pointer ? event.clientY : rect.top + 28, target })
  }
  const menuKey = (event: KeyboardEvent<HTMLElement>, target: RecordingCompositionMenuState['target']) => {
    if (event.key === 'ContextMenu' || (event.shiftKey && event.key === 'F10')) openMenu(event, target)
  }
  const closeMenu = () => {
    setMenu(null)
    requestAnimationFrame(() => {
      const target = triggerRef.current?.isConnected ? triggerRef.current : previewRef.current
      target?.focus({ preventScroll: true })
    })
  }
  const preset = studioBackgroundPreset(studio.background)
  const offlineFixture = typeof window !== 'undefined' && new URLSearchParams(window.location.search).get('mock') === '1'
  const liveCaptureId = phase === 'recording' && activeCaptureId ? activeCaptureId : null
  const polledLive = useRecordingLiveFrame(livePresentation?.captureId === liveCaptureId ? null : liveCaptureId)
  const live = liveCaptureId && livePresentation?.captureId === liveCaptureId ? livePresentation : polledLive
  const showLive = phase === 'recording'
  const showSource = phase === 'idle' || phase === 'countdown'
  const frameUrl = showLive ? live?.state === 'ready' ? live.frameUrl : null : showSource ? sourcePreview.frameUrl : null
  const showSourcePreviewAction = showSource && !frameUrl
    && !['starting', 'ready', 'paused'].includes(sourcePreview.state)
  const sourceLabel = showLive ? frameUrl ? 'Recording source · live'
    : live?.state === 'connecting' || live?.state === 'awaiting_source' || live?.state === 'awaiting_frame'
      ? 'Waiting for recording preview' : 'Recording preview unavailable'
    : frameUrl ? offlineFixture ? 'Offline fixture · not captured' : 'Selected source · before recording'
      : sourcePreview.state === 'idle' || sourcePreview.state === 'stopped' ? 'Source preview is off' : 'No source frame'
  const unavailableDetail = showLive
    ? liveCaptureId ? live?.detail ?? 'Waiting for frames from this recording.' : 'Waiting for a confirmed recording capture.'
    : showSource ? sourcePreview.state === 'idle' || sourcePreview.state === 'stopped'
      ? 'Click Preview source to see real pixels from the selected display or window.' : sourcePreview.detail
      : phase === 'starting' ? 'Waiting for recording to start.'
      : phase === 'finalizing' ? 'Recording has stopped. Finishing the file.'
        : 'The recording is no longer live. Select a source to preview again.'
  return (
    <div className="rec-studio-preview__frame">
      <div className="rec-studio-preview__chrome">
        <div className="rec-studio-preview__heading">
          <span>{showLive ? 'Recording source · live' : '2 · Selected source'}</span>
          <small>{showLive ? 'Live source pixels from this capture' : 'Preview starts only when you ask'}</small>
        </div>
      </div>
      <div
        ref={previewRef}
        className={`rec-studio-preview rec-studio-preview--${rawCapture ? 'none' : studio.background}`}
        role="group"
        tabIndex={actions ? 0 : undefined}
        aria-haspopup={actions ? 'menu' : undefined}
        aria-expanded={actions ? menu?.target === 'composition' : undefined}
        onContextMenu={(event) => openMenu(event, 'composition')}
        onKeyDown={(event) => menuKey(event, 'composition')}
        data-cut-studio-preview
        data-cut-action="record-composition-context-menu"
        data-cut-studio-background={studio.background}
        data-cut-studio-preset={preset.id}
        data-cut-studio-camera-enabled={studio.camera.enabled && !rawCapture ? 'true' : 'false'}
        aria-label={showLive ? 'Live recording source preview' : rawCapture ? 'Raw source preview' : `Studio preview: ${preset.description}`}
      >
        <div
          className="rec-studio-preview__screen"
          data-cut-rec-native-preview-state={showLive ? live?.state ?? 'connecting' : showSource ? sourcePreview.state : 'idle'}
          data-cut-rec-native-preview-kind={frameUrl ? showLive ? 'recording' : offlineFixture ? 'fixture' : 'source' : 'unavailable'}
          data-cut-rec-live-capture-id={liveCaptureId ?? undefined}
        >
          {frameUrl ? (
            <img
              className="rec-studio-preview__native-frame"
              data-cut-rec-native-preview-frame
              src={frameUrl}
              alt={showLive ? 'Current frame from this recording' : offlineFixture ? 'Offline source preview fixture, not desktop capture' : 'Selected source preview before recording'}
            />
          ) : (
            <div className="rec-studio-preview__empty" data-cut-rec-preview-unavailable>
              <strong>{sourceLabel}</strong>
              <span>{unavailableDetail}</span>
              {showSourcePreviewAction && onSourcePreviewStart && (
                <button type="button" className="rec-studio-preview__source-action"
                  data-cut-action="record-source-preview" onClick={onSourcePreviewStart}
                  disabled={!sourcePreviewCanStart}>Preview source</button>
              )}
            </div>
          )}
          {frameUrl && <span className="rec-studio-preview__screen-label">{sourceLabel}</span>}
        </div>
        {studio.camera.enabled && !rawCapture && (
          <div
            className={`rec-studio-preview__camera rec-studio-preview__camera--${studio.camera.shape}`}
            data-cut-rec-camera-layout-preview
            data-cut-action="record-camera-menu"
            role={actions ? 'button' : undefined}
            tabIndex={actions ? 0 : undefined}
            aria-haspopup={actions ? 'menu' : undefined}
            aria-expanded={actions ? menu?.target === 'camera' : undefined}
            aria-label="Camera layout preview actions"
            onClick={(event) => openMenu(event, 'camera')}
            onContextMenu={(event) => openMenu(event, 'camera')}
            onKeyDown={(event) => {
              if (event.key === 'Enter' || event.key === ' ') openMenu(event, 'camera')
              else menuKey(event, 'camera')
            }}
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
        <div className="rec-studio-preview__status" data-cut-rec-preview-status role="status" aria-live="polite">
          <span data-cut-rec-preview-phase={phase}>
            {phase === 'recording' ? 'REC' : phase === 'countdown' || phase === 'starting' ? 'Starting'
              : phase === 'finalizing' ? 'Finishing' : phase === 'error' ? 'Needs attention' : phase === 'done' ? 'Recorded' : 'Ready'}
          </span>
          <span data-cut-rec-scene-preview={sceneState}>{sceneName}</span>
          <span>{elapsed}</span>
        </div>
      </div>
      {menu && actions && <RecordingCompositionMenu menu={menu} studio={studio} actions={actions} onClose={closeMenu} />}
    </div>
  )
}
