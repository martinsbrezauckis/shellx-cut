// Exhaustive conditional Autopilot action coverage.
//
// The main `autopilot` full-coverage section runs the real render/verify job.
// Its report is content-dependent, so it cannot guarantee the conditional
// Apply, Restore, and Open Inspect controls exist. This helper supplies closed
// preview/auto-fix job reports while retaining the installed drawer, exact
// action requests, poll transitions, restore request, and Inspect routing.

export function createAutopilotActionCoverage({
  probe,
  sleep,
  freshProject,
  closeOverlays,
  primaryMedia,
}) {
  const surface = 'autopilot-actions'
  const checkpoint = 'cp_fcv_autopilot'

  async function installFixture(page) {
    await page.evaluate((fixtureCheckpoint) => {
      const target = window
      if (!target.__fcvAutopilotOriginalFetch) target.__fcvAutopilotOriginalFetch = window.fetch
      const originalFetch = target.__fcvAutopilotOriginalFetch
      target.__fcvAutopilotFixture = {
        runCalls: [],
        statusCalls: [],
        revertCalls: [],
      }
      const fixture = target.__fcvAutopilotFixture
      const envelope = (body) => new Response(JSON.stringify(body), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      })
      const requestArgs = (options) => {
        try { return JSON.parse(options?.body || '{}') } catch { return {} }
      }
      const plan = [{
        check: 'loudness',
        fix_verb: 'render.final',
        auto_fixable: true,
        rationale: 'Normalize delivery loudness.',
      }]
      const report = (policy) => ({
        summary_line: policy === 'preview'
          ? 'Preview ready: one low-risk delivery fix.'
          : 'Applied one low-risk delivery fix.',
        checks_pass: policy !== 'preview',
        policy,
        checkpoint: fixtureCheckpoint,
        plan,
        fixes_applied: policy === 'preview' ? [] : [{
          check: 'loudness',
          via: 'render.final',
        }],
        changed: policy === 'preview' ? { ops: 0 } : { ops: 1, duration_delta_ms: 0 },
        receipt_ids: ['receipt_fcv_autopilot'],
        iterations: policy === 'preview' ? 0 : 1,
      })

      window.fetch = async (...args) => {
        const input = args[0]
        const options = args[1]
        const url = typeof input === 'string' ? input : input?.url || ''
        let pathname = ''
        try { pathname = new URL(String(url), window.location.href).pathname } catch {}

        if (pathname === '/api/verb/autopilot.run') {
          const body = requestArgs(options)
          fixture.runCalls.push(body)
          const suffix = body.policy === 'auto_low_risk' ? 'auto' : 'preview'
          return envelope({ ok: true, result: { job_id: `job_fcv_autopilot_${suffix}` } })
        }
        if (pathname === '/api/verb/jobs.status') {
          const body = requestArgs(options)
          if (String(body.job_id || '').startsWith('job_fcv_autopilot_')) {
            fixture.statusCalls.push(body)
            const policy = String(body.job_id).endsWith('_auto') ? 'auto_low_risk' : 'preview'
            return envelope({
              ok: true,
              result: {
                job_id: body.job_id,
                kind: 'autopilot',
                state: 'done',
                progress: 1,
                created_ts: '2026-07-29T00:00:00Z',
                updated_ts: '2026-07-29T00:00:01Z',
                result: report(policy),
              },
            })
          }
        }
        if (pathname === '/api/verb/project.revert') {
          const body = requestArgs(options)
          fixture.revertCalls.push(body)
          return envelope({ ok: true, result: { to: body.to, restored: true } })
        }
        return originalFetch(...args)
      }
    }, checkpoint)
  }

  async function fixtureState(page) {
    return page.evaluate(() => JSON.parse(JSON.stringify(window.__fcvAutopilotFixture)))
  }

  async function restoreFixture(page) {
    await page.evaluate(() => {
      const target = window
      if (target.__fcvAutopilotOriginalFetch) window.fetch = target.__fcvAutopilotOriginalFetch
      delete target.__fcvAutopilotOriginalFetch
      delete target.__fcvAutopilotFixture
    })
  }

  async function run(page) {
    await freshProject(page, 'autopilot_actions', primaryMedia)
    await closeOverlays(page)
    await installFixture(page)
    try {
      await page.locator('[data-cut-autopilot-btn]').first().click()
      const drawer = page.locator('[data-cut-autopilot-open="true"]').first()
      await drawer.waitFor({ state: 'visible', timeout: 8000 })
      await page.locator('[data-cut-autopilot-goal]').first().fill('Make this delivery-ready')
      await page.locator('[data-cut-policy="preview"]').first().click()
      await page.locator('[data-cut-autopilot-run]').first().click()
      await page.locator('[data-cut-autopilot-apply]').first().waitFor({
        state: 'visible',
        timeout: 8000,
      })

      const apply = page.locator('[data-cut-autopilot-apply]').first()
      await probe(page, {
        surface,
        name: 'autopilot-apply-previewed-fixes',
        actionId: 'autopilot-apply',
        sel: apply,
        group: drawer,
        groupName: 'autopilot-preview-report',
        doClick: async () => {
          await apply.click()
          await page.locator('[data-cut-autopilot-fixes]').first().waitFor({
            state: 'visible',
            timeout: 8000,
          })
        },
        assertResult: async () => {
          const fixture = await fixtureState(page)
          const call = fixture.runCalls[1]
          const closeBox = await page.locator('[data-cut-autopilot-close]').first().boundingBox()
          const drawerBox = await drawer.boundingBox()
          const closeContained = !!closeBox && !!drawerBox
            && closeBox.x >= drawerBox.x
            && closeBox.x + closeBox.width <= drawerBox.x + drawerBox.width
          return {
            ok: fixture.runCalls.length === 2
              && call?.goal === 'Make this delivery-ready'
              && call?.policy === 'auto_low_risk'
              && call?.max_fix_iters === 3
              && fixture.statusCalls.some((entry) => entry.job_id === 'job_fcv_autopilot_auto')
              && await page.locator('[data-cut-autopilot-fixes]').first().isVisible()
              && closeContained,
            detail: `run calls=${fixture.runCalls.length}; apply args=${JSON.stringify(call)}; auto status=${fixture.statusCalls.some((entry) => entry.job_id === 'job_fcv_autopilot_auto')}; close contained=${closeContained}`,
          }
        },
      })

      const restore = page.locator('[data-cut-autopilot-restore]').first()
      await probe(page, {
        surface,
        name: 'autopilot-restore-whole-run',
        actionId: 'autopilot-restore',
        sel: restore,
        group: drawer,
        groupName: 'autopilot-applied-report',
        doClick: async () => {
          await restore.click()
          await page.locator('[data-cut-autopilot-restored]').first().waitFor({
            state: 'visible',
            timeout: 5000,
          })
        },
        assertResult: async () => {
          const fixture = await fixtureState(page)
          const call = fixture.revertCalls[0]
          return {
            ok: fixture.revertCalls.length === 1
              && call?.to === checkpoint
              && call?.rationale === 'undo autopilot run'
              && (await page.locator('[data-cut-autopilot-restored]').first().textContent())?.includes('reverted to checkpoint'),
            detail: `revert calls=${fixture.revertCalls.length}; args=${JSON.stringify(call)}; restored=${await page.locator('[data-cut-autopilot-restored]').first().textContent()}`,
          }
        },
      })

      const inspect = page.locator('[data-cut-autopilot-inspect]').first()
      await probe(page, {
        surface,
        name: 'autopilot-open-inspect',
        actionId: 'autopilot-inspect',
        sel: inspect,
        group: drawer,
        groupName: 'autopilot-restored-report',
        doClick: async () => {
          await inspect.click()
          await page.locator('[data-cut-review-tab="receipts"][aria-selected="true"]').first().waitFor({
            state: 'visible',
            timeout: 5000,
          })
        },
        assertResult: async () => ({
          ok: await page.locator('[data-cut-review-tab="receipts"][aria-selected="true"]').first().isVisible()
            && await drawer.isVisible(),
          detail: `receipts tab=${await page.locator('[data-cut-review-tab="receipts"]').first().getAttribute('aria-selected')}; drawer retained=${await drawer.isVisible()}`,
        }),
      })
      await page.locator('[data-cut-autopilot-close]').first().click()
      await sleep(100)
    } finally {
      await restoreFixture(page)
    }
  }

  return { run }
}
