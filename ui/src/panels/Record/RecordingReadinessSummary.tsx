import { recordCardLabel, type RecordCard } from './recordingUiModel'
import type { ScreenCheck } from './recordDoctorRead'

interface RecordingReadinessSummaryProps {
  readonly cards: readonly RecordCard[]
  readonly ready: boolean | null
  readonly startAllowed: boolean | null
  /** What the status read knows about screen capture without capturing. */
  readonly screenCheck?: ScreenCheck | null
  /** Run the full check, including the short screen capture, on request. */
  readonly onCheck?: () => void
}

function cardStatus(card: RecordCard): 'ok' | 'degraded' | 'unknown' | 'missing' {
  if (card.status === 'ok') return 'ok'
  if (card.status === 'degraded') return 'degraded'
  if (card.status === 'unknown') return 'unknown'
  return 'missing'
}

function readinessSummary(ready: boolean | null, startAllowed: boolean | null, screenCheck: ScreenCheck | null | undefined): { label: string; detail: string; state: string } {
  if (screenCheck === 'permission_needed') {
    return {
      label: 'Screen recording permission needed',
      detail: 'Turn on ShellX Cut in System Settings > Privacy & Security > Screen & System Audio Recording, then quit and reopen ShellX Cut. Check again afterwards.',
      state: 'attention',
    }
  }
  if (screenCheck === 'not_checked' && startAllowed === true) {
    return {
      label: 'Screen capture not checked yet',
      detail: 'ShellX Cut checks it when you start recording. Check now to see the result first; macOS may ask for screen access.',
      state: 'pending',
    }
  }
  if (startAllowed === false) {
    return {
      label: 'Capture setup needs attention',
      detail: 'Screen capture is not ready on this machine. Review the checks before recording.',
      state: 'attention',
    }
  }
  if (ready === false && startAllowed === true) {
    return {
      label: 'Source check remains',
      detail: 'Choose what to share in the system picker when you start.',
      state: 'pending',
    }
  }
  if (ready === true && startAllowed === true) {
    return {
      label: 'Capture checks passed',
      detail: 'Choose a source to continue.',
      state: 'ready',
    }
  }
  return {
    label: 'Checking capture setup',
    detail: 'An unknown status is not ready to record.',
    state: 'unknown',
  }
}

/** Compact outcome first; individual Doctor facts stay available without occupying setup flow. */
export function RecordingReadinessSummary({ cards, ready, startAllowed, screenCheck, onCheck }: RecordingReadinessSummaryProps) {
  const summary = readinessSummary(ready, startAllowed, screenCheck)
  const offerCheck = onCheck && (screenCheck === 'not_checked' || screenCheck === 'permission_needed')
  return (
    <section className="rec__readiness" data-cut-rec-cards data-cut-rec-readiness data-cut-rec-readiness-state={summary.state} data-cut-rec-screen-check={screenCheck ?? undefined}>
      <div className="rec__readiness-summary">
        <span className="rec__eyebrow">Capture readiness</span>
        <strong data-cut-rec-readiness-label>{summary.label}</strong>
        <p data-cut-rec-readiness-detail>{summary.detail}</p>
        {offerCheck && (
          <button type="button" className="rec__readiness-check" data-cut-action="record-capture-check" onClick={onCheck}>
            {screenCheck === 'permission_needed' ? 'Check again' : 'Check now'}
          </button>
        )}
      </div>
      <details className="rec__readiness-details" data-cut-rec-readiness-details>
        <summary data-cut-action="record-readiness-details-toggle">{cards.length} technical checks</summary>
        <div className="rec__cards">
          {cards.map((card) => (
            <div key={card.name} className={`rec__card rec__card--${cardStatus(card)}`} data-cut-rec-card={card.name} data-cut-rec-card-status={cardStatus(card)}>
              <span className="rec__card-name">{recordCardLabel(card.name)}</span>
              <span className="rec__card-detail">{card.detail}</span>
            </div>
          ))}
        </div>
      </details>
    </section>
  )
}
