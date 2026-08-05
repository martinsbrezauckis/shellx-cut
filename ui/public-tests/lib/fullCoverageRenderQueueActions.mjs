// Deterministic coverage for conditional Render Queue row and terminal-state
// controls. The provider-backed queue section retains real rendering proof;
// this companion drives every local UI branch without encoding five videos.

export function sameRenderQueueJobs(actual, expected) {
  if (!Array.isArray(actual) || actual.length !== expected.length) return false
  return actual.every((job, index) => {
    const expectedJob = expected[index]
    if (!job || typeof job !== 'object' || Array.isArray(job)) return false
    const actualKeys = Object.keys(job).sort()
    const expectedKeys = Object.keys(expectedJob).sort()
    return actualKeys.length === expectedKeys.length
      && actualKeys.every((key, keyIndex) =>
        key === expectedKeys[keyIndex] && job[key] === expectedJob[key])
  })
}

export function createRenderQueueActionCoverage({
  probe,
  sleep,
  freshProject,
  closeOverlays,
  primaryMedia,
}) {
  const surface = 'render-queue-actions'

  async function installFixture(page) {
    await page.evaluate(() => {
      const target = window
      target.__fcvRenderQueueOriginalFetch = window.fetch
      target.__fcvRenderQueueFixture = {
        fail: false,
        queueCalls: [],
        statusCalls: [],
        outputDirCalls: [],
      }
      const fixture = target.__fcvRenderQueueFixture
      const originalFetch = target.__fcvRenderQueueOriginalFetch
      const envelope = (body) => new Response(JSON.stringify(body), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      })
      const requestArgs = (options) => {
        try { return JSON.parse(options?.body || '{}') } catch { return {} }
      }
      window.fetch = async (...args) => {
        const input = args[0]
        const options = args[1]
        const url = typeof input === 'string' ? input : input?.url || ''
        let pathname = ''
        try { pathname = new URL(String(url), window.location.href).pathname } catch {}
        const body = requestArgs(options)
        if (pathname === '/api/verb/project.set_output_dir') {
          fixture.outputDirCalls.push(body)
          return envelope({ ok: true, result: {
            output_dir: body.dir || null,
          } })
        }
        if (pathname === '/api/verb/render.queue') {
          fixture.queueCalls.push(body)
          if (fixture.fail) {
            return envelope({ ok: false, error: {
              code: 'invalid_queue',
              message: 'Delivery 2 has an unsupported output combination. Choose another format and try again.',
            } })
          }
          return envelope({ ok: true, result: {
            queue_id: 'queue_fcv_actions',
            count: body.jobs?.length ?? 0,
            jobs: (body.jobs ?? []).map((job, idx) => ({
              idx,
              output: job.output || `/fixture/export-${idx + 1}.mp4`,
              job_id: `render_fcv_${idx + 1}`,
              state: 'pending',
            })),
          } })
        }
        if (pathname === '/api/verb/jobs.status' && body.job_id === 'queue_fcv_actions') {
          fixture.statusCalls.push(body)
          return envelope({ ok: true, result: {
            job_id: body.job_id,
            state: 'done',
            progress: 1,
            result: {
              queue_id: body.job_id,
              count: 2,
              jobs: [
                { idx: 0, output: '/fixture/export-1.mp4', job_id: 'render_fcv_1', state: 'done' },
                { idx: 1, output: '/fixture/export-2.mp4', job_id: 'render_fcv_2', state: 'done' },
              ],
            },
          } })
        }
        return originalFetch(...args)
      }
    })
  }

  async function fixtureState(page) {
    return page.evaluate(() => JSON.parse(JSON.stringify(window.__fcvRenderQueueFixture)))
  }

  async function restoreFixture(page) {
    await page.evaluate(() => {
      const target = window
      if (target.__fcvRenderQueueOriginalFetch) window.fetch = target.__fcvRenderQueueOriginalFetch
      delete target.__fcvRenderQueueOriginalFetch
      delete target.__fcvRenderQueueFixture
    })
  }

  async function openQueue(page) {
    await page.locator('[data-cut-export-btn]').click()
    await page.locator('[data-cut-export-menu]').waitFor({ state: 'visible', timeout: 5_000 })
    await page.locator('[data-cut-render-queue-open]').click()
    await page.locator('[data-cut-render-queue-form]').waitFor({ state: 'visible', timeout: 5_000 })
  }

  async function run(page) {
    await freshProject(page, 'render_queue_actions', primaryMedia)
    await closeOverlays(page)
    await installFixture(page)
    try {
      await openQueue(page)
      const queue = page.locator('[data-cut-render-queue]')

      await probe(page, {
        surface,
        name: 'add-render-queue-delivery',
        actionId: 'render-queue-add',
        sel: queue.locator('[data-cut-render-queue-add]'),
        group: queue.locator('[data-cut-render-queue-form]'),
        groupName: 'render-queue-add',
        doClick: async () => {
          await queue.locator('[data-cut-render-queue-add]').click()
          await queue.locator('[data-cut-render-queue-row="2"]').waitFor({ state: 'visible', timeout: 5_000 })
        },
        assertResult: async () => ({
          ok: await queue.locator('[data-cut-render-queue-row]').count() === 3
            && await queue.locator('[data-cut-render-queue-count]').getAttribute('data-cut-render-queue-count') === '3',
          detail: `rows=${await queue.locator('[data-cut-render-queue-row]').count()}; count=${await queue.locator('[data-cut-render-queue-count]').getAttribute('data-cut-render-queue-count')}`,
        }),
      })

      await probe(page, {
        surface,
        name: 'remove-render-queue-delivery',
        actionId: 'render-queue-remove',
        sel: queue.locator('[data-cut-render-queue-remove="2"]'),
        group: queue.locator('[data-cut-render-queue-row="2"]'),
        groupName: 'render-queue-remove',
        doClick: async () => {
          await queue.locator('[data-cut-render-queue-remove="2"]').click()
          await queue.locator('[data-cut-render-queue-row="2"]').waitFor({ state: 'detached', timeout: 5_000 })
        },
        assertResult: async () => ({
          ok: await queue.locator('[data-cut-render-queue-row]').count() === 2
            && await queue.locator('[data-cut-render-queue-count]').getAttribute('data-cut-render-queue-count') === '2',
          detail: `rows=${await queue.locator('[data-cut-render-queue-row]').count()}; count=${await queue.locator('[data-cut-render-queue-count]').getAttribute('data-cut-render-queue-count')}`,
        }),
      })

      await queue.locator('[data-cut-render-queue-output="0"]').fill('/fixture/master.mp4')
      await queue.locator('[data-cut-render-queue-output="1"]').fill('/fixture/vertical.mp4')
      await queue.locator('[data-cut-render-queue-preset="0"]').selectOption('high')
      await queue.locator('[data-cut-render-queue-aspect="1"]').selectOption('9:16')
      await queue.locator('[data-cut-render-queue-start]').click()
      await queue.locator('[data-cut-render-queue-progress="done"]').waitFor({ state: 'visible', timeout: 12_000 })
      const successful = await fixtureState(page)
      const jobs = successful.queueCalls[0]?.jobs
      if (!sameRenderQueueJobs(jobs, [
        { preset: 'high', output: '/fixture/master.mp4' },
        { preset: 'standard', aspect: '9:16', output: '/fixture/vertical.mp4' },
      ])) {
        throw new Error(`render.queue jobs=${JSON.stringify(jobs)}`)
      }
      if (successful.outputDirCalls.length < 2
        || successful.outputDirCalls[0]?.dir !== '/fixture') {
        throw new Error(`project.set_output_dir calls=${JSON.stringify(successful.outputDirCalls)}`)
      }

      await probe(page, {
        surface,
        name: 'close-completed-render-queue',
        actionId: 'render-queue-done-close',
        sel: queue.locator('[data-cut-render-queue-done-close]'),
        group: queue.locator('[data-cut-render-queue-progress="done"]'),
        groupName: 'render-queue-completed',
        doClick: async () => {
          await queue.locator('[data-cut-render-queue-done-close]').click()
          await queue.waitFor({ state: 'detached', timeout: 5_000 })
        },
        assertResult: async () => ({
          ok: await queue.count() === 0 && successful.statusCalls.length === 1,
          detail: `modal count=${await queue.count()}; queue status calls=${successful.statusCalls.length}`,
        }),
      })

      await page.evaluate(() => { window.__fcvRenderQueueFixture.fail = true })
      await openQueue(page)
      await queue.locator('[data-cut-render-queue-start]').click()
      await queue.locator('[data-cut-render-queue-error]').waitFor({ state: 'visible', timeout: 5_000 })
      await probe(page, {
        surface,
        name: 'back-from-render-queue-error',
        actionId: 'render-queue-error-back',
        sel: queue.locator('[data-cut-render-queue-error-back]'),
        group: queue.locator('[data-cut-render-queue-error]'),
        groupName: 'render-queue-error-back',
        doClick: async () => {
          await queue.locator('[data-cut-render-queue-error-back]').click()
          await queue.locator('[data-cut-render-queue-form]').waitFor({ state: 'visible', timeout: 5_000 })
        },
        assertResult: async () => ({
          ok: await queue.locator('[data-cut-render-queue-form]').isVisible()
            && await queue.locator('[data-cut-render-queue-row]').count() === 2,
          detail: `form visible=${await queue.locator('[data-cut-render-queue-form]').isVisible()}; preserved rows=${await queue.locator('[data-cut-render-queue-row]').count()}`,
        }),
      })

      await queue.locator('[data-cut-render-queue-start]').click()
      await queue.locator('[data-cut-render-queue-error]').waitFor({ state: 'visible', timeout: 5_000 })
      await probe(page, {
        surface,
        name: 'close-render-queue-error',
        actionId: 'render-queue-error-close',
        sel: queue.locator('[data-cut-render-queue-error-close]'),
        group: queue.locator('[data-cut-render-queue-error]'),
        groupName: 'render-queue-error-close',
        doClick: async () => {
          await queue.locator('[data-cut-render-queue-error-close]').click()
          await queue.waitFor({ state: 'detached', timeout: 5_000 })
        },
        assertResult: async () => {
          const fixture = await fixtureState(page)
          return {
            ok: await queue.count() === 0 && fixture.queueCalls.length === 3,
            detail: `modal count=${await queue.count()}; total queue calls=${fixture.queueCalls.length}`,
          }
        },
      })
    } finally {
      await restoreFixture(page)
      await closeOverlays(page)
    }
  }

  return { run }
}
