import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { chromium } from 'playwright'

const CUTD = process.env.SWEEP_CUTD || 'http://127.0.0.1:6277'
const APP = process.env.SWEEP_APP || CUTD
const temp = mkdtempSync(join(tmpdir(), 'cut-chat-prompts-'))
const projectDir = join(temp, 'chat-prompts.cutproj')
const checks = []

function check(name, pass, detail = '') {
  checks.push({ name, pass, detail })
  console.log(`${pass ? 'PASS' : 'FAIL'}  ${name}${detail ? `  ${detail}` : ''}`)
}

async function verb(name, args = {}) {
  const response = await fetch(`${CUTD}/api/verb/${name}`, {
    method: 'POST',
    headers: { 'content-type': 'application/json', 'x-cut-actor': 'human:chat-prompts-gate:ui' },
    body: JSON.stringify(args),
    signal: AbortSignal.timeout(30_000),
  })
  return response.json()
}

let browser
try {
  const created = await verb('project.create', { name: 'chat-prompts', dir: projectDir })
  if (!created.ok) throw new Error(created.error?.message || 'project.create failed')

  browser = await chromium.launch({ headless: true })
  const page = await browser.newPage({ viewport: { width: 1100, height: 680 } })
  page.setDefaultTimeout(7_000)
  const consoleErrors = []
  let agentCalls = 0
  page.on('console', (message) => {
    if (message.type() === 'error') consoleErrors.push(message.text())
  })
  await page.route('**/api/verb/agent.chat', async (route) => {
    agentCalls += 1
    await route.abort()
  })

  await page.goto(APP, { waitUntil: 'domcontentloaded' })
  const dismissWizard = page.locator('[data-cut-wizard-dismiss]')
  await dismissWizard.waitFor({ state: 'visible', timeout: 3_000 }).catch(() => {})
  if (await dismissWizard.isVisible()) await dismissWizard.click()
  const expandRail = page.locator('[data-cut-action="expand-rail"]')
  if (await expandRail.count()) await expandRail.click()
  await page.locator('[data-cut-right-tab="chat"]').click()
  await page.locator('[data-cut-chat]').waitFor()

  const trigger = page.locator('[data-cut-chat-prompt-library]')
  await trigger.click()
  const menu = page.locator('[data-cut-chat-prompt-menu]')
  await menu.waitFor()
  check('library exposes four outcome groups', await page.locator('[data-cut-chat-prompt-group]').count() === 4)
  check('library exposes eight curated prompts', await page.locator('[data-cut-chat-prompt]').count() === 8)

  const clarity = page.locator('[data-cut-chat-prompt="edit-for-clarity"]')
  const clarityPrompt = (await clarity.locator('small').textContent())?.trim() ?? ''
  await clarity.click()
  check('selection closes the menu', await menu.count() === 0)
  check('selection pre-fills the exact editable prompt', await page.locator('[data-cut-chat-input]').inputValue() === clarityPrompt)
  check('selection never auto-sends an agent turn', agentCalls === 0)

  await trigger.click()
  await menu.waitFor()
  await page.keyboard.press('Escape')
  check('Escape closes the library', await menu.count() === 0)

  await page.locator('[data-cut-chat-chip="Dub to Latvian"]').click()
  check('quick action replaces the composer text', (await page.locator('[data-cut-chat-input]').inputValue()).startsWith('Dub the timeline audio into Latvian'))
  check('quick action also remains prefill-only', agentCalls === 0)

  if (process.env.CHAT_PROMPTS_SCREENSHOT) {
    await trigger.click()
    await menu.waitFor()
    await page.waitForTimeout(200)
    await page.screenshot({ path: process.env.CHAT_PROMPTS_SCREENSHOT, fullPage: true })
  }
  check('browser console clean', consoleErrors.length === 0, consoleErrors.join(' | '))
} catch (error) {
  check('gate completed', false, error instanceof Error ? error.stack || error.message : String(error))
} finally {
  await browser?.close().catch(() => {})
  rmSync(temp, { recursive: true, force: true })
}

if (checks.some((item) => !item.pass)) process.exitCode = 1
else console.log(`PASS agent-chat-prompts (${checks.length} checks)`)
