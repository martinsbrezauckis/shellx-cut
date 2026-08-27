// External documentation is deliberately separate from the bundled manual.
// Contextual help stays in Cut unless a caller explicitly chooses this helper.
export const CUT_MANUAL_ONLINE_URL = 'https://docs.theshellx.com/manual/cut/'

export function externalCutManualFeatureUrl(featureId?: string): string {
  if (!featureId) return CUT_MANUAL_ONLINE_URL
  const url = new URL(CUT_MANUAL_ONLINE_URL)
  url.searchParams.set('feature', featureId)
  return url.toString()
}

/** Open the bundled read-only manual at an article. It never opens a browser. */
export function openCutManual(featureId?: string): void {
  document.dispatchEvent(new CustomEvent('cut:open-manual', {
    detail: featureId ? { feature: featureId } : undefined,
  }))
}

/** Use only for an explicit user choice to leave Cut for online documentation. */
export function openExternalCutManual(featureId?: string): void {
  window.open(externalCutManualFeatureUrl(featureId), '_blank', 'noopener,noreferrer')
}
