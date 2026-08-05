import type { VerbResult } from '../lib/client'

export function trackAuditionExportError(result: VerbResult): string | null {
  if (!result.ok) return result.error?.message || 'Could not render this track for listening.'
  const path = (result.result as { path?: unknown } | undefined)?.path
  return typeof path === 'string' && path.length > 0
    ? null
    : 'Track audio export returned no playable file.'
}
