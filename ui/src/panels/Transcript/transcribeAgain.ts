/** Decide whether a direct transcription may start for the current asset. */
export function planTranscription(
  asset: string | null,
  hasTranscript: boolean,
  confirmReplace: () => boolean,
): string | null {
  if (!asset) return null
  if (hasTranscript && !confirmReplace()) return null
  return asset
}
