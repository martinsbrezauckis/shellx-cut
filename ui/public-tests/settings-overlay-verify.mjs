// Focused runtime proof for the shared Settings blocking-overlay contract.
//
// RUN:
//   SWEEP_CUTD=http://127.0.0.1:6171 SWEEP_APP=http://127.0.0.1:6171 \
//     npm run verify-settings-overlay

import { chromium } from 'playwright'

const CUTD = process.env.SWEEP_CUTD || 'http://127.0.0.1:6171'
const APP = process.env.SWEEP_APP || CUTD

const results = []
function check(name, ok, detail = '') {
  results.push({ name, ok: Boolean(ok), detail })
  console.log(`${ok ? 'PASS' : 'FAIL'} ${name}${detail ? ` - ${detail}` : ''}`)
}

async function main() {
  const browser = await chromium.launch({ headless: true })
  const page = await browser.newPage({ viewport: { width: 1440, height: 900 } })
  const verbRequests = []
  const consoleErrors = []
  page.on('console', (message) => {
    if (message.type() === 'error') consoleErrors.push(message.text())
  })
  page.on('request', (request) => {
    if (request.method() === 'POST' && request.url().includes('/api/verb/')) {
      verbRequests.push(request.url().split('/api/verb/')[1] || request.url())
    }
  })

  try {
    await page.goto(APP, { waitUntil: 'networkidle' })
    const wizard = page.locator('[data-cut-wizard]')
    if (await wizard.count()) {
      await page.locator('[data-cut-wizard-dismiss]').click()
      await wizard.waitFor({ state: 'detached' })
    }

    const opener = page.locator('[data-cut-settings-btn]')
    await opener.waitFor({ state: 'visible', timeout: 8000 })
    await opener.focus()
    await opener.click()

    const dialog = page.locator('[data-cut-environment]')
    await dialog.waitFor({ state: 'visible', timeout: 8000 })
    check('settings-is-modal-dialog', await dialog.getAttribute('aria-modal') === 'true')
    check('settings-has-blocking-contract', await dialog.getAttribute('data-cut-blocking-overlay') !== null)
    check(
      'settings-focus-enters-dialog',
      await page.evaluate(() => Boolean(document.querySelector('[data-cut-environment]')?.contains(document.activeElement))),
      await page.evaluate(() => document.activeElement?.getAttribute('data-cut-environment-refresh') !== null ? 'refresh' : document.activeElement?.tagName || ''),
    )

    const background = await page.evaluate(() => {
      const root = document.querySelector('[data-cut-app-root]')
      const dialog = document.querySelector('[data-cut-environment]')
      return {
        marker: document.documentElement.dataset.cutBlockingOverlay,
        rows: root && dialog
          ? [...root.children]
              .filter((element) => !element.contains(dialog))
              .map((element) => ({
                inert: element.inert,
                ariaHidden: element.getAttribute('aria-hidden'),
              }))
          : [],
      }
    })
    check(
      'settings-background-is-inert',
      background.marker === '1' && background.rows.length > 0 &&
        background.rows.every((row) => row.inert === true && row.ariaHidden === 'true'),
      JSON.stringify(background),
    )

    check('settings-category-navigation-complete', (await dialog.locator('[data-cut-settings-category]').count()) === 10)
    check('settings-opens-on-overview', (await dialog.locator('[data-cut-settings-body="overview"]').count()) === 1)
    const search = dialog.locator('[data-cut-settings-search]')
    await search.fill('MCP')
    check('settings-search-finds-agent-control', (await dialog.locator('[data-cut-settings-search-result="agent-control"]').count()) === 1)
    await search.fill('')
    await dialog.locator('[data-cut-settings-category="editing"]').click()
    await dialog.locator('[data-cut-settings-body="editing"]').waitFor()

    const shortcutDetails = dialog.locator('[data-cut-keymap-details]')
    check(
      'settings-shortcuts-collapsed-by-default',
      (await shortcutDetails.count()) === 1 && (await shortcutDetails.getAttribute('open')) === null,
    )
    check(
      'settings-fixed-recording-keys-not-remappable',
      (await dialog.locator('[data-cut-keymap-bind^="recording."]').count()) === 0 &&
        (await dialog.locator('[data-cut-keymap-fixed-row="recording.toggle"]').count()) === 1,
    )

    const focusableState = await dialog.evaluate((root) => {
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
      elements.forEach((element, index) => element.setAttribute('data-cut-overlay-test-focus', String(index)))
      const describe = (element) => ({
        tag: element?.tagName || null,
        text: element?.textContent?.trim().slice(0, 80) || null,
        testId: element?.getAttribute('data-cut-overlay-test-focus') || null,
      })
      return {
        count: elements.length,
        first: describe(elements[0]),
        last: describe(elements[elements.length - 1]),
        lastIndex: elements.length - 1,
      }
    })
    const focusableCount = focusableState.count
    check(
      'settings-has-focusable-controls',
      focusableCount > 1,
      `count=${focusableCount} first=${JSON.stringify(focusableState.first)} last=${JSON.stringify(focusableState.last)}`,
    )
    if (focusableCount > 1) {
      const firstFocusable = dialog.locator('[data-cut-overlay-test-focus="0"]')
      const lastFocusable = dialog.locator(`[data-cut-overlay-test-focus="${focusableState.lastIndex}"]`)
      await lastFocusable.focus()
      await page.keyboard.press('Tab')
      check(
        'settings-tab-wraps-last-to-first',
        await firstFocusable.evaluate((element) => element === document.activeElement),
        await page.evaluate(() => document.activeElement?.outerHTML?.slice(0, 240) || ''),
      )
      await firstFocusable.focus()
      await page.keyboard.press('Shift+Tab')
      check(
        'settings-shift-tab-wraps-first-to-last',
        await lastFocusable.evaluate((element) => element === document.activeElement),
        await page.evaluate(() => document.activeElement?.outerHTML?.slice(0, 240) || ''),
      )
    }

    verbRequests.length = 0
    for (const key of [
      ']',
      'Space',
      'Delete',
      'F9',
      'F10',
      'F11',
      'Shift+F11',
      'F12',
      'Control+Shift+C',
      'Control+K',
      '?',
      'R',
      '\\',
    ]) {
      await page.keyboard.press(key)
    }
    await page.waitForTimeout(250)
    const mutationRequests = verbRequests.filter((name) =>
      /^(screen_record\.|edit\.|project\.(save|undo|redo)|ui\.select)/.test(name),
    )
    check('settings-contains-editor-shortcuts', await dialog.isVisible())
    check('settings-shortcuts-dispatch-no-mutations', mutationRequests.length === 0, mutationRequests.join(','))
    check('settings-shortcuts-open-no-background-surface',
      (await page.locator('[data-cut-panel="comments"]').count()) === 0 &&
      (await page.locator('.cmdk, [data-cut-keymap]').count()) === 0,
    )

    await page.keyboard.press('Escape')
    await dialog.waitFor({ state: 'detached', timeout: 3000 })
    await page.waitForTimeout(40)
    check('settings-escape-closes-once', (await page.locator('[data-cut-environment]').count()) === 0)
    check('settings-returns-focus-to-opener', await opener.evaluate((element) => element === document.activeElement))
    check(
      'settings-restores-background',
      await page.evaluate(() => {
        const root = document.querySelector('[data-cut-app-root]')
        return !document.documentElement.dataset.cutBlockingOverlay &&
          Boolean(root) &&
          [...root.children].every((element) => element.inert === false && element.getAttribute('aria-hidden') !== 'true')
      }),
    )

    await opener.click()
    await dialog.waitFor({ state: 'visible', timeout: 3000 })
    await page.locator('[data-cut-environment-scrim]').click({ position: { x: 20, y: 20 } })
    await dialog.waitFor({ state: 'detached', timeout: 3000 })
    check('settings-scrim-closes-top-overlay', (await page.locator('[data-cut-environment]').count()) === 0)
    check('settings-runtime-console-clean', consoleErrors.length === 0, consoleErrors.join(' | '))
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
