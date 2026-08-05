import type { PregateReport, PregateRisk } from '../lib/client'
import { openCutManual } from '../lib/manual'

interface PreflightWarningProps {
  report: PregateReport
  actionLabel: string
  onCancel: () => void
  onContinue: () => void
}

const severityLabel = (severity: PregateRisk['severity']) => {
  if (severity === 'high') return 'Fix first'
  if (severity === 'med') return 'Review'
  return 'Note'
}

const RISK_COPY: Record<string, { title: string; guidance: string; guide: string }> = {
  empty_tail: {
    title: 'Black ending',
    guidance: 'The timeline continues after the last video. Trim the tail, shorten audio, or add picture before exporting.',
    guide: 'cut.export.preflight.black_tail',
  },
  black_or_frozen: {
    title: 'Black or frozen footage',
    guidance: 'A clip contains dead frames in the edited range. Trim around that part or replace the shot.',
    guide: 'cut.export.preflight.dead_frames',
  },
  slideshow_risk: {
    title: 'Long holds',
    guidance: 'The base story has very few cuts. Add cuts, motion, or shorten holds if that is not intentional.',
    guide: 'cut.export.preflight.pacing',
  },
  silent_output: {
    title: 'Silent export',
    guidance: 'The timeline appears silent. Add or unmute audio, or continue if this is meant to be silent.',
    guide: 'cut.export.preflight.silent_audio',
  },
  tiny_or_zero_clips: {
    title: 'Tiny clip',
    guidance: 'One or more clips are shorter than a frame. Delete them or extend the clip before exporting.',
    guide: 'cut.export.preflight.tiny_clips',
  },
  uniform_border: {
    title: 'Black border',
    guidance: 'Source media appears letterboxed or pillarboxed. Crop to the visible content if you do not want bands in the export.',
    guide: 'cut.export.preflight.borders',
  },
}

const riskCopyFor = (risk: PregateRisk) =>
  RISK_COPY[risk.kind] ?? {
    title: 'Export warning',
    guidance: risk.detail || 'Review this item before exporting.',
    guide: 'cut.export.preflight',
  }

const formatRange = (risk: PregateRisk) => {
  const range = risk.range_ms
  if (!range) return ''
  const start = Math.max(0, Math.round(range[0] / 100) / 10)
  const end = Math.max(start, Math.round(range[1] / 100) / 10)
  return ` (${start}s-${end}s)`
}

export default function PreflightWarning({ report, actionLabel, onCancel, onContinue }: PreflightWarningProps) {
  const risks = report.risks ?? []
  const uninstrumented = report.uninstrumented_assets ?? []
  const blocked = report.pass === false || risks.some((risk) => risk.severity === 'high')
  const guideFeature = risks.map((risk) => riskCopyFor(risk).guide).find(Boolean) ?? 'cut.export.preflight'
  const summary =
    report.summary
    || (blocked
      ? `Cut found an export issue before ${actionLabel}.`
      : `Cut found warnings before ${actionLabel}.`)

  return (
    <section
      className={`tb-pregate${blocked ? ' tb-pregate--blocked' : ''}`}
      data-cut-pregate-warning
      data-cut-pregate-blocked={blocked ? 'true' : 'false'}
      role="dialog"
      aria-modal="false"
      aria-label="Render preflight warning"
    >
      <div className="tb-pregate-head">
        <span className="tb-pregate-kicker">Preflight</span>
        <button type="button" className="tb-pregate-close" data-cut-pregate-close onClick={onCancel} aria-label="Close preflight warning">x</button>
      </div>
      <h2>{blocked ? 'Fix before export' : 'Review before export'}</h2>
      <p>{summary}</p>
      {(risks.length > 0 || uninstrumented.length > 0) && (
        <ul className="tb-pregate-risks">
          {risks.map((risk, index) => (
            <li key={`${risk.kind}-${index}`} data-cut-pregate-risk data-cut-pregate-risk-kind={risk.kind} data-severity={risk.severity}>
              <span className="tb-pregate-sev">{severityLabel(risk.severity)}</span>
              <span>
                <b>{riskCopyFor(risk).title}</b>
                {formatRange(risk)}
                <small>{riskCopyFor(risk).guidance}</small>
              </span>
            </li>
          ))}
          {uninstrumented.length > 0 && (
            <li data-cut-pregate-risk data-severity="low">
              <span className="tb-pregate-sev">Note</span>
              <span>{uninstrumented.length} asset{uninstrumented.length === 1 ? '' : 's'} could not be checked fully.</span>
            </li>
          )}
        </ul>
      )}
      <details className="tb-pregate-details" data-cut-pregate-details>
        <summary data-cut-pregate-details-toggle>Details</summary>
        <dl>
          <div>
            <dt>Assets checked</dt>
            <dd>{report.perception_assets ?? 0}</dd>
          </div>
          <div>
            <dt>Needs deeper check</dt>
            <dd>{uninstrumented.length}</dd>
          </div>
          {risks.length > 0 && (
            <div>
              <dt>Risk details</dt>
              <dd>
                <ul className="tb-pregate-detail-list">
                  {risks.map((risk, index) => (
                    <li key={`${risk.kind}-detail-${index}`}>
                      <code>{risk.kind}</code>{risk.detail ? ` - ${risk.detail}` : ''}
                    </li>
                  ))}
                </ul>
              </dd>
            </div>
          )}
        </dl>
      </details>
      <div className="tb-pregate-actions">
        <button type="button" className="tb-btn tb-btn--secondary" data-cut-pregate-cancel onClick={onCancel}>
          {blocked ? 'Close' : 'Cancel'}
        </button>
        <button
          type="button"
          className="tb-btn tb-btn--secondary"
          data-cut-pregate-guide
          data-cut-pregate-guide-feature={guideFeature}
          onClick={() => {
            if (guideFeature === 'cut.export.preflight') openCutManual('cut.export.preflight')
            else openCutManual(guideFeature)
          }}
        >
          Guide
        </button>
        <button
          type="button"
          className="tb-btn tb-btn--primary"
          data-cut-pregate-continue
          disabled={blocked}
          onClick={onContinue}
          title={blocked ? 'Resolve the high-risk preflight issues before exporting' : `Continue ${actionLabel}`}
        >
          Continue
        </button>
      </div>
    </section>
  )
}
