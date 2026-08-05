// Canonical app-shell action coverage: sidebars, command palette, and theme.
// These controls do not mutate the project, but every one must leave an
// observable UI/persistence effect in the final native action matrix.

export function createAppChromeActionCoverage({
  probe,
  sleep,
  freshProject,
  closeOverlays,
  primaryMedia,
}) {
  const surface = 'app-chrome-actions'

  async function run(page) {
    await freshProject(page, 'app_chrome_actions', primaryMedia)
    await closeOverlays(page)

    const leftPanel = page.locator('[data-cut-leftpanel]').first()
    const collapseLeft = page.locator('[data-cut-action="collapse-left"]').first()
    await probe(page, {
      surface,
      name: 'left-sidebar-collapse',
      actionId: 'collapse-left',
      sel: collapseLeft,
      group: leftPanel,
      groupName: 'left-sidebar',
      doClick: async () => { await collapseLeft.click() },
      assertResult: async () => {
        const expand = page.locator('[data-cut-action="expand-left"]').first()
        return {
          ok: await expand.isVisible() && !(await leftPanel.isVisible()),
          detail: `expand control visible=${await expand.isVisible()}; left panel visible=${await leftPanel.isVisible()}`,
        }
      },
    })

    const expandLeft = page.locator('[data-cut-action="expand-left"]').first()
    await probe(page, {
      surface,
      name: 'left-sidebar-expand',
      actionId: 'expand-left',
      sel: expandLeft,
      group: page.locator('.app__split').first(),
      groupName: 'editor-shell',
      doClick: async () => { await expandLeft.click() },
      assertResult: async () => ({
        ok: await leftPanel.isVisible() && await collapseLeft.isVisible(),
        detail: `left panel visible=${await leftPanel.isVisible()}; collapse control visible=${await collapseLeft.isVisible()}`,
      }),
    })

    const shortcut = process.platform === 'darwin' ? 'Meta+K' : 'Control+K'
    await page.keyboard.press(shortcut)
    const palette = page.locator('[data-cut-command-palette]').first()
    await palette.waitFor({ state: 'visible', timeout: 5000 })
    const search = page.locator('[data-cut-command-search]').first()
    await probe(page, {
      surface,
      name: 'command-palette-search',
      actionId: 'command-search',
      sel: search,
      group: palette,
      groupName: 'command-palette',
      doClick: async () => { await search.fill('audio mixer') },
      assertResult: async () => {
        const rows = await page.locator('[data-cut-command]').count()
        const mixer = await page.locator('[data-cut-command="mixer"]').count()
        return { ok: rows === 1 && mixer === 1, detail: `filtered rows=${rows}; mixer row=${mixer}` }
      },
    })

    const mixerCommand = page.locator('[data-cut-command="mixer"]').first()
    await probe(page, {
      surface,
      name: 'command-palette-run',
      actionId: 'command',
      sel: mixerCommand,
      group: palette,
      groupName: 'command-palette',
      doClick: async () => {
        await mixerCommand.click()
        await page.locator('[data-cut-mixer-embed]').first().waitFor({ state: 'visible', timeout: 8000 })
      },
      assertResult: async () => ({
        ok: await page.locator('[data-cut-command-palette]').count() === 0
          && (await page.locator('[data-cut-right-tab="audio"]').first().getAttribute('aria-selected')) === 'true',
        detail: 'palette closed and Audio mixer command opened its canonical right-rail tab',
      }),
    })

    const rightRail = page.locator('.app__rail').first()
    const closeRail = page.locator('[data-cut-rail-close]').first()
    await probe(page, {
      surface,
      name: 'right-tools-close',
      actionId: 'rail-close',
      sel: closeRail,
      group: rightRail,
      groupName: 'right-tools',
      doClick: async () => { await closeRail.click() },
      assertResult: async () => {
        const expand = page.locator('[data-cut-action="expand-rail"]').first()
        return {
          ok: await expand.isVisible() && (await rightRail.getAttribute('class'))?.includes('app__rail--collapsed'),
          detail: `Tools strip visible=${await expand.isVisible()}; rail class="${await rightRail.getAttribute('class')}"`,
        }
      },
    })

    const theme = page.locator('[data-cut-theme-toggle]').first()
    const before = await theme.getAttribute('data-cut-theme')
    await probe(page, {
      surface,
      name: 'topbar-theme-toggle',
      actionId: 'theme-toggle',
      sel: theme,
      group: page.locator('[data-cut-panel="topbar"]').first(),
      groupName: 'topbar-theme',
      doClick: async () => { await theme.click(); await sleep(80) },
      assertResult: async () => {
        const after = await theme.getAttribute('data-cut-theme')
        const persisted = await page.evaluate(() => localStorage.getItem('cut.theme'))
        const root = await page.locator('html').getAttribute('data-theme')
        const expected = before === 'light' ? 'dark' : 'light'
        return {
          ok: after === expected && persisted === expected && (expected === 'light' ? root === 'light' : root == null),
          detail: `theme ${before}→${after}; stored=${persisted}; root=${root ?? '(dark default)'}`,
        }
      },
    })
  }

  return { run }
}
