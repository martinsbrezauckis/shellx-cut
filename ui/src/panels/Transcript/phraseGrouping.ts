import type { TimelineWord } from '../../lib/client'

/** Phrases are presentation-only views over the already authoritative EDL words. */
export interface TranscriptPhrase {
  id: string
  asset: string
  clipId: string | null
  track: string
  speaker?: string
  startMs: number
  endMs: number
  words: TimelineWord[]
}

export const TRANSCRIPT_PHRASE_MAX_GAP_MS = 900
export const TRANSCRIPT_PHRASE_RENDER_LIMIT = 120

const terminalPunctuation = /[.!?…]["')\]]*$/u

function sameOccurrence(a: TimelineWord, b: TimelineWord): boolean {
  return a.asset === b.asset && a.clip_id === b.clip_id && a.track === b.track
}

function sameSpeaker(a: TimelineWord, b: TimelineWord): boolean {
  return (a.speaker ?? null) === (b.speaker ?? null)
}

function phraseId(first: TimelineWord, ordinal: number): string {
  return [first.asset, first.clip_id ?? 'none', first.track, first.word_index, ordinal].join(':')
}

/**
 * Group words with only deterministic editorial boundaries: an EDL occurrence,
 * a diarized speaker, terminal punctuation, or a bounded silent gap. It never
 * invents text, timing, or speaker identity.
 */
export function groupTimelinePhrases(
  entries: TimelineWord[],
  maxGapMs = TRANSCRIPT_PHRASE_MAX_GAP_MS,
): TranscriptPhrase[] {
  const phrases: TranscriptPhrase[] = []
  let current: TimelineWord[] = []

  const finish = () => {
    const first = current[0]
    const last = current.at(-1)
    if (!first || !last) return
    phrases.push({
      id: phraseId(first, phrases.length),
      asset: first.asset,
      clipId: first.clip_id,
      track: first.track,
      ...(first.speaker ? { speaker: first.speaker } : {}),
      startMs: first.timeline_start_ms,
      endMs: last.timeline_end_ms,
      words: current,
    })
    current = []
  }

  for (const word of entries) {
    const previous = current.at(-1)
    const gap = previous ? word.timeline_start_ms - previous.timeline_end_ms : 0
    const mustBreak = previous && (
      !sameOccurrence(previous, word)
      || !sameSpeaker(previous, word)
      || terminalPunctuation.test(previous.word.trim())
      || gap > maxGapMs
      || gap < 0
    )
    if (mustBreak) finish()
    current.push(word)
  }
  finish()
  return phrases
}

export function activePhraseIndex(phrases: TranscriptPhrase[], playheadMs: number): number {
  for (let index = 0; index < phrases.length; index++) {
    const phrase = phrases[index]
    if (phrase.startMs <= playheadMs && playheadMs < phrase.endMs) return index
  }
  return -1
}

/** Timeline navigation always uses the phrase's EDL occurrence, never source time. */
export function phraseTimelineSeek(phrase: TranscriptPhrase): number {
  return phrase.startMs
}

/** Keep the DOM window bounded even when a long edit has thousands of phrases. */
export function boundedPhraseWindow(
  phrases: TranscriptPhrase[],
  requestedStart: number,
  limit = TRANSCRIPT_PHRASE_RENDER_LIMIT,
): { start: number; phrases: TranscriptPhrase[] } {
  const safeLimit = Math.max(1, Math.round(limit))
  const lastStart = Math.max(0, phrases.length - safeLimit)
  const start = Math.max(0, Math.min(lastStart, Math.round(requestedStart)))
  return { start, phrases: phrases.slice(start, start + safeLimit) }
}

/** Offer stable pages while preserving the exact playback-follow window. */
export function phraseWindowStarts(
  phraseCount: number,
  currentStart: number,
  limit = TRANSCRIPT_PHRASE_RENDER_LIMIT,
): number[] {
  const safeLimit = Math.max(1, Math.round(limit))
  const lastStart = Math.max(0, Math.round(phraseCount) - safeLimit)
  const boundedCurrent = Math.max(0, Math.min(lastStart, Math.round(currentStart)))
  const starts = [0, boundedCurrent]
  for (let start = safeLimit; start < lastStart; start += safeLimit) starts.push(start)
  starts.push(lastStart)
  return [...new Set(starts)].sort((left, right) => left - right)
}

export function formatTranscriptTime(ms: number): string {
  const total = Math.max(0, Math.round(ms))
  const minutes = Math.floor(total / 60_000)
  const seconds = Math.floor((total % 60_000) / 1_000)
  const milliseconds = total % 1_000
  return `${minutes}:${String(seconds).padStart(2, '0')}.${String(milliseconds).padStart(3, '0')}`
}
