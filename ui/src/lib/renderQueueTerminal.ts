/** Durable `jobs.status` summary for a completed render queue. */
export interface RenderQueueTerminalResult {
  count?: number
  succeeded?: number
  failed?: number
  jobs?: Array<{ ok?: boolean }>
}

/**
 * A render queue may have terminal `state: done` after processing every child,
 * including a child that failed. Only fully reconciled child accounting can
 * enter the completed presentation.
 */
export function renderQueueTerminalError(result: RenderQueueTerminalResult | undefined): string | null {
  const count = result?.count
  const succeeded = result?.succeeded
  const failed = result?.failed
  if (typeof count !== 'number' || !Number.isInteger(count) || count < 1
    || typeof succeeded !== 'number' || !Number.isInteger(succeeded) || succeeded < 0
    || typeof failed !== 'number' || !Number.isInteger(failed) || failed < 0
    || succeeded + failed !== count) {
    return 'Render queue finished without a valid delivery summary.'
  }
  if (Array.isArray(result?.jobs)) {
    if (result.jobs.length !== count) return 'Render queue finished without a valid delivery summary.'
    let succeededRows = 0
    let failedRows = 0
    for (const job of result.jobs) {
      if (job?.ok === true) succeededRows += 1
      else if (job?.ok === false) failedRows += 1
      else return 'Render queue finished without a valid delivery summary.'
    }
    if (succeededRows !== succeeded || failedRows !== failed) return 'Render queue finished without a valid delivery summary.'
  }
  if (failed > 0) {
    return `Render queue completed with ${failed} failed ${failed === 1 ? 'delivery' : 'deliveries'} out of ${count}.`
  }
  return null
}
