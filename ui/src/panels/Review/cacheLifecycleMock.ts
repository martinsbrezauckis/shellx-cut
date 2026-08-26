const CACHE_JOB_ID = 'job_mock_cache_purge'
let cancelled = false

/** Deterministic UI-only fixture for the native action sweep. */
export function handleCacheLifecycleMock(
  name: string,
  args: Record<string, unknown>,
): unknown | undefined {
  switch (name) {
    case 'project.cache_preview':
      return {
        ok: true,
        result: {
          schema: 'shellx-cut/cache-purge-preview/1',
          status: 'ready',
          plan_id: 'cache_plan_1',
          minimum_age_ms: 86_400_000,
          inventory: { files: 1, bytes: 4096 },
          purgeable: { files: 1, bytes: 4096 },
          categories: [
            { kind: 'proxies', files: 1, bytes: 4096, purgeable_files: 1, purgeable_bytes: 4096 },
            { kind: 'thumbnails', files: 0, bytes: 0, purgeable_files: 0, purgeable_bytes: 0 },
          ],
          blocked_reasons: [],
        },
      }
    case 'project.cache_purge':
      cancelled = false
      return { ok: true, result: { job_id: CACHE_JOB_ID, status: 'queued' } }
    case 'jobs.status':
      if (args.job_id !== CACHE_JOB_ID) return undefined
      return {
        ok: true,
        result: {
          job_id: CACHE_JOB_ID,
          state: cancelled ? 'failed' : 'running',
          progress: cancelled ? 1 : 0.35,
          message: cancelled ? 'Cache cleanup cancelled.' : 'Revalidating cache ownership.',
        },
      }
    case 'jobs.cancel':
      if (args.job_id !== CACHE_JOB_ID) return undefined
      cancelled = true
      return { ok: true, result: { job_id: CACHE_JOB_ID, cancelled: true } }
    default:
      return undefined
  }
}
