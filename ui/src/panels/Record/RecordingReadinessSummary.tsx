import { recordCardLabel, type RecordCard } from './recordingUiModel'

interface RecordingReadinessSummaryProps {
  readonly cards: readonly RecordCard[]
  readonly ready: boolean | null
  readonly startAllowed: boolean | null
}

function cardStatus(card: RecordCard): 'ok' | 'degraded' | 'unknown' | 'missing' {
  if (card.status === 'ok') return 'ok'
  if (card.status === 'degraded') return 'degraded'
  if (card.status === 'unknown') return 'unknown'
  return 'missing'
}

function readinessSummary(ready: boolean | null, startAllowed: boolean | null): { label: string; detail: string; state: string } {
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
export function RecordingReadinessSummary({ cards, ready, startAllowed }: RecordingReadinessSummaryProps) {
  const summary = readinessSummary(ready, startAllowed)
  const visibleCards = cards.filter((card) => card.name !== 'webcam')
  return (
    <section className="rec__readiness" data-cut-rec-cards data-cut-rec-readiness data-cut-rec-readiness-state={summary.state}>
      <div className="rec__readiness-summary">
        <span className="rec__eyebrow">Capture readiness</span>
        <strong data-cut-rec-readiness-label>{summary.label}</strong>
        <p data-cut-rec-readiness-detail>{summary.detail}</p>
      </div>
      <details className="rec__readiness-details" data-cut-rec-readiness-details>
        <summary data-cut-action="record-readiness-details-toggle">{visibleCards.length} technical checks</summary>
        <div className="rec__cards">
          {visibleCards.map((card) => (
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
