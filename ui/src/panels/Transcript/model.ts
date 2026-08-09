import type { TimelineWord, WordSpan } from '../../lib/client'

export type Aggressiveness = 'calm' | 'natural' | 'jumpy'

export function timelineEntriesFrom(v: unknown): TimelineWord[] | null {
  if (!isObject(v)) return null
  const entries = Reflect.get(v, 'entries')
  return Array.isArray(entries) ? entries : null
}

export function searchResultFrom(v: unknown): { match_count: number; matches: { at_ms: number; text: string }[] } | null {
  if (!isObject(v)) return null
  const matchCount = Reflect.get(v, 'match_count')
  const matchesValue = Reflect.get(v, 'matches')
  if (typeof matchCount !== 'number' || !Array.isArray(matchesValue)) return null
  const matches: { at_ms: number; text: string }[] = []
  for (const item of matchesValue) {
    if (!isObject(item)) continue
    const atMs = Reflect.get(item, 'at_ms')
    const text = Reflect.get(item, 'text')
    if (typeof atMs === 'number' && typeof text === 'string') matches.push({ at_ms: atMs, text })
  }
  return { match_count: matchCount, matches }
}

export function isObject(v: unknown): v is object {
  return v !== null && typeof v === 'object'
}

export function numberField(v: object, name: string): number | undefined {
  const value = Reflect.get(v, name)
  return typeof value === 'number' ? value : undefined
}

export function chaptersOf(v: unknown): { start_ms: number; title?: string }[] {
  if (!isObject(v)) return []
  const value = Reflect.get(v, 'chapters')
  if (!Array.isArray(value)) return []
  const chapters: { start_ms: number; title?: string }[] = []
  for (const ch of value) {
    if (!isObject(ch)) continue
    const startMs = Reflect.get(ch, 'start_ms')
    const title = Reflect.get(ch, 'title')
    if (typeof startMs === 'number') chapters.push({ start_ms: startMs, title: typeof title === 'string' ? title : undefined })
  }
  return chapters
}

export interface Sel {
  asset: string
  anchor: number
  head: number
}

export const selRange = (s: Sel): [number, number] => [Math.min(s.anchor, s.head), Math.max(s.anchor, s.head)]

export interface ReelSpan {
  asset: string
  range: [number, number]
  snippet: string
}

export function reelSnippet(words: WordSpan[], lo: number, hi: number): string {
  const slice = words.slice(lo, hi + 1).map((w) => w.word)
  if (slice.length <= 6) return slice.join(' ')
  return `${slice.slice(0, 3).join(' ')} … ${slice.slice(-2).join(' ')}`
}
