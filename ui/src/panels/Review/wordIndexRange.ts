/** Optional imported word annotations must address an ordered inclusive index range. */
export function wordIndexRangeFrom(value: unknown): [number, number] | null {
  if (!Array.isArray(value) || value.length !== 2) return null
  const [start, end] = value
  return Number.isSafeInteger(start) && Number.isSafeInteger(end) && start >= 0 && end >= start
    ? [start, end]
    : null
}
