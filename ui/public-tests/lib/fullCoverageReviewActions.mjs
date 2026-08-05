// Exhaustive conditional Review-rail action coverage.
//
// The app ships a `?mock=1` Review harness for deterministic UI verification.
// It mounts the real App/Review/OpsFeed/Receipts/DiffView components, routes
// their requests through the normal HTTP client, and publishes op/receipt
// events through the normal WebSocket client. This module uses that installed
// surface to reach conditional evidence, judge, rebase, and refusal controls
// without depending on a particular render receipt or op graph on the test rig.

import { resolveCoverageAppUrl } from './fullCoverageAppUrl.mjs'

export function createReviewActionCoverage({
  app,
  probe,
  sleep,
  closeOverlays,
  ensureReviewPanel,
  reviewTab,
}) {
  const surface = 'review-actions'

  function mockUrl(activeApp) {
    const url = new URL(activeApp)
    url.searchParams.set('mock', '1')
    return url.toString()
  }

  async function installCapture(page) {
    await page.evaluate(() => {
      const target = window
      const originalFetch = window.fetch
      target.__fcvReviewActionFixture = {
        calls: [],
        failNextRebase: 0,
      }
      target.__fcvReviewActionOriginalFetch = originalFetch
      const fixture = target.__fcvReviewActionFixture
      const response = (body) => new Response(JSON.stringify(body), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      })
      window.fetch = async (...args) => {
        const input = args[0]
        const options = args[1]
        const url = typeof input === 'string' ? input : input?.url || ''
        let pathname = ''
        try { pathname = new URL(String(url), window.location.href).pathname } catch {}
        const match = /^\/api\/verb\/([a-z_.]+)$/.exec(pathname)
        let body = {}
        try { body = JSON.parse(options?.body || '{}') } catch {}
        if (match) fixture.calls.push({ name: match[1], args: body })
        if (match?.[1] === 'edit.restore' && body.mode === 'rebase' && fixture.failNextRebase > 0) {
          fixture.failNextRebase -= 1
          return response({
            ok: false,
            error: {
              code: 'rebase_dependency',
              message: 'selective restore refused because later edits depend on this operation',
              cause: 'op_000006 (transcript.cut_words via c1) depends on the target',
              suggested_action: 'use project.revert to restore an earlier point and drop later edits',
            },
          })
        }
        return originalFetch(...args)
      }
    })
  }

  async function fixture(page) {
    return page.evaluate(() => JSON.parse(JSON.stringify(window.__fcvReviewActionFixture)))
  }

  async function failNextRebase(page) {
    await page.evaluate(() => { window.__fcvReviewActionFixture.failNextRebase += 1 })
  }

  async function callSince(page, name, before) {
    const current = await fixture(page)
    return current.calls.slice(before).find((entry) => entry.name === name)
  }

  async function callCount(page) {
    return (await fixture(page)).calls.length
  }

  function matchesArgs(actual, expected) {
    return Object.entries(expected).every(([key, value]) => actual?.[key] === value)
  }

  async function openRebaseConfirm(page, opId) {
    const row = page.locator(`[data-cut-op="${opId}"]`).first()
    await row.scrollIntoViewIfNeeded()
    await row.focus()
    await row.locator('[data-cut-action="rebase-reject-op"]').click()
    await row.locator(`[data-cut-rebase-confirm="${opId}"]`).waitFor({
      state: 'visible',
      timeout: 5000,
    })
    return row
  }

  async function triggerRefusal(page, opId) {
    await failNextRebase(page)
    const row = await openRebaseConfirm(page, opId)
    await row.locator('[data-cut-action="rebase-confirm"]').click()
    await page.locator('[data-cut-undo-guidance]').waitFor({
      state: 'visible',
      timeout: 5000,
    })
  }

  async function run(page) {
    const activeApp = await resolveCoverageAppUrl(page, app)
    // Reset persisted local review verdicts before the mock App mounts. A page
    // navigation also resets the mock module's in-memory op/receipt fixtures.
    await page.evaluate(() => localStorage.removeItem('shellx-cut:reviewed:demo-cut'))
    await page.goto(mockUrl(activeApp), { waitUntil: 'domcontentloaded' })
    await sleep(150)
    await closeOverlays(page)
    await ensureReviewPanel(page)
    await page.locator('[data-cut-op="op_000002"]').waitFor({ state: 'visible', timeout: 8000 })
    await installCapture(page)

    try {
      // RECEIPTS: expand deterministic check evidence, then seek its measured
      // loudness window through the real shared playhead action.
      let panel = await reviewTab(page, 'receipts', '[data-cut-receipts]', 8000)
      await page.locator('[data-cut-receipt="r_0003"]').waitFor({ state: 'attached', timeout: 8000 })
      const failedReceipt = page.locator('[data-cut-receipt="r_0001"]').first()
      await failedReceipt.scrollIntoViewIfNeeded()
      await probe(page, {
        surface,
        name: 'receipt-check-evidence-toggle',
        actionId: 'receipt-check-toggle',
        sel: failedReceipt.locator('[data-cut-receipt-check-toggle="lufs"]'),
        group: failedReceipt,
        groupName: 'receipt-check-collapsed',
        doClick: async () => {
          await failedReceipt.locator('[data-cut-receipt-check-toggle="lufs"]').click()
          await failedReceipt.locator('[data-cut-evidence="lufs"]').waitFor({
            state: 'visible',
            timeout: 5000,
          })
        },
        assertResult: async () => ({
          ok: await failedReceipt.locator('[data-cut-evidence="lufs"]').isVisible()
            && (await failedReceipt.locator('[data-cut-evidence="lufs"]').textContent())?.includes('loudest_window_ms'),
          detail: 'Loudness evidence expanded with its measured timecode',
        }),
      })

      let before = await callCount(page)
      await probe(page, {
        surface,
        name: 'receipt-check-seek',
        actionId: 'seek',
        sel: failedReceipt.locator('[data-cut-seek="41200"]'),
        group: failedReceipt.locator('[data-cut-evidence="lufs"]'),
        groupName: 'receipt-check-evidence',
        doClick: async () => {
          before = await callCount(page)
          await failedReceipt.locator('[data-cut-seek="41200"]').click()
        },
        assertResult: async () => {
          const call = await callSince(page, 'ui.playhead', before)
          return {
            ok: call?.args?.at_ms === 41200,
            detail: `ui.playhead args=${JSON.stringify(call?.args)}`,
          }
        },
      })

      // JUDGE: reveal the completed perceptual review and exercise its own
      // issue seek, rather than treating all shared seek buttons as one path.
      const judgedReceipt = page.locator('[data-cut-receipt="r_0003"]').first()
      await judgedReceipt.scrollIntoViewIfNeeded()
      await probe(page, {
        surface,
        name: 'receipt-judge-toggle',
        actionId: 'receipt-judge-toggle',
        sel: judgedReceipt.locator('[data-cut-receipt-judge-toggle]'),
        group: judgedReceipt,
        groupName: 'receipt-judge-collapsed',
        doClick: async () => {
          await judgedReceipt.locator('[data-cut-receipt-judge-toggle]').click()
          await judgedReceipt.locator('[data-cut-judge-issue="0"]').waitFor({
            state: 'visible',
            timeout: 5000,
          })
        },
        assertResult: async () => ({
          ok: await judgedReceipt.locator('[data-cut-judge-issue="0"]').isVisible()
            && (await judgedReceipt.textContent())?.includes('darkens noticeably'),
          detail: 'Completed judge review expanded with the real issue and suggested fix',
        }),
      })

      await probe(page, {
        surface,
        name: 'receipt-judge-issue-seek',
        actionId: 'seek',
        sel: judgedReceipt.locator('[data-cut-seek="25300"]'),
        group: judgedReceipt.locator('[data-cut-judge-issue="0"]'),
        groupName: 'receipt-judge-issue',
        doClick: async () => {
          before = await callCount(page)
          await judgedReceipt.locator('[data-cut-seek="25300"]').click()
        },
        assertResult: async () => {
          const call = await callSince(page, 'ui.playhead', before)
          return {
            ok: call?.args?.at_ms === 25300,
            detail: `judge issue ui.playhead args=${JSON.stringify(call?.args)}`,
          }
        },
      })

      // DIFF: the mock project owns two checkpoints and a non-empty exact diff.
      panel = await reviewTab(page, 'diff', '[data-cut-diff]', 8000)
      await page.locator('[data-cut-diff-op="op_000002"]').waitFor({
        state: 'visible',
        timeout: 8000,
      })
      const diff = page.locator('[data-cut-diff]').first()
      await probe(page, {
        surface,
        name: 'diff-op-seek',
        actionId: 'diff-op',
        sel: page.locator('[data-cut-diff-op="op_000002"]'),
        group: diff,
        groupName: 'review-diff',
        doClick: async () => {
          before = await callCount(page)
          await page.locator('[data-cut-diff-op="op_000002"]').click()
        },
        assertResult: async () => {
          const call = await callSince(page, 'ui.playhead', before)
          return {
            ok: call?.args?.at_ms === 900
              && (await diff.textContent())?.includes('−3 clips'),
            detail: `diff op ui.playhead args=${JSON.stringify(call?.args)}; summary="${(await diff.textContent())?.replace(/\s+/g, ' ').trim().slice(0, 90)}"`,
          }
        },
      })

      // OPS: accept is a durable local review marker.
      panel = await reviewTab(page, 'ops', '[data-cut-ops-feed]', 8000)
      const acceptRow = panel.locator('[data-cut-op="op_000001"]').first()
      await acceptRow.scrollIntoViewIfNeeded()
      await acceptRow.focus()
      await probe(page, {
        surface,
        name: 'accept-operation',
        actionId: 'accept-op',
        sel: acceptRow.locator('[data-cut-action="accept-op"]'),
        group: acceptRow,
        groupName: 'review-op-accept',
        doClick: async () => {
          await acceptRow.focus()
          await acceptRow.locator('[data-cut-action="accept-op"]').click()
        },
        assertResult: async () => {
          const stored = await page.evaluate(() => JSON.parse(localStorage.getItem('shellx-cut:reviewed:demo-cut') || '{}'))
          return {
            ok: await acceptRow.evaluate((row) => row.classList.contains('rr-op--accepted'))
              && stored.op_000001 === 'accepted',
            detail: `accepted class=${await acceptRow.getAttribute('class')} stored=${stored.op_000001}`,
          }
        },
      })

      // Open a selective-undo confirmation and cancel it locally.
      let rebaseRow = await openRebaseConfirm(page, 'op_000002')
      await probe(page, {
        surface,
        name: 'rebase-cancel',
        actionId: 'rebase-cancel',
        sel: rebaseRow.locator('[data-cut-action="rebase-cancel"]'),
        group: rebaseRow,
        groupName: 'review-rebase-confirm',
        doClick: async () => { await rebaseRow.locator('[data-cut-action="rebase-cancel"]').click() },
        assertResult: async () => ({
          ok: await rebaseRow.locator('[data-cut-rebase-confirm="op_000002"]').count() === 0
            && await rebaseRow.locator('[data-cut-action="rebase-reject-op"]').isVisible(),
          detail: 'Selective-undo confirmation cancelled and original action restored',
        }),
      })

      // Re-open through the visible history-surgery affordance.
      await probe(page, {
        surface,
        name: 'rebase-reject-open',
        actionId: 'rebase-reject-op',
        sel: rebaseRow.locator('[data-cut-action="rebase-reject-op"]'),
        group: rebaseRow,
        groupName: 'review-rebase-idle',
        doClick: async () => {
          await rebaseRow.locator('[data-cut-action="rebase-reject-op"]').click()
          await rebaseRow.locator('[data-cut-rebase-confirm="op_000002"]').waitFor({
            state: 'visible',
            timeout: 5000,
          })
        },
        assertResult: async () => ({
          ok: await rebaseRow.locator('[data-cut-rebase-confirm="op_000002"]').isVisible(),
          detail: 'Selective-undo confirmation names the exact operation',
        }),
      })

      // Confirm and prove the full exact edit.restore request plus resulting row.
      await probe(page, {
        surface,
        name: 'rebase-confirm',
        actionId: 'rebase-confirm',
        sel: rebaseRow.locator('[data-cut-action="rebase-confirm"]'),
        group: rebaseRow,
        groupName: 'review-rebase-ready',
        doClick: async () => {
          before = await callCount(page)
          await rebaseRow.locator('[data-cut-action="rebase-confirm"]').click()
          await page.locator('[data-cut-op="op_000002"].rr-op--rejected').waitFor({
            state: 'visible',
            timeout: 5000,
          })
        },
        assertResult: async () => {
          const call = await callSince(page, 'edit.restore', before)
          const expected = {
            op_id: 'op_000002',
            rationale: 'user rebase-reject: op_000002 (transcript.cut_words) from history',
            mode: 'rebase',
          }
          return {
            ok: matchesArgs(call?.args, expected)
              && await page.locator('[data-cut-op="op_000002"].rr-op--rejected').isVisible(),
            detail: `edit.restore args=${JSON.stringify(call?.args)}; rejected=${await page.locator('[data-cut-op="op_000002"].rr-op--rejected').isVisible()}`,
          }
        },
      })

      // The plain Reject action uses tip mode (no mode property) and must mark
      // the chosen operation as rejected through the same live op event path.
      const rejectRow = panel.locator('[data-cut-op="op_000005"]').first()
      await rejectRow.scrollIntoViewIfNeeded()
      await rejectRow.focus()
      await probe(page, {
        surface,
        name: 'reject-operation',
        actionId: 'reject-op',
        sel: rejectRow.locator('[data-cut-action="reject-op"]'),
        group: rejectRow,
        groupName: 'review-op-reject',
        doClick: async () => {
          before = await callCount(page)
          await rejectRow.focus()
          await rejectRow.locator('[data-cut-action="reject-op"]').click()
          await page.locator('[data-cut-op="op_000005"].rr-op--rejected').waitFor({
            state: 'visible',
            timeout: 5000,
          })
        },
        assertResult: async () => {
          const call = await callSince(page, 'edit.restore', before)
          const expected = { op_id: 'op_000005', rationale: 'rail reject' }
          return {
            ok: matchesArgs(call?.args, expected)
              && await page.locator('[data-cut-op="op_000005"].rr-op--rejected').isVisible(),
            detail: `tip edit.restore args=${JSON.stringify(call?.args)}; rejected=${await page.locator('[data-cut-op="op_000005"].rr-op--rejected').isVisible()}`,
          }
        },
      })

      // A deterministic dependent-op refusal exposes the engine-guidance card.
      await triggerRefusal(page, 'op_000003')
      let guidance = page.locator('[data-cut-undo-guidance]').first()
      await probe(page, {
        surface,
        name: 'dismiss-restore-guidance',
        actionId: 'dismiss-guidance',
        sel: guidance.locator('[data-cut-action="dismiss-guidance"]'),
        group: guidance,
        groupName: 'review-restore-guidance-dismiss',
        doClick: async () => {
          await guidance.locator('[data-cut-action="dismiss-guidance"]').click()
          await guidance.waitFor({ state: 'detached', timeout: 5000 })
        },
        assertResult: async () => ({
          ok: await page.locator('[data-cut-undo-guidance]').count() === 0,
          detail: 'Restore guidance dismissed without an additional engine call',
        }),
      })

      // Reproduce the refusal, then exercise the history-changing recovery last.
      await triggerRefusal(page, 'op_000003')
      guidance = page.locator('[data-cut-undo-guidance]').first()
      await probe(page, {
        surface,
        name: 'guidance-revert',
        actionId: 'guidance-revert',
        sel: guidance.locator('[data-cut-action="guidance-revert"]'),
        group: guidance,
        groupName: 'review-restore-guidance',
        doClick: async () => {
          before = await callCount(page)
          await guidance.locator('[data-cut-action="guidance-revert"]').click()
          await guidance.waitFor({ state: 'detached', timeout: 5000 })
        },
        assertResult: async () => {
          const call = await callSince(page, 'project.revert', before)
          const expected = {
            to: 'op_000002',
            rationale: 'revert to op_000002 (from rebase guidance)',
          }
          return {
            ok: matchesArgs(call?.args, expected),
            detail: `project.revert args=${JSON.stringify(call?.args)}`,
          }
        },
      })

      // Collapse is last because it intentionally hides the whole Review rail.
      panel = await reviewTab(page, 'ops', '[data-cut-ops-feed]', 8000)
      await probe(page, {
        surface,
        name: 'collapse-review-rail',
        actionId: 'collapse-rail',
        sel: panel.locator('[data-cut-action="collapse-rail"]'),
        group: panel,
        groupName: 'review-before-collapse',
        doClick: async () => {
          await panel.locator('[data-cut-action="collapse-rail"]').click()
          await page.locator('[data-cut-action="expand-rail"]').waitFor({
            state: 'visible',
            timeout: 5000,
          })
        },
        assertResult: async () => ({
          ok: await page.locator('[data-cut-action="expand-rail"]').isVisible()
            && !(await panel.isVisible()),
          detail: `expand affordance visible=${await page.locator('[data-cut-action="expand-rail"]').isVisible()}; review visible=${await panel.isVisible()}`,
        }),
      })
    } finally {
      // Leave no fake fetch/WebSocket state for the next stateful section.
      await page.goto(activeApp, { waitUntil: 'domcontentloaded' }).catch(() => {})
      await sleep(250)
    }
  }

  return { run }
}
