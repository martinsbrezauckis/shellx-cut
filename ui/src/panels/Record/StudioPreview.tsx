import { useRef, useState, type KeyboardEvent, type MouseEvent } from 'react'
import { studioBackgroundPreset, type StudioState } from './studioTypes'
import type { RecordingSourcePreviewPresentation } from './recordingNativeSourcePreview'
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
  actions?: RecordingCompositionActions
}

export function StudioPreview({ studio, phase, elapsed, sceneName, sceneState, sourcePreview, actions }: StudioPreviewProps) {
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
  const showsNativeFrame = Boolean(sourcePreview.frameUrl)
  const sourceLabel = showsNativeFrame
    ? offlineFixture ? 'Offline fixture · not captured' : 'Native source'
    : 'Layout preview · source preview off'
  return (
    <div className="rec-studio-preview__frame">
      <div className="rec-studio-preview__chrome">
        <div className="rec-studio-preview__heading">
          <span>{rawCapture ? 'Raw source preview' : 'Composition preview'}</span>
          <small>Controls stay outside the recorded frame</small>
        </div>
        {actions && <button
          type="button" className="rec-studio-preview__actions"
          data-cut-action="record-composition-menu"
          aria-label="Composition actions" aria-haspopup="menu"
          aria-expanded={menu?.target === 'composition'}
          title="Composition actions (also available with right-click or Shift+F10)"
          onClick={(event) => openMenu(event, 'composition')}
        >Actions</button>}
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
        aria-label={rawCapture ? 'Raw source preview' : `Studio preview: ${preset.description}`}
      >
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
