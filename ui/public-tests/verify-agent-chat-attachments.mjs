import { mkdtempSync, rmSync } from 'node:fs'
import { spawnSync } from 'node:child_process'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { chromium } from 'playwright'

const CUTD = process.env.SWEEP_CUTD || 'http://127.0.0.1:6277'
const APP = process.env.SWEEP_APP || CUTD
const temp = mkdtempSync(join(tmpdir(), 'cut-chat-attachments-'))
const projectDir = join(temp, 'chat-attachments.cutproj')
const MEDIA = process.env.CHAT_ATTACHMENT_MEDIA || join(temp, 'reference.mp4')
const checks = []

function check(name, pass, detail = '') {
  checks.push({ name, pass, detail })
  console.log(`${pass ? 'PASS' : 'FAIL'}  ${name}${detail ? `  ${detail}` : ''}`)
}

async function verb(name, args = {}) {
  const response = await fetch(`${CUTD}/api/verb/${name}`, {
    method: 'POST',
    headers: { 'content-type': 'application/json', 'x-cut-actor': 'test:chat-attachments' },
    body: JSON.stringify(args),
    signal: AbortSignal.timeout(30_000),
  })
  return response.json()
}

let browser
try {
  let assetId = process.env.CHAT_ATTACHMENT_ASSET_ID || ''
  if (!assetId && !process.env.CHAT_ATTACHMENT_MEDIA) {
    const generated = spawnSync(process.env.FFMPEG || 'ffmpeg', [
      '-hide_banner', '-loglevel', 'error', '-f', 'lavfi', '-i', 'color=c=blue:s=320x180:d=1',
      '-c:v', 'libx264', '-pix_fmt', 'yuv420p', '-y', MEDIA,
    ], { encoding: 'utf8' })
    if (generated.status !== 0) throw new Error(`test media generation failed: ${generated.stderr.trim()}`)
  }
  if (!assetId) {
    const created = await verb('project.create', { name: 'chat-attachments', dir: projectDir })
    if (!created.ok) throw new Error(created.error?.message || 'project.create failed')
    const imported = await verb('media.import', { path: MEDIA, proxy: false })
    if (!imported.ok) throw new Error(imported.error?.message || 'media.import failed')
    assetId = imported.result.asset_id
  }

  browser = await chromium.launch({ headless: true })
  const page = await browser.newPage({ viewport: { width: 1100, height: 680 } })
  page.setDefaultTimeout(5_000)
  const consoleErrors = []
  page.on('console', (message) => {
    if (message.type() === 'error') consoleErrors.push(message.text())
  })

  let sentArgs = null
  await page.route('**/api/verb/agent.chat', async (route) => {
    sentArgs = route.request().postDataJSON()
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({
        ok: true,
        result: {
          ok: false,
          agent: null,
          reply: 'No agent was started by this UI gate.',
          reason: 'No agent was started by this UI gate.',
          error: 'not_available',
          actions: [],
          attachments: [assetId],
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

  const attach = page.locator('[data-cut-chat-attach]')
  await attach.click()
  check('picker opens', await page.locator('[data-cut-chat-attachment-menu]').isVisible())
  await page.locator(`[data-cut-chat-attachment="${assetId}"]`).click()
  check('registered asset selected', await page.locator(`[data-cut-chat-attachment-remove="${assetId}"]`).isVisible())
  await attach.click()
  await page.locator(`[data-cut-chat-attachment-remove="${assetId}"]`).click()
  check('selected asset removable', await page.locator('[data-cut-chat-attachments]').count() === 0)

  await attach.click()
  await page.locator(`[data-cut-chat-attachment="${assetId}"]`).click()
  await page.locator('[data-cut-chat-input]').fill('Use this reference for the edit')
  await page.locator('[data-cut-chat-send]').click()
  await page.locator(`[data-cut-chat-turn-attachment="${assetId}"]`).waitFor()
  const interceptedReply = page.getByText('No agent was started by this UI gate.').first()
  await interceptedReply.waitFor()

  check('request carries registered ID', JSON.stringify(sentArgs?.attachments) === JSON.stringify([assetId]), JSON.stringify(sentArgs))
  check('request excludes source path', !JSON.stringify(sentArgs).includes(MEDIA), JSON.stringify(sentArgs))
  check('turn keeps attachment receipt', await page.locator(`[data-cut-chat-turn-attachment="${assetId}"]`).isVisible())
  check('agent call was intercepted', await interceptedReply.isVisible())
  check('browser console clean', consoleErrors.length === 0, consoleErrors.join(' | '))
} catch (error) {
  check('gate completed', false, error instanceof Error ? error.stack || error.message : String(error))
} finally {
  await browser?.close().catch(() => {})
  rmSync(temp, { recursive: true, force: true })
}

if (checks.some((item) => !item.pass)) process.exitCode = 1
else console.log(`PASS agent-chat-attachments (${checks.length} checks)`)
