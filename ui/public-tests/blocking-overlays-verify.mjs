// Runtime proof for the shared blocking-overlay contract. The source contract
// test inventories every dialog; this test exercises representative keyboard,
// nested-app, drawer, and document-body portal surfaces in the real bundle.
//
// RUN:
//   SWEEP_CUTD=http://127.0.0.1:6171 SWEEP_APP=http://127.0.0.1:6171 \
//     npm run verify-blocking-overlays

import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { chromium } from 'playwright'

const HERE = dirname(fileURLToPath(import.meta.url))
const REPO = resolve(HERE, '../..')
const SOURCE_CLIP = resolve(REPO, 'testdata/insert_clip.mp4')
const CUTD = process.env.SWEEP_CUTD || 'http://127.0.0.1:6171'
const APP = process.env.SWEEP_APP || CUTD
const EVIDENCE_DIR = process.env.CUT_OVERLAY_EVIDENCE_DIR || ''
const temp = mkdtempSync(join(tmpdir(), 'cut-blocking-overlays-'))
const projectDir = join(temp, 'blocking-overlays.cutproj')
const checks = []

function check(name, ok, detail = '') {
  checks.push({ name, ok: Boolean(ok), detail })
  console.log(`${ok ? 'PASS' : 'FAIL'} ${name}${detail ? ` - ${detail}` : ''}`)
}

async function verb(name, args = {}) {
  const response = await fetch(`${CUTD}/api/verb/${name}`, {
    method: 'POST',
    headers: { 'content-type': 'application/json', 'x-cut-actor': 'human:ui:blocking-overlays-verify' },
    body: JSON.stringify(args),
    signal: AbortSignal.timeout(30_000),
  })
  return response.json()
}

async function waitForProject(predicate, timeoutMs = 20_000) {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    const state = await verb('project.state')
    if (state.ok && predicate(state.result)) return state.result
    await new Promise((resolveWait) => setTimeout(resolveWait, 120))
  }
  throw new Error('timed out waiting for project state')
}

function dialogLocator(page, surfaceSelector) {
  return page.locator(`${surfaceSelector}[role="dialog"], ${surfaceSelector} [role="dialog"]`).first()
}

async function assertOpenContract(page, name, surfaceSelector) {
  const surface = page.locator(surfaceSelector).first()
  await surface.waitFor({ state: 'visible', timeout: 8_000 })
  const dialog = dialogLocator(page, surfaceSelector)
  await dialog.waitFor({ state: 'visible', timeout: 8_000 })

  const state = await dialog.evaluate((root) => ({
    ariaModal: root.getAttribute('aria-modal'),
    blocking: root.hasAttribute('data-cut-blocking-overlay'),
    focusInside: root.contains(document.activeElement),
    marker: document.documentElement.dataset.cutBlockingOverlay || '',
    inertRegions: document.querySelectorAll('[inert][aria-hidden="true"]').length,
    portal: !document.querySelector('[data-cut-app-root]')?.contains(root),
  }))
  check(`${name}-modal-contract`, state.ariaModal === 'true' && state.blocking, JSON.stringify(state))
  check(`${name}-focus-enters`, state.focusInside, JSON.stringify(state))
  check(`${name}-background-isolated`, Number(state.marker) >= 1 && state.inertRegions >= 1, JSON.stringify(state))

  const focusState = await dialog.evaluate((root) => {
    const selector = 'a[href],button:not([disabled]),input:not([disabled]),select:not([disabled]),textarea:not([disabled]),summary,[tabindex]:not([tabindex="-1"])'
    const elements = [...root.querySelectorAll(selector)].filter((element) => {
      const rect = element.getBoundingClientRect()
      if (rect.width <= 0 || rect.height <= 0 || element.getAttribute('aria-hidden') === 'true') return false
      for (let ancestor = element.parentElement; ancestor && ancestor !== root; ancestor = ancestor.parentElement) {
        if (ancestor instanceof HTMLDetailsElement && !ancestor.open) {
          const summary = [...ancestor.children].find((child) => child.tagName === 'SUMMARY')
          if (!summary?.contains(element)) return false
        }
      }
      return true
    })
    elements.forEach((element, index) => element.setAttribute('data-cut-blocking-test-focus', String(index)))
    return { count: elements.length, last: elements.length - 1 }
  })

  if (focusState.count > 1) {
    const first = dialog.locator('[data-cut-blocking-test-focus="0"]')
    const last = dialog.locator(`[data-cut-blocking-test-focus="${focusState.last}"]`)
    await last.focus()
    await page.keyboard.press('Tab')
    const forward = await first.evaluate((element) => element === document.activeElement)
    await first.focus()
    await page.keyboard.press('Shift+Tab')
    const reverse = await last.evaluate((element) => element === document.activeElement)
    check(`${name}-focus-wraps`, forward && reverse, `forward=${forward} reverse=${reverse} count=${focusState.count}`)
  } else {
    await dialog.focus()
    await page.keyboard.press('Tab')
    check(`${name}-empty-focus-stays-contained`, await dialog.evaluate((root) => root.contains(document.activeElement)), `count=${focusState.count}`)
  }
  return { surface, dialog, state }
}

async function closeWithEscape(page, name, surfaceSelector, opener = null) {
  await page.keyboard.press('Escape')
  await page.locator(surfaceSelector).waitFor({ state: 'detached', timeout: 5_000 })
  await page.waitForTimeout(40)
  check(`${name}-escape-closes`, (await page.locator(surfaceSelector).count()) === 0)
  if (opener) {
    check(`${name}-returns-focus`, await opener.evaluate((element) => element === document.activeElement))
  }
  check(
    `${name}-restores-background`,
    await page.evaluate(() => !document.documentElement.dataset.cutBlockingOverlay && document.querySelectorAll('[inert][aria-hidden="true"]').length === 0),
  )
}

let browser
let ownsProject = false
let safeToRemove = true
try {
  const created = await verb('project.create', {
    name: 'blocking-overlays',
    dir: projectDir,
    settings: { width: 1280, height: 720, fps: 30 },
  })
  if (!created.ok) throw new Error(`project.create failed: ${JSON.stringify(created.error)}`)
  ownsProject = true
  const imported = await verb('media.import', { path: SOURCE_CLIP, proxy: false })
  if (!imported.ok) throw new Error(`media.import failed: ${JSON.stringify(imported.error)}`)
  const project = await waitForProject((state) => Object.values(state.assets || {}).some((asset) => asset.path === SOURCE_CLIP))
  const sourceAsset = Object.entries(project.assets).find(([, asset]) => asset.path === SOURCE_CLIP)?.[0]
  if (!sourceAsset) throw new Error('source-monitor fixture asset was not registered')

  browser = await chromium.launch({ headless: true })
  const page = await browser.newPage({ viewport: { width: 1440, height: 900 } })
  const consoleErrors = []
  page.on('console', (message) => { if (message.type() === 'error') consoleErrors.push(message.text()) })
  await page.goto(APP, { waitUntil: 'networkidle' })
  const wizard = page.locator('[data-cut-wizard]')
  if (await wizard.count()) {
    await page.locator('[data-cut-wizard-dismiss]').click()
    await wizard.waitFor({ state: 'detached' })
  }

  const keyboardOpener = page.locator('[data-cut-settings-btn]')
  await keyboardOpener.focus()
  await page.keyboard.press('Control+K')
  await assertOpenContract(page, 'command-palette', '[data-cut-command-palette]')
  await closeWithEscape(page, 'command-palette', '[data-cut-command-palette]', keyboardOpener)

  await keyboardOpener.focus()
  await page.keyboard.press('?')
  const keymap = await assertOpenContract(page, 'shortcut-reference', '[data-cut-keymap]')
  check('shortcut-reference-is-nested', keymap.state.portal === false, JSON.stringify(keymap.state))
  await closeWithEscape(page, 'shortcut-reference', '[data-cut-keymap]', keyboardOpener)

  const assembleOpener = page.locator('[data-cut-assemble-btn]')
  await assembleOpener.click()
  await assertOpenContract(page, 'assemble', '[data-cut-assemble]')
  await closeWithEscape(page, 'assemble', '[data-cut-assemble]')

  const titleOpener = page.locator('[data-cut-title-btn]')
  await titleOpener.click()
  await assertOpenContract(page, 'title-preset', '[data-cut-title]')
  await page.locator('[data-cut-title-mode="free"]').click()
  check(
    'title-free-placement-becomes-nonmodal',
    await page.locator('[data-cut-title]').evaluate((root) => root.getAttribute('aria-modal') === 'false'
      && !root.hasAttribute('data-cut-blocking-overlay')
      && !document.documentElement.dataset.cutBlockingOverlay
      && document.querySelectorAll('[inert][aria-hidden="true"]').length === 0),
  )
  await page.keyboard.press('Escape')
  await page.locator('[data-cut-title]').waitFor({ state: 'detached', timeout: 5_000 })
  check('title-free-placement-escape-closes', true)

  await page.locator('[data-cut-export-btn]').click()
  await page.locator('[data-cut-render-queue-open]').click()
  await assertOpenContract(page, 'render-queue', '[data-cut-render-queue]')
  await page.locator('[data-cut-render-queue]').click({ position: { x: 4, y: 4 } })
  await page.locator('[data-cut-render-queue]').waitFor({ state: 'detached', timeout: 5_000 })
  check('render-queue-scrim-closes', true)
  check('render-queue-restores-background', await page.evaluate(() => !document.documentElement.dataset.cutBlockingOverlay))

  await page.locator('[data-cut-render-opts]').click()
  await page.locator('[data-cut-render-aspect]').selectOption('9:16')
  await page.locator('[data-cut-director-open]').click()
  await assertOpenContract(page, 'director', '[data-cut-director]')
  await closeWithEscape(page, 'director', '[data-cut-director]')

  await page.locator('[data-cut-left-tab="assets"]').click()
  const sourceOpener = page.locator(`[data-cut-source-monitor-open="${sourceAsset}"]`)
  await sourceOpener.waitFor({ state: 'visible', timeout: 8_000 })
  await sourceOpener.click()
  const sourceMonitor = await assertOpenContract(page, 'source-monitor', `[data-cut-source-monitor="${sourceAsset}"]`)
  check('source-monitor-is-portal', sourceMonitor.state.portal === true, JSON.stringify(sourceMonitor.state))
  await closeWithEscape(page, 'source-monitor', `[data-cut-source-monitor="${sourceAsset}"]`, sourceOpener)

  check('blocking-overlays-console-clean', consoleErrors.length === 0, consoleErrors.join(' | '))

  const receipt = {
    schema: 'shellx-cut/blocking-overlays-verify@1',
    ok: checks.every((entry) => entry.ok),
    cutd: CUTD,
    app: APP,
    checks,
    consoleErrors,
  }
  if (EVIDENCE_DIR) writeFileSync(join(EVIDENCE_DIR, 'blocking-overlays-receipt.json'), `${JSON.stringify(receipt, null, 2)}\n`)
  console.log(`SUMMARY pass=${checks.filter((entry) => entry.ok).length} fail=${checks.filter((entry) => !entry.ok).length}`)
  if (!receipt.ok) process.exitCode = 1
} finally {
  await browser?.close().catch(() => {})
  if (ownsProject) {
    const closed = await verb('project.close').catch(() => null)
    if (!closed?.ok) safeToRemove = false
    if (safeToRemove) {
      const forgotten = await verb('project.forget', { path: projectDir }).catch(() => null)
      if (!forgotten?.ok) {
        safeToRemove = false
        process.stderr.write(`blocking-overlays-verify: could not forget disposable project: ${JSON.stringify(forgotten?.error)}\n`)
        process.exitCode = 1
      }
    }
  }
  if (safeToRemove) rmSync(temp, { recursive: true, force: true })
}
