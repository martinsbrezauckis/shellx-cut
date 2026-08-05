// Runtime behavior proof for the compact/searchable Settings shortcut manager.
//
// RUN:
//   SWEEP_APP=http://127.0.0.1:6171 npm run verify-settings-shortcuts

import { chromium } from 'playwright'

const APP = process.env.SWEEP_APP || 'http://127.0.0.1:6171'
const results = []

function check(name, ok, detail = '') {
  results.push({ name, ok: Boolean(ok), detail })
  console.log(`${ok ? 'PASS' : 'FAIL'} ${name}${detail ? ` - ${detail}` : ''}`)
}

async function text(locator) {
  return (await locator.textContent())?.replace(/\s+/g, ' ').trim() || ''
}

async function openCategory(dialog, id) {
  const button = dialog.locator(`[data-cut-settings-category="${id}"]`)
  if (await button.isVisible()) {
    await button.click()
  } else {
    await dialog.locator('[data-cut-settings-category-select]').selectOption(id)
  }
}

async function main() {
  const browser = await chromium.launch({ headless: true })
  const page = await browser.newPage({ viewport: { width: 1100, height: 680 } })
  const consoleErrors = []
  page.on('console', (message) => {
    if (message.type() === 'error') consoleErrors.push(message.text())
  })
  await page.addInitScript(() => localStorage.removeItem('cut.keymap'))

  try {
    await page.goto(APP, { waitUntil: 'networkidle' })
    const wizard = page.locator('[data-cut-wizard]')
    if (await wizard.count()) {
      await page.locator('[data-cut-wizard-dismiss]').click()
      await wizard.waitFor({ state: 'detached' })
    }
    await page.locator('[data-cut-settings-btn]').click()
    const dialog = page.locator('[data-cut-environment]')
    await dialog.waitFor({ state: 'visible' })
    await openCategory(dialog, 'editing')

    const details = dialog.locator('[data-cut-keymap-details]')
    check('shortcuts-collapsed-on-entry', (await details.getAttribute('open')) === null)
    check('shortcuts-summary-command-count', await text(details.locator('[data-cut-keymap-command-count]')) === '23 commands')
    check('shortcuts-summary-zero-changes', await text(details.locator('[data-cut-keymap-changed-count]')) === '0 changed')
    check('shortcuts-summary-zero-conflicts', await text(details.locator('[data-cut-keymap-conflict-count]')) === '0 conflicts')
    check('shortcuts-reset-all-hidden-at-defaults', (await details.locator('[data-cut-action="keymap-reset-all"]').count()) === 0)

    await details.locator('> summary').click()
    check('shortcuts-expanded-all-command-rows', (await details.locator('[data-cut-keymap-row], [data-cut-keymap-fixed-row]').count()) === 23)
    check('shortcuts-fixed-rows-are-visible-and-locked',
      (await details.locator('[data-cut-keymap-fixed-row]').count()) === 6 &&
      (await details.locator('[data-cut-keymap-bind^="recording."]').count()) === 0,
    )
    check('shortcuts-empty-status-filters-disabled',
      await details.locator('[data-cut-keymap-filter="changed"]').isDisabled() &&
      await details.locator('[data-cut-keymap-filter="conflicts"]').isDisabled(),
    )

    const search = details.locator('[data-cut-keymap-search]')
    await search.fill('marker')
    check('shortcuts-search-spans-editable-and-fixed', await text(details.locator('[data-cut-keymap-results]')) === 'Showing 4 of 23')
    await search.fill('')
    await details.locator('[data-cut-keymap-group]').selectOption('timeline')
    check('shortcuts-group-filter-is-bounded', await text(details.locator('[data-cut-keymap-results]')) === 'Showing 11 of 23')
    await openCategory(dialog, 'general')
    await openCategory(dialog, 'editing')
    check('shortcuts-view-state-survives-category-navigation',
      (await details.getAttribute('open')) !== null &&
      await details.locator('[data-cut-keymap-group]').inputValue() === 'timeline' &&
      await text(details.locator('[data-cut-keymap-results]')) === 'Showing 11 of 23',
    )
    await details.locator('[data-cut-keymap-group]').selectOption('all')

    const playPause = details.locator('[data-cut-keymap-bind="preview.playPause"]')
    await playPause.click()
    check('shortcuts-capture-is-inline', await text(playPause) === 'Press a key…')
    await page.keyboard.press('Alt+1')
    check('shortcuts-remap-applies-live', await text(playPause) === 'Alt+1')
    check('shortcuts-summary-updates-after-remap', await text(details.locator('[data-cut-keymap-changed-count]')) === '1 changed')
    check('shortcuts-reset-all-appears-after-remap', (await details.locator('[data-cut-action="keymap-reset-all"]').count()) === 1)

    await details.locator('[data-cut-keymap-filter="changed"]').click()
    check('shortcuts-changed-filter-is-exact',
      await text(details.locator('[data-cut-keymap-results]')) === 'Showing 1 of 23' &&
      (await details.locator('[data-cut-keymap-row="preview.playPause"]').count()) === 1,
    )
    await details.locator('[data-cut-keymap-filter="all"]').click()

    const split = details.locator('[data-cut-keymap-bind="timeline.split"]')
    await split.click()
    await page.keyboard.press('Alt+1')
    check('shortcuts-collision-is-explained',
      (await text(details.locator('[data-cut-keymap-note]'))).includes('already used by Play / pause'),
      await text(details.locator('[data-cut-keymap-note]')),
    )
    check('shortcuts-collision-keeps-safe-capture', await text(split) === 'Press a key…')
    await page.keyboard.press('Escape')
    check('shortcuts-escape-cancels-capture', await text(split) === 'S')

    await split.click()
    await page.keyboard.press('F9')
    check('shortcuts-fixed-native-collision-is-explained',
      (await text(details.locator('[data-cut-keymap-note]'))).includes('already used by Start / stop recording'),
      await text(details.locator('[data-cut-keymap-note]')),
    )
    await page.keyboard.press('Alt+2')
    check('shortcuts-second-safe-remap-applies', await text(split) === 'Alt+2')

    const resetSplit = details.locator('[data-cut-keymap-clear="timeline.split"]')
    await resetSplit.click()
    check('shortcuts-reset-one-restores-default', await text(split) === 'S')
    check('shortcuts-reset-one-returns-focus', await split.evaluate((element) => element === document.activeElement))
    check('shortcuts-reset-one-updates-count', await text(details.locator('[data-cut-keymap-changed-count]')) === '1 changed')

    await details.locator('[data-cut-action="keymap-reset-all"]').click()
    check('shortcuts-reset-all-restores-defaults',
      await text(playPause) === 'Space' &&
      await text(details.locator('[data-cut-keymap-changed-count]')) === '0 changed',
    )
    await page.waitForFunction(() => document.activeElement?.hasAttribute('data-cut-keymap-search'))
    check('shortcuts-reset-all-returns-focus', await search.evaluate((element) => element === document.activeElement))
    check(
      'shortcuts-storage-cleared-after-reset',
      await page.evaluate(() => localStorage.getItem('cut.keymap') === null),
    )
    check('shortcuts-runtime-console-clean', consoleErrors.length === 0, consoleErrors.join(' | '))
  } finally {
    await page.evaluate(() => localStorage.removeItem('cut.keymap')).catch(() => {})
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
