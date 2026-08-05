// Exhaustive status-bar actions: settings shortcuts, cancellable live job, and
// receipt navigation. This lane uses real engine jobs so the bottom bar proves
// its WebSocket wiring rather than a DOM-only fixture.

export function createStatusbarActionCoverage({
  probe,
  verb,
  state,
  waitForState,
  awaitJob,
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

  async function extendTimelineForCancellation() {
    const snapshot = await state()
    const videoTrack = snapshot.tracks.find((track) => track.kind === 'video' && track.clips.some((clip) => clip.asset))
    const mediaClip = videoTrack?.clips.find((clip) => clip.asset)
    if (!videoTrack || !mediaClip) throw new Error('status-bar cancellation coverage needs video media')
    const clipDuration = (clip) => {
      if (clip.kind === 'gap') return clip.duration_ms
      const raw = Math.max(0, clip.src_out_ms - clip.src_in_ms)
      const speed = Number.isFinite(clip.speed) && clip.speed > 0 ? clip.speed : 1
      return Math.round(raw / speed)
    }
    const trackEnd = (track) => track.clips.reduce((cursor, clip) => {
      const duration = clipDuration(clip)
      const overlap = Math.min(Math.max(0, clip.xfade_in_ms || 0), duration)
      return Math.max(0, cursor - overlap) + duration
    }, 0)
    const sourceDuration = Math.max(1000, mediaClip.src_out_ms - mediaClip.src_in_ms)
    const duration = Math.min(5000, sourceDuration)
    const srcIn = mediaClip.src_in_ms
    let atMs = trackEnd(videoTrack)
    const expectedClipCount = videoTrack.clips.length + 12
    for (let index = 0; index < 12; index += 1) {
      const insertAtMs = Math.round(atMs)
      const inserted = await verb('edit.insert', {
        asset: mediaClip.asset,
        track: videoTrack.id,
        at_ms: insertAtMs,
        src_range_ms: [srcIn, srcIn + duration],
        rationale: 'fcv: make the status-bar cancellation job observable',
      })
      if (!inserted.ok) throw new Error(`could not extend cancellation timeline: ${inserted.error?.message || inserted.error?.code}`)
      atMs = insertAtMs + Math.round(duration)
    }
    return { durationMs: atMs, expectedClipCount, trackId: videoTrack.id }
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

    const extension = await extendTimelineForCancellation()
    await waitForState((project) => {
      const track = project.tracks.find((candidate) => candidate.id === extension.trackId)
      return (track?.clips.length || 0) >= extension.expectedClipCount
    }, 10_000)
    const queued = await verb('render.final', {
      preset: 'high',
      hardware: 'off',
      profile: 'silent_screen_demo',
      rationale: 'fcv: status-bar cancel action',
    })
    const cancelJobId = queued.result?.job_id
    if (!queued.ok || !cancelJobId) {
      throw new Error(`status-bar cancellable render did not queue: ${queued.error?.message || queued.error?.code}`)
    }
    const cancel = page.locator(`[data-cut-job-cancel="${cancelJobId}"]`).first()
    await cancel.waitFor({ state: 'visible', timeout: 12_000 })
    let cancelled = null
    let terminal = null
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
        ok: (cancelled?.ok || cancelled?.error?.code === 'job_cancel_pending')
          && terminal?.state === 'failed'
          && terminal?.error?.code === 'job_cancelled'
          && await page.locator(`[data-cut-job="${cancelJobId}"]`).count() === 0,
        detail: `jobs.cancel ok=${cancelled?.ok} code=${cancelled?.error?.code || 'none'}; terminal=${terminal?.state}/${terminal?.error?.code}; pill removed=${await page.locator(`[data-cut-job="${cancelJobId}"]`).count() === 0}`,
      }),
    })
  }

  return { run }
}
