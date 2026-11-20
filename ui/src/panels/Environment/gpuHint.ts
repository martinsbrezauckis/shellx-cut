// panels/Environment/gpuHint.ts — what the faster-exports card says while no
// hardware encoder works. Pure.
// Callers: EnvCardRow.tsx. Deps: none.

/** The line under the card when exports run in software. A cause the person can
 *  act on (an old graphics driver, no graphics card, an encoder that would not
 *  start) is named with what to do; the generic line stays for an ffmpeg build
 *  that simply has no hardware encoder. */
export function softwareExportHint(details: Record<string, unknown> | null | undefined): string {
  const reason = details?.hardware_reason
  const reasonText = details?.hardware_reason_text
  const action = details?.enable_help
  if (
    (reason === 'driver_too_old' || reason === 'cannot_open' || reason === 'no_gpu') &&
    typeof reasonText === 'string' &&
    typeof action === 'string'
  ) {
    return `Software export works. ${reasonText} ${action}`
  }
  return 'Software export works. Add GPU support only if you need faster renders.'
}

/** Advanced rows naming the largest delivery size each hardware encoder took
 *  (doctor `size_limits`), e.g. ['Largest size, h264_nvenc', '4096x2160 (not 7680x4320)'].
 *  A render above that size uses software encoding and its plan says why. */
export function encoderSizeRows(details: Record<string, unknown> | null | undefined): Array<[string, string]> {
  const limits = details?.size_limits
  if (!Array.isArray(limits)) return []
  const rows: Array<[string, string]> = []
  for (const limit of limits) {
    if (typeof limit !== 'object' || limit === null) continue
    const encoder = (limit as Record<string, unknown>).encoder
    const largest = (limit as Record<string, unknown>).largest_accepted
    const refused = (limit as Record<string, unknown>).smallest_refused
    if (typeof encoder !== 'string') continue
    const max = typeof largest === 'string' ? largest : 'below 3840x2160'
    rows.push([`Largest size, ${encoder}`, typeof refused === 'string' ? `${max} (not ${refused})` : max])
  }
  return rows
}
