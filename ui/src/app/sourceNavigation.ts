// app/sourceNavigation.ts — identity-only routing into existing media surfaces.
//
// Timeline and Source Monitor occurrences dispatch one small UI event instead
// of reaching into Assets or Library directly. The app shell owns the layout
// switch and passes the selected registered asset identity to the mounted
// surface; no source path crosses this boundary.

export type SourceNavigationDestination = 'project' | 'library'

export interface SourceNavigationRequest {
  assetId: string
  destination: SourceNavigationDestination
}

export const sourceNavigationEvent = 'cut:reveal-registered-source'

export function sourceNavigationRequest(value: unknown): SourceNavigationRequest | null {
  if (!value || typeof value !== 'object') return null
  const candidate = value as Partial<SourceNavigationRequest>
  if (typeof candidate.assetId !== 'string' || !candidate.assetId.trim()) return null
  if (candidate.destination !== 'project' && candidate.destination !== 'library') return null
  return { assetId: candidate.assetId.trim(), destination: candidate.destination }
}

/** Request an internal reveal by registered asset identity. Safe in tests/SSR. */
export function requestSourceNavigation(destination: SourceNavigationDestination, assetId: string): void {
  const request = sourceNavigationRequest({ destination, assetId })
  if (!request || typeof document === 'undefined') return
  document.dispatchEvent(new CustomEvent<SourceNavigationRequest>(sourceNavigationEvent, { detail: request }))
}
