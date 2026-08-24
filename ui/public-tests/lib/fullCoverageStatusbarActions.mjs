// Exhaustive status-bar actions: settings shortcuts, cancellable live job, and
// receipt navigation. This lane uses real engine jobs so the bottom bar proves
// its WebSocket wiring rather than a DOM-only fixture.

export async function reapStatusbarCancellation({
  jobId,
  terminal,
  primaryError,
  cancelAndAwaitTerminal,
}) {
  if (terminal?.state === 'done' || terminal?.state === 'failed') return terminal
  let recovery
  try {
    recovery = await cancelAndAwaitTerminal(jobId, { timeoutMs: 45_000 })
  } catch (cleanupError) {
    if (primaryError) {
      throw new AggregateError([primaryError, cleanupError], primaryError.message)
    }
    throw cleanupError
  }
  if (recovery.terminal) return recovery.terminal

  const cleanupError = new Error(
    `status-bar cancellation recovery did not reap ${jobId}; ` +
    `cancel=${recovery.cancellation?.ok === true}; last=${recovery.lastStatus?.state || 'missing'}`,
  )
  if (primaryError) {
    throw new AggregateError([primaryError, cleanupError], primaryError.message)
  }
  throw cleanupError
}

export function createStatusbarActionCoverage({
  probe,
  verb,
  awaitJob,
  cancelAndAwaitTerminal,
  captureVerbResp,
  sleep,
  freshProject,
  closeOverlays,
  primaryMedia,
}) {
  const surface = 'statusbar-actions'

  async function openSettingsShortcut(page, {
    name,
    actionId,
    selector,
    category,
  }) {
    const statusbar = page.locator('[data-cut-panel="statusbar"]').first()
    const control = page.locator(selector).first()
    await probe(page, {
      surface,
      name,
      actionId,
      sel: control,
      group: statusbar,
      groupName: 'statusbar',
      doClick: async () => {
        await control.click()
        await page.locator(`[data-cut-settings-body="${category}"]`).first().waitFor({
          state: 'visible',
          timeout: 8000,
        })
      },
      assertResult: async () => ({
        ok: (await page.locator(`[data-cut-settings-category="${category}"]`).first().getAttribute('aria-current')) === 'page',
        detail: `${category} Settings category opened`,
      }),
    })
    await page.locator('[data-cut-environment-close]').first().click()
  }

  async function run(page) {
    await freshProject(page, 'statusbar_actions', primaryMedia)
    await closeOverlays(page)
    const statusbar = page.locator('[data-cut-panel="statusbar"]').first()

    await openSettingsShortcut(page, {
      name: 'statusbar-environment-settings',
      actionId: 'env-chip',
      selector: '[data-cut-env-chip]',
      category: 'overview',
    })
    await openSettingsShortcut(page, {
      name: 'statusbar-output-settings',
      actionId: 'output-chip',
      selector: '[data-cut-output-chip]',
      category: 'general',
    })

    const rendered = await verb('render.final', {
      preset: 'draft',
      hardware: 'off',
      profile: 'silent_screen_demo',
      rationale: 'fcv: create status-bar receipt',
    })
    const renderJob = rendered.result?.job_id
    if (!rendered.ok || !renderJob) {
      throw new Error(`status-bar receipt render did not queue: ${rendered.error?.message || rendered.error?.code}`)
    }
    const renderTerminal = await awaitJob(renderJob, 120_000)
    if (renderTerminal?.state !== 'done') {
      throw new Error(`status-bar receipt render failed: ${renderTerminal?.error?.message || renderTerminal?.state}`)
    }
    const receiptButton = page.locator('button[data-cut-last-receipt]:not([data-cut-last-receipt="none"])').first()
    await receiptButton.waitFor({ state: 'visible', timeout: 15_000 })
    await probe(page, {
      surface,
      name: 'statusbar-open-last-receipt',
      actionId: 'last-receipt',
      sel: receiptButton,
      group: statusbar,
      groupName: 'statusbar',
      doClick: async () => {
        await receiptButton.click()
        await page.locator('[data-cut-review-tab="receipts"][aria-selected="true"]').first().waitFor({
          state: 'visible',
          timeout: 8000,
        })
      },
      assertResult: async () => {
        const receiptId = await receiptButton.getAttribute('data-cut-last-receipt')
        return {
          ok: receiptId !== 'none'
            && await page.locator(`[data-cut-receipt="${receiptId}"]`).first().isVisible(),
          detail: `receipt ${receiptId} opened in the Inspect rail`,
        }
      },
    })

    // Cancellation is a product job-control contract, not a provider contract.
    // Queue a deliberately slow, local-only software render so the status-bar
    // event subscription and jobs.cancel button can be exercised without
    // shadowing or spending any of the user's real generation providers. The
    // explicit 8K/high/software shape makes the running window deterministic
    // on release hosts; the test cancels it as soon as the running pill exists.
    const queued = await verb('render.final', {
      preset: 'high',
      format: 'h264',
      hardware: 'off',
      width: 7680,
      height: 4320,
      fit: 'contain',
      profile: 'silent_screen_demo',
      rationale: 'fcv: status-bar cancellation proof',
    })
    const cancelJobId = queued.result?.job_id
    if (!queued.ok || !cancelJobId) {
      throw new Error(`status-bar cancellable render did not queue: ${queued.error?.message || queued.error?.code}`)
    }
    let running = null
    let cancelled = null
    let terminal = null
    let primaryError = null
    try {
      const runningDeadline = Date.now() + 12_000
      while (Date.now() < runningDeadline) {
        const status = await verb('jobs.status', { job_id: cancelJobId })
        const job = status.result
        if (job?.state === 'running') {
          running = job
          break
        }
        if (job?.state === 'done' || job?.state === 'failed') {
          terminal = job
          throw new Error(
            `status-bar cancellation render reached terminal ${job.state}/${job.error?.code || 'none'} before it could be cancelled`,
          )
        }
        await sleep(120)
      }
      if (!running) {
        throw new Error(`status-bar cancellation render ${cancelJobId} did not reach running state before timeout`)
      }
      const cancel = page.locator(`[data-cut-job-cancel="${cancelJobId}"]`).first()
      await cancel.waitFor({ state: 'visible', timeout: 12_000 })
      await probe(page, {
        surface,
        name: 'statusbar-cancel-live-job',
        actionId: 'job-cancel',
        sel: cancel,
        group: statusbar,
        groupName: 'statusbar-job',
        doClick: async () => {
          cancelled = await captureVerbResp(page, 'jobs.cancel', () => cancel.click(), 30_000)
          terminal = await awaitJob(cancelJobId, 30_000)
          await sleep(120)
        },
        assertResult: async () => ({
          ok: running?.state === 'running'
            && (cancelled?.ok || cancelled?.error?.code === 'job_cancel_pending')
            && terminal?.state === 'failed'
            && terminal?.error?.code === 'job_cancelled'
            && await page.locator(`[data-cut-job="${cancelJobId}"]`).count() === 0,
          detail: `job=software-8k-render; observed running=${running?.state === 'running'}; jobs.cancel ok=${cancelled?.ok} code=${cancelled?.error?.code || 'none'}; terminal=${terminal?.state}/${terminal?.error?.code}; pill removed=${await page.locator(`[data-cut-job="${cancelJobId}"]`).count() === 0}`,
        }),
      })
    } catch (error) {
      primaryError = error
      throw error
    } finally {
      terminal = await reapStatusbarCancellation({
        jobId: cancelJobId,
        terminal,
        primaryError,
        cancelAndAwaitTerminal,
      })
    }
  }

  return { run }
}
