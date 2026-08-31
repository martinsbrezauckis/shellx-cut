import { useBlockingOverlay } from '../../components/overlay/useBlockingOverlay'

interface RecordingCountdownOverlayProps {
  remaining: number
  onCancel: () => void
}

/**
 * This is setup only: no capture id, backend reservation, or elapsed recording
 * time exists while it is visible.
 */
export function RecordingCountdownOverlay({ remaining, onCancel }: RecordingCountdownOverlayProps) {
  const overlay = useBlockingOverlay<HTMLElement>(onCancel)

  return (
    <div className="rec-countdown" data-cut-rec-countdown-overlay>
      <div className="rec-countdown__backdrop" aria-hidden="true" />
      <section
        ref={overlay.dialogRef}
        className="rec-countdown__surface"
        data-cut-blocking-overlay
        data-cut-overlay-part
        role="dialog"
        aria-modal="true"
        aria-labelledby="cut-rec-countdown-title"
        aria-describedby="cut-rec-countdown-description"
        tabIndex={-1}
        onKeyDown={overlay.onDialogKeyDown}
      >
        <p className="rec-countdown__label">Get ready</p>
        <strong id="cut-rec-countdown-title" className="rec-countdown__number" data-cut-rec-countdown-remaining={remaining}>
          {remaining}
        </strong>
        <p id="cut-rec-countdown-description" className="rec-countdown__description">
          Recording has not started yet.
        </p>
        <button
          type="button"
          className="rec__stop rec-countdown__cancel"
          data-cut-action="record-countdown-cancel"
          onClick={onCancel}
        >
          Cancel (Esc)
        </button>
      </section>
    </div>
  )
}
