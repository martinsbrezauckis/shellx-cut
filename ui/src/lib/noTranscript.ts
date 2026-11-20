// "No transcript yet" is a normal state, not a read error. Verbs that work from
// the spoken words refuse, with a structured `error.requirement`, when nothing is
// transcribed: `captions_runtime` (the captions tools are not installed) or
// `transcript` (they are, but this footage was not transcribed). The panels read
// that structure, never the message text, and say the true thing in plain words.

import type { CutError } from './clientModel'

export type NoTranscriptKind = 'captions_missing' | 'not_transcribed'

/** Which "no transcript yet" refusal this is, or null for any other error. */
export function noTranscriptKind(error: Pick<CutError, 'requirement'> | null | undefined): NoTranscriptKind | null {
  const id = error?.requirement?.id
  if (id === 'captions_runtime') return 'captions_missing'
  if (id === 'transcript') return 'not_transcribed'
  return null
}

/** What a person reads about it (plain words; never a verb name: a person cannot run one). */
export function noTranscriptSentence(kind: NoTranscriptKind): string {
  return kind === 'captions_missing'
    ? 'Captions and transcription are not installed. Choose Install captions (in the Transcript panel or Settings) to make transcripts.'
    : 'No transcript yet. One is made after a clip is imported.'
}

/** One line for a failed action: the real reason when it is "no transcript yet", else the engine's message. */
export function transcriptActionFailure(label: string, error: CutError | undefined, fallback = 'error'): string {
  const kind = noTranscriptKind(error)
  if (kind) return `${label}: ${noTranscriptSentence(kind)}`
  return `${label}: ${error?.message ?? fallback}`
}

/** What the Transcript pane shows while no asset has a transcript. */
export type TranscriptEmptyState = 'checking' | 'install' | 'transcribing' | 'empty'

export function transcriptEmptyState(input: {
  /** null while the doctor has not answered yet. */
  captionsReady: boolean | null
  /** An import or transcription job is running right now. */
  transcribingNow: boolean
}): TranscriptEmptyState {
  if (input.captionsReady === null) return 'checking'
  if (input.captionsReady === false) return 'install'
  return input.transcribingNow ? 'transcribing' : 'empty'
}
