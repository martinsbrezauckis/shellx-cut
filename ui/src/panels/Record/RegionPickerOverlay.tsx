import React, { useEffect, useMemo, useRef, useState, type KeyboardEvent, type PointerEvent } from 'react'
import { useBlockingOverlay } from '../../components/overlay/useBlockingOverlay'
import {
  fitRegionToDisplay,
  nudgeRegion,
  regionPreviewStyle,
  type RegionPickerCapability,
  type RegionPickerDisplay,
  type RegionRect,
} from './regionPickerModel'

// The retained source-contract renderer compiles TSX with the classic factory;
// Vite's automatic runtime still tree-shakes this binding from the product build.
void React

interface Point { x: number; y: number }

export interface RegionPickerOverlayProps {
  readonly capability: Extract<RegionPickerCapability, { availability: 'available' }>
  readonly onCancel: () => void
  /** A later native bridge must convert this UI-local choice to an opaque one-use ticket. */
  readonly onConfirm: (selection: { displayUiId: string; rect: RegionRect }) => void
}

function displayById(displays: readonly RegionPickerDisplay[], uiId: string): RegionPickerDisplay | null {
  return displays.find((display) => display.uiId === uiId) ?? null
}

function stagePoint(event: PointerEvent<HTMLDivElement>, display: RegionPickerDisplay): Point {
  const bounds = event.currentTarget.getBoundingClientRect()
  const x = Math.min(display.width, Math.max(0, ((event.clientX - bounds.left) / bounds.width) * display.width))
  const y = Math.min(display.height, Math.max(0, ((event.clientY - bounds.top) / bounds.height) * display.height))
  return { x, y }
}

function dragRegion(start: Point, end: Point, display: RegionPickerDisplay): RegionRect | null {
  return fitRegionToDisplay({
    x: Math.min(start.x, end.x),
    y: Math.min(start.y, end.y),
    width: Math.abs(end.x - start.x),
    height: Math.abs(end.y - start.y),
  }, display)
}

export function RegionPickerOverlay({ capability, onCancel, onConfirm }: RegionPickerOverlayProps) {
  const [displayUiId, setDisplayUiId] = useState(capability.displays[0]?.uiId ?? '')
  const [selection, setSelection] = useState<RegionRect | null>(null)
  const overlay = useBlockingOverlay<HTMLDivElement>(onCancel)
  const dragStartRef = useRef<Point | null>(null)
  const display = useMemo(() => displayById(capability.displays, displayUiId), [capability.displays, displayUiId])
  const lastRegion = capability.lastRegion
  const suggestion = lastRegion && lastRegion.displayUiId === displayUiId ? lastRegion.rect : null

  useEffect(() => {
    if (displayById(capability.displays, displayUiId)) return
    setDisplayUiId(capability.displays[0]?.uiId ?? '')
    setSelection(null)
  }, [capability.displays, displayUiId])

  const useSuggestion = () => {
    if (!display || !suggestion) return
    setSelection(fitRegionToDisplay(suggestion, display))
  }

  const onRegionKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (event.defaultPrevented) return
    if (event.key === 'Enter') {
      if (!selection || !display) return
      event.preventDefault()
      onConfirm({ displayUiId: display.uiId, rect: selection })
      return
    }
    const direction = event.key === 'ArrowLeft' ? 'left'
      : event.key === 'ArrowRight' ? 'right'
        : event.key === 'ArrowUp' ? 'up'
          : event.key === 'ArrowDown' ? 'down' : null
    if (!direction || !selection || !display) return
    event.preventDefault()
    setSelection(nudgeRegion(selection, display, direction, event.shiftKey))
  }

  return (
    <div
      ref={overlay.dialogRef}
      className="rec-region-picker"
      data-cut-rec-region-picker
      data-cut-rec-region-state={selection ? 'selected' : 'no-selection'}
      data-cut-blocking-overlay
      data-cut-overlay-part
      role="dialog"
      aria-modal="true"
      aria-labelledby="cut-region-picker-title"
      aria-describedby="cut-region-picker-help"
      tabIndex={-1}
      onKeyDown={(event) => { overlay.onDialogKeyDown(event); onRegionKeyDown(event) }}
    >
      <div className="rec-region-picker__backdrop" aria-hidden="true" />
      <section className="rec-region-picker__surface">
        <header className="rec-region-picker__header">
          <div>
            <p className="rec-region-picker__eyebrow">Record a region</p>
            <h2 id="cut-region-picker-title">Choose the part of one display to record</h2>
          </div>
          <button type="button" className="rec-region-picker__cancel" data-cut-action="record-region-cancel" onClick={onCancel}>
            Cancel
          </button>
        </header>

        {capability.displays.length === 0 || !display ? (
          <p className="rec-region-picker__status" data-cut-rec-region-no-display role="status">
            No display is available right now. Choose Display or Window instead.
          </p>
        ) : (
          <>
            <label className="rec-region-picker__display">
              <span>Display</span>
              <select
                value={displayUiId}
                data-cut-rec-region-display
                onChange={(event) => {
                  setDisplayUiId(event.target.value)
                  setSelection(null)
                }}
              >
                {capability.displays.map((candidate) => (
                  <option key={candidate.uiId} value={candidate.uiId}>
                    {candidate.label} — {candidate.width} × {candidate.height}
                  </option>
                ))}
              </select>
            </label>

            <div
              className="rec-region-picker__stage"
              data-cut-rec-region-stage
              data-cut-rec-region-display-name={display.label}
              style={{ aspectRatio: `${display.width} / ${display.height}` }}
              onPointerDown={(event) => {
                const point = stagePoint(event, display)
                dragStartRef.current = point
                event.currentTarget.setPointerCapture(event.pointerId)
              }}
              onPointerMove={(event) => {
                const start = dragStartRef.current
                if (!start) return
                setSelection(dragRegion(start, stagePoint(event, display), display))
              }}
              onPointerUp={(event) => {
                dragStartRef.current = null
                event.currentTarget.releasePointerCapture(event.pointerId)
              }}
              onPointerCancel={() => { dragStartRef.current = null }}
            >
              <div className="rec-region-picker__display-label" data-cut-rec-region-display-facts>
                <strong>{display.label}</strong>
                <span>{display.width} × {display.height}</span>
              </div>
              <div className="rec-region-picker__canvas-hint" aria-hidden="true">Drag to choose an area</div>
              {selection && (
                <div className="rec-region-picker__selection" data-cut-rec-region-selection style={regionPreviewStyle(selection, display)}>
                  <span>{selection.width} × {selection.height}</span>
                </div>
              )}
            </div>

            <div className="rec-region-picker__guidance" id="cut-region-picker-help">
              {selection ? (
                <p data-cut-rec-region-selection-summary>
                  {display.label}, {selection.width} × {selection.height}. This selection stays on this display.
                </p>
              ) : (
                <p data-cut-rec-region-no-selection>Drag over the preview to choose what to record.</p>
              )}
              {suggestion && (
                <button type="button" className="rec-region-picker__suggestion" data-cut-action="record-region-use-last" onClick={useSuggestion}>
                  Use last region as a starting point
                </button>
              )}
              <p className="rec-region-picker__keys">Arrow keys move the selection. Hold Shift to move farther. Enter uses it; Escape cancels.</p>
            </div>
          </>
        )}

        <footer className="rec-region-picker__footer">
          <button type="button" className="rec__export-btn rec__export-btn--ghost" data-cut-action="record-region-cancel-footer" onClick={onCancel}>
            Cancel
          </button>
          <button
            type="button"
            className="rec__start rec-region-picker__use"
            data-cut-action="record-region-use"
            disabled={!selection || !display}
            onClick={() => {
              if (selection && display) onConfirm({ displayUiId: display.uiId, rect: selection })
            }}
          >
            Use this region
          </button>
        </footer>
      </section>
    </div>
  )
}
