// Cross-viewport Settings qualification against a real served Cut UI. This is
// focused source/runtime evidence; it does not replace the final installed
// every-action matrix on the native macOS, Windows, and Linux surfaces.

import { mkdir } from 'node:fs/promises'
import { resolve } from 'node:path'
import { chromium } from 'playwright'

const APP = process.env.SWEEP_APP || 'http://127.0.0.1:6171'
const EVIDENCE_DIR = process.env.CUT_SETTINGS_EVIDENCE_DIR || ''
const CATEGORIES = [
  'overview',
  'general',
  'editing',
  'video-performance',
  'ai-transcription',
  'recording',
  'services-integrations',
  'agent-control',
  'storage-privacy',
  'about',
]
const results = []

function check(name, ok, detail = '') {
  results.push({ name, ok: Boolean(ok), detail })
  console.log(`${ok ? 'PASS' : 'FAIL'} ${name}${detail ? ` - ${detail}` : ''}`)
}

async function screenshot(page, name) {
  if (!EVIDENCE_DIR) return
  await mkdir(EVIDENCE_DIR, { recursive: true })
  await page.screenshot({ path: resolve(EVIDENCE_DIR, `${name}.png`), fullPage: false })
}

async function preparePage(browser, width, height, theme = 'dark') {
  const page = await browser.newPage({ viewport: { width, height } })
  const consoleErrors = []
  page.on('console', (message) => {
    if (message.type() === 'error') consoleErrors.push(message.text())
  })
  await page.addInitScript((selectedTheme) => {
    if (selectedTheme === 'light') localStorage.setItem('cut.theme', 'light')
    else localStorage.removeItem('cut.theme')
  }, theme)
  await page.goto(APP, { waitUntil: 'networkidle' })
  const wizard = page.locator('[data-cut-wizard]')
  if (await wizard.count()) {
    await page.locator('[data-cut-wizard-dismiss]').click()
    await wizard.waitFor({ state: 'detached' })
  }
  await page.locator('[data-cut-settings-btn]').click()
  const dialog = page.locator('[data-cut-environment]')
  await dialog.waitFor({ state: 'visible' })
  return { page, dialog, consoleErrors }
}

async function layoutState(page, dialog) {
  return dialog.evaluate((root) => {
    const rect = root.getBoundingClientRect()
    const close = root.querySelector('[data-cut-environment-close]')?.getBoundingClientRect()
    const body = root.querySelector('.settings-body')
    return {
      dialogOverflow: root.scrollWidth - root.clientWidth,
      bodyOverflow: body ? body.scrollWidth - body.clientWidth : -1,
      dialogInViewport: rect.left >= -1 && rect.right <= window.innerWidth + 1 &&
        rect.top >= -1 && rect.bottom <= window.innerHeight + 1,
      closeInViewport: Boolean(close) && close.left >= 0 && close.right <= window.innerWidth &&
        close.top >= 0 && close.bottom <= window.innerHeight,
      mobileSelectVisible: (root.querySelector('[data-cut-settings-category-select]')?.getClientRects().length ?? 0) > 0,
      navVisible: (root.querySelector('[data-cut-settings-categories]')?.getClientRects().length ?? 0) > 0,
    }
  })
}

async function verifyViewport(browser, width, height, theme, label) {
  const { page, dialog, consoleErrors } = await preparePage(browser, width, height, theme)
  try {
    const layout = await layoutState(page, dialog)
    check(`${label}-dialog-contained`, layout.dialogInViewport, JSON.stringify(layout))
    check(`${label}-no-horizontal-overflow`, layout.dialogOverflow <= 1 && layout.bodyOverflow <= 1, JSON.stringify(layout))
    check(`${label}-close-visible`, layout.closeInViewport, JSON.stringify(layout))
    check(`${label}-navigation-mode`,
      width <= 1120 ? layout.mobileSelectVisible && !layout.navVisible : !layout.mobileSelectVisible && layout.navVisible,
      JSON.stringify(layout),
    )
    check(`${label}-accessible-shell`,
      await dialog.getAttribute('role') === 'dialog' &&
      await dialog.getAttribute('aria-label') === 'Settings' &&
      (await dialog.locator('nav[aria-label="Settings categories"]').count()) === 1,
    )
    check(`${label}-console-clean`, consoleErrors.length === 0, consoleErrors.join(' | '))
    await screenshot(page, label)
  } finally {
    await page.close()
  }
}

async function verifyCategoriesAndTasks(browser) {
  const { page, dialog, consoleErrors } = await preparePage(browser, 1440, 900, 'dark')
  try {
    let routed = 0
    for (const category of CATEGORIES) {
      await dialog.locator(`[data-cut-settings-category="${category}"]`).click()
      if ((await dialog.locator(`[data-cut-settings-body="${category}"]`).count()) === 1) routed++
    }
    check('settings-all-ten-categories-route', routed === CATEGORIES.length, `routed=${routed}`)

    await dialog.locator('[data-cut-settings-category="overview"]').click()
    check('settings-overview-is-task-bounded',
      (await dialog.locator('[data-cut-settings-overview-row]').count()) === 5 &&
      (await dialog.locator('[data-cut-settings-overview-row="video"] button').count()) === 1,
    )
    await dialog.locator('[data-cut-settings-overview-row="destination"] button').click()
    check('settings-export-task-routes-to-general',
      (await dialog.locator('[data-cut-settings-body="general"] [data-cut-export-default-folder]').count()) === 1,
    )
    await dialog.locator('[data-cut-settings-category="recording"]').click()
    check('settings-recording-task-is-discoverable',
      (await dialog.locator('[data-cut-settings-open-recording]').count()) === 1 &&
      (await dialog.locator('[data-cut-settings-recording-keys]').count()) === 1,
    )
    await dialog.locator('[data-cut-settings-category="agent-control"]').click()
    await dialog.locator('[data-cut-agent-control-config]').waitFor({ state: 'attached' })
    check('settings-mcp-task-is-discoverable',
      (await dialog.locator('[data-cut-agent-control-config]').count()) === 1 &&
      (await dialog.locator('[data-cut-agent-control-test]').count()) === 1,
    )
    const clientGuide = await dialog.locator('[data-cut-agent-control-client-guide]').textContent()
    check('settings-mcp-client-setup-is-explained',
      clientGuide?.includes('CALI') &&
      clientGuide.includes('mcpServers') &&
      clientGuide.includes('shellx-cut') &&
      clientGuide.includes('same Cut engine'),
    )
    await dialog.locator('[data-cut-settings-category="storage-privacy"]').click()
    const network = dialog.locator('[data-cut-network-activity]')
    const networkText = await network.textContent()
    check('settings-network-activity-is-disclosed',
      networkText?.includes('contacts GitHub when it opens, and then every 6 hours while it stays open') &&
      networkText.includes('normal request metadata such as your IP address') &&
      networkText.includes('sends no project, media, edit history, or analytics payload'),
    )
    check('settings-update-opt-out-is-installed-only',
      await network.locator('[data-cut-update-check-on-launch]').isDisabled() &&
      networkText?.includes('Available in the installed desktop app.'),
    )
    await screenshot(page, 'settings-storage-privacy')
    check('settings-category-task-console-clean', consoleErrors.length === 0, consoleErrors.join(' | '))
  } finally {
    await page.close()
  }
}

async function verifyFirstRunAndErrors(browser) {
  const page = await browser.newPage({ viewport: { width: 1100, height: 680 } })
  const consoleErrors = []
  page.on('console', (message) => {
    if (message.type() === 'error') consoleErrors.push(message.text())
  })
  await page.route('**/api/verb/system.doctor', async (route) => {
    const response = await route.fetch()
    const payload = await response.json()
    payload.result.essential_ok = false
    payload.result.cards = payload.result.cards.map((card) => {
      if (card.id === 'ffmpeg' || card.id === 'ffprobe') {
        return {
          ...card,
          status: 'missing',
          source: 'missing',
          hint: 'Required video tools are missing. Install them, then re-scan before importing or exporting.',
        }
      }
      if (card.id === 'gpu-encode') {
        return {
          ...card,
          status: 'degraded',
          hint: 'The selected executable could not be verified after a machine move.',
          details: {
            ...card.details,
            resolved: 'C:\\Users\\A very long profile name\\AppData\\Local\\ShellX Cut\\tools\\ffmpeg\\nested build folder\\ffmpeg.exe',
          },
        }
      }
      if (card.kind === 'service') {
        return {
          ...card,
          status: 'missing',
          hint: 'This optional service is unavailable. Start it or update its endpoint, then re-scan.',
        }
      }
      return card
    })
    await route.fulfill({ response, json: payload })
  })

  try {
    await page.goto(APP, { waitUntil: 'networkidle' })
    const wizard = page.locator('[data-cut-wizard]')
    await wizard.waitFor({ state: 'visible' })
    check('settings-first-run-wizard-explains-essential-gap',
      await wizard.getAttribute('aria-label') === 'ShellX Cut setup' &&
      await wizard.getAttribute('data-cut-wizard-essential-ok') === 'false' &&
      (await wizard.locator('[data-cut-setup-step]').count()) === 3 &&
      (await wizard.locator('[data-cut-wizard-dismiss]').textContent())?.trim() === 'Continue without',
    )
    await screenshot(page, 'first-run-multiple-errors')
    await wizard.locator('[data-cut-wizard-dismiss]').click()
    await page.locator('[data-cut-settings-btn]').click()
    const dialog = page.locator('[data-cut-environment]')
    await dialog.locator('[data-cut-settings-overview-row="video"]').waitFor()
    check('settings-overview-prioritizes-essential-error',
      (await dialog.locator('[data-cut-settings-overview-row="video"]').textContent())?.includes('needs setup'),
    )
    await dialog.locator('[data-cut-settings-category-select]').selectOption('video-performance')
    const advanced = dialog.locator('[data-cut-env-advanced="gpu-encode"]')
    await advanced.evaluate((element) => {
      element.open = true
    })
    const longPath = advanced.locator('dd[title*="A very long profile name"]')
    await longPath.scrollIntoViewIfNeeded()
    const state = await layoutState(page, dialog)
    check('settings-long-path-and-multiple-errors-stay-bounded',
      state.dialogOverflow <= 1 && state.bodyOverflow <= 1 &&
      await longPath.isVisible() &&
      (await longPath.evaluate((element) => element.scrollWidth - element.clientWidth)) <= 1,
      JSON.stringify(state),
    )
    await screenshot(page, 'settings-long-path-errors')
    check('settings-error-profile-console-clean', consoleErrors.length === 0, consoleErrors.join(' | '))
  } finally {
    await page.close()
  }
}

async function main() {
  const browser = await chromium.launch({ headless: true })
  try {
    await verifyViewport(browser, 1100, 680, 'dark', 'settings-1100x680-dark')
    await verifyViewport(browser, 1280, 760, 'light', 'settings-1280x760-light')
    await verifyViewport(browser, 1440, 900, 'dark', 'settings-1440x900-dark')
    await verifyViewport(browser, 1920, 1080, 'light', 'settings-1920x1080-light')
    await verifyViewport(browser, 550, 340, 'dark', 'settings-effective-200pct')
    await verifyCategoriesAndTasks(browser)
    await verifyFirstRunAndErrors(browser)
  } finally {
    await browser.close()
  }

  const failed = results.filter((result) => !result.ok)
  console.log(`SUMMARY pass=${results.length - failed.length} fail=${failed.length}`)
  if (failed.length > 0) process.exitCode = 1
}

main().catch((error) => {
  console.error(error?.stack || String(error))
  process.exit(1)
})
