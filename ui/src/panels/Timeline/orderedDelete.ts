/** Cumulative editorial positions move after a ripple delete on the same track. */
export interface TimelineDeleteRange {
  track: string
  start: number
  dur: number
  id: string
}

export async function runOrderedTimelineDeletes(
  ranges: TimelineDeleteRange[],
  ripple: boolean,
  dispatch: (range: TimelineDeleteRange) => Promise<boolean>,
): Promise<boolean> {
  // Descending positions remain valid as earlier material shifts left. Keep the
  // original order at equal positions so linked tracks stay adjacent in history.
  const ordered = ripple
    ? ranges.map((range, index) => ({ range, index }))
      .sort((a, b) => b.range.start - a.range.start || a.index - b.index)
      .map(({ range }) => range)
    : ranges
  for (const range of ordered) {
    if (!await dispatch(range)) return false
  }
  return true
}
