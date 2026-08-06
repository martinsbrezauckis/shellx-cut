// Installed/native Settings scenarios for the exhaustive verifier.
//
// Settings used to have only a separate Chromium layout check. These actions
// run through the same Page adapter and receipt model as the three-host native
// sweep, so a control that works in Chromium but not WKWebView/WebView2/
// WebKitGTK is release-visible.

import { resolveCoverageAppUrl } from './fullCoverageAppUrl.mjs'
import { createSettingsEnvironmentCoverage } from './fullCoverageSettingsEnvironment.mjs'
import { createSettingsTaskCoverage } from './fullCoverageSettingsTasks.mjs'
import { createSettingsUpdateCoverage } from './fullCoverageSettingsUpdate.mjs'

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

const OVERVIEW_ROUTES = {
  video: 'video-performance',
  destination: 'general',
  ai: 'ai-transcription',
  agent: 'agent-control',
}

export function createFullCoverageSettings({
  app,
  probe,
  verb,
  captureVerbResp,
  sleep,
  closeOverlays,
  nativePickerClickNa = '',
}) {
  if (typeof probe !== 'function' || typeof verb !== 'function' || typeof captureVerbResp !== 'function') {
    throw new TypeError('createFullCoverageSettings requires probe, verb, and captureVerbResp')
  }
  const runEnvironmentCoverage = createSettingsEnvironmentCoverage({
    probe,
    verb,
    sleep,
    nativePickerClickNa,
  })
  const runTaskCoverage = createSettingsTaskCoverage({ probe, verb, captureVerbResp, sleep })

  async function waitFor(locator, state = 'visible', timeout = 12_000) {
    await locator.waitFor({ state, timeout })
    return locator
  }

  async function openSettings(page, category = '') {
    if ((await page.locator('[data-cut-environment]').count()) === 0) {
      await page.locator('[data-cut-setup-btn]').click()
      await waitFor(page.locator('[data-cut-environment]'))
    }
    if (category) {
      const compactCategory = page.locator('[data-cut-settings-category-select]')
      if (await compactCategory.isVisible().catch(() => false)) {
        await compactCategory.selectOption(category)
      } else {
        await page.locator(`[data-cut-settings-category="${category}"]`).click()
      }
      await waitFor(page.locator(`[data-cut-settings-body="${category}"]`))
    }
    return page.locator('[data-cut-environment]').first()
  }

  const runUpdateSurfaceCoverage = createSettingsUpdateCoverage({
    probe,
    sleep,
    openSettings,
    waitFor,
  })

  async function secSettings(page) {
    const S = 'settings'
    const initialTheme = await page.evaluate(() => localStorage.getItem('cut.theme') === 'light' ? 'light' : 'dark')
    await closeOverlays(page)
    await page.locator('[data-cut-wizard-dismiss]').click().catch(() => {})
    await page.locator('[data-cut-environment-close]').click().catch(() => {})

    await probe(page, {
      surface: S,
      name: 'settings-open',
      actionId: 'setup-btn',
      sel: page.locator('[data-cut-setup-btn]'),
      group: page.locator('[data-cut-panel="topbar"]').first(),
      groupName: 'topbar-settings',
      doClick: async () => {
        await page.locator('[data-cut-setup-btn]').click()
        await waitFor(page.locator('[data-cut-environment]'))
      },
      assertResult: async () => ({
        ok: (await page.locator('[data-cut-environment-open="true"]').count()) === 1,
        detail: 'Settings dialog mounted and reported open',
      }),
    })

    let panel = await openSettings(page)
    for (const category of CATEGORIES) {
      await probe(page, {
        surface: S,
        name: `settings-category-${category}`,
        actionId: `settings-category:${category}`,
        sel: page.locator(`[data-cut-settings-category="${category}"]`),
        group: panel,
        groupName: 'settings-shell',
        doClick: async () => {
          await page.locator(`[data-cut-settings-category="${category}"]`).click()
          await waitFor(page.locator(`[data-cut-settings-body="${category}"]`))
        },
        assertResult: async () => ({
          ok: (await page.locator(`[data-cut-settings-body="${category}"]`).count()) === 1,
          detail: `active settings body=${category}`,
        }),
      })
    }

    const originalViewport = page.viewportSize()
    try {
      await page.setViewportSize({ width: 1100, height: Math.max(680, originalViewport?.height || 0) })
      await waitFor(page.locator('[data-cut-settings-category-select]'))
      await probe(page, {
        surface: S,
        name: 'settings-category-select',
        actionId: 'settings-category-select',
        sel: page.locator('[data-cut-settings-category-select]'),
        group: panel,
        groupName: 'settings-shell-narrow',
        doClick: async () => {
          await page.locator('[data-cut-settings-category-select]').selectOption('agent-control')
          await waitFor(page.locator('[data-cut-settings-body="agent-control"]'))
        },
        assertResult: async () => ({
          ok: (await page.locator('[data-cut-settings-category-select]').inputValue()) === 'agent-control'
            && (await page.locator('[data-cut-settings-body="agent-control"]').count()) === 1,
          detail: 'minimum-width settings category selector routed to Agent control',
        }),
      })
    } finally {
      await page.setViewportSize(originalViewport || { width: 1600, height: 900 })
      const restoredCategoryControl = (originalViewport?.width || 1600) <= 1120
        ? page.locator('[data-cut-settings-category-select]')
        : page.locator('[data-cut-settings-category="overview"]')
      await waitFor(restoredCategoryControl)
    }

    await openSettings(page, 'overview')
    const search = page.locator('[data-cut-settings-search]')
    await probe(page, {
      surface: S, name: 'settings-search', actionId: 'settings-search',
      sel: search, group: panel, groupName: 'settings-shell',
      doClick: async () => { await search.fill('mcp'); await sleep(120) },
      assertResult: async () => {
        const count = await page.locator('[data-cut-settings-search-result]').count()
        return { ok: count > 0, detail: `mcp search destinations=${count}` }
      },
    })
    await probe(page, {
      surface: S, name: 'settings-search-result-agent', actionId: 'settings-search-result:agent-control',
      sel: page.locator('[data-cut-settings-search-result="agent-control"]'),
      group: panel, groupName: 'settings-shell',
      doClick: async () => {
        await page.locator('[data-cut-settings-search-result="agent-control"]').click()
        await waitFor(page.locator('[data-cut-settings-body="agent-control"]'))
      },
      assertResult: async () => ({
        ok: (await search.inputValue()) === '' &&
          (await page.locator('[data-cut-settings-body="agent-control"]').count()) === 1,
        detail: 'search result routed to Agent control and cleared query',
      }),
    })

    for (const [id, destination] of Object.entries(OVERVIEW_ROUTES)) {
      await openSettings(page, 'overview')
      await probe(page, {
        surface: S, name: `settings-overview-${id}`, actionId: `settings-overview-action:${id}`,
        sel: page.locator(`[data-cut-settings-overview-action="${id}"]`),
        group: panel, groupName: 'settings-overview',
        doClick: async () => {
          await page.locator(`[data-cut-settings-overview-action="${id}"]`).click()
          await waitFor(page.locator(`[data-cut-settings-body="${destination}"]`))
        },
        assertResult: async () => ({
          ok: (await page.locator(`[data-cut-settings-body="${destination}"]`).count()) === 1,
          detail: `${id} routed to ${destination}`,
        }),
      })
    }

    await openSettings(page, 'general')
    for (const theme of ['dark', 'light']) {
      await probe(page, {
        surface: S, name: `settings-theme-${theme}`, actionId: `theme-set:${theme}`,
        sel: page.locator(`[data-cut-theme-set="${theme}"]`),
        group: panel, groupName: 'settings-general',
        doClick: async () => { await page.locator(`[data-cut-theme-set="${theme}"]`).click(); await sleep(80) },
        assertResult: async () => {
          const states = await page.locator('[data-cut-theme]').evaluateAll((nodes) =>
            nodes.map((node) => node.getAttribute('data-cut-theme')))
          return {
            ok: states.length >= 2 && states.every((state) => state === theme),
            detail: `theme controls=${states.join(',')}`,
          }
        },
      })
    }
    await probe(page, {
      surface: S, name: 'settings-export-folder-picker', actionId: 'export-default-pick',
      sel: page.locator('[data-cut-export-default-pick]'),
      group: panel, groupName: 'settings-general',
      clickNa: nativePickerClickNa,
      nativeAction: { mode: 'cancel' },
      doClick: async () => { await page.locator('[data-cut-export-default-pick]').click() },
      assertResult: async () => ({
        ok: /desktop app/i.test(await page.locator('[data-cut-export-default-note]').textContent().catch(() => '')),
        detail: 'browser fallback explains that the folder picker is desktop-only',
      }),
    })

    await openSettings(page, 'editing')
    const keymap = page.locator('[data-cut-keymap-details]')
    await probe(page, {
      surface: S, name: 'settings-keymap-expand', actionId: 'keymap-toggle',
      sel: page.locator('[data-cut-keymap-toggle]'), group: panel, groupName: 'settings-editing',
      doClick: async () => { await page.locator('[data-cut-keymap-toggle]').click(); await sleep(100) },
      assertResult: async () => ({
        ok: (await keymap.getAttribute('open')) !== null,
        detail: `shortcut details open=${(await keymap.getAttribute('open')) !== null}`,
      }),
    })
    await probe(page, {
      surface: S, name: 'settings-keymap-search', actionId: 'keymap-search',
      sel: page.locator('[data-cut-keymap-search]'), group: panel, groupName: 'settings-editing',
      doClick: async () => { await page.locator('[data-cut-keymap-search]').fill('split'); await sleep(80) },
      assertResult: async () => ({
        ok: (await page.locator('[data-cut-keymap-row="timeline.split"]').count()) === 1,
        detail: 'search narrowed to Split at playhead',
      }),
    })
    await page.locator('[data-cut-keymap-search]').fill('')
    await probe(page, {
      surface: S, name: 'settings-keymap-group', actionId: 'keymap-group',
      sel: page.locator('[data-cut-keymap-group]'), group: panel, groupName: 'settings-editing',
      doClick: async () => { await page.locator('[data-cut-keymap-group]').selectOption('timeline'); await sleep(80) },
      assertResult: async () => ({
        ok: (await page.locator('[data-cut-keymap-group]').inputValue()) === 'timeline',
        detail: `group=${await page.locator('[data-cut-keymap-group]').inputValue()}`,
      }),
    })
    await probe(page, {
      surface: S, name: 'settings-keymap-filter-all', actionId: 'keymap-filter:all',
      sel: page.locator('[data-cut-keymap-filter="all"]'), group: panel, groupName: 'settings-editing',
      doClick: async () => { await page.locator('[data-cut-keymap-filter="all"]').click() },
      assertResult: async () => ({
        ok: (await page.locator('[data-cut-keymap-filter="all"]').getAttribute('aria-pressed')) === 'true',
        detail: 'All shortcut filter active',
      }),
    })
    await probe(page, {
      surface: S, name: 'settings-keymap-bind', actionId: 'keymap-bind:timeline.split',
      sel: page.locator('[data-cut-keymap-bind="timeline.split"]'), group: panel, groupName: 'settings-editing',
      doClick: async () => {
        await page.locator('[data-cut-keymap-bind="timeline.split"]').click()
        await page.keyboard.press('F8')
        await sleep(120)
      },
      assertResult: async () => ({
        ok: /F8/.test(await page.locator('[data-cut-keymap-bind="timeline.split"]').textContent()),
        detail: `binding=${await page.locator('[data-cut-keymap-bind="timeline.split"]').textContent()}`,
      }),
    })
    await probe(page, {
      surface: S, name: 'settings-keymap-reset-all', actionId: 'keymap-reset-all',
      sel: page.locator('[data-cut-action="keymap-reset-all"]'), group: panel, groupName: 'settings-editing',
      doClick: async () => { await page.locator('[data-cut-action="keymap-reset-all"]').click(); await sleep(80) },
      assertResult: async () => ({
        ok: (await page.locator('[data-cut-keymap-changed-count]').textContent()) === '0 changed',
        detail: `changed=${await page.locator('[data-cut-keymap-changed-count]').textContent()}`,
      }),
    })
    await page.locator('[data-cut-keymap-bind="timeline.split"]').click()
    await page.keyboard.press('F8')
    await sleep(80)
    await probe(page, {
      surface: S, name: 'settings-keymap-clear', actionId: 'keymap-clear:timeline.split',
      sel: page.locator('[data-cut-keymap-clear="timeline.split"]'), group: panel, groupName: 'settings-editing',
      doClick: async () => { await page.locator('[data-cut-keymap-clear="timeline.split"]').click(); await sleep(80) },
      assertResult: async () => ({
        ok: (await page.locator('[data-cut-keymap-clear="timeline.split"]').count()) === 0,
        detail: 'custom binding removed and default restored',
      }),
    })

    await openSettings(page, 'recording')
    await probe(page, {
      surface: S, name: 'settings-recording-keys', actionId: 'settings-recording-keys-toggle',
      sel: page.locator('[data-cut-settings-recording-keys-toggle]'), group: panel, groupName: 'settings-recording',
      doClick: async () => { await page.locator('[data-cut-settings-recording-keys-toggle]').click(); await sleep(80) },
      assertResult: async () => ({
        ok: (await page.locator('[data-cut-settings-recording-keys]').getAttribute('open')) !== null,
        detail: 'fixed recording shortcuts expanded on demand',
      }),
    })

    await openSettings(page, 'storage-privacy')
    const updateToggle = page.locator('[data-cut-update-check-on-launch]')
    await waitFor(updateToggle, 'attached')
    const installedShell = await page.evaluate(() => Boolean(globalThis.__TAURI__?.core?.invoke))
    const originalUpdatePreference = await updateToggle.isChecked()
    await probe(page, {
      surface: S,
      name: 'settings-update-check-on-launch',
      actionId: 'update-check-on-launch',
      sel: updateToggle,
      group: panel,
      groupName: 'settings-storage-privacy',
      clickNa: installedShell ? '' : 'The launch updater exists only in the installed desktop shell.',
      doClick: async () => { await updateToggle.click() },
      assertResult: async () => {
        let changed = false
        for (let i = 0; i < 60; i++) {
          changed = await updateToggle.isChecked() !== originalUpdatePreference
          if (changed) break
          await sleep(100)
        }
        const changedMessage = await page.locator('[data-cut-update-check-status]').textContent().catch(() => '')
        if (changed) await updateToggle.click()
        let restored = false
        for (let i = 0; i < 60; i++) {
          restored = await updateToggle.isChecked() === originalUpdatePreference
          if (restored) break
          await sleep(100)
        }
        // The feedback line states the real scope either way: on = launch +
        // 6-hourly cadence, off = manual About check still works.
        const honest = /automatic update checks are (on|off)/i.test(changedMessage)
        return {
          ok: changed && honest && restored,
          detail: `automatic update preference changed=${changed}; honest scope message=${honest}; restored=${restored}`,
        }
      },
    })

    await runUpdateSurfaceCoverage(page, panel, S)

    await runEnvironmentCoverage(page, panel, S)
    await runTaskCoverage(page, S)

    await openSettings(page, 'agent-control')
    await waitFor(page.locator('[data-cut-agent-control-config]'), 'attached')
    for (const [kind, selector, expected] of [
      ['rest', '[data-cut-agent-control-copy-rest]', 'REST route copied'],
      ['mcp', '[data-cut-agent-control-copy-mcp]', 'MCP setup copied'],
    ]) {
      await probe(page, {
        surface: S, name: `settings-copy-${kind}`, actionId: `agent-control-copy-${kind}`,
        sel: page.locator(selector), group: panel, groupName: 'settings-agent-control',
        doClick: async () => { await page.locator(selector).click(); await sleep(120) },
        assertResult: async () => {
          const note = await page.locator('[data-cut-agent-control-test-result]').textContent().catch(() => '')
          return { ok: note.includes(expected), detail: `copy note="${note.trim()}"` }
        },
      })
    }
    await probe(page, {
      surface: S, name: 'settings-agent-advanced', actionId: 'agent-control-advanced-toggle',
      sel: page.locator('[data-cut-agent-control-advanced-toggle]'), group: panel, groupName: 'settings-agent-control',
      doClick: async () => { await page.locator('[data-cut-agent-control-advanced-toggle]').click(); await sleep(80) },
      assertResult: async () => ({
        ok: (await page.locator('[data-cut-agent-control-advanced]').getAttribute('open')) !== null,
        detail: 'advanced agent connection details expanded',
      }),
    })

    await openSettings(page, 'general')
    await page.locator(`[data-cut-theme-set="${initialTheme}"]`).click()
    await sleep(80)
    await probe(page, {
      surface: S, name: 'settings-refresh', actionId: 'environment-refresh',
      sel: page.locator('[data-cut-environment-refresh]'), group: panel, groupName: 'settings-shell',
      doClick: async () => { await page.locator('[data-cut-environment-refresh]').click(); await sleep(350) },
      assertResult: async () => ({
        ok: (await page.locator('[data-cut-environment-open="true"]').count()) === 1,
        detail: 'doctor re-scan settled with Settings still usable',
      }),
    })

    await probe(page, {
      surface: S, name: 'settings-close', actionId: 'environment-close',
      sel: page.locator('[data-cut-environment-close]'), group: panel, groupName: 'settings-shell',
      doClick: async () => {
        await page.locator('[data-cut-environment-close]').click()
        await waitFor(page.locator('[data-cut-environment]'), 'detached')
      },
      assertResult: async () => ({
        ok: (await page.locator('[data-cut-environment]').count()) === 0,
        detail: 'Settings dialog unmounted',
      }),
    })

    await verb('ui.open', { panel: 'wizard' })
    await waitFor(page.locator('[data-cut-wizard]'))
    const wizard = page.locator('[data-cut-wizard]').first()
    await probe(page, {
      surface: S, name: 'setup-wizard-refresh', actionId: 'wizard-refresh',
      sel: page.locator('[data-cut-wizard-refresh]'), group: wizard, groupName: 'setup-wizard',
      doClick: async () => { await page.locator('[data-cut-wizard-refresh]').click(); await sleep(350) },
      assertResult: async () => ({
        ok: (await page.locator('[data-cut-wizard]').count()) === 1,
        detail: 'wizard re-scan settled without losing the setup surface',
      }),
    })
    await probe(page, {
      surface: S, name: 'setup-wizard-dismiss', actionId: 'wizard-dismiss',
      sel: page.locator('[data-cut-wizard-dismiss]'), group: wizard, groupName: 'setup-wizard',
      doClick: async () => {
        await page.locator('[data-cut-wizard-dismiss]').click()
        await waitFor(page.locator('[data-cut-wizard]'), 'detached')
      },
      assertResult: async () => ({
        ok: (await page.locator('[data-cut-wizard]').count()) === 0,
        detail: 'setup wizard dismissed',
      }),
    })

    const activeApp = await resolveCoverageAppUrl(page, app)
    const conditionalUrl = new URL(activeApp)
    conditionalUrl.searchParams.set('mock', '1')
    conditionalUrl.searchParams.set('mockEnvironment', '1')
    await page.goto(conditionalUrl.toString(), { waitUntil: 'domcontentloaded' })
    await closeOverlays(page)
    await page.locator('[data-cut-setup-btn]').click()
    await waitFor(page.locator('[data-cut-environment]'))
    await page.locator('[data-cut-settings-category="video-performance"]').click()
    await waitFor(page.locator('[data-cut-settings-body="video-performance"]'))
    const conditionalPanel = page.locator('[data-cut-environment]').first()
    await probe(page, {
      surface: S,
      name: 'settings-gpu-help-conditional',
      actionId: 'env-gpu-help-toggle',
      sel: page.locator('[data-cut-env-gpu-help-toggle]'),
      group: conditionalPanel,
      groupName: 'settings-video-performance-conditional',
      doClick: async () => {
        await page.locator('[data-cut-env-gpu-help-toggle]').click()
        await sleep(80)
      },
      assertResult: async () => ({
        ok: await page.locator('[data-cut-env-gpu-help]').getAttribute('open') !== null,
        detail: 'conditional GPU enablement help expanded',
      }),
    })
    await probe(page, {
      surface: S,
      name: 'settings-card-rescan-conditional',
      actionId: 'env-rescan',
      sel: page.locator('[data-cut-env-rescan="gpu-encode"]'),
      group: conditionalPanel,
      groupName: 'settings-video-performance-conditional',
      doClick: async () => {
        await page.locator('[data-cut-env-rescan="gpu-encode"]').click()
        await sleep(300)
      },
      assertResult: async () => ({
        ok: await page.locator('[data-cut-env-rescan="gpu-encode"]').count() === 1
          && await page.locator('[data-cut-environment-open="true"]').count() === 1,
        detail: 'conditional unknown-card re-scan completed with Settings responsive',
      }),
    })
    await page.goto(activeApp, { waitUntil: 'domcontentloaded' })
    await sleep(250)
  }

  return secSettings
}
