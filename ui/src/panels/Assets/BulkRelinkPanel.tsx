import { useMemo, useState } from 'react'
import { callVerb } from '../../lib/client'
import { mediaBasename } from '../../lib/mediaPath'
import { isTauri, pickFolder } from '../../lib/tauri'

type Disposition = 'eligible_exact_hash' | 'ambiguous_exact_hash' | 'metadata_only' | 'ambiguous_metadata' | 'hash_unavailable' | 'no_match'
interface PreviewRow {
  asset: string
  expected_hash: string
  display_name: string
  disposition: Disposition
  diagnostics?: string[]
}
interface Preview {
  schema: 'shellx-cut/media-relink-preview/1'
  project_revision: string
  plan_hash: string
  scan: { files: number; directories: number }
  assets: PreviewRow[]
}
interface Receipt { grouped_op_id: string; assets: Array<{ asset: string }> }

function requestId(): string {
  return `bulk-relink-${globalThis.crypto?.randomUUID?.() ?? `${Date.now()}-${Math.random().toString(16).slice(2)}`}`
}

function dispositionLabel(row: PreviewRow): string {
  if (row.disposition === 'eligible_exact_hash') return 'Exact match'
  if (row.disposition === 'ambiguous_exact_hash') return 'Multiple exact matches — choose elsewhere'
  if (row.disposition === 'metadata_only') return 'Metadata resemblance only — refused'
  if (row.disposition === 'ambiguous_metadata') return 'Ambiguous metadata — refused'
  if (row.disposition === 'hash_unavailable') return 'No complete source hash — refused'
  return 'No match found'
}

/** Human-only B5 recovery surface.  It never renders selected folder paths. */
export default function BulkRelinkPanel({
  offlineCount,
  onProjectChanged,
  onRefresh,
}: {
  offlineCount: number
  onProjectChanged?: () => void | Promise<void>
  onRefresh: () => Promise<void>
}) {
  const [phase, setPhase] = useState<'idle' | 'previewing' | 'ready' | 'applying' | 'done'>('idle')
  const [preview, setPreview] = useState<Preview | null>(null)
  const [root, setRoot] = useState<string | null>(null)
  const [selected, setSelected] = useState<Set<string>>(new Set())
  const [request, setRequest] = useState('')
  const [note, setNote] = useState<string | null>(null)
  const [receipt, setReceipt] = useState<Receipt | null>(null)
  const eligible = useMemo(() => preview?.assets.filter((row) => row.disposition === 'eligible_exact_hash') ?? [], [preview])

  const start = async () => {
    setNote(null)
    setReceipt(null)
    if (!isTauri()) { setNote('Bulk relink needs the desktop app to choose a local recovery folder.'); return }
    const root = await pickFolder({ title: 'Find offline media — ShellX Cut' })
    if (!root) return
    setPhase('previewing')
    try {
      const response = await callVerb('media.relink_preview', { root })
      if (!response.ok || !response.result) { setNote(response.error?.message ?? 'Could not inspect the selected folder.'); setPhase('idle'); return }
      const next = response.result as Preview
      setPreview(next)
      setRoot(root)
      setSelected(new Set(next.assets.filter((row) => row.disposition === 'eligible_exact_hash').map((row) => row.asset)))
      setRequest(requestId())
      setPhase('ready')
    } catch { setNote('Recovery preview could not reach the local engine.'); setPhase('idle') }
  }

  const cancel = () => {
    if (phase === 'previewing' || phase === 'applying') return
    setPreview(null); setRoot(null); setSelected(new Set()); setRequest(''); setReceipt(null); setNote(null); setPhase('idle')
  }
  const apply = async () => {
    if (!preview || selected.size === 0) return
    setPhase('applying'); setNote(null)
    try {
      if (!root) { setNote('Preview origin is unavailable; choose the folder again.'); setPhase('idle'); return }
      const response = await callVerb('media.relink_apply', {
        root,
        plan_hash: preview.plan_hash,
        accept: [...selected],
        request_id: request,
        expected_revision: preview.project_revision,
        rationale: 'Grouped exact-hash offline-media recovery',
      })
      if (!response.ok || !response.result) { setNote(response.error?.message ?? 'No media was relinked.'); setPhase('ready'); return }
      setReceipt(response.result as Receipt)
      setPhase('done')
      await onProjectChanged?.()
      await onRefresh()
    } catch { setNote('Bulk relink could not reach the local engine. No success was confirmed.'); setPhase('ready') }
  }

  return (
    <section className="assets__bulk-relink" data-cut-media-relink-bulk data-cut-media-relink-state={phase}>
      <div className="assets__bulk-relink-copy">
        <strong>Recover missing media</strong>
        <span>{offlineCount} offline {offlineCount === 1 ? 'asset' : 'assets'} · exact files only</span>
      </div>
      {phase === 'idle' && <button type="button" className="assets__health-btn" data-cut-media-relink-bulk-open onClick={() => void start()}>Find files…</button>}
      {(phase === 'previewing' || phase === 'applying') && <span className="assets__bulk-relink-busy" aria-live="polite">{phase === 'previewing' ? 'Checking files…' : 'Relinking selected files…'}</span>}
      {note && <p className="assets__bulk-relink-note" role="status">{note}</p>}
      {preview && (phase === 'ready' || phase === 'done') && (
        <div className="assets__bulk-relink-preview" data-cut-media-relink-preview>
          <p>{preview.scan.files} files checked. Only complete SHA-256 matches can be applied; ambiguous and metadata-only rows stay refused.</p>
          <ul>
            {preview.assets.map((row) => {
              const selectable = row.disposition === 'eligible_exact_hash'
              return <li key={row.asset} data-cut-media-relink-row={row.asset} data-cut-media-relink-disposition={row.disposition}>
                <label>
                  <input data-cut-media-relink-accept={row.asset} type="checkbox" checked={selected.has(row.asset)} disabled={!selectable || phase === 'done'} onChange={() => setSelected((prior) => {
                    const next = new Set(prior); next.has(row.asset) ? next.delete(row.asset) : next.add(row.asset); return next
                  })} />
                  <span>{mediaBasename(row.display_name)}</span>
                </label>
                <em>{dispositionLabel(row)}</em>
              </li>
            })}
          </ul>
          {phase === 'ready' && <div className="assets__bulk-relink-actions">
            <button type="button" data-cut-media-relink-cancel onClick={cancel}>Cancel</button>
            <button type="button" className="assets__health-btn" data-cut-media-relink-apply disabled={selected.size === 0 || eligible.length === 0} onClick={() => void apply()}>Relink {selected.size || ''} selected</button>
          </div>}
          {receipt && <p className="assets__bulk-relink-success" role="status">Relinked {receipt.assets.length} assets in one project operation. The receipt is retained with the project.</p>}
        </div>
      )}
    </section>
  )
}
