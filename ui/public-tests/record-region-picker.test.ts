// ui/public-tests/record-region-picker.test.ts — bounded region-picker model
// and rendered-UI contract. This proves the present source slice is honest:
// a full DOM picker exists for a future capability bridge, while the live
// recorder hides a capability which cannot yet work, while retaining a
// fail-closed repair for any stale persisted Region state.

import assert from 'node:assert/strict'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import { RegionPickerOverlay } from '../src/panels/Record/RegionPickerOverlay'
import { RecordingSourceControl } from '../src/panels/Record/RecordingSourceControl'
import {
  fitRegionToDisplay,
  nudgeRegion,
  REGION_PICKER_UNAVAILABLE,
  type RegionPickerCapability,
} from '../src/panels/Record/regionPickerModel'

const display = { uiId: 'display-ui-a', label: 'Built-in display', width: 1920, height: 1080 }
const capability: Extract<RegionPickerCapability, { availability: 'available' }> = {
  availability: 'available',
  displays: [display],
  lastRegion: { displayUiId: display.uiId, rect: { x: 100, y: 80, width: 800, height: 600 } },
}

{
  const html = renderToStaticMarkup(createElement(RegionPickerOverlay, {
    capability,
    onCancel: () => {},
    onConfirm: () => {},
  }))
  assert.match(html, /data-cut-rec-region-picker/, 'the focused picker has a stable root selector')
  assert.match(html, /role="dialog"/, 'the picker is announced as a dialog')
  assert.match(html, /aria-modal="true"/, 'the focused picker excludes the background from modal navigation')
  assert.match(html, /data-cut-rec-region-state="no-selection"/, 'a last region is not silently reused as the active selection')
  assert.match(html, /Built-in display.*1920 × 1080/, 'the active display has a visible name and dimensions')
  assert.match(html, /data-cut-rec-region-no-selection/, 'the no-selection state tells the user what to do')
  assert.match(html, /data-cut-rec-region-display/, 'the display choice has a stable action identity')
  assert.match(html, /data-cut-action="record-region-cancel"/, 'the header Cancel action is explicit')
  assert.match(html, /data-cut-action="record-region-cancel-footer"/, 'the footer Cancel action is explicit')
  assert.match(html, /data-cut-action="record-region-use-last"/, 'the prior region is an explicit suggestion action')
  assert.match(html, /Arrow keys move the selection. Hold Shift to move farther. Enter uses it; Escape cancels./, 'keyboard alternatives are visible')
  assert.match(html, /data-cut-action="record-region-use"[^>]*disabled=""/, 'Use this region cannot submit without a selection')
}

{
  const sourceProps = {
    monitors: [
      { id: 'opaque-monitor-id-not-rendered', index: 1, name: 'Main display', width: 1920, height: 1080, primary: true },
      { id: 'opaque-monitor-id-not-rendered-2', index: 2, name: 'Presentation display', width: 2560, height: 1440, primary: false },
    ],
    monitorIdx: 1,
    windows: [{ id: 'opaque-window-id-not-rendered', title: 'Edit plan', app: 'ShellX Cut' }],
    windowTargetId: null,
    selectedWindowMissing: false,
    disabled: false,
    regionCapability: REGION_PICKER_UNAVAILABLE,
    onRefresh: () => {},
    onSourceKindChange: () => {},
    onMonitorChange: () => {},
    onWindowChange: () => {},
  } as const
  const displayHtml = renderToStaticMarkup(createElement(RecordingSourceControl, {
    ...sourceProps,
    sourceKind: 'display',
  }))
  assert.match(displayHtml, /data-cut-rec-source-kind="display"/, 'Display remains the current first-level source')
  assert.match(displayHtml, /data-cut-rec-source-kind-button="display"/, 'Display is available')
  assert.match(displayHtml, /data-cut-rec-source-kind-button="window"/, 'Window is available')
  assert.doesNotMatch(displayHtml, /data-cut-rec-source-kind-button="region"/, 'unavailable Region is not advertised as a dead control')
  assert.match(displayHtml, /Main display/, 'Display mode lists live displays')
  assert.match(displayHtml, /Presentation display/, 'Display mode lists every live display')
  assert.doesNotMatch(displayHtml, /Edit plan/, 'Display mode never mixes in application windows')
  assert.doesNotMatch(displayHtml, /opaque-monitor-id-not-rendered/, 'the UI contract never exposes a native monitor identity')

  const windowHtml = renderToStaticMarkup(createElement(RecordingSourceControl, {
    ...sourceProps,
    sourceKind: 'window',
  }))
  assert.match(windowHtml, /data-cut-rec-source-kind="window"/, 'Window selection has its own first-level state')
  assert.match(windowHtml, /Choose an application…/, 'Window mode starts with an explicit no-selection option')
  assert.match(windowHtml, /Edit plan — ShellX Cut/, 'Window mode lists live application windows')
  assert.doesNotMatch(windowHtml, /Main display/, 'Window mode never mixes in displays')

  const staleRegionHtml = renderToStaticMarkup(createElement(RecordingSourceControl, {
    ...sourceProps,
    sourceKind: 'region',
  }))
  assert.match(staleRegionHtml, /data-cut-rec-source-kind="display"/, 'a stale unavailable Region state repairs to the safe visible mode')
  assert.doesNotMatch(staleRegionHtml, /data-cut-rec-region-ready/, 'a stale state cannot expose the future picker path')
}

{
  const fitted = fitRegionToDisplay({ x: 1_800, y: 900, width: 800, height: 500 }, display)
  assert.deepEqual(fitted, { x: 1_120, y: 580, width: 800, height: 500 }, 'oversize/off-edge input is contained inside its one display')
  const atEdge = nudgeRegion({ x: 0, y: 0, width: 400, height: 300 }, display, 'left')
  assert.deepEqual(atEdge, { x: 0, y: 0, width: 400, height: 300 }, 'arrow movement never crosses a display edge')
  const fast = nudgeRegion({ x: 100, y: 100, width: 400, height: 300 }, display, 'down', true)
  assert.deepEqual(fast, { x: 100, y: 120, width: 400, height: 300 }, 'Shift-arrow uses the documented faster nudge')
}

console.log('PASS recorder region-picker bounded UI and geometry contract')
