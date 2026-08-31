import { useCallback, useEffect, useRef, useState } from 'react'
import { useBlockingOverlay } from '../../components/overlay/useBlockingOverlay'
import { Icon } from '../../icons'
import { callVerb, type Project } from '../../lib/client'
import type { PreviewComparisonResult } from '../../lib/clientResults'
import { timecode } from '../Timeline/layout'
import './previewComparison.css'

interface PreviewComparisonProps {
  project: Project | null
  playheadMs: number
  revision?: string
  onPause: () => void
}

function isComparisonResult(value: unknown): value is PreviewComparisonResult {
  if (!value || typeof value !== 'object') return false
  const result = value as Partial<PreviewComparisonResult>
  return result.schema === 'shellx-cut/preview-comparison/1'
    && typeof result.at_ms === 'number'
    && typeof result.current_revision === 'string'
    && typeof result.prior_revision === 'string'
    && typeof result.compared_operation?.id === 'string'
    && typeof result.compared_operation?.verb === 'string'
    && typeof result.before?.base64 === 'string'
    && typeof result.current?.base64 === 'string'
}

function frameUrl(base64: string): string {
  return `data:image/jpeg;base64,${base64}`
}

/** A locked, read-only revision pair. The actual historical replay stays in
 * Cut's server; this UI never mutates the timeline or simulates Undo. */
export default function PreviewComparison({ project, playheadMs, revision, onPause }: PreviewComparisonProps) {
  const [comparison, setComparison] = useState<PreviewComparisonResult | null>(null)
  const [busy, setBusy] = useState(false)
  const [notice, setNotice] = useState<string | null>(null)
  const requestId = useRef(0)
  const close = useCallback(() => {
    requestId.current += 1
    setComparison(null)
    setBusy(false)
  }, [])
  const overlay = useBlockingOverlay<HTMLElement>(close, Boolean(comparison))

  const open = useCallback(async () => {
    if (!project) return
    if (!revision) {
      setNotice('Make a saved edit before comparing it with the earlier frame.')
      return
    }
    const atMs = Math.max(0, Math.round(playheadMs))
    const request = requestId.current + 1
    requestId.current = request
    onPause()
    setNotice(null)
    setBusy(true)
    try {
      const response = await callVerb('render.compare', { at_ms: atMs, revision })
      if (request !== requestId.current) return
      if (!response.ok) {
        setNotice(response.error?.message ?? 'Cut could not prepare this comparison.')
        return
      }
      if (!isComparisonResult(response.result)) {
        setNotice('Cut returned an incomplete comparison. Try again after the edit settles.')
        return
      }
      // A server response can only paint while it still names the same live
      // head and playhead. This second guard prevents an old async answer from
      // covering a newer Preview frame between React commits.
      if (response.result.current_revision !== revision || response.result.at_ms !== atMs) {
        setNotice('The edit changed while comparison was loading. Try again.')
        return
      }
      setComparison(response.result)
    } catch {
      if (request === requestId.current) setNotice('Cut could not be reached to prepare comparison.')
    } finally {
      if (request === requestId.current) setBusy(false)
    }
  }, [onPause, playheadMs, project, revision])

  useEffect(() => {
    if (!comparison) return
    if (comparison.current_revision !== revision || comparison.at_ms !== Math.max(0, Math.round(playheadMs))) {
      close()
      setNotice('Comparison closed because the playhead or edit changed.')
    }
  }, [close, comparison, playheadMs, revision])

  if (!project) return null
  return (
    <>
      <button
        type="button"
        className="pv-compare-trigger"
        data-cut-preview-compare-trigger
        data-cut-preview-compare-state={busy ? 'loading' : comparison ? 'open' : notice ? 'refused' : 'idle'}
        aria-busy={busy || undefined}
        aria-haspopup="dialog"
        disabled={busy}
        title="Compare this exact frame with the timeline state before the latest edit"
        onClick={() => void open()}
      >
        <Icon name="diff" size={16} />
        <span>{busy ? 'Comparing…' : 'Compare'}</span>
      </button>
      {notice && !comparison && (
        <p className="pv-compare-notice" data-cut-preview-compare-notice role="status">{notice}</p>
      )}
      {comparison && (
        <section
          ref={overlay.dialogRef}
          className="pv-compare"
          data-cut-preview-comparison
          data-cut-preview-compare-at-ms={comparison.at_ms}
          data-cut-blocking-overlay
          data-cut-overlay-part
          role="dialog"
          aria-modal="true"
          aria-label={`Comparison at ${timecode(comparison.at_ms)}`}
          tabIndex={-1}
          onKeyDown={overlay.onDialogKeyDown}
        >
          <header className="pv-compare__head">
            <div>
              <strong>Compare frame</strong>
              <span>Paused at {timecode(comparison.at_ms)}</span>
            </div>
            <span className="pv-compare__exact">Exact composed frames</span>
            <button
              type="button"
              className="pv-compare__close"
              data-cut-action="close-preview-comparison"
              onClick={close}
            >
              <Icon name="close" size={16} />
              <span>Close</span>
            </button>
          </header>
          <div className="pv-compare__frames">
            <figure className="pv-compare__frame" data-cut-preview-compare-before>
              <figcaption>
                <strong>Before</strong>
                <span>Before the latest edit</span>
              </figcaption>
              <img src={frameUrl(comparison.before.base64)} alt={`Before the current edit at ${timecode(comparison.at_ms)}`} />
            </figure>
            <figure className="pv-compare__frame" data-cut-preview-compare-current>
              <figcaption>
                <strong>Current</strong>
                <span data-cut-preview-compare-operation={comparison.compared_operation.verb}>Current edit</span>
              </figcaption>
              <img src={frameUrl(comparison.current.base64)} alt={`Current edit at ${timecode(comparison.at_ms)}`} />
            </figure>
          </div>
        </section>
      )}
    </>
  )
}
