export type MotionTrackingMode = 'point' | 'planar'
export type MotionTrackingModel = 'translation' | 'similarity' | 'homography'

export interface MotionTrackingRegionPercent {
  x: number
  y: number
  width: number
  height: number
}

export function trackingModelForMode(mode: MotionTrackingMode): MotionTrackingModel {
  return mode === 'point' ? 'translation' : 'homography'
}

export function defaultTrackingAnalysisId(clipId: string): string {
  const safe = clipId
    .replace(/[^A-Za-z0-9._-]+/g, '-')
    .replace(/^[^A-Za-z0-9]+/, '')
    .slice(0, 96)
  return `${safe || 'clip'}-track`
}

export function normalizedTrackingRegion(region: MotionTrackingRegionPercent): {
  x: number
  y: number
  width: number
  height: number
} | null {
  const values = [region.x, region.y, region.width, region.height]
  if (!values.every(Number.isFinite)) return null
  if (region.x < 0 || region.y < 0 || region.width <= 0 || region.height <= 0) return null
  if (region.x + region.width > 100 || region.y + region.height > 100) return null
  return {
    x: region.x / 100,
    y: region.y / 100,
    width: region.width / 100,
    height: region.height / 100,
  }
}

export function trackingVerificationLabel(verification: {
  attached?: boolean
  current?: boolean
  reasons?: string[]
}): string {
  if (verification.attached && verification.current) return 'Verified: stabilization and source are current.'
  const reason = verification.reasons?.find(Boolean)
  return reason ? `Needs attention: ${reason}` : 'Tracking verification did not pass.'
}
