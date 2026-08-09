import type { Reviewed } from './shared'

const REVIEWED_STORAGE_PREFIX = 'shellx-cut:reviewed:'
export const REVIEW_MARKERS_EVENT = 'cut:review-markers'

export interface ReviewMarkersDetail {
  projectName: string
  opIds: string[]
  verdict: Reviewed[string]
}

export function reviewedStorageKey(projectName: string | null | undefined): string {
  return `${REVIEWED_STORAGE_PREFIX}${projectName ?? 'no-project'}`
}

export function loadReviewMarkers(projectName: string | null | undefined): Reviewed {
  try {
    const raw = localStorage.getItem(reviewedStorageKey(projectName))
    const parsed = raw ? JSON.parse(raw) : {}
    return parsed && typeof parsed === 'object' && !Array.isArray(parsed) ? parsed as Reviewed : {}
  } catch {
    return {}
  }
}

export function saveReviewMarkers(projectName: string, reviewed: Reviewed): void {
  try {
    localStorage.setItem(reviewedStorageKey(projectName), JSON.stringify(reviewed))
  } catch {
    // Hardened/private webviews may disable storage. The live event still keeps
    // the mounted Review rail consistent for this session.
  }
}

export function markReviewOps(projectName: string, opIds: string[], verdict: Reviewed[string]): Reviewed {
  const next = { ...loadReviewMarkers(projectName) }
  for (const opId of opIds) next[opId] = verdict
  saveReviewMarkers(projectName, next)
  document.dispatchEvent(new CustomEvent<ReviewMarkersDetail>(REVIEW_MARKERS_EVENT, {
    detail: { projectName, opIds, verdict },
  }))
  return next
}
