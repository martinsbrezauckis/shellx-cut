import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { chromium } from 'playwright'

const CUTD = process.env.SWEEP_CUTD || 'http://127.0.0.1:6214'
const APP = process.env.SWEEP_APP || CUTD
const temp = mkdtempSync(join(tmpdir(), 'cut-sequences-'))
const projectDir = join(temp, 'sequence-rig.cutproj')
const checks = []

function check(name, pass, detail) {
  checks.push({ name, pass, detail })
  console.log(`${pass ? 'PASS' : 'FAIL'}  ${name}  ${detail}`)
}

async function verb(name, args = {}) {
  const response = await fetch(`${CUTD}/api/verb/${name}`, {
    method: 'POST',
    headers: { 'content-type': 'application/json', 'x-cut-actor': 'human:ui:ui' },
    body: JSON.stringify(args),
    signal: AbortSignal.timeout(30_000),
  })
  return response.json()
}

async function state() {
  const envelope = await verb('project.state')
  if (!envelope.ok) throw new Error(envelope.error?.message || 'project.state failed')
  return envelope.result
}

async function waitForState(predicate, timeoutMs = 10_000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const project = await state()
    if (predicate(project)) return project
    await new Promise((resolveWait) => setTimeout(resolveWait, 100))
  }
  throw new Error('timed out waiting for project state')
}

let browser
try {
  const created = await verb('project.create', {
    name: 'sequence-rig',
    dir: projectDir,
    settings: { width: 1280, height: 720, fps: 30 },
  })
  if (!created.ok) throw new Error(created.error?.message || 'project.create failed')
  await verb('edit.add_marker', { at_ms: 100, label: 'main' })

  browser = await chromium.launch({ headless: true })
  const page = await browser.newPage({ viewport: { width: 1600, height: 900 } })
  await page.goto(APP, { waitUntil: 'domcontentloaded' })
  const trigger = page.locator('[data-cut-sequence-trigger]')
  await trigger.waitFor()
  check('topbar exposes active Main sequence', await trigger.isVisible() && (await trigger.textContent()).includes('Main'), `label=${await trigger.textContent()}`)

  await trigger.click()
  await page.locator('[data-cut-sequence-new]').click()
  await page.locator('[data-cut-sequence-name]').fill('Review')
  await page.locator('[data-cut-sequence-create] button[type="submit"]').click()
  await waitForState((project) => project.active_sequence === 'seq2' && project.markers.length === 0)
  await page.locator('[data-cut-sequence-menu]').waitFor({ state: 'detached' })
  await page.locator('[data-cut-sequence-active="seq2"]').waitFor()
  check('create Empty activates an independent timeline', true, 'seq2 active; markers=0')

  await verb('edit.add_marker', { at_ms: 200, label: 'review' })
  await trigger.click()
  const [switchResponse] = await Promise.all([
    page.waitForResponse((response) => response.url().includes('/api/verb/project.sequence_switch')),
    page.locator('[data-cut-sequence-switch="seq1"]').click(),
  ])
  const switchEnvelope = await switchResponse.json()
  if (!switchEnvelope.ok) throw new Error(`UI sequence switch failed: ${JSON.stringify(switchEnvelope.error)}`)
  const main = await waitForState((project) => (project.active_sequence ?? 'seq1') === 'seq1')
  await page.locator('[data-cut-sequence-menu]').waitFor({ state: 'detached' })
  check('switch restores Main composition', main.markers.length === 1 && main.markers[0].label === 'main', `markers=${main.markers.map((marker) => marker.label).join(',')}`)

  await trigger.click()
  await page.locator('[data-cut-sequence-rename="seq2"]').click()
  await page.locator('[data-cut-sequence-rename-input="seq2"]').fill('Review cut')
  await page.locator('[data-cut-sequence-rename-input="seq2"]').press('Enter')
  await page.locator('[data-cut-sequence-row="seq2"]').filter({ hasText: 'Review cut' }).waitFor()
  check('rename updates the inactive sequence in place', true, 'seq2=Review cut')

  await page.locator('[data-cut-sequence-new]').click()
  await page.locator('[data-cut-sequence-name]').fill('Temporary')
  await page.locator('[data-cut-sequence-from="active"]').click()
  await page.locator('[data-cut-sequence-create] button[type="submit"]').click()
  await waitForState((project) => project.active_sequence === 'seq3')
  await trigger.click()
  await page.locator('[data-cut-sequence-switch="seq1"]').click()
  await waitForState((project) => (project.active_sequence ?? 'seq1') === 'seq1')
  page.once('dialog', (dialog) => dialog.accept())
  await trigger.click()
  await page.locator('[data-cut-sequence-delete="seq3"]').click()
  const afterDelete = await waitForState((project) => !project.sequences.some((sequence) => sequence.id === 'seq3'))
  check('delete removes only the inactive sequence', afterDelete.sequences.length === 2 && Object.keys(afterDelete.assets).length === 0, `sequences=${afterDelete.sequences.map((sequence) => sequence.name).join(',')}`)

  await verb('project.close')
  const reopened = await verb('project.open', { path: projectDir })
  if (!reopened.ok) throw new Error(reopened.error?.message || 'project.open failed')
  await page.reload({ waitUntil: 'domcontentloaded' })
  await page.locator('[data-cut-sequence-active="seq1"]').waitFor()
  const persisted = await verb('project.sequence_list')
  check('sequence bank survives close and reopen', persisted.ok && persisted.result.sequences.length === 2 && persisted.result.sequences[1].name === 'Review cut', `active=${persisted.result?.active_sequence} count=${persisted.result?.sequences?.length}`)

  await page.setViewportSize({ width: 1100, height: 680 })
  await trigger.click()
  const layout = await page.evaluate(() => {
    const menu = document.querySelector('[data-cut-sequence-menu]')?.getBoundingClientRect()
    return {
      rootOverflow: document.documentElement.scrollWidth - document.documentElement.clientWidth,
      menuInside: !!menu && menu.left >= 0 && menu.right <= innerWidth && menu.top >= 0 && menu.bottom <= innerHeight,
      triggerVisible: !!document.querySelector('[data-cut-sequence-trigger]')?.getClientRects().length,
    }
  })
  check('sequence control fits the supported minimum window', layout.rootOverflow === 0 && layout.menuInside && layout.triggerVisible, JSON.stringify(layout))
} finally {
  await browser?.close().catch(() => {})
  await verb('project.close').catch(() => {})
  await verb('project.forget', { path: projectDir }).catch(() => {})
  rmSync(temp, { recursive: true, force: true })
}

if (checks.some((entry) => !entry.pass)) process.exitCode = 1
