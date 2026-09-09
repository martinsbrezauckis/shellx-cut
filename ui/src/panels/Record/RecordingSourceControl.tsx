import React, { useState } from 'react'
import {
  type RecordingSourceKind,
  type RegionPickerCapability,
} from './regionPickerModel'

// See RegionPickerOverlay: the source-contract renderer needs the classic JSX
// factory binding although the product build uses the automatic runtime.
void React

export interface MonitorInfo {
  id?: string
  index: number
  name: string
  width: number
  height: number
  primary: boolean
}

export interface WindowInfo {
  id: string
  title: string
  app: string
}

interface RecordingSourceControlProps {
  readonly sourceKind: RecordingSourceKind
  readonly monitors: readonly MonitorInfo[]
  readonly monitorIdx: number | null
  readonly windows: readonly WindowInfo[]
  readonly windowTargetId: string | null
  readonly selectedWindowMissing: boolean
  readonly disabled: boolean
  readonly allowWindow?: boolean
  readonly regionCapability: RegionPickerCapability
  readonly onRefresh: () => void | Promise<unknown>
  readonly onSourceKindChange: (source: RecordingSourceKind) => void
  readonly onMonitorChange: (index: number) => void
  readonly onWindowChange: (id: string | null) => void
}

function monitorLabel(monitor: MonitorInfo, monitorCount: number): string {
  return `${monitorCount >= 2 ? `Monitor ${monitor.index}` : 'Full screen'}${monitor.name ? ` — ${monitor.name}` : ''}${monitor.width && monitor.height ? ` (${monitor.width}×${monitor.height})` : ''}${monitor.primary && monitorCount >= 2 ? ' (primary)' : ''}`
}

function displayValue(monitors: readonly MonitorInfo[], monitorIdx: number | null): string {
  return `mon:${monitorIdx ?? (monitors.find((monitor) => monitor.primary)?.index ?? monitors[0]?.index ?? 1)}`
}

export function RecordingSourceControl({
  sourceKind,
  monitors,
  monitorIdx,
  windows,
  windowTargetId,
  selectedWindowMissing,
  disabled,
  allowWindow = true,
  regionCapability,
  onRefresh,
  onSourceKindChange,
  onMonitorChange,
  onWindowChange,
}: RecordingSourceControlProps) {
  const [refreshing, setRefreshing] = useState(false)
  const [refreshError, setRefreshError] = useState<string | null>(null)
  const refresh = async (): Promise<boolean> => {
    if (disabled || refreshing) return false
    setRefreshing(true)
    setRefreshError(null)
    try {
      const refreshed = await onRefresh()
      if (refreshed === false) {
        setRefreshError('Could not refresh sources. Try again.')
        return false
      }
      return true
    }
    catch {
      setRefreshError('Could not refresh sources. Try again.')
      return false
    }
    finally { setRefreshing(false) }
  }
  const regionUnavailable = regionCapability.availability === 'unavailable'
  // A persisted/stale `region` source must not make an unavailable control
  // visible. Record keeps its own fail-closed start guard; this surface repairs
  // to the normal Display presentation while React settles that stale state.
  const visibleSourceKind = sourceKind === 'region' && regionUnavailable ? 'display' : sourceKind
  const selectionValue = visibleSourceKind === 'window' && !selectedWindowMissing && windowTargetId
    ? `win:${windowTargetId}`
    : visibleSourceKind === 'window' ? '' : displayValue(monitors, monitorIdx)
  const sourceKinds = !allowWindow
    ? (['display'] as const)
    : regionCapability.availability === 'available'
    ? (['display', 'window', 'region'] as const)
    : (['display', 'window'] as const)

  return (
    <div className="rec__source-control" data-cut-rec-source-kind={visibleSourceKind}>
      <div className="rec__seg rec__source-kinds" role="group" aria-label="Recording source type">
        {sourceKinds.map((kind) => (
          <button
            key={kind}
            type="button"
            className={`rec__seg-btn${visibleSourceKind === kind ? ' rec__seg-btn--on' : ''}`}
            data-cut-rec-source-kind-button={kind}
            aria-pressed={visibleSourceKind === kind}
            disabled={disabled}
            onClick={() => {
              void refresh()
              onSourceKindChange(kind)
            }}
          >
            {kind === 'display' ? 'Display' : kind === 'window' ? 'Window' : 'Region'}
          </button>
        ))}
      </div>

      <button
        type="button"
        className="rec__export-btn rec__export-btn--ghost rec__export-btn--small"
        data-cut-action="record-source-refresh"
        disabled={disabled || refreshing}
        aria-busy={refreshing}
        title="Refresh the available displays and application windows"
        onClick={() => { void refresh() }}
      >
        {refreshing ? 'Refreshing…' : 'Refresh sources'}
      </button>
      {refreshError && <p className="rec__source-note" data-cut-rec-source-refresh-error role="status">{refreshError}</p>}

      {visibleSourceKind === 'region' ? (
        <p className="rec__source-note" data-cut-rec-region-ready role="status">
          Choose a region from the focused display picker.
        </p>
      ) : (
        <>
          <select
            className="rec__select"
            data-cut-rec-source={visibleSourceKind}
            data-cut-rec-monitor={visibleSourceKind === 'window' ? '' : (monitorIdx ?? '')}
            data-cut-rec-window={visibleSourceKind === 'window' ? (windowTargetId ?? '') : ''}
            disabled={disabled}
            onMouseDown={() => { void refresh() }}
            value={selectionValue}
            onChange={(event) => {
              const value = event.target.value
              if (value.startsWith('win:')) {
                onSourceKindChange('window')
                onWindowChange(value.slice(4))
              } else if (value.startsWith('mon:')) {
                onSourceKindChange('display')
                onMonitorChange(Number(value.slice(4)))
              } else {
                onSourceKindChange('window')
                onWindowChange(null)
              }
            }}
            aria-label={visibleSourceKind === 'display'
              ? 'Choose a display to record'
              : 'Choose an application window to record'}
          >
            {visibleSourceKind === 'display' ? (
              <optgroup label={monitors.length >= 2 ? 'Displays' : 'Screen'}>
                {(monitors.length >= 1 ? monitors : [{ index: 1, name: '', width: 0, height: 0, primary: true }]).map((monitor) => (
                  <option key={`mon-${monitor.index}`} value={`mon:${monitor.index}`}>
                    {monitorLabel(monitor, monitors.length)}
                  </option>
                ))}
              </optgroup>
            ) : (
              <optgroup label="Windows — record one app">
                <option value="">Choose an application…</option>
                {windows.map((window) => (
                  <option key={`win-${window.id}`} value={`win:${window.id}`}>
                    {`${window.title}${window.app ? ` — ${window.app}` : ''}`}
                  </option>
                ))}
              </optgroup>
            )}
          </select>

          {visibleSourceKind === 'window' && !windowTargetId && !selectedWindowMissing && (
            <p className="rec__source-note" data-cut-rec-window-no-selection>
              Choose the app window to record. If it just opened, refresh sources.
            </p>
          )}
          {selectedWindowMissing && (
            <p className="rec__source-note" data-cut-rec-window-missing>
              The selected window closed or changed identity. Choose another source before recording.
            </p>
          )}
          {visibleSourceKind === 'display' && monitors.length < 2 && windows.length < 1 && (
            <p className="rec__source-note" data-cut-rec-source-note>
              Choose a display or window here when one is listed. Otherwise, choose what to share in the system picker when you start.
            </p>
          )}
        </>
      )}
    </div>
  )
}
