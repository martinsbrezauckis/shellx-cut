/**
 * Recorder setup countdown state is deliberately separate from a live capture.
 * A cancelled generation can never become a later `screen_record.start` call.
 */
export class RecordingCountdownGuard {
  private generation = 0
  private handedOffGeneration: number | null = null

  begin(): number {
    this.generation += 1
    this.handedOffGeneration = null
    return this.generation
  }

  cancel(): void {
    this.generation += 1
    this.handedOffGeneration = null
  }

  isCurrent(generation: number): boolean {
    return generation === this.generation
  }

  /** Claims the one legal zero handoff for this still-current setup timer. */
  handoff(generation: number, onStart: () => void): boolean {
    if (!this.isCurrent(generation) || this.handedOffGeneration === generation) return false
    this.handedOffGeneration = generation
    onStart()
    return true
  }
}

/** Visible whole seconds; zero is the single handoff point to capture start. */
export function countdownRemainingSeconds(deadlineMs: number, nowMs = Date.now()): number {
  return Math.max(0, Math.ceil((deadlineMs - nowMs) / 1000))
}

export const RECORDING_COUNTDOWN_CHOICES = [0, 3, 5] as const
export type RecordingCountdownSeconds = (typeof RECORDING_COUNTDOWN_CHOICES)[number]
