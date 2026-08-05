// Exhaustive Find moment action coverage.
//
// SigLIP indexing is an optional multi-gigabyte runtime, so the UI sweep
// supplies deterministic media.index/status/search envelopes while retaining
// the real Search component, request arguments, source-to-timeline mapping,
// ui.playhead relay, and Source monitor routing. Release dependency checks own
// the installed perception runtime itself.

export function createSearchActionCoverage({
  probe,
  verb,
  captureVerbResp,
  sleep,
  freshProject,
  closeOverlays,
  primaryMedia,
}) {
  const surface = 'search-actions'

  async function waitFor(check, timeoutMs = 10_000) {
    const deadline = Date.now() + timeoutMs
    let last = null
    while (Date.now() < deadline) {
      try {
        last = await check()
        if (last) return last
      } catch {}
      await sleep(90)
    }
    return last
  }

  async function installFixture(page, assetId) {
    await page.evaluate((fixtureAsset) => {
      const target = window
      if (!target.__fcvSearchOriginalFetch) target.__fcvSearchOriginalFetch = window.fetch
      const originalFetch = target.__fcvSearchOriginalFetch
      target.__fcvSearchFixture = {
        assetId: fixtureAsset,
        indexCalls: [],
        searchCalls: [],
      }
      const fixture = target.__fcvSearchFixture
      const envelope = (body) => new Response(JSON.stringify(body), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      })
      const requestArgs = (options) => {
        try { return JSON.parse(options?.body || '{}') } catch { return {} }
      }

      window.fetch = async (...args) => {
        const input = args[0]
        const options = args[1]
        const url = typeof input === 'string' ? input : input?.url || ''
        let pathname = ''
        try { pathname = new URL(String(url), window.location.href).pathname } catch {}

        if (pathname === '/api/verb/media.index_status') {
          return envelope({ ok: true, result: { count: 0, assets: [] } })
        }
        if (pathname === '/api/verb/media.index') {
          const body = requestArgs(options)
          fixture.indexCalls.push(body)
          return envelope({
            ok: true,
            result: {
              asset: fixtureAsset,
              indexed_frames: 7,
              dim: 4,
              model: 'fcv-search-fixture',
            },
          })
        }
        if (pathname === '/api/verb/media.search') {
          const body = requestArgs(options)
          fixture.searchCalls.push(body)
          return envelope({
            ok: true,
            result: {
              count: 1,
              hits: [{
                asset: fixtureAsset,
                start_ms: 500,
                end_ms: 1500,
                peak_ms: 1000,
                score: 0.93,
              }],
            },
          })
        }
        return originalFetch(...args)
      }
    }, assetId)
  }

  async function fixtureState(page) {
    return page.evaluate(() => JSON.parse(JSON.stringify(window.__fcvSearchFixture)))
  }

  async function restoreFixture(page) {
    await page.evaluate(() => {
      const target = window
      if (target.__fcvSearchOriginalFetch) window.fetch = target.__fcvSearchOriginalFetch
      delete target.__fcvSearchOriginalFetch
      delete target.__fcvSearchFixture
    })
  }

  async function run(page) {
    const created = await freshProject(page, 'search_actions', primaryMedia)
    await closeOverlays(page)
    if (!created.assetId) throw new Error('Find moment coverage needs an imported video asset')
    // media.import registers an asset; it does not place that asset on the
    // timeline. Find-moment coverage needs both source and timeline routing, so
    // seed one normal edit.insert explicitly and then wait for engine + React
    // state instead of depending on an accidental clip from another section.
    const projectState = await verb('project.state', {})
    let importedClip = projectState?.result?.tracks
      ?.find((track) => track.kind === 'video')
      ?.clips?.find((clip) => clip.asset === created.assetId)
    if (!importedClip?.id) {
      const videoTrack = projectState?.result?.tracks?.find((track) => track.kind === 'video')
      if (!videoTrack?.id) throw new Error('Find moment coverage needs a video track')
      const inserted = await verb('edit.insert', {
        asset: created.assetId,
        track: videoTrack.id,
        at_ms: 0,
        rationale: 'full coverage: place indexed media for source/timeline routing',
      })
      if (!inserted?.ok) {
        throw new Error(`Find moment coverage could not place ${created.assetId}: ${JSON.stringify(inserted?.error || inserted)}`)
      }
      importedClip = await waitFor(async () => {
        const state = await verb('project.state', {})
        return state?.result?.tracks
          ?.find((track) => track.kind === 'video')
          ?.clips?.find((clip) => clip.asset === created.assetId) || null
      }, 15_000)
    }
    if (!importedClip?.id) {
      throw new Error(`Find moment coverage cannot locate ${created.assetId} on the video timeline`)
    }
    await page.locator(`[data-cut-clip="${importedClip.id}"]`).first().waitFor({
      state: 'visible',
      timeout: 12_000,
    })
    await installFixture(page, created.assetId)
    try {
      // Force a remount after the fetch fixture is installed so index_status is
      // deterministic even when the persisted sidebar previously opened Find.
      await page.locator('[data-cut-left-tab="projects"]').first().click()
      await page.locator('[data-cut-left-tab="find"]').first().click()
      await page.locator('[data-cut-find-tab="find-moment"]').first().click()
      const panel = page.locator('[data-cut-search-embed]').first()
      await panel.waitFor({ state: 'visible', timeout: 8000 })

      const index = page.locator(`[data-cut-search-index="${created.assetId}"]`).first()
      await probe(page, {
        surface,
        name: 'search-index-video',
        actionId: 'search-index',
        sel: index,
        group: panel,
        groupName: 'find-moment',
        doClick: async () => {
          await index.click()
          await page.locator('[data-cut-search-note]').filter({ hasText: 'Indexed 7 frames' }).waitFor({
            state: 'visible',
            timeout: 8000,
          })
        },
        assertResult: async () => {
          const fixture = await fixtureState(page)
          return {
            ok: fixture.indexCalls.length === 1
              && fixture.indexCalls[0].asset === created.assetId
              && fixture.indexCalls[0].fps === 1
              && await index.isDisabled(),
            detail: `index calls=${fixture.indexCalls.length}; asset=${fixture.indexCalls[0]?.asset}; fps=${fixture.indexCalls[0]?.fps}; disabled=${await index.isDisabled()}`,
          }
        },
      })

      const query = page.locator('[data-cut-search-query]').first()
      await probe(page, {
        surface,
        name: 'search-query-input',
        actionId: 'search-query',
        sel: query,
        group: panel,
        groupName: 'find-moment',
        doClick: async () => { await query.fill('red scene') },
        assertResult: async () => ({
          ok: await query.inputValue() === 'red scene',
          detail: `query="${await query.inputValue()}"`,
        }),
      })

      const go = page.locator('[data-cut-search-go]').first()
      await probe(page, {
        surface,
        name: 'search-indexed-content',
        actionId: 'search-go',
        sel: go,
        group: panel,
        groupName: 'find-moment',
        doClick: async () => {
          await go.click()
          await page.locator('[data-cut-search-hit="0"]').first().waitFor({
            state: 'visible',
            timeout: 8000,
          })
        },
        assertResult: async () => {
          const fixture = await fixtureState(page)
          const result = await page.locator('[data-cut-search-hit="0"]').first().textContent()
          return {
            ok: fixture.searchCalls.length === 1
              && fixture.searchCalls[0].query === 'red scene'
              && fixture.searchCalls[0].top_k === 8
              && result?.includes('source 1.0s')
              && result?.includes('timeline 1.0s'),
            detail: `search calls=${fixture.searchCalls.length}; args=${JSON.stringify(fixture.searchCalls[0])}; result="${result?.replace(/\s+/g, ' ').trim()}"`,
          }
        },
      })

      const jump = page.locator('[data-cut-search-jump="0"]').first()
      let playheadResponse = null
      let playheadState = null
      await probe(page, {
        surface,
        name: 'search-jump-to-timeline',
        actionId: 'search-jump',
        sel: jump,
        group: panel,
        groupName: 'find-moment-results',
        doClick: async () => {
          playheadResponse = await captureVerbResp(page, 'ui.playhead', () => jump.click(), 20_000)
          playheadState = await waitFor(async () => {
            const response = await verb('ui.state', {})
            return response?.ok && response.result?.playhead_ms === 1000 ? response.result : null
          }, 8000)
        },
        assertResult: async () => ({
          ok: playheadResponse?.ok && playheadState?.playhead_ms === 1000,
          detail: `ui.playhead ok=${playheadResponse?.ok}; connected state=${playheadState?.playhead_ms}`,
        }),
      })

      const source = page.locator('[data-cut-search-source="0"]').first()
      await probe(page, {
        surface,
        name: 'search-open-source-monitor',
        actionId: 'search-source',
        sel: source,
        group: panel,
        groupName: 'find-moment-results',
        doClick: async () => {
          await source.click()
          await page.locator(`[data-cut-source-monitor="${created.assetId}"]`).first().waitFor({
            state: 'visible',
            timeout: 8000,
          })
        },
        assertResult: async () => {
          const sourceTime = await page.locator('[data-cut-source-current]').first().textContent().catch(() => '')
          return {
            ok: await page.locator('[data-cut-left-tab="assets"]').first().getAttribute('aria-selected') === 'true'
              && await page.locator(`[data-cut-source-monitor="${created.assetId}"]`).first().isVisible()
              && sourceTime?.includes('0:01.000'),
            detail: `assets tab=${await page.locator('[data-cut-left-tab="assets"]').first().getAttribute('aria-selected')}; source=${sourceTime}`,
          }
        },
      })
      await page.locator('[data-cut-source-monitor-close]').first().click()
    } finally {
      await restoreFixture(page)
    }
  }

  return { run }
}
