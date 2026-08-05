// Assets/Source coverage: OS pickers stay N/A in WebView-only candidates; the installed gate owns them.
import { unlinkSync } from 'node:fs'
import { createAssetsPickerProbe } from './fullCoverageAssetsPickerActions.mjs'
import { createAssetsSetupActions } from './fullCoverageAssetsSetupActions.mjs'
import { runOfflineMediaRelinkCoverage } from './fullCoverageOfflineMediaActions.mjs'
export function createAssetsActionCoverage({
  probe,
  verb,
  state,
  waitForState,
  captureVerbResp,
  sleep,
  reloadApp,
  closeOverlays,
  freshProject,
  awaitImportJobs,
  makeRelinkPair,
  makeToneAudio,
  basenameHostPath,
  nativePickerClickNa,
  nativeOsActionsEnabled,
  primaryMedia,
  trace,
}) {
  const pickerProbe = createAssetsPickerProbe({
    probe,
    captureVerbResp,
    awaitImportJobs,
    waitForState,
    sleep,
    basenameHostPath,
    nativePickerClickNa,
    nativeOsActionsEnabled,
  })
  const { openAssets, toggleFilter, runEmptyImport } = createAssetsSetupActions({
    probe,
    verb,
    sleep,
    reloadApp,
    closeOverlays,
    pickerProbe,
    primaryMedia,
    trace,
  })
  async function importReady(path, { proxy = false } = {}) {
    const imported = await verb('media.import', { path, proxy })
    if (imported.ok) await awaitImportJobs(imported)
    const asset = imported.result?.asset_id || ''
    if (asset) await waitForState((project) => !!project.assets?.[asset], 12_000)
    return { ...imported, asset }
  }
  async function run(page, { secondMedia }) {
    trace('assets', 'setup-populated-project', 'start')
    const projectCtx = await freshProject(page, 'assets')
    trace('assets', 'setup-populated-project', 'opened')
    let panel = await openAssets(page)
    await pickerProbe(page, {
      name: 'import-asset',
      actionId: 'import-asset',
      selector: '[data-cut-action="import-asset"]',
      panel,
      selectPath: secondMedia,
      browserEvidence: async (browserPage) => (
        (await browserPage.locator('[data-cut-asset-note]').textContent().catch(() => ''))
          .includes('desktop app')
      ),
    })
    await pickerProbe(page, {
      name: 'import-otio',
      actionId: 'import-otio',
      selector: '[data-cut-import-otio]',
      panel,
      browserEvidence: async (browserPage) => (
        (await browserPage.locator('[data-cut-topbar-note]').textContent().catch(() => ''))
          .includes('desktop app')
      ),
    })
    // Source Monitor must exercise real playback. The heavy fixture is HEVC,
    // intentionally undecodable by WebKitGTK without an editing proxy, so make
    // this second registered copy proxy-ready instead of testing an expected
    // codec refusal as though it were a broken Play action.
    const secondary = await importReady(secondMedia, { proxy: true })
    trace('assets', 'setup-fixture-secondary', secondary.asset || 'missing')
    const relinkPair = makeRelinkPair()
    const missing = relinkPair ? await importReady(relinkPair.originalEngine) : { ok: false, asset: '' }
    trace('assets', 'setup-fixture-relink', missing.asset || 'missing')
    const removablePath = makeToneAudio(1.5)
    const removable = removablePath ? await importReady(removablePath) : { ok: false, asset: '' }
    trace('assets', 'setup-fixture-removable', removable.asset || 'missing')
    if (!secondary.asset || !missing.asset || !removable.asset) {
      throw new Error(
        `Assets fixtures incomplete: secondary=${secondary.asset || 'none'} ` +
        `missing=${missing.asset || 'none'} removable=${removable.asset || 'none'}`,
      )
    }
    const fixtureState = await state()
    const fixtureVideoTrack = fixtureState.tracks?.find((track) => track.kind === 'video')?.id
    const placedMissing = fixtureVideoTrack
      ? await verb('edit.insert', {
        asset: missing.asset,
        track: fixtureVideoTrack,
        at_ms: 0,
        ripple: true,
        rationale: 'fcv: offline placeholder fixture',
      })
      : { ok: false }
    if (!placedMissing.ok) throw new Error('could not place the offline placeholder fixture on the timeline')
    await waitForState((value) => value.tracks?.some((track) => (
      track.id === fixtureVideoTrack && track.clips?.some((clip) => clip.asset === missing.asset)
    )), 12_000)
    await verb('ui.playhead', { at_ms: 100 })
    await sleep(1_500)
    trace('assets', 'setup-fixtures', 'refresh-panel')
    panel = await openAssets(page)
    await page.locator(`[data-cut-asset-card="${secondary.asset}"]`).waitFor({
      state: 'visible',
      timeout: 12_000,
    })
    const proxyToggle = page.locator('[data-cut-proxy-toggle]').first()
    const proxyBefore = await proxyToggle.isChecked()
    await probe(page, {
      surface: 'assets',
      name: 'proxy-toggle',
      actionId: 'proxy-toggle',
      sel: proxyToggle,
      group: panel,
      groupName: 'assets-header',
      doClick: async () => { await proxyToggle.click(); await sleep(120) },
      assertResult: async () => ({
        ok: (await proxyToggle.isChecked()) !== proxyBefore,
        detail: `future-import proxy preference toggled from ${proxyBefore}`,
      }),
    })
    const healthProxy = page.locator('[data-cut-media-health-proxies]').first()
    const healthBefore = await healthProxy.getAttribute('data-cut-media-health-proxies-on')
    await probe(page, {
      surface: 'assets',
      name: 'media-health-proxies',
      actionId: 'media-health-proxies',
      sel: healthProxy,
      group: panel,
      groupName: 'media-health',
      doClick: async () => { await healthProxy.click(); await sleep(120) },
      assertResult: async () => ({
        ok: (await healthProxy.getAttribute('data-cut-media-health-proxies-on')) !== healthBefore,
        detail: `Media Health proxy action toggled from ${healthBefore}`,
      }),
    })
    const advanced = page.locator('[data-cut-media-health-advanced]').first()
    await probe(page, {
      surface: 'assets',
      name: 'media-health-advanced-toggle',
      actionId: 'media-health-advanced-toggle',
      sel: page.locator('[data-cut-media-health-advanced-toggle]').first(),
      group: panel,
      groupName: 'media-health',
      doClick: async () => {
        await page.locator('[data-cut-media-health-advanced-toggle]').first().click()
        await sleep(120)
      },
      assertResult: async () => ({
        ok: (await advanced.getAttribute('open')) !== null,
        detail: 'advanced Media Health metrics disclosed',
      }),
    })
    await page.evaluate(() => {
      window.__fcvMediaHealthManual = ''
      window.open = (url) => {
        window.__fcvMediaHealthManual = String(url || '')
        return null
      }
    })
    await probe(page, {
      surface: 'assets',
      name: 'media-health-manual',
      actionId: 'media-health-manual',
      sel: page.locator('[data-cut-media-health-manual]').first(),
      group: panel,
      groupName: 'media-health',
      doClick: async () => { await page.locator('[data-cut-media-health-manual]').first().click() },
      assertResult: async () => {
        const url = await page.evaluate(() => window.__fcvMediaHealthManual || '')
        return {
          ok: url.includes('/manual/cut/') && url.includes('cut.left.media_health'),
          detail: `manual route=${url || 'none'}`,
        }
      },
    })
    let refreshResponse = null
    await probe(page, {
      surface: 'assets',
      name: 'media-health-refresh',
      actionId: 'media-health-refresh',
      sel: page.locator('[data-cut-media-health-refresh]').first(),
      group: panel,
      groupName: 'media-health',
      doClick: async () => {
        refreshResponse = await captureVerbResp(
          page,
          'media.check',
          () => page.locator('[data-cut-media-health-refresh]').first().click(),
          12_000,
        )
        await sleep(220)
      },
      assertResult: async () => ({
        ok: !!refreshResponse?.ok && Number.isInteger(refreshResponse?.result?.count),
        detail: `media.check count=${refreshResponse?.result?.count ?? '?'}`,
      }),
    })
    const assetName = basenameHostPath(secondMedia)
    const textFilter = page.locator('[data-cut-asset-filter]').first()
    await probe(page, {
      surface: 'assets',
      name: 'asset-filter',
      actionId: 'asset-filter',
      sel: textFilter,
      group: panel,
      groupName: 'assets-filters',
      doClick: async () => { await textFilter.fill(assetName); await sleep(220) },
      assertResult: async () => ({
        ok: (await page.locator(`[data-cut-asset-card="${secondary.asset}"]`).count()) === 1,
        detail: `name filter retains ${assetName}`,
      }),
    })
    await textFilter.fill('')
    await sleep(180)
    await toggleFilter(page, panel, {
      name: 'asset-kind-filter',
      actionId: 'asset-kind-filter',
      selector: '[data-cut-asset-kind-filter="video"]',
      assertResult: async () => {
        const visibleKinds = await page.locator('[data-cut-asset-card]').count()
        const videoKinds = await page.locator('[data-cut-asset-card][data-cut-asset-kind="video"]').count()
        return { ok: visibleKinds > 0 && visibleKinds === videoKinds, detail: `video cards=${videoKinds}/${visibleKinds}` }
      },
    })
    await toggleFilter(page, panel, {
      name: 'asset-unused-filter',
      actionId: 'asset-unused-filter',
      selector: '[data-cut-asset-unused-filter]',
      assertResult: async () => ({
        ok: (await page.locator(`[data-cut-asset-card="${projectCtx.assetId}"]`).count()) === 0
          && (await page.locator(`[data-cut-asset-card="${secondary.asset}"]`).count()) === 1,
        detail: 'used primary hidden while unused secondary remains',
      }),
    })
    const beforeLarge = await page.locator('[data-cut-asset-card]').count()
    await toggleFilter(page, panel, {
      name: 'asset-resolution-filter',
      actionId: 'asset-resolution-filter',
      selector: '[data-cut-asset-resolution-filter]',
      assertResult: async () => ({
        ok: (await page.locator('[data-cut-asset-resolution-filter]').first().getAttribute('class'))
          ?.includes('assets__chip--on')
          && (await page.locator('[data-cut-asset-card]').count()) <= beforeLarge,
        detail: `4K+ filter reduced/kept ${beforeLarge} cards`,
      }),
    })
    await toggleFilter(page, panel, {
      name: 'asset-recent-filter',
      actionId: 'asset-recent-filter',
      selector: '[data-cut-asset-recent-filter]',
      assertResult: async () => ({
        ok: (await page.locator('[data-cut-asset-recent-filter]').first().getAttribute('class'))
          ?.includes('assets__chip--on')
          && (await page.locator(`[data-cut-asset-card="${missing.asset}"]`).count()) === 1,
        detail: 'freshly generated fixture remains under recent filter',
      }),
    })
    const binName = `fcv-ui-${Math.random().toString(36).slice(2, 6)}`
    await page.locator('[data-cut-asset-kind-filter="video"]').click()
    const onPrompt = (dialog) => { dialog.accept(binName).catch(() => {}) }
    page.on('dialog', onPrompt)
    let binSaveResponse = null
    await probe(page, {
      surface: 'assets',
      name: 'bin-save',
      actionId: 'bin-save',
      sel: page.locator('[data-cut-action="bin-save"]').first(),
      group: panel,
      groupName: 'assets-filters',
      doClick: async () => {
        binSaveResponse = await captureVerbResp(
          page,
          'media.bin_save',
          () => page.locator('[data-cut-action="bin-save"]').first().click(),
          12_000,
        )
        await page.locator(`[data-cut-bin="${binName}"]`).waitFor({ state: 'visible', timeout: 8_000 })
      },
      assertResult: async () => ({
        ok: !!binSaveResponse?.ok && (await page.locator(`[data-cut-bin="${binName}"]`).count()) === 1,
        detail: `saved bin rendered immediately=${!!binSaveResponse?.ok}`,
      }),
    })
    page.off('dialog', onPrompt)
    const binOpen = page.locator(`[data-cut-bin-open="${binName}"]`).first()
    await probe(page, {
      surface: 'assets',
      name: 'bin-open',
      actionId: 'bin-open',
      sel: binOpen,
      group: panel,
      groupName: 'assets-filters',
      doClick: async () => { await binOpen.click(); await sleep(150) },
      assertResult: async () => ({
        ok: (await page.locator(`[data-cut-bin="${binName}"][data-cut-bin-active]`).count()) === 0,
        detail: 'active smart bin toggled off',
      }),
    })
    await binOpen.click()
    await sleep(120)
    let binDeleteResponse = null
    await probe(page, {
      surface: 'assets',
      name: 'bin-delete',
      actionId: 'bin-delete',
      sel: page.locator(`[data-cut-bin-delete="${binName}"]`).first(),
      group: panel,
      groupName: 'assets-filters',
      doClick: async () => {
        binDeleteResponse = await captureVerbResp(
          page,
          'media.bin_delete',
          () => page.locator(`[data-cut-bin-delete="${binName}"]`).first().click(),
          12_000,
        )
        await sleep(220)
      },
      assertResult: async () => ({
        ok: !!binDeleteResponse?.ok && (await page.locator(`[data-cut-bin="${binName}"]`).count()) === 0,
        detail: `deleted bin disappeared immediately=${!!binDeleteResponse?.ok}`,
      }),
    })
    await page.locator('[data-cut-asset-kind-filter="all"]').click()
    await sleep(150)
    unlinkSync(relinkPair.originalDriver)
    refreshResponse = null
    await page.locator('[data-cut-media-health-refresh]').first().click()
    await page.locator(`[data-cut-asset-offline="${missing.asset}"]`).waitFor({
      state: 'visible',
      timeout: 12_000,
    })
    await toggleFilter(page, panel, {
      name: 'asset-offline-filter',
      actionId: 'asset-offline-filter',
      selector: '[data-cut-asset-offline-filter]',
      assertResult: async () => ({
        ok: (await page.locator(`[data-cut-asset-card="${missing.asset}"]`).count()) === 1
          && (await page.locator('[data-cut-asset-card]').count()) === 1,
        detail: 'missing filter isolates the deleted-source fixture',
      }),
    })
    await toggleFilter(page, panel, {
      name: 'asset-attention-filter',
      actionId: 'asset-attention-filter',
      selector: '[data-cut-asset-attention-filter]',
      assertResult: async () => ({
        ok: (await page.locator(`[data-cut-asset-card="${missing.asset}"]`).count()) === 1,
        detail: 'needs-action filter retains the offline fixture',
      }),
    })
    await pickerProbe(page, {
      name: 'media-health-relink-first',
      actionId: 'media-health-relink-first',
      selector: `[data-cut-media-health-relink-first="${missing.asset}"]`,
      panel,
      selectPath: relinkPair.replacementEngine,
      selectVerb: 'media.relink',
      selectAsset: missing.asset,
      browserEvidence: async (browserPage) => (
        (await browserPage.locator('[data-cut-asset-note]').textContent().catch(() => ''))
          .includes('desktop app')
      ),
    })
    panel = await runOfflineMediaRelinkCoverage({ page, panel, asset: missing.asset,
      relinkPair, pickerProbe, nativeOsActionsEnabled, openAssets })
    await pickerProbe(page, {
      name: 'relink-asset',
      actionId: 'relink-asset',
      selector: '[data-cut-action="relink-asset"]',
      panel,
      selectPath: relinkPair.fourthReplacementEngine,
      selectVerb: 'media.relink',
      selectAsset: missing.asset,
      browserEvidence: async (browserPage) => (
        (await browserPage.locator('[data-cut-asset-note]').textContent().catch(() => ''))
          .includes('desktop app')
      ),
    })
    await sleep(800)
    panel = await openAssets(page)
    await page.locator(`[data-cut-source-monitor-open="${secondary.asset}"]`).waitFor({
      state: 'visible',
      timeout: 12_000,
    })
    const openSource = page.locator(`[data-cut-action="open-source-monitor"][data-cut-source-monitor-open="${secondary.asset}"]`).first()
    await probe(page, {
      surface: 'assets',
      name: 'open-source-monitor',
      actionId: 'open-source-monitor',
      sel: openSource,
      group: panel,
      groupName: 'asset-card-actions',
      doClick: async () => {
        await openSource.click()
        await page.locator(`[data-cut-source-monitor="${secondary.asset}"]`).waitFor({
          state: 'visible',
          timeout: 10_000,
        })
      },
      assertResult: async () => ({
        ok: (await page.locator(`[data-cut-source-monitor="${secondary.asset}"]`).count()) === 1,
        detail: 'Source monitor dialog mounted',
      }),
    })
    const sourceDialog = page.locator(`[data-cut-source-monitor="${secondary.asset}"]`).first()
    await probe(page, {
      surface: 'assets',
      name: 'source-monitor-close',
      actionId: 'source-monitor-close',
      sel: page.locator('[data-cut-source-monitor-close]').first(),
      group: sourceDialog,
      groupName: 'source-monitor',
      doClick: async () => {
        await page.locator('[data-cut-source-monitor-close]').first().click()
        await sourceDialog.waitFor({ state: 'detached', timeout: 8_000 })
      },
      assertResult: async () => ({
        ok: (await sourceDialog.count()) === 0,
        detail: 'Source monitor detached',
      }),
    })
    await openSource.click()
    await sourceDialog.waitFor({ state: 'visible', timeout: 10_000 })
    for (let attempt = 0; attempt < 40; attempt += 1) {
      const duration = await page.evaluate((assetId) => {
        const media = document.querySelector(`[data-cut-source-monitor="${assetId}"] video, [data-cut-source-monitor="${assetId}"] audio`)
        return media instanceof HTMLMediaElement && Number.isFinite(media.duration) ? media.duration : 0
      }, secondary.asset)
      if (duration > 0) break
      await sleep(250)
    }
    let sourceAdvanced = false
    const sourcePlay = page.locator('[data-cut-action="source-monitor-play"]').first()
    await probe(page, {
      surface: 'assets',
      name: 'source-monitor-play',
      actionId: 'source-monitor-play',
      sel: sourcePlay,
      group: sourceDialog,
      groupName: 'source-monitor',
      doClick: async () => {
        await sourcePlay.click()
        for (let attempt = 0; attempt < 40; attempt += 1) {
          const current = await page.locator('[data-cut-source-current]').textContent().catch(() => '')
          if (current && current !== '0:00.000') { sourceAdvanced = true; break }
          await sleep(100)
        }
      },
      assertResult: async () => ({
        ok: sourceAdvanced,
        detail: `source time=${await page.locator('[data-cut-source-current]').textContent().catch(() => '?')}`,
      }),
    })
    if (!sourceAdvanced) {
      const mediaDiagnostic = await page.evaluate((assetId) => {
        const media = document.querySelector(`[data-cut-source-monitor="${assetId}"] video, [data-cut-source-monitor="${assetId}"] audio`)
        if (!(media instanceof HTMLMediaElement)) return JSON.stringify({ found: false })
        return JSON.stringify({
          found: true,
          src: media.getAttribute('src'),
          currentSrc: media.currentSrc,
          currentTime: Number.isFinite(media.currentTime) ? media.currentTime : null,
          duration: Number.isFinite(media.duration) ? media.duration : null,
          readyState: media.readyState,
          networkState: media.networkState,
          paused: media.paused,
          error: media.error ? { code: media.error.code, message: String(media.error.message || '') } : null,
          location: window.location.href,
        })
      }, secondary.asset)
      throw new Error(`source monitor native Play control did not advance playback: ${mediaDiagnostic}`)
    }
    await sourcePlay.click()
    await probe(page, {
      surface: 'assets',
      name: 'source-mark-in',
      actionId: 'source-mark-in',
      sel: page.locator('[data-cut-source-mark-in]').first(),
      group: sourceDialog,
      groupName: 'source-monitor',
      doClick: async () => { await page.locator('[data-cut-source-mark-in]').first().click(); await sleep(100) },
      assertResult: async () => ({
        ok: (await page.locator('[data-cut-source-in]').textContent().catch(() => '')) !== '0:00.000',
        detail: `In=${await page.locator('[data-cut-source-in]').textContent().catch(() => '?')}`,
      }),
    })
    await sleep(250)
    await probe(page, {
      surface: 'assets',
      name: 'source-mark-out',
      actionId: 'source-mark-out',
      sel: page.locator('[data-cut-source-mark-out]').first(),
      group: sourceDialog,
      groupName: 'source-monitor',
      doClick: async () => { await page.locator('[data-cut-source-mark-out]').first().click(); await sleep(100) },
      assertResult: async () => {
        const sourceIn = await page.locator('[data-cut-source-in]').textContent().catch(() => '')
        const sourceOut = await page.locator('[data-cut-source-out]').textContent().catch(() => '')
        return { ok: !!sourceIn && !!sourceOut && sourceOut !== sourceIn, detail: `range=${sourceIn}-${sourceOut}` }
      },
    })
    await probe(page, {
      surface: 'assets',
      name: 'source-insert',
      actionId: 'source-insert',
      sel: page.locator('[data-cut-source-insert]').first(),
      group: sourceDialog,
      groupName: 'source-monitor',
      doClick: async () => { await page.locator('[data-cut-source-insert]').first().click() },
      assertResult: async () => {
        const inserted = await waitForState((project) => (
          project.tracks?.some((track) => track.clips?.some((clip) => (
            clip.asset === secondary.asset
            && Number.isFinite(clip.src_in_ms)
            && clip.src_out_ms > clip.src_in_ms
          )))
        ), 12_000)
        return { ok: !!inserted, detail: 'marked source range landed on the timeline' }
      },
    })
    if (await sourceDialog.count()) { await page.locator('[data-cut-source-monitor-close]').first().click(); await sourceDialog.waitFor({ state: 'detached', timeout: 8_000 }) } // Source Insert deliberately keeps the Source Monitor open; close it before modal-blocked asset actions.
    await sleep(500); panel = await openAssets(page)
    const missingBefore = (await state()).tracks
      ?.flatMap((track) => track.clips || [])
      .filter((clip) => clip.asset === missing.asset).length || 0
    await probe(page, {
      surface: 'assets',
      name: 'insert-asset',
      actionId: 'insert-asset',
      sel: page.locator(`[data-cut-asset-card="${missing.asset}"] [data-cut-action="insert-asset"]`).first(),
      group: panel,
      groupName: 'asset-card-actions',
      doClick: async () => {
        await page.locator(`[data-cut-asset-card="${missing.asset}"] [data-cut-action="insert-asset"]`).first().click()
      },
      assertResult: async () => {
        const inserted = await waitForState((project) => (
          (project.tracks || []).flatMap((track) => track.clips || [])
            .filter((clip) => clip.asset === missing.asset).length > missingBefore
        ), 12_000)
        return { ok: !!inserted, detail: `timeline gained clips for ${missing.asset}` }
      },
    })
    await sleep(500)
    panel = await openAssets(page)
    const onConfirm = (dialog) => { dialog.accept().catch(() => {}) }
    page.on('dialog', onConfirm)
    let removeResponse = null
    await probe(page, {
      surface: 'assets',
      name: 'remove-asset',
      actionId: 'remove-asset',
      sel: page.locator(`[data-cut-action="remove-asset"][data-cut-asset-remove="${removable.asset}"]`).first(),
      group: panel,
      groupName: 'asset-card-actions',
      nativeAction: { mode: 'accept', useDoClick: true, verifyResult: true },
      doClick: async () => {
        removeResponse = await captureVerbResp(
          page,
          'media.remove',
          () => page.locator(`[data-cut-action="remove-asset"][data-cut-asset-remove="${removable.asset}"]`).first().click(),
          12_000,
        )
        await sleep(250)
      },
      assertResult: async () => ({
        ok: !!removeResponse?.ok
          && !(await state()).assets?.[removable.asset]
          && (await page.locator(`[data-cut-asset-card="${removable.asset}"]`).count()) === 0,
        detail: `unused asset removed=${!!removeResponse?.ok}; source retained`,
      }),
    })
    page.off('dialog', onConfirm)
    return projectCtx
  }
  return { runEmptyImport, run }
}
