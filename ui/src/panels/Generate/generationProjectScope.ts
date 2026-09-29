import type { Project } from '../../lib/client'

export interface StoredGenerationJob {
  job_id: string
  project_key: string
  retry_placement?: { mode: 'replace'; target_clip: string }
}

const pageNonce = Math.random().toString(36).slice(2)
const STORAGE_PREFIX = 'cut.generate.active-job:'

export function generationProjectKey(project: Project | null, projectScope: number, nonce = pageNonce): string | null {
  if (!project) return null
  const identity = project.project_identity
  return identity
    ? `${identity.origin_path_sha256}\u0000${identity.project_name}`
    : `session:${nonce}:${projectScope}`
}

export function generationJobStorageKey(projectKey: string): string {
  return `${STORAGE_PREFIX}${encodeURIComponent(projectKey)}`
}

export function generationHistoryMatchesScope(historyOwnerScope: number | null, currentScope: number): boolean {
  return historyOwnerScope === currentScope
}

export function readStoredGenerationJob(value: string | null, projectKey: string): StoredGenerationJob | null {
  if (!value) return null
  try {
    const parsed = JSON.parse(value) as Partial<StoredGenerationJob>
    if (parsed.project_key !== projectKey || typeof parsed.job_id !== 'string' || !parsed.job_id) return null
    return {
      job_id: parsed.job_id,
      project_key: projectKey,
      retry_placement: parsed.retry_placement?.mode === 'replace'
        && typeof parsed.retry_placement.target_clip === 'string'
        ? parsed.retry_placement
        : undefined,
    }
  } catch {
    return null
  }
}
