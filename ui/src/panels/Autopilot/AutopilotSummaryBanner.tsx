import { Icon } from '../../icons'
import type { AutopilotReport } from '../../lib/client'

export function AutopilotSummaryBanner({
  report,
}: {
  report: Pick<AutopilotReport, 'checks_pass' | 'policy' | 'plan' | 'summary_line'>
}) {
  return (
    <div
      className={`ap-summary ${report.checks_pass ? 'ap-summary--pass' : 'ap-summary--warn'}`}
      data-cut-autopilot-summary
      data-cut-autopilot-pass={String(report.checks_pass)}
    >
      <span className="ap-summary-badge">{report.checks_pass
        ? <Icon name="check" size={16} tone="success" label="all checks pass" />
        : report.policy === 'preview' && report.plan.some((p) => p.auto_fixable)
          ? <Icon name="pending" size={16} tone="brand" label="preview" />
          : <Icon name="warning" size={16} tone="warn" label="needs attention" />}</span>
      <span>{report.summary_line}</span>
    </div>
  )
}
