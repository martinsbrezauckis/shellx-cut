// fullCoverageJobs.mjs - bounded job waiters for the exhaustive UI verifier.
//
// Keep these helpers dependency-injected so the full-coverage runner controls
// its verb transport and sleep implementation while this module owns polling
// semantics. Installed sweeps rely on awaitImportJobs() to drain import/enrich
// work before the next section starts, which prevents ffmpeg/python fan-out on
// qualification machines.

export function createJobWaiters({ verb, sleep }) {
  if (typeof verb !== 'function') throw new TypeError('createJobWaiters requires verb')
  if (typeof sleep !== 'function') throw new TypeError('createJobWaiters requires sleep')

  async function awaitJob(jobId, timeoutMs = 240000) {
    const deadline = Date.now() + timeoutMs
    while (Date.now() < deadline) {
      const js = (await verb('jobs.status', { job_id: jobId })).result
      if (js?.state === 'done' || js?.state === 'failed') return js
      await sleep(900)
    }
    return null
  }

  async function awaitImportJobs(
    imp,
    timeoutMs = Number(process.env.FCV_IMPORT_DRAIN_TIMEOUT_MS || 600_000),
  ) {
    const jobId = imp?.result?.job_id
    let importJob = null
    if (jobId) {
      importJob = await awaitJob(jobId, timeoutMs)
      if (!importJob) {
        throw new Error(`media import job ${jobId} timed out after ${timeoutMs}ms`)
      }
      if (importJob.state === 'failed') {
        throw new Error(
          `media import job ${jobId} failed: ` +
          `${JSON.stringify(importJob.error || importJob.result || importJob).slice(0, 500)}`,
        )
      }
    }
    const enrichJob = importJob?.result?.enrich_job || imp?.result?.enrich_job
    if (enrichJob) {
      const enrich = await awaitJob(enrichJob, timeoutMs)
      if (!enrich) {
        throw new Error(`media enrichment job ${enrichJob} timed out after ${timeoutMs}ms`)
      }
      if (enrich.state === 'failed') {
        throw new Error(
          `media enrichment job ${enrichJob} failed: ` +
          `${JSON.stringify(enrich.error || enrich.result || enrich).slice(0, 500)}`,
        )
      }
      return enrich
    }
    return importJob
  }

  return { awaitJob, awaitImportJobs }
}
