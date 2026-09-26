import { createPortal } from 'react-dom'
import type { ButtonHTMLAttributes } from 'react'
import ContextMenuFrame from '../../components/ContextMenuFrame'
import { cameraLayoutUnavailableReason, type CameraCapability } from './CameraControl'
import {
  studioBackgroundPreset, cameraPositionLabel,
  type StudioState, type StudioBackground, type StudioCameraPosition, type StudioCameraShape,
} from './studioTypes'

export interface RecordingCompositionActions {
  cameraCapability: CameraCapability
  cameraDeviceId: string | null
  rawCapture: boolean
  configurationDisabled: boolean
  liveAdjustDisabled: boolean
  onCameraEnabled(enabled: boolean): void
  onCameraPosition(position: StudioCameraPosition): void
  onCameraShape(shape: StudioCameraShape): void
  onCameraReset(): void
  onBackground(background: StudioBackground): void
}

export interface RecordingCompositionMenuState {
  x: number
  y: number
  target: 'composition' | 'camera'
}

/** Alternative access to the same guarded composition controls, never a
 * separate scene or capture mutation path. */
export function RecordingCompositionMenu({ menu, studio, actions, onClose }: {
  menu: RecordingCompositionMenuState
  studio: StudioState
  actions: RecordingCompositionActions
  onClose(): void
}) {
  const cameraReason = actions.configurationDisabled
    ? 'Camera sources cannot change during recording or another capture operation.'
    : actions.rawCapture ? 'Switch to Auto-edit to add an editable camera take.'
    : !studio.camera.enabled && (!actions.cameraCapability.supported || actions.cameraCapability.devices.length === 0)
      ? actions.cameraCapability.detail : null
  const layoutReason = cameraLayoutUnavailableReason(
    actions.cameraCapability, studio.camera.enabled, actions.cameraDeviceId,
    actions.rawCapture, actions.liveAdjustDisabled,
  )
  const backgroundReason = actions.rawCapture
    ? 'Composition is available in Auto-edit mode.'
    : actions.liveAdjustDisabled ? 'Wait for the current capture operation to finish.' : null
  const cameraVisible = studio.camera.enabled && !actions.rawCapture
  const run = (action: () => void) => { action(); onClose() }
  const item = (label: string, selected: boolean, reason: string | null, detail: string, action: () => void): ButtonHTMLAttributes<HTMLButtonElement> => ({
    type: 'button', className: 'tl-ctx__item', role: 'menuitem',
    disabled: Boolean(reason), title: reason ?? detail,
    'aria-label': `${label}${selected ? ', selected' : ''}`,
    children: `${selected ? '✓ ' : ''}${label}`,
    onClick: () => run(action),
  })
  const positionItem = (position: StudioCameraPosition) => item(
    cameraPositionLabel(position), studio.camera.position === position, layoutReason,
    `Move camera to ${cameraPositionLabel(position).toLowerCase()}`, () => actions.onCameraPosition(position),
  )
  const shapeItem = (shape: StudioCameraShape) => item(
    shape === 'circle' ? 'Circle' : 'Rounded rectangle', studio.camera.shape === shape, layoutReason,
    'Change the camera frame shape', () => actions.onCameraShape(shape),
  )
  const backgroundItem = (background: StudioBackground) => item(
    studioBackgroundPreset(background).label, studio.background === background, backgroundReason,
    studioBackgroundPreset(background).description, () => actions.onBackground(background),
  )

  return createPortal(
    <ContextMenuFrame
      x={menu.x} y={menu.y}
      menuId="data-cut-record-composition-menu"
      backdropId="data-cut-record-composition-backdrop"
      ariaLabel={menu.target === 'camera' ? 'Camera actions' : 'Composition actions'}
      onClose={onClose}
    >
      <span className="tl-ctx__label" aria-hidden="true">{menu.target === 'camera' ? 'Camera' : 'Composition'}</span>
      <button
        type="button" className="tl-ctx__item" role="menuitem"
        data-cut-action="record-context-camera-toggle"
        disabled={Boolean(cameraReason)}
        title={cameraReason ?? (cameraVisible ? 'Remove the camera from this recording setup' : 'Add a separate, editable camera take')}
        onClick={() => run(() => actions.onCameraEnabled(!cameraVisible))}
      >{cameraVisible ? 'Remove camera' : 'Add camera'}</button>
      {menu.target === 'camera' && cameraVisible && <>
        <span className="tl-ctx__sep" aria-hidden="true" />
        <span className="tl-ctx__label" aria-hidden="true">Position</span>
        <button data-cut-action="record-context-camera-position-top_left" {...positionItem('top_left')} />
        <button data-cut-action="record-context-camera-position-top_right" {...positionItem('top_right')} />
        <button data-cut-action="record-context-camera-position-bottom_right" {...positionItem('bottom_right')} />
        <button data-cut-action="record-context-camera-position-bottom_left" {...positionItem('bottom_left')} />
        <span className="tl-ctx__sep" aria-hidden="true" />
        <button data-cut-action="record-context-camera-shape-circle" {...shapeItem('circle')} />
        <button data-cut-action="record-context-camera-shape-rounded_rect" {...shapeItem('rounded_rect')} />
        <span className="tl-ctx__sep" aria-hidden="true" />
        <button data-cut-action="record-context-camera-layout-reset" {...item(
          'Reset camera layout', false, layoutReason,
          'Restore this scene’s camera layout', actions.onCameraReset,
        )} />
      </>}
      {menu.target === 'composition' && <>
        <span className="tl-ctx__sep" aria-hidden="true" />
        <span className="tl-ctx__label" aria-hidden="true">Background</span>
        <button data-cut-action="record-context-background-gradient" {...backgroundItem('gradient')} />
        <button data-cut-action="record-context-background-solid" {...backgroundItem('solid')} />
        <button data-cut-action="record-context-background-blur_screen" {...backgroundItem('blur_screen')} />
        <button data-cut-action="record-context-background-none" {...backgroundItem('none')} />
      </>}
    </ContextMenuFrame>, document.body,
  )
}
