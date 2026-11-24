// One quiet line for the export panel and the finished file.
import type { ExportCheckPhase, ExportChecks } from '../../lib/exportChecks'
import { CheckMark } from './ExportCheckList'
import { summarizeExportChecks } from './model'
import './exportChecks.css'

export interface ExportCheckSummaryLineProps {
  checks: ExportChecks | null | undefined
  phase?: ExportCheckPhase
}

export default function ExportCheckSummaryLine({ checks, phase }: ExportCheckSummaryLineProps) {
  const summary = summarizeExportChecks(checks, phase)
  if (!summary) return null
  return (
    <div className="ecs" data-cut-export-summary data-mark={summary.mark}>
      <CheckMark mark={summary.mark} />
      <span className="ecs-text">{summary.text}</span>
    </div>
  )
}
