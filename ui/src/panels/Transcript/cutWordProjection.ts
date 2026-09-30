import type { WordSpan } from '../../lib/client'
import type { CutSpan } from '../Review/shared'
import { wordIndexRangeFrom } from '../Review/wordIndexRange'

/** Imported asset IDs are data, including names inherited by ordinary objects. */
export function cutSpansByAsset(cuts: CutSpan[]): Map<string, CutSpan[]> {
  const result = new Map<string, CutSpan[]>()
  for (const cut of cuts) {
    const group = result.get(cut.asset)
    if (group) group.push(cut)
    else result.set(cut.asset, [cut])
  }
  return result
}

/** Project onto loaded word identities, never enumerate an imported range. */
export function cutWordsByIndex(words: WordSpan[], cuts: CutSpan[]): Map<number, CutSpan> {
  const validCuts = cuts.filter((cut) => wordIndexRangeFrom(cut.wordRange) !== null)
  const result = new Map<number, CutSpan>()
  for (const word of words) {
    const cut = validCuts.find(({ wordRange: [start, end] }) => word.idx >= start && word.idx <= end)
    if (cut) result.set(word.idx, cut)
  }
  return result
}
