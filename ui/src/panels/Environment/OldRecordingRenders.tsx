import { useEffect, useRef, useState } from 'react'
import { callVerb } from '../../lib/client'
import type { RecordingRenderCachePreview } from '../../lib/clientResults'
import { revealRegisteredSource } from '../../lib/tauri'

export default function OldRecordingRenders({ hasProject }: { hasProject: boolean }) {
  const [preview, setPreview] = useState<RecordingRenderCachePreview | null>(null)
  const [busy, setBusy] = useState(false)
  const [review, setReview] = useState(false)
  const [message, setMessage] = useState('')
  const live = useRef(true)

  const refresh = async () => {
    if (!hasProject) return
    setBusy(true)
    try {
      const response = await callVerb('screen_record.render_cache_preview', {})
      if (!live.current) return
      if (response.ok && response.result) setPreview(response.result)
      else { setPreview(null); setMessage(response.error?.message ?? 'Old recording renders could not be checked. Nothing was removed.') }
    } catch {
      if (live.current) { setPreview(null); setMessage('Old recording renders could not be checked. Try again.') }
    } finally { if (live.current) setBusy(false) }
  }

  useEffect(() => {
    live.current = true
    void refresh()
    return () => { live.current = false }
    // The parent remounts this project-bound inventory on every project switch.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [hasProject])

  const remove = async () => {
    if (!preview || busy) return
    setBusy(true)
    setMessage('')
    try {
      const response = await callVerb('screen_record.render_cache_remove', { plan_id: preview.plan_id, confirm: true })
      if (!live.current) return
      if (response.ok && response.result) setMessage(response.result.results.map(item => item.message).join('; ') || 'No old recording renders needed removal.')
      else setMessage(response.error?.message ?? 'Nothing was removed. Refresh the list and try again.')
      setReview(false)
      await refresh()
    } catch {
      if (live.current) setMessage('Removal could not be confirmed. Refresh the list before trying again.')
    } finally { if (live.current) setBusy(false) }
  }

  const legacy = preview?.legacy
  return <section className="settings-cache-lifecycle-cleanup" data-cut-old-recording-renders aria-label="Old recording renders">
    <strong>Old recording renders: {preview ? `${preview.files.length} files, ${(preview.bytes / 1e9).toFixed(3)} GB` : 'not checked'}</strong>
    <p>Only videos made for this recording by this editor can be removed. Original recordings and videos still used by clips are kept. Undo can rebuild a removed video with Update video.</p>
    <div className="settings-cache-lifecycle-actions">
      <button type="button" className="env-btn env-btn--ghost" data-cut-old-renders-refresh disabled={!hasProject || busy} title={!hasProject ? 'Open a project to check its recording renders.' : undefined} onClick={() => { setReview(false); void refresh() }}>{busy ? 'Checking…' : 'Refresh recording storage'}</button>
      <button type="button" className="env-btn" data-cut-old-renders-remove disabled={busy || !preview?.removable_files} title={!preview?.removable_files ? 'No safely removable old recording renders. Referenced or unproven files are kept.' : undefined} onClick={() => setReview(true)}>Remove</button>
    </div>
    {review && preview && <div data-cut-old-renders-review>
      <p>These are the only files that will be removed:</p>
      <ul>{preview.files.filter(file => file.removable).map(file => <li key={file.name}>{file.name} — {(file.bytes / 1e6).toFixed(1)} MB</li>)}</ul>
      <button type="button" className="env-btn env-btn--ghost" data-cut-old-renders-cancel onClick={() => setReview(false)} disabled={busy}>Keep files</button>
      <button type="button" className="env-btn" data-cut-old-renders-confirm onClick={() => void remove()} disabled={busy}>Remove listed files</button>
    </div>}
    {preview?.files.some(file => !file.removable) && <ul data-cut-old-renders-kept aria-label="Recording renders kept">{preview.files.filter(file => !file.removable).map(file => <li key={file.name}>{file.name} — {(file.bytes / 1e6).toFixed(1)} MB{file.size_is_recorded ? ' (size when made)' : ''} — kept: {file.reason}</li>)}</ul>}
    {!!legacy?.files.length && <div data-cut-old-renders-legacy>
      <p>Older or unrecorded renders kept: {legacy.files.length} files, {(legacy.bytes / 1e9).toFixed(3)} GB (their recording cannot be proven). The editor will not remove them; use Show in folder to look at them by hand.</p>
      <button type="button" className="env-btn env-btn--ghost" data-cut-old-renders-reveal-legacy disabled={!legacy.reveal_asset} title={!legacy.reveal_asset ? 'No file next to these renders is registered in the project, so the folder cannot be opened from here.' : undefined} onClick={() => { if (legacy.reveal_asset) void revealRegisteredSource(legacy.reveal_asset).then(reply => { if (live.current) setMessage(reply.message) }) }}>Show in folder</button>
    </div>}
    {message && <p role="status" data-cut-old-renders-status>{message}</p>}
  </section>
}
