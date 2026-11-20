/** A missing microphone prerequisite as the engine reports it. */
export interface MicrophoneProblem {
  reason?: string
  message?: string
  suggested_action?: string
  detail?: string
}

function sentence(text: string): string {
  return text.charAt(0).toUpperCase() + text.slice(1)
}

/** What is missing and what to do about it, as one readable line. */
export function microphoneProblemText(problem: MicrophoneProblem | null | undefined): string | null {
  if (!problem) return null
  const parts = [problem.message, problem.suggested_action]
    .filter((part): part is string => Boolean(part))
    .map(sentence)
  return parts.length ? parts.join('. ') : null
}

/** The status line for a take that failed after it started. */
export function voiceoverFailureText(failure: MicrophoneProblem | null | undefined): string {
  const why = microphoneProblemText(failure)
  return why ? `Voiceover failed. ${why}` : 'Voiceover failed; its private recovery state is retained.'
}
