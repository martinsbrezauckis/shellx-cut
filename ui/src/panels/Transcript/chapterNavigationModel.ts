import type { SourceTimelineOccurrence } from '../Timeline/sourceMapping'

export interface TranscriptChapter {
  start_ms: number
  end_ms?: number
  title?: string
}

export type ChapterAssetAvailability = 'checking' | 'available' | 'offline' | 'unknown'

export type ChapterNavigationDisposition =
  | { kind: 'checking'; message: string }
  | { kind: 'offline'; message: string }
  | { kind: 'unavailable'; message: string }
  | { kind: 'edited-out'; message: string }
  | { kind: 'direct'; occurrence: SourceTimelineOccurrence }
  | { kind: 'choose'; occurrences: readonly SourceTimelineOccurrence[] }

/** The compact Chapter rail pages instead of mounting every generated chapter. */
export const CHAPTER_RENDER_LIMIT = 12
// The engine accepts up to 50 generated chapters. Ask for more than one
// rendered page so the Chapter page control can expose long transcripts.
export const TRANSCRIPT_CHAPTER_REQUEST_LIMIT = 50

/**
 * Make a navigation decision from the already-authoritative occurrence route.
 * This deliberately stores no translated time: a single occurrence can seek;
 * reused source media remains a human choice; no exact video occurrence refuses.
 */
export function chapterNavigationDisposition(
  availability: ChapterAssetAvailability,
  occurrences: readonly SourceTimelineOccurrence[],
): ChapterNavigationDisposition {
  if (availability === 'checking') {
    return { kind: 'checking', message: 'Checking source availability…' }
  }
  if (availability === 'offline') {
    return { kind: 'offline', message: 'Source media is offline; seek is unavailable.' }
  }
  if (availability === 'unknown') {
    return { kind: 'unavailable', message: 'Source availability could not be verified; seek is disabled.' }
  }
  if (occurrences.length === 0) {
    return {
      kind: 'edited-out',
      message: 'No exact occurrence is available in the current video edit.',
    }
  }
  if (occurrences.length === 1) return { kind: 'direct', occurrence: occurrences[0] }
  return { kind: 'choose', occurrences }
}

export function boundedChapterWindow<T>(items: readonly T[], start = 0): { start: number; items: readonly T[] } {
  const maxStart = Math.max(0, items.length - CHAPTER_RENDER_LIMIT)
  const safeStart = Math.max(0, Math.min(start, maxStart))
  return { start: safeStart, items: items.slice(safeStart, safeStart + CHAPTER_RENDER_LIMIT) }
}

export function chapterWindowStarts(length: number, activeStart = 0): number[] {
  if (length <= CHAPTER_RENDER_LIMIT) return [0]
  const finalStart = Math.max(0, length - CHAPTER_RENDER_LIMIT)
  const starts = Array.from({ length: Math.ceil(finalStart / CHAPTER_RENDER_LIMIT) + 1 }, (_, index) => (
    Math.min(index * CHAPTER_RENDER_LIMIT, finalStart)
  ))
  return starts.includes(activeStart) ? starts : [...starts, activeStart].sort((a, b) => a - b)
}
