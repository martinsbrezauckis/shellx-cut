// Shared Assets setup/navigation probes kept separate from the long action matrix.
export function createAssetsSetupActions({
  probe,
  verb,
  sleep,
  reloadApp,
  closeOverlays,
  pickerProbe,
  primaryMedia,
  trace,
}) {
  async function openAssets(page) {
    await closeOverlays(page)
    await page.locator('[data-cut-mode="edit"]').click().catch(() => {})
    await page.locator('[data-cut-left-tab="assets"]').click()
    await page.locator('[data-cut-panel="assets"]').waitFor({ state: 'visible', timeout: 12_000 })
    await sleep(250)
    return page.locator('[data-cut-panel="assets"]').first()
  }

  async function toggleFilter(page, panel, {
    name,
    actionId,
    selector,
    assertResult,
  }) {
    const control = page.locator(selector).first()
    await probe(page, {
      surface: 'assets',
      name,
      actionId,
      sel: control,
      group: panel,
      groupName: 'assets-filters',
      doClick: async () => {
        await control.click()
        await sleep(220)
      },
      assertResult,
    })
    await control.click()
    await sleep(180)
  }

  async function runEmptyImport(page) {
    trace('assets', 'setup-empty-project', 'start')
    await verb('project.create', {
      name: `fcv_assets_empty_${Math.random().toString(36).slice(2, 6)}`,
      settings: { width: 1280, height: 720, fps: 30 },
    })
    await sleep(1_200)
    trace('assets', 'setup-empty-project', 'reload')
    await reloadApp(page)
    const panel = await openAssets(page)
    trace('assets', 'setup-empty-project', 'probe')
    await pickerProbe(page, {
      name: 'import-cta',
      actionId: 'import-cta',
      selector: '[data-cut-import-cta]',
      panel,
      selectPath: primaryMedia,
      browserEvidence: async (browserPage) => (
        (await browserPage.locator('[data-cut-asset-note]').textContent().catch(() => ''))
          .includes('desktop app')
      ),
    })
  }

  return { openAssets, toggleFilter, runEmptyImport }
}
