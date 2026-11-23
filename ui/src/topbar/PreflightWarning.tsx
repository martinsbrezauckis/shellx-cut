import type { PregateReport, PregateRisk } from '../lib/client'
import { openCutManual } from '../lib/manual'

interface PreflightWarningProps {
  report: PregateReport
  actionLabel: string
  onCancel: () => void
  onContinue: () => void
  overModal?: boolean
}

const severityLabel = (severity: PregateRisk['severity']) => {
  if (severity === 'high') return 'Warning'
  if (severity === 'med') return 'Review'
  return 'Note'
}

const RISK_COPY: Record<string, { title: string; guidance: string; guide: string }> = {
  empty_tail: {
    title: 'Black ending',
    guidance: 'The timeline continues after the last picture. In the timeline, trim the ending, shorten the sound, or add picture. You can export anyway if the black ending is intentional.',
    guide: 'cut.export.preflight.black_tail',
  },
  black_or_frozen: {
    title: 'Black or frozen footage',
    guidance: 'A clip contains black or still frames. A still picture can be intentional in a screen recording. In the timeline, trim or replace unwanted footage. You can export anyway if the still picture is intentional.',
    guide: 'cut.export.preflight.dead_frames',
  },
  slideshow_risk: {
    title: 'Long holds',
    guidance: 'The main video track has very few cuts. In the timeline, add cuts or movement, or shorten shots if the long holds are not intentional.',
    guide: 'cut.export.preflight.pacing',
  },
  silent_output: {
    title: 'Silent export',
    guidance: 'The timeline appears silent. In the timeline, add sound or unmute a sound track. You can export anyway if this is meant to be silent.',
    guide: 'cut.export.preflight.silent_audio',
  },
  tiny_or_zero_clips: {
    title: 'Tiny clip',
    guidance: 'One or more clips are shorter than a single frame. In the timeline, delete or lengthen them if they should be visible.',
    guide: 'cut.export.preflight.tiny_clips',
  },
  uniform_border: {
    title: 'Black border',
    guidance: 'A source file has black bands around the picture. Select the clip in the timeline and crop it in the Inspector if you do not want the bands.',
    guide: 'cut.export.preflight.borders',
  },
}

const RESOURCE_COPY: Record<string, { title: string; guidance: string }> = {
  memory_short: { title: 'Not enough free memory', guidance: 'This export may run slowly or stop because little memory is free. Close other apps, or reduce the picture size in Export settings.' },
  out_of_memory: { title: 'Too large for this computer', guidance: 'This export is estimated to need more memory than this computer has. Reduce the picture size in Export settings, or shorten the timeline.' },
  disk_short: { title: 'Output drive almost full', guidance: 'The export may run out of space. Free space on the drive containing the export folder, or choose another folder in Export settings.' },
  not_enough_space: { title: 'Not enough free space', guidance: 'The export is estimated to need more space than is available. Free space on the drive containing the export folder, or choose another folder in Export settings.' },
}

const riskCopyFor = (risk: PregateRisk, incomplete = false) => {
  if (risk.kind === 'checks_unavailable') return {
    title: 'Export checks could not run',
    guidance: risk.detail || 'Picture and sound were not checked because no check results were returned. Review the timeline and check the captions tools in Settings > AI & transcription.',
    guide: 'cut.export.preflight',
  }
  if (risk.kind === 'silent_output' && incomplete) return {
    title: 'Sound not fully checked',
    guidance: 'Sound was not checked for every file because some files have no saved analysis results. Listen to the timeline and check that the sound tracks are not muted before exporting.',
    guide: 'cut.export.preflight.silent_audio',
  }
  return RISK_COPY[risk.kind] ?? {
    ...(RESOURCE_COPY[risk.kind] ?? {
      title: 'Export warning',
      guidance: 'This check could not be explained. Review the picture and sound in the timeline before exporting; see Guide for export checks.',
    }),
    guide: 'cut.export.preflight',
  }
}

// Resource warnings currently carry their measurements in the sentence only.
// Keep those amounts without repeating command names or processing terms.
const resourceMeasurements = (risk: PregateRisk) => {
  if (!RESOURCE_COPY[risk.kind]) return ''
  const amounts = risk.detail?.match(/\d+(?:\.\d+)? GB/g) ?? []
  if (amounts.length < 2) return ''
  const memory = risk.kind === 'memory_short' || risk.kind === 'out_of_memory'
  return ` Estimated ${memory ? 'memory' : 'space'} needed: ${amounts[0]}. ${risk.kind === 'out_of_memory' ? 'Computer memory' : 'Available'}: ${amounts[1]}.${amounts[2] ? ` ${memory ? 'Memory including extra space on disk' : 'Space to free'}: ${amounts[2]}.` : ''}`
}

const formatRange = (risk: PregateRisk) => {
  const range = risk.range_ms
  if (!range) return ''
  const start = Math.max(0, Math.round(range[0] / 100) / 10)
  const end = Math.max(start, Math.round(range[1] / 100) / 10)
  return ` (${start}s-${end}s)`
}

export default function PreflightWarning({ report, actionLabel, onCancel, onContinue, overModal = false }: PreflightWarningProps) {
  const risks = report.risks ?? []
  const uninstrumented = report.uninstrumented_assets ?? []
  const continueLabel = actionLabel === 'rendering queued deliveries' ? 'Queue anyway' : 'Export anyway'
  const guideFeature = risks.map((risk) => riskCopyFor(risk).guide).find(Boolean) ?? 'cut.export.preflight'
  // Pregate predicts quality; actual tool, media, and output failures remain
  // enforced by the export action. A user can deliberately keep a still screen.
  const summary = `Review these warnings and checks that could not be completed before ${actionLabel}. You can cancel to make changes or choose to continue anyway.`

  return (
    <section
      className={`tb-pregate${overModal ? ' tb-pregate--over-modal' : ''}`}
      data-cut-pregate-warning
      data-cut-pregate-blocked="false"
      role="dialog"
      aria-modal="false"
      aria-label="Export checks"
    >
      <div className="tb-pregate-head">
        <span className="tb-pregate-kicker">Export checks</span>
        <button type="button" className="tb-pregate-close" data-cut-pregate-close onClick={onCancel} aria-label="Close export checks">x</button>
      </div>
      <h2>Review before export</h2>
      <p>{summary}</p>
      {(risks.length > 0 || uninstrumented.length > 0) && (
        <ul className="tb-pregate-risks">
          {risks.map((risk, index) => (
            <li key={`${risk.kind}-${index}`} data-cut-pregate-risk data-cut-pregate-risk-kind={risk.kind} data-severity={risk.severity}>
              <span className="tb-pregate-sev">{severityLabel(risk.severity)}</span>
              <span>
                <b>{riskCopyFor(risk, uninstrumented.length > 0).title}</b>
                {formatRange(risk)}
                <small>{riskCopyFor(risk, uninstrumented.length > 0).guidance}</small>
              </span>
            </li>
          ))}
          {uninstrumented.length > 0 && (
            <li data-cut-pregate-risk data-severity="low">
              <span className="tb-pregate-sev">Note</span>
              <span>Black or frozen picture, sound levels and black borders were not checked for {uninstrumented.length} file{uninstrumented.length === 1 ? '' : 's'} because there are no saved analysis results. Review these files in the timeline before exporting. If analysis is unavailable, check the captions tools in Settings &gt; AI &amp; transcription.</span>
            </li>
          )}
        </ul>
      )}
      <details className="tb-pregate-details" data-cut-pregate-details>
        <summary data-cut-pregate-details-toggle>Details</summary>
        <dl>
          <div>
            <dt>Files with saved analysis</dt>
            <dd>{report.perception_assets ?? 0}</dd>
          </div>
          <div>
            <dt>Files not checked</dt>
            <dd>{uninstrumented.length}</dd>
          </div>
          {risks.length > 0 && (
            <div>
              <dt>Check details</dt>
              <dd>
                <ul className="tb-pregate-detail-list">
                  {risks.map((risk, index) => (
                    <li key={`${risk.kind}-detail-${index}`}>
                      <b>{riskCopyFor(risk, uninstrumented.length > 0).title}</b>{formatRange(risk)} — {riskCopyFor(risk, uninstrumented.length > 0).guidance}{resourceMeasurements(risk)}
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
          Cancel
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
          onClick={onContinue}
          title={continueLabel}
        >
          {continueLabel}
        </button>
      </div>
    </section>
  )
}
