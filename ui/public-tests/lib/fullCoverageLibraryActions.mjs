// Native action coverage for the Library workspace controls added by the
// navigation/scale refactor. The legacy Library section still owns verb-domain
// coverage; this helper owns every concrete browsing, selection and bulk action.

import { resolveCoverageAppUrl } from './fullCoverageAppUrl.mjs'

export function createLibraryActionCoverage({
  app,
  probe,
  verb,
  state,
  waitForState,
  captureVerbResp,
  sleep,
  closeOverlays,
  nativeOsActionsEnabled = false,
}) {
  // The response is dispatched only after the host confirmation closes. A
  // Windows TaskDialog recovery can legitimately outlive the ordinary 12s API
  // capture window, so keep the listener alive for the same native-action
  // deadline plus a small response-delivery margin.
  const bulkRemoveResponseTimeoutMs = nativeOsActionsEnabled
    ? Number(process.env.FCV_NATIVE_ACTION_TIMEOUT_MS || 20_000) + 5_000
    : 12_000

  async function refresh(page, id = '') {
    await page.locator('[data-cut-library-close]').click().catch(() => {})
    await sleep(120)
    await page.locator('[data-cut-library-btn]').click()
    await page.locator('[data-cut-panel="library"]').waitFor({ state: 'visible', timeout: 12_000 })
    if (id) {
      await page.locator(`[data-cut-library-card="${id}"]`).waitFor({
        state: 'visible',
        timeout: 12_000,
      })
    }
  }

  async function select(page, id) {
    const control = page.locator(`[data-cut-library-select="${id}"]`)
    await control.waitFor({ state: 'visible', timeout: 12_000 })
    if ((await control.getAttribute('aria-pressed')) !== 'true') await control.click()
    await page.locator('[data-cut-library-bulkbar]').waitFor({ state: 'visible', timeout: 8_000 })
  }

  async function listItem(id) {
    return (await verb('library.list', { ids: [id], limit: 1 })).result?.items?.[0] ?? null
  }

  async function waitForCardCount(page, expected) {
    const cards = page.locator('[data-cut-library-card]')
    for (let attempt = 0; attempt < 50; attempt += 1) {
      if ((await cards.count()) === expected) return true
      await sleep(100)
    }
    return false
  }

  async function runPagination(page) {
    const activeApp = await resolveCoverageAppUrl(page, app)
    const url = new URL(activeApp)
    url.searchParams.set('mock', '1')
    url.searchParams.set('mockLibraryTotal', '101')
    await page.goto(url.toString(), { waitUntil: 'domcontentloaded' })
    if (closeOverlays) await closeOverlays(page)
    await page.locator('[data-cut-library-btn]').click()
    const status = page.locator('[data-cut-library-page-status]')
    await status.filter({ hasText: '1–100 of 101' }).waitFor({ state: 'visible', timeout: 10_000 })
    const panel = page.locator('[data-cut-panel="library"]').first()
    await probe(page, {
      surface: 'library',
      name: 'library-page-next',
      actionId: 'library-page-next',
      sel: page.locator('[data-cut-library-page-next]'),
      group: panel,
      groupName: 'library-pagination',
      doClick: async () => {
        await page.locator('[data-cut-library-page-next]').click()
        await status.filter({ hasText: '101–101 of 101' }).waitFor({ state: 'visible', timeout: 5000 })
        await waitForCardCount(page, 1)
      },
      assertResult: async () => ({
        ok: (await status.textContent())?.includes('Page 2 of 2') === true
          && await page.locator('[data-cut-library-card]').count() === 1,
        detail: `next → "${(await status.textContent())?.trim()}"; cards=${await page.locator('[data-cut-library-card]').count()}`,
      }),
    })
    await probe(page, {
      surface: 'library',
      name: 'library-page-previous',
      actionId: 'library-page-prev',
      sel: page.locator('[data-cut-library-page-prev]'),
      group: panel,
      groupName: 'library-pagination',
      doClick: async () => {
        await page.locator('[data-cut-library-page-prev]').click()
        await status.filter({ hasText: '1–100 of 101' }).waitFor({ state: 'visible', timeout: 5000 })
        await waitForCardCount(page, 100)
      },
      assertResult: async () => ({
        ok: (await status.textContent())?.includes('Page 1 of 2') === true
          && await page.locator('[data-cut-library-card]').count() === 100,
        detail: `previous → "${(await status.textContent())?.trim()}"; cards=${await page.locator('[data-cut-library-card]').count()}`,
      }),
    })
    await page.goto(activeApp, { waitUntil: 'domcontentloaded' })
    await sleep(250)
  }

  async function run(page, { id, secondMedia }) {
    const surface = 'library'
    await refresh(page, id)
    let panel = page.locator('[data-cut-panel="library"]').first()
    const item = await listItem(id)
    const itemName = item?.name || id
    const itemType = item?.type || 'video'

    const portableToggle = page.locator('[data-cut-library-portable-toggle]')
    await probe(page, {
      surface, name: 'library-portable-toggle', actionId: 'library-portable-toggle',
      sel: portableToggle, group: panel, groupName: 'library-panel',
      doClick: async () => { await portableToggle.click() },
      assertResult: async () => ({
        ok: await portableToggle.isChecked(),
        detail: 'future Browse imports set to keep a managed copy',
      }),
    })
    if (await portableToggle.isChecked()) await portableToggle.click()

    const typeTab = page.locator(`[data-cut-library-tab="${itemType}"]`)
    await probe(page, {
      surface, name: 'library-type-tab', actionId: 'library-tab',
      sel: typeTab, group: panel, groupName: 'library-panel',
      doClick: async () => { await typeTab.click(); await sleep(250) },
      assertResult: async () => ({
        ok: (await typeTab.getAttribute('data-cut-on')) === 'true',
        detail: `${itemType} type tab active`,
      }),
    })
    await page.locator('[data-cut-library-tab="all"]').click()
    await sleep(200)

    const sort = page.locator('[data-cut-library-sort]')
    await probe(page, {
      surface, name: 'library-sort', actionId: 'library-sort',
      sel: sort, group: panel, groupName: 'library-panel',
      doClick: async () => { await sort.selectOption('name'); await sleep(250) },
      assertResult: async () => ({
        ok: (await sort.inputValue()) === 'name',
        detail: 'Library sorted by name',
      }),
    })

    const search = page.locator('[data-cut-library-search]')
    await probe(page, {
      surface, name: 'library-search', actionId: 'library-search',
      sel: search, group: panel, groupName: 'library-panel',
      doClick: async () => { await search.fill(itemName); await sleep(450) },
      assertResult: async () => ({
        ok: (await page.locator(`[data-cut-library-card="${id}"]`).count()) === 1,
        detail: `search found ${itemName}`,
      }),
    })
    await search.fill('')
    await sleep(350)

    const tagCollection = page.locator('[data-cut-library-collection-tag="fcv"]')
    await probe(page, {
      surface, name: 'library-tag-collection', actionId: 'library-collection-tag',
      sel: tagCollection, group: panel, groupName: 'library-panel',
      doClick: async () => { await tagCollection.click(); await sleep(350) },
      assertResult: async () => ({
        ok: (await page.locator('[data-cut-library-tagfilter]').count()) === 1,
        detail: '#fcv collection filter is visible',
      }),
    })
    const clearTag = page.locator('[data-cut-library-tagfilter-clear]')
    await probe(page, {
      surface, name: 'library-tag-filter-clear', actionId: 'library-tagfilter-clear',
      sel: clearTag, group: panel, groupName: 'library-panel',
      doClick: async () => { await clearTag.click(); await sleep(300) },
      assertResult: async () => ({
        ok: (await page.locator('[data-cut-library-tagfilter]').count()) === 0,
        detail: 'tag filter cleared',
      }),
    })

    const listView = page.locator('[data-cut-library-view-list]')
    await probe(page, {
      surface, name: 'library-view-list', actionId: 'library-view-list',
      sel: listView, group: panel, groupName: 'library-panel',
      doClick: async () => { await listView.click(); await sleep(250) },
      assertResult: async () => ({
        ok: (await page.locator('[data-cut-library-list]').count()) === 1,
        detail: 'list view mounted',
      }),
    })
    const listSort = page.locator('[data-cut-library-list-sort-name]')
    await probe(page, {
      surface, name: 'library-list-sort-name', actionId: 'library-list-sort-name',
      sel: listSort, group: panel, groupName: 'library-panel',
      doClick: async () => { await listSort.click(); await sleep(250) },
      assertResult: async () => ({
        ok: (await sort.inputValue()) === 'name',
        detail: 'visible Name header selected name sorting',
      }),
    })
    const gridView = page.locator('[data-cut-library-view-grid]')
    await probe(page, {
      surface, name: 'library-view-grid', actionId: 'library-view-grid',
      sel: gridView, group: panel, groupName: 'library-panel',
      doClick: async () => { await gridView.click(); await sleep(250) },
      assertResult: async () => ({
        ok: (await page.locator('.lb-grid[data-cut-library-grid]').count()) === 1,
        detail: 'grid view restored',
      }),
    })

    const selectControl = page.locator(`[data-cut-library-select="${id}"]`)
    await probe(page, {
      surface, name: 'library-select', actionId: 'library-select',
      sel: selectControl, group: panel, groupName: 'library-panel',
      doClick: async () => { await selectControl.click(); await sleep(120) },
      assertResult: async () => ({
        ok: (await selectControl.getAttribute('aria-pressed')) === 'true'
          && (await page.locator('[data-cut-library-bulkbar]').count()) === 1,
        detail: 'item selected and bulk actions mounted',
      }),
    })
    const bulkClear = page.locator('[data-cut-library-bulk-clear]')
    await probe(page, {
      surface, name: 'library-bulk-clear', actionId: 'library-bulk-clear',
      sel: bulkClear, group: panel, groupName: 'library-panel',
      doClick: async () => { await bulkClear.click(); await sleep(120) },
      assertResult: async () => ({
        ok: (await page.locator('[data-cut-library-bulkbar]').count()) === 0,
        detail: 'selection cleared',
      }),
    })

    await select(page, id)
    const bulkTag = page.locator('[data-cut-library-bulk-tag]')
    await probe(page, {
      surface, name: 'library-bulk-tag', actionId: 'library-bulk-tag',
      sel: bulkTag, group: panel, groupName: 'library-panel',
      doClick: async () => { await bulkTag.click() },
      assertResult: async () => ({
        ok: (await page.locator('[data-cut-library-bulk-taginput]').count()) === 1,
        detail: 'bulk tag editor opened',
      }),
    })
    const bulkTagInput = page.locator('[data-cut-library-bulk-taginput]')
    let bulkTagResponse = null
    await probe(page, {
      surface, name: 'library-bulk-tag-input', actionId: 'library-bulk-taginput',
      sel: bulkTagInput, group: panel, groupName: 'library-panel',
      doClick: async () => {
        await bulkTagInput.fill('native-checked')
        bulkTagResponse = await captureVerbResp(page, 'library.tag', () => bulkTagInput.press('Enter'), 12_000)
        await sleep(350)
      },
      assertResult: async () => ({
        ok: !!bulkTagResponse?.ok && (await listItem(id))?.tags?.includes('native-checked'),
        detail: `bulk tag persisted=${!!bulkTagResponse?.ok}`,
      }),
    })

    await select(page, id)
    const bulkMove = page.locator('[data-cut-library-bulk-move]')
    let bulkMoveResponse = null
    await probe(page, {
      surface, name: 'library-bulk-move', actionId: 'library-bulk-move',
      sel: bulkMove, group: panel, groupName: 'library-panel',
      doClick: async () => {
        bulkMoveResponse = await captureVerbResp(page, 'library.move', () => bulkMove.selectOption('__root'), 12_000)
        await sleep(350)
      },
      assertResult: async () => ({
        ok: !!bulkMoveResponse?.ok && ((await listItem(id))?.folder ?? null) === null,
        detail: `bulk move to All persisted=${!!bulkMoveResponse?.ok}`,
      }),
    })

    await select(page, id)
    const bulkProject = page.locator('[data-cut-library-bulk-toproject]')
    let bulkProjectResponse = null
    await probe(page, {
      surface, name: 'library-bulk-add-to-project', actionId: 'library-bulk-toproject',
      sel: bulkProject, group: panel, groupName: 'library-panel',
      doClick: async () => {
        bulkProjectResponse = await captureVerbResp(
          page,
          'library.add_to_project',
          () => bulkProject.click(),
          20_000,
        )
        await sleep(400)
      },
      assertResult: async () => ({
        ok: !!bulkProjectResponse?.ok,
        detail: `bulk add to project ok=${bulkProjectResponse?.ok}`,
      }),
    })

    await refresh(page, id)
    const beforeClips = (await state()).tracks.reduce((count, track) => count + (track.clips?.length || 0), 0)
    const insert = page.locator(`[data-cut-library-insert="${id}"]`)
    await probe(page, {
      surface, name: 'library-insert-at-playhead', actionId: 'library-insert',
      sel: insert, group: panel, groupName: 'library-panel',
      doClick: async () => { await insert.click(); await sleep(700) },
      assertResult: async () => {
        const updated = await waitForState(
          (project) => project.tracks.reduce((count, track) => count + (track.clips?.length || 0), 0) > beforeClips,
          12_000,
        )
        return { ok: !!updated, detail: `timeline clip count increased from ${beforeClips}` }
      },
    })

    await refresh(page, id)
    panel = page.locator('[data-cut-panel="library"]').first()
    const portable = page.locator(`[data-cut-library-portable="${id}"]`)
    await probe(page, {
      surface, name: 'library-keep-copy', actionId: 'library-portable',
      sel: portable, group: panel, groupName: 'library-panel',
      doClick: async () => { await portable.click(); await sleep(700) },
      assertResult: async () => {
        let stored = await listItem(id)
        for (let index = 0; index < 30 && !stored?.blob; index += 1) {
          await sleep(200)
          stored = await listItem(id)
        }
        return { ok: !!stored?.blob, detail: `managed copy stored=${!!stored?.blob}` }
      },
    })

    await refresh(page, id)
    const beforeFavorite = (await listItem(id))?.favorite === true
    const card = page.locator(`[data-cut-library-card="${id}"]`)
    await card.click({ button: 'right', force: true }).catch(() => {})
    if ((await page.locator('[data-cut-library-card-menu]').count()) === 0) {
      await page.evaluate((itemId) => {
        Array.from(document.querySelectorAll('[data-cut-library-card]'))
          .find((element) => element.getAttribute('data-cut-library-card') === itemId)
          ?.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, clientX: 400, clientY: 320 }))
      }, id)
    }
    const contextFavorite = page.locator('[data-cut-library-card-ctx="favorite"]')
    let favoriteResponse = null
    await probe(page, {
      surface, name: 'library-card-context-action', actionId: 'library-card-ctx',
      sel: contextFavorite, group: panel, groupName: 'library-panel',
      doClick: async () => {
        favoriteResponse = await captureVerbResp(page, 'library.favorite', () => contextFavorite.click(), 12_000)
        await sleep(350)
      },
      assertResult: async () => ({
        ok: !!favoriteResponse?.ok && ((await listItem(id))?.favorite === true) !== beforeFavorite,
        detail: `context-menu favorite toggled=${!!favoriteResponse?.ok}`,
      }),
    })

    const extra = await verb('library.add', {
      path: secondMedia,
      name: `FCV bulk remove ${Math.random().toString(36).slice(2, 6)}`,
      source: 'user',
    })
    const extraId = extra.result?.item?.id || ''
    await refresh(page, extraId)
    await select(page, extraId)
    const bulkRemove = page.locator('[data-cut-library-bulk-remove]')
    let bulkRemoveResponse = null
    // A paired installed OS controller must be the sole owner of the native
    // confirmation. Registering Playwright's dialog acceptor at the same time
    // races the host controller: CDP can dismiss the TaskDialog before the
    // controller observes it, producing a false timeout after the action
    // already succeeded.
    const accept = (dialog) => { dialog.accept().catch(() => {}) }
    if (!nativeOsActionsEnabled) page.on('dialog', accept)
    await probe(page, {
      surface, name: 'library-bulk-remove', actionId: 'library-bulk-remove',
      sel: bulkRemove, group: panel, groupName: 'library-panel',
      nativeAction: { mode: 'accept', useDoClick: true, verifyResult: true },
      doClick: async () => {
        bulkRemoveResponse = await captureVerbResp(
          page,
          'library.remove',
          () => bulkRemove.click(),
          bulkRemoveResponseTimeoutMs,
        )
        await sleep(500)
      },
      assertResult: async () => ({
        ok: !!bulkRemoveResponse?.ok && (await listItem(extraId)) == null,
        detail: `throwaway bulk removal ok=${bulkRemoveResponse?.ok}`,
      }),
    })
    if (!nativeOsActionsEnabled) page.off('dialog', accept)
    panel = page.locator('[data-cut-panel="library"]').first()
    await runPagination(page)
  }

  return run
}
