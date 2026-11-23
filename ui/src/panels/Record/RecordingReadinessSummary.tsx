import { useCallback, useRef, type KeyboardEvent } from 'react'
import { useDismissOnOutside } from './useDismissOnOutside'
import { recordingNotice, recordingCardNotice, recordCardLabel, recordingInputAdvice, recordingInputUnavailable, type RecordCard } from './recordingUiModel'
import { RECORDING_FRAME_RATE_MIN } from './recordingFrameRate'
import type { ScreenCheck } from './recordDoctorRead'

interface RecordingReadinessSummaryProps {
  readonly cards: readonly RecordCard[]
  readonly videoEncoder?: unknown
  readonly ready: boolean | null
  readonly startAllowed: boolean | null
  /** What the status read knows about screen capture without capturing. */
  readonly screenCheck?: ScreenCheck | null
  /** Run the full check, including the short screen capture, on request. */
  readonly onCheck?: () => void
  /** The engine's operating system from the environment report ('macos', 'windows', 'linux'); null until it is known. */
  readonly hostOs?: string | null
}

function cardStatus(card: RecordCard): 'ok' | 'degraded' | 'unknown' | 'missing' {
  // Doctor explicitly reports compiled support, not a startup check.
  if (card.status === 'ok' && /\bunchecked\b|\bnot checked\b/i.test(card.detail)) return 'unknown'
  if (card.status === 'ok') return 'ok'
  if (card.status === 'degraded') return 'degraded'
  if (card.status === 'unknown') return 'unknown'
  return 'missing'
}

/** The state as a word, so the small colour mark beside it is never the only signal. */
const CARD_STATE_WORDS: Record<ReturnType<typeof cardStatus>, string> = {
  ok: 'OK',
  degraded: 'Needs attention',
  unknown: 'Not checked',
  missing: 'Unavailable',
}

function closeChecksOnEscape(event: KeyboardEvent<HTMLDetailsElement>) {
  if (event.key !== 'Escape' || !event.currentTarget.open) return
  event.currentTarget.open = false
  event.currentTarget.querySelector('summary')?.focus()
}

/** What a screen capture that has not been checked yet says. Only macOS may ask for screen access at the check, so that clause is shown there alone. */
export function screenNotCheckedDetail(hostOs: string | null | undefined): string {
  const start = 'ShellX Cut checks it when you start recording. Check now to see the result first'
  return hostOs === 'macos' ? `${start}; macOS may ask for screen access.` : `${start}.`
}

function readinessSummary(cards: readonly RecordCard[], videoEncoder: unknown, ready: boolean | null, startAllowed: boolean | null, screenCheck: ScreenCheck | null | undefined, hostOs: string | null | undefined): { label: string; detail: string; state: string } {
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
      detail: screenNotCheckedDetail(hostOs),
      state: 'pending',
    }
  }
  if (startAllowed === false) {
    const encoderCard = cards.find(card => card.name === 'video_encoder' && card.status === 'degraded')
    if (encoderCard?.detail.trim()) {
      const encoder = videoEncoder !== null && typeof videoEncoder === 'object'
        ? videoEncoder as Record<string, unknown> : null
      // Doctor checks the exact source/rate. Only its settings-specific refusal
      // permits a lower-rate retry; it does not promise that retry will work.
      const retry = encoder?.reason === 'video_encoder_unavailable' && encoder.ready === false
        && encoder.checked === true && typeof encoder.fps === 'number' && Number.isFinite(encoder.fps)
        && encoder.fps > RECORDING_FRAME_RATE_MIN
      return {
        label: recordingNotice(encoderCard.detail, 'Video recording is unavailable at these settings.'),
        detail: retry ? 'Try a lower frame rate, then check again.'
          : encoder?.reason === 'video_encoder_unavailable'
            ? "This PC's graphics adapter cannot record at these settings."
            : '',
        state: 'attention',
      }
    }
    return {
      label: 'Capture setup needs attention',
      detail: 'Screen capture is not ready on this machine. Review the checks before recording.',
      state: 'attention',
    }
  }
  if (ready === false && startAllowed === true) {
    return {
      label: 'Choose a screen to record',
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
export function RecordingReadinessSummary({ cards, videoEncoder, ready, startAllowed, screenCheck, onCheck, hostOs = null }: RecordingReadinessSummaryProps) {
  const inputUnavailable = recordingInputUnavailable(cards)
  const summary = readinessSummary(cards, videoEncoder, ready, startAllowed, screenCheck, hostOs)
  const encoderBlocked = startAllowed === false && cards.some(card => card.name === 'video_encoder' && card.status === 'degraded')
  const offerCheck = onCheck && (screenCheck === 'not_checked' || screenCheck === 'permission_needed')
  // The list is a native <details>: a click or focus move outside it closes it, as every other drop-down does.
  const detailsRef = useRef<HTMLDetailsElement>(null)
  const checksOpen = useCallback(() => detailsRef.current?.open === true, [])
  const closeChecks = useCallback(() => { if (detailsRef.current) detailsRef.current.open = false }, [])
  useDismissOnOutside(detailsRef, checksOpen, closeChecks)
  return (
    <section className="rec__readiness" data-cut-rec-cards data-cut-rec-encoder-blocked={encoderBlocked || undefined} data-cut-rec-readiness data-cut-rec-readiness-state={summary.state} data-cut-rec-screen-check={screenCheck ?? undefined}>
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
      {inputUnavailable !== null && (
        <p className="rec__source-note rec__source-note--unavailable" data-cut-rec-input-unavailable role="status">
          Show keystrokes, click highlights and click-based automatic zoom are unavailable. Scroll and input activity data will also be absent.
          {' '}You can start a plain recording with screen and sound. Turn off Show keystrokes if it was selected.
          {' '}{recordingInputAdvice(inputUnavailable)}
        </p>
      )}
      <details ref={detailsRef} className="rec__readiness-details" data-cut-rec-readiness-details onKeyDown={closeChecksOnEscape}>
        <summary data-cut-action="record-readiness-details-toggle">{cards.length} technical checks</summary>
        <div className="rec__cards" role="list">
          {cards.map((card) => {
            const status = cardStatus(card)
            return (
              <div key={card.name} className={`rec__card rec__card--${status}`} data-cut-rec-card={card.name} data-cut-rec-card-status={status} role="listitem">
                <span className="rec__card-name">{recordCardLabel(card.name)}</span>
                <span className="rec__card-state"><span className="rec__card-mark" aria-hidden="true" />{CARD_STATE_WORDS[status]}</span>
                <span className="rec__card-detail">{recordingCardNotice(card, status === 'unknown' ? 'Not checked yet. Check when you start recording.' : status === 'ok' ? 'Available for recording.' : 'Unavailable with the current setup. Check your devices and system permissions, then check again.')}</span>
              </div>
            )
          })}
        </div>
      </details>
    </section>
  )
}
