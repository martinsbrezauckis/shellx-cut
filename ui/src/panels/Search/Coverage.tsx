import type { MediaEvidenceKind, MediaIntelligenceStatusResult } from '../../lib/client'
import { Icon } from '../../icons'
import { coverageSummary, KIND_PRESENTATION } from './model'

interface CoverageProps {
  status: MediaIntelligenceStatusResult | null
  busy: boolean
  rebuild: { progress: number; message: string; cancelling: boolean } | null
  onPrepare: () => void
  onCancel: () => void
  onRefresh: () => void
}

const KINDS: MediaEvidenceKind[] = ['transcript', 'visual', 'scene', 'beat', 'marker', 'metadata']

export default function Coverage({ status, busy, rebuild, onPrepare, onCancel, onRefresh }: CoverageProps) {
  const needsAttention = !status?.index_id || status?.stale || !status?.complete
  const actionLabel = status?.index_id ? 'Refresh search' : 'Prepare search'

  return (
    <section className="mi-coverage" data-cut-intelligence-coverage={status?.complete ? 'complete' : 'partial'}>
      <div className="mi-coverage__summary">
        <span className="mi-coverage__state" aria-live="polite">
          <Icon name={rebuild || busy ? 'spinner' : status?.stale ? 'warning' : status?.index_id ? 'check' : 'info'} size={14} tone={status?.stale ? 'warn' : status?.index_id ? 'success' : 'default'} />
          {rebuild ? rebuild.message : coverageSummary(status)}
        </span>
        <div className="mi-coverage__actions">
          {rebuild ? (
            <button type="button" className="cd-btn cd-btn--sm cd-btn--ghost" onClick={onCancel} disabled={rebuild.cancelling} data-cut-intelligence-cancel>
              {rebuild.cancelling ? 'Cancelling…' : 'Cancel'}
            </button>
          ) : (
            <button type="button" className="cd-btn cd-btn--sm" onClick={onPrepare} disabled={busy} data-cut-intelligence-prepare>{actionLabel}</button>
          )}
          <button type="button" className="mi-icon-button" onClick={onRefresh} disabled={busy || Boolean(rebuild)} title="Check search coverage again" aria-label="Check search coverage again" data-cut-intelligence-refresh>
            <Icon name="reset" size={14} />
          </button>
        </div>
      </div>
      {rebuild && (
        <div className="mi-progress" role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(rebuild.progress * 100)}>
          <span style={{ width: `${Math.max(3, Math.round(rebuild.progress * 100))}%` }} />
        </div>
      )}
      {status && needsAttention && (
        <details className="mi-coverage__details" data-cut-intelligence-coverage-details>
          <summary data-cut-intelligence-coverage-toggle>Coverage details</summary>
          <dl>
            {KINDS.map((kind) => {
              const coverage = status.coverage[kind]
              return <div key={kind}><dt>{KIND_PRESENTATION[kind].label}</dt><dd>{coverage.ready}/{coverage.total} ready{coverage.stale ? ` · ${coverage.stale} stale` : ''}</dd></div>
            })}
          </dl>
          <p>Prepare search only derives citations from analysis already stored in this project.</p>
        </details>
      )}
    </section>
  )
}
