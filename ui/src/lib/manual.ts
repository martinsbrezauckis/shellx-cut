// Canonical ShellX Cut user manual URL. Keep this in-app pointer aligned with
// the docs site so shipped builds can open the current web manual.
export const CUT_MANUAL_URL = 'https://docs.theshellx.com/manual/cut/'

export function cutManualFeatureUrl(featureId?: string): string {
  if (!featureId) return CUT_MANUAL_URL
  const url = new URL(CUT_MANUAL_URL)
  url.searchParams.set('feature', featureId)
  return url.toString()
}

export function openCutManual(featureId?: string): void {
  window.open(cutManualFeatureUrl(featureId), '_blank', 'noopener,noreferrer')
}
