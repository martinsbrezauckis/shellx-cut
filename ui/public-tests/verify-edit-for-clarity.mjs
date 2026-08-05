import { mkdtempSync, rmSync } from 'node:fs'
import { spawnSync } from 'node:child_process'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { chromium } from 'playwright'

const CUTD = process.env.SWEEP_CUTD || 'http://127.0.0.1:6277'
const APP = process.env.SWEEP_APP || CUTD
const temp = mkdtempSync(join(tmpdir(), 'cut-edit-for-clarity-'))
const projectDir = join(temp, 'edit-for-clarity.cutproj')
const media = join(temp, 'spoken-reference.mp4')
const checks = []

function check(name, pass, detail = '') {
  checks.push({ name, pass, detail })
  console.log(`${pass ? 'PASS' : 'FAIL'}  ${name}${detail ? `  ${detail}` : ''}`)
}

async function verb(name, args = {}) {
  const response = await fetch(`${CUTD}/api/verb/${name}`, {
    method: 'POST',
    headers: { 'content-type': 'application/json', 'x-cut-actor': 'human:clarity-gate:ui' },
    body: JSON.stringify(args),
    signal: AbortSignal.timeout(30_000),
  })
  return response.json()
}

let browser
try {
  const generated = spawnSync(process.env.FFMPEG || 'ffmpeg', [
    '-hide_banner', '-loglevel', 'error',
    '-f', 'lavfi', '-i', 'color=c=0x35506b:s=320x180:d=1',
    '-f', 'lavfi', '-i', 'anullsrc=r=48000:cl=stereo',
    '-shortest', '-c:v', 'libx264', '-pix_fmt', 'yuv420p', '-c:a', 'aac', '-y', media,
  ], { encoding: 'utf8' })
  if (generated.status !== 0) throw new Error(`test media generation failed: ${generated.stderr.trim()}`)

  const created = await verb('project.create', { name: 'edit-for-clarity', dir: projectDir })
  if (!created.ok) throw new Error(created.error?.message || 'project.create failed')
  const imported = await verb('media.import', { path: media, proxy: false })
  if (!imported.ok) throw new Error(imported.error?.message || 'media.import failed')
  const assetId = imported.result.asset_id

  browser = await chromium.launch({ headless: true })
  const page = await browser.newPage({ viewport: { width: 1180, height: 760 } })
  page.setDefaultTimeout(7_000)
  const consoleErrors = []
  page.on('console', (message) => {
    if (message.type() === 'error') consoleErrors.push(message.text())
  })

  await page.goto(APP, { waitUntil: 'domcontentloaded' })
  const dismissWizard = page.locator('[data-cut-wizard-dismiss]')
  await dismissWizard.waitFor({ state: 'visible', timeout: 3_000 }).catch(() => {})
  if (await dismissWizard.isVisible()) await dismissWizard.click()
  await page.locator('[data-cut-recipes-btn]').click()
  await page.locator('[data-cut-recipe="edit-for-clarity"]').click()
  const detail = page.locator('[data-cut-recipe-detail="edit-for-clarity"]')
  await detail.waitFor()

  const intensity = page.locator('[data-cut-recipe-param-input="intensity"]')
  check('recipe is discoverable', await page.getByRole('dialog', { name: 'Edit for clarity' }).isVisible())
  check('asset is selected from the open project', await page.locator('[data-cut-recipe-param-input="asset"]').inputValue() === assetId)
  check('intensity defaults to Natural', await intensity.inputValue() === 'natural')
  check('intensity exposes three levels', await intensity.locator('option').allTextContents().then((items) => items.join('|') === 'Calm|Natural|Tight'))
  check('retake cleanup is visible in the plan', await page.locator('[data-cut-recipe-stage="retakes"]').getByText('Remove repeated takes').isVisible())
  check('Run waits for preview', await page.locator('[data-cut-recipe-run]').isDisabled())

  await intensity.selectOption('jumpy')
  const beforePreview = await verb('project.ops')
  const beforePreviewTip = beforePreview.result.ops.at(-1)?.op_id
  await page.locator('[data-cut-recipe-preview]').click()
  await page.locator('[data-cut-recipe-plan-status="planned"]').waitFor()
  check('preview returns all five resolved stages', await page.locator('[data-cut-recipe-plan-op]').count() === 5)
  await page.locator('[data-cut-recipe-plan-technical="tighten"]').click()
  check('preview resolves Tight intensity', await page.locator('[data-cut-recipe-plan-op="tighten"]').getByText('aggressiveness: jumpy', { exact: false }).isVisible())
  check('Run unlocks only after current preview', !(await page.locator('[data-cut-recipe-run]').isDisabled()))
  if (process.env.CLARITY_SCREENSHOT) {
    await page.waitForTimeout(300)
    await page.screenshot({ path: process.env.CLARITY_SCREENSHOT, fullPage: true })
  }

  const after = await verb('project.ops')
  check('preview is non-mutating', after.result.ops.at(-1)?.op_id === beforePreviewTip)
  check('browser console clean', consoleErrors.length === 0, consoleErrors.join(' | '))
} catch (error) {
  check('gate completed', false, error instanceof Error ? error.stack || error.message : String(error))
} finally {
  await browser?.close().catch(() => {})
  rmSync(temp, { recursive: true, force: true })
}

if (checks.some((item) => !item.pass)) process.exitCode = 1
else console.log(`PASS edit-for-clarity (${checks.length} checks)`)
