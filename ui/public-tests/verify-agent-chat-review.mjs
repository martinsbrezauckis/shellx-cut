import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { chromium } from 'playwright'

const CUTD = process.env.SWEEP_CUTD || 'http://127.0.0.1:6277'
const APP = process.env.SWEEP_APP || CUTD
const temp = mkdtempSync(join(tmpdir(), 'cut-chat-review-'))
const projectDir = join(temp, 'chat-review.cutproj')
const checks = []

function check(name, pass, detail = '') {
  checks.push({ name, pass, detail })
  console.log(`${pass ? 'PASS' : 'FAIL'}  ${name}${detail ? `  ${detail}` : ''}`)
}

async function verb(name, args = {}) {
  const response = await fetch(`${CUTD}/api/verb/${name}`, {
    method: 'POST',
    headers: { 'content-type': 'application/json', 'x-cut-actor': 'human:chat-review-gate:ui' },
    body: JSON.stringify(args),
    signal: AbortSignal.timeout(30_000),
  })
  return response.json()
}

let browser
try {
  const created = await verb('project.create', { name: 'chat-review', dir: projectDir })
  if (!created.ok) throw new Error(created.error?.message || 'project.create failed')
  const checkpoint = await verb('project.checkpoint', { name: 'before-chat-review-gate' })
  if (!checkpoint.ok) throw new Error(checkpoint.error?.message || 'project.checkpoint failed')
  const baseline = checkpoint.result.checkpoint.id
  const marker = await verb('edit.add_marker', { at_ms: 1200, label: 'Agent review gate' })
  if (!marker.ok) throw new Error(marker.error?.message || 'edit.add_marker failed')
  const ops = await verb('project.ops')
  const tip = ops.result.ops.at(-1).op_id
  const diff = await verb('project.diff', { from: baseline, to: tip })
  if (!diff.ok) throw new Error(diff.error?.message || 'project.diff failed')

  browser = await chromium.launch({ headless: true })
  const page = await browser.newPage({ viewport: { width: 1180, height: 760 } })
  page.setDefaultTimeout(7_000)
  const consoleErrors = []
  page.on('console', (message) => {
    if (message.type() === 'error') consoleErrors.push(message.text())
  })

  const request = 'Add a review marker at 1.2 seconds'
  let sentArgs = null
  let revertArgs = null
  page.on('request', (request) => {
    if (request.url().endsWith('/api/verb/project.revert')) revertArgs = request.postDataJSON()
  })
  await page.route('**/api/verb/agent.chat', async (route) => {
    sentArgs = route.request().postDataJSON()
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        ok: true,
        result: {
          ok: true,
          agent: 'gate',
          reply: 'Added the review marker.',
          actions: [{ op_id: tip, verb: 'edit.add_marker' }],
          attachments: [],
          plan: {
            request,
            reference_ids: [],
            policy: ['inspect the open project', 'apply only reversible editing verbs', 'return op-log receipts'],
          },
          review: {
            turn_id: 'chat-review-gate',
            baseline,
            checkpoint: baseline,
            tip,
            diff: diff.result,
            diff_error: null,
            revert_safe: true,
            concurrent_actions: [],
          },
          cost_usd: null,
        },
      }),
    })
  })

  await page.goto(APP, { waitUntil: 'domcontentloaded' })
  const dismissWizard = page.locator('[data-cut-wizard-dismiss]')
  await dismissWizard.waitFor({ state: 'visible', timeout: 3_000 }).catch(() => {})
  if (await dismissWizard.isVisible()) await dismissWizard.click()
  const expandRail = page.locator('[data-cut-action="expand-rail"]')
  if (await expandRail.count()) await expandRail.click()
  await page.locator('[data-cut-right-tab="chat"]').click()
  await page.locator('[data-cut-chat]').waitFor()
  await page.locator('[data-cut-chat-input]').fill(request)
  await page.locator('[data-cut-chat-send]').click()
  const review = page.locator('[data-cut-chat-review="pending"]')
  await review.waitFor()
  if (process.env.CHAT_REVIEW_SCREENSHOT) {
    await page.screenshot({ path: process.env.CHAT_REVIEW_SCREENSHOT, fullPage: true })
  }

  check('agent invocation intercepted', sentArgs?.message === request, JSON.stringify(sentArgs))
  check('turn exposes plan and review status', await page.locator('[data-cut-chat-plan]').getByText(request).isVisible())
  check('turn is group-revertible', await review.getAttribute('data-cut-chat-revert-safe') === 'true')

  await page.locator('[data-cut-chat-preview]').click()
  check('Preview focuses the real monitor', await page.locator('[data-cut-panel="preview"]').evaluate((node) => document.activeElement === node))

  await page.locator('[data-cut-chat-diff]').click()
  await page.locator('[data-cut-review-tab="diff"][aria-selected="true"]').waitFor()
  await page.locator('[data-cut-diff-from]').waitFor()
  check('Diff opens exact turn baseline', await page.locator('[data-cut-diff-from]').inputValue() === baseline)
  check('Diff opens exact turn tip', await page.locator('[data-cut-diff-to]').inputValue() === tip)

  await page.locator('[data-cut-chat-accept]').click()
  await page.locator('[data-cut-chat-review="accepted"]').waitFor()
  const stored = await page.evaluate((name) => JSON.parse(localStorage.getItem(`shellx-cut:reviewed:${name}`) || '{}'), 'chat-review')
  check('Accept persists Review marker', stored[tip] === 'accepted', JSON.stringify(stored))
  await page.locator('[data-cut-review-tab="ops"]').click()
  await page.locator(`[data-cut-op="${tip}"] .rr-op__check`).waitFor()
  check('Accept updates mounted Review rail', await page.locator(`[data-cut-op="${tip}"] .rr-op__check`).isVisible())

  await page.locator('[data-cut-chat-retry]').click()
  await page.locator('[data-cut-chat-review="retry"]').waitFor()
  check('Try again restores request without auto-send', await page.locator('[data-cut-chat-input]').inputValue() === request)
  const state = await verb('project.state')
  check('Try again atomically reverts turn first', state.ok && !state.result.markers.some((entry) => entry.label === 'Agent review gate'))
  check('Try again guards revert at the reviewed tip', revertArgs?.if_tip === tip, JSON.stringify(revertArgs))
  const rejected = await page.evaluate((name) => JSON.parse(localStorage.getItem(`shellx-cut:reviewed:${name}`) || '{}'), 'chat-review')
  check('Revert shares rejected marker with Review', rejected[tip] === 'rejected', JSON.stringify(rejected))
  check('browser console clean', consoleErrors.length === 0, consoleErrors.join(' | '))
} catch (error) {
  check('gate completed', false, error instanceof Error ? error.stack || error.message : String(error))
} finally {
  await browser?.close().catch(() => {})
  rmSync(temp, { recursive: true, force: true })
}

if (checks.some((item) => !item.pass)) process.exitCode = 1
else console.log(`PASS agent-chat-review (${checks.length} checks)`)
