export const LIBRARY_MEMBERSHIP_BATCH_SIZE = 500

/** Deduplicate exact library ids and split them to the public API's bounded cap. */
export function libraryMembershipBatches(
  ids: readonly string[],
  size = LIBRARY_MEMBERSHIP_BATCH_SIZE,
): string[][] {
  if (!Number.isInteger(size) || size < 1) throw new Error('membership batch size must be positive')
  const unique = Array.from(new Set(ids))
  const batches: string[][] = []
  for (let start = 0; start < unique.length; start += size) {
    batches.push(unique.slice(start, start + size))
  }
  return batches
}
