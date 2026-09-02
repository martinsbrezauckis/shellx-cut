const CACHE_JOB_ID = 'job_mock_cache_purge'
const REBUILD_JOB_ID = 'job_mock_cache_rebuild'
const keepRebuildRunning = typeof location !== 'undefined'
  && new URLSearchParams(location.search).has('mockCacheRebuildRunning')
const returnAlreadyQueued = typeof location !== 'undefined'
  && new URLSearchParams(location.search).has('mockCacheRebuildAlreadyQueued')
const mockLargeBatch = typeof location !== 'undefined'
  && new URLSearchParams(location.search).has('mockCacheRebuildLarge')
const mockJobRetry = typeof location !== 'undefined'
  && new URLSearchParams(location.search).has('mockJobRetry')
const RETRY_SOURCE_JOB_ID = 'fixture-health-retry'
let cancelledJob: string | null = null
let rebuildStatusReads = 0
let retryAdmitted = false

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
      cancelledJob = null
      return { ok: true, result: { job_id: CACHE_JOB_ID, status: 'queued' } }
    case 'project.cache_rebuild':
      if (mockLargeBatch && (!Array.isArray(args.asset_ids) || args.asset_ids.length !== 64)) {
        return { ok: false, error: { code: 'validation', message: 'The UI must submit one bounded cache batch.' } }
      }
      const scheduledAssets = Array.isArray(args.asset_ids) ? args.asset_ids.length : 0
      const estimateOnly = args.estimate_only === true
      const estimate = {
        basis: 'source_hash_match_and_current_import_metadata',
        assets: scheduledAssets,
        verified_source_bytes: scheduledAssets * 4096,
        proxy_outputs: mockLargeBatch ? 64 : 0,
        filmstrip_outputs: mockLargeBatch ? 64 : scheduledAssets,
        proxy_duration_ms: 0,
      }
      if (estimateOnly) {
        return {
          ok: true,
          result: {
            schema: 'shellx-cut/cache-rebuild/1',
            status: 'estimated',
            scheduled_assets: scheduledAssets,
            scheduled_outputs: mockLargeBatch ? 128 : scheduledAssets,
            counts: {
              fresh_assets: mockLargeBatch ? 0 : 1,
              source_changed: 1,
              source_unavailable: 0,
              unowned_outputs: 0,
              legacy_outputs: 0,
              unsupported_assets: 0,
            },
            estimate,
          },
        }
      }
      cancelledJob = null
      rebuildStatusReads = 0
      return {
        ok: true,
        result: {
          schema: 'shellx-cut/cache-rebuild/1',
          status: returnAlreadyQueued ? 'already_queued' : 'queued',
          job_id: REBUILD_JOB_ID,
          deduplicated: returnAlreadyQueued,
          scheduled_assets: scheduledAssets,
          scheduled_outputs: mockLargeBatch ? 128 : scheduledAssets,
          counts: {
            fresh_assets: mockLargeBatch ? 0 : 1,
            source_changed: 1,
            source_unavailable: 0,
            unowned_outputs: 0,
            legacy_outputs: 0,
            unsupported_assets: 0,
          },
          estimate,
        },
      }
    case 'jobs.status':
      if (args.job_id === REBUILD_JOB_ID) {
        const cancelled = cancelledJob === REBUILD_JOB_ID
        const done = !cancelled && !keepRebuildRunning && rebuildStatusReads++ > 0
        return {
          ok: true,
          result: {
            job_id: REBUILD_JOB_ID,
            kind: 'cache_rebuild',
            state: cancelled ? 'failed' : done ? 'done' : 'running',
            progress: cancelled || done ? 1 : 0.35,
            message: cancelled
              ? 'Cache rebuild cancelled. Missing cache can be rebuilt again.'
              : done
                ? 'Cache rebuild completed.'
                : 'Rebuilding missing editing cache.',
          },
        }
      }
      if (args.job_id !== CACHE_JOB_ID) return undefined
      const cancelled = cancelledJob === CACHE_JOB_ID
      return {
        ok: true,
        result: {
          job_id: CACHE_JOB_ID,
          kind: 'cache_purge',
          state: cancelled ? 'failed' : 'running',
          progress: cancelled ? 1 : 0.35,
          message: cancelled ? 'Cache cleanup cancelled.' : 'Revalidating cache ownership.',
          outcome: cancelled ? 'cancelled' : undefined,
          result: cancelled
            ? {
                schema: 'shellx-cut/cache-purge-reconciliation/1',
                status: 'cancelled',
                reconciliation: {
                  before: { files: 1, bytes: 4096 },
                  planned: { files: 1, bytes: 4096 },
                  removed: { files: 1, bytes: 4096 },
                  after: { files: 0, bytes: 0 },
                  balanced: true,
                  partial_progress: true,
                  after_basis: 'strict_scan',
                  ledger_recovery_required: false,
                },
              }
            : undefined,
        },
      }
    case 'jobs.cancel':
      if (args.job_id !== CACHE_JOB_ID && args.job_id !== REBUILD_JOB_ID) return undefined
      cancelledJob = args.job_id as string
      return { ok: true, result: { job_id: cancelledJob, cancelled: true } }
    case 'jobs.list':
      if (!mockJobRetry) return undefined
      return {
        ok: true,
        result: {
          jobs: [{
            job_id: RETRY_SOURCE_JOB_ID,
            kind: 'screen_record_export',
            state: 'failed',
            outcome: 'failed',
            outcome_reason: 'true_failure',
            progress: 1,
            created_ts: '2026-09-02T00:00:00Z',
            updated_ts: '2026-09-02T00:00:01Z',
            retry: retryAdmitted
              ? {
                  eligible: false,
                  reason: 'a retry attempt has already been admitted',
                  root_job_id: RETRY_SOURCE_JOB_ID,
                  attempt: 1,
                  retried_by: 'fixture-health-retry-child',
                }
              : {
                  eligible: true,
                  root_job_id: RETRY_SOURCE_JOB_ID,
                  attempt: 1,
                },
          }],
          persistence_notices: [],
        },
      }
    case 'jobs.retry':
      if (!mockJobRetry || args.job_id !== RETRY_SOURCE_JOB_ID) return undefined
      if (retryAdmitted) {
        return { ok: false, error: { code: 'conflict', message: 'fixture retry is not eligible' } }
      }
      retryAdmitted = true
      return {
        ok: true,
        result: {
          job_id: 'fixture-health-retry-child',
          retry_of: RETRY_SOURCE_JOB_ID,
          root_job_id: RETRY_SOURCE_JOB_ID,
          attempt: 2,
          path: '/fixture/retry.mp4',
          format: 'mp4',
          status: 'queued',
        },
      }
    default:
      return undefined
  }
}
