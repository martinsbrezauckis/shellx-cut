// Deterministic installed-WebView coverage for the Scopes configuration
// controls. The provider-backed Review section retains real verify.scopes
// proof; this companion proves every local toggle branch and its exact request.

export function createScopesActionCoverage({
  probe,
  renderGroup,
  freshProject,
  closeOverlays,
  ensureReviewPanel,
  reviewTab,
  primaryMedia,
}) {
  const surface = 'scopes-actions'
  const scopeImage = '/api/frame?at_ms=0&compose=1'

  async function installFixture(page) {
    await page.evaluate((fixtureImage) => {
      const target = window
      target.__fcvScopesOriginalFetch = window.fetch
      target.__fcvScopesFixture = { calls: [] }
      const fixture = target.__fcvScopesFixture
      const originalFetch = target.__fcvScopesOriginalFetch
      const response = (body) => new Response(JSON.stringify(body), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      })
      window.fetch = async (...args) => {
        const input = args[0]
        const options = args[1]
        const url = typeof input === 'string' ? input : input?.url || ''
        let pathname = ''
        try { pathname = new URL(String(url), window.location.href).pathname } catch {}
        if (pathname === '/api/verb/verify.scopes') {
          let body = {}
          try { body = JSON.parse(options?.body || '{}') } catch {}
          fixture.calls.push(body)
          return response({
            ok: true,
            result: {
              at_ms: body.at_ms,
              source: 'timeline',
              pass: false,
              luma: { min: 0, avg: 45, max: 102 },
              clipping: { highlights: true, shadows: false },
              broadcast_legal: false,
              saturation: { avg: 42, max: 110 },
              white_balance: { u_avg: 1, v_avg: -2, cast: 'cool' },
              hue: { avg: 15, med: 12 },
              flags: ['clipped_highlights', 'illegal_levels'],
              scopes: {
                waveform: fixtureImage,
                histogram: fixtureImage,
              },
            },
          })
        }
        return originalFetch(...args)
      }
    }, scopeImage)
  }

  async function fixtureState(page) {
    return page.evaluate(() => JSON.parse(JSON.stringify(window.__fcvScopesFixture)))
  }

  async function restoreFixture(page) {
    await page.evaluate(() => {
      const target = window
      if (target.__fcvScopesOriginalFetch) window.fetch = target.__fcvScopesOriginalFetch
      delete target.__fcvScopesOriginalFetch
      delete target.__fcvScopesFixture
    })
  }

  async function selectedKinds(page) {
    return page.locator('[data-cut-scopes-kind][aria-pressed="true"]').evaluateAll(
      (buttons) => buttons.map((button) => button.getAttribute('data-cut-scopes-kind')),
    )
  }

  async function kindProbe(page, kind, name, expected) {
    const selector = `[data-cut-scopes-kind="${kind}"]`
    await probe(page, {
      surface,
      name,
      actionId: 'scopes-kind',
      sel: page.locator(selector),
      group: page.locator('[data-cut-scopes-kinds]'),
      groupName: name,
      doClick: async () => {
        await page.locator(selector).click()
      },
      assertResult: async () => {
        const selected = await selectedKinds(page)
        return {
          ok: JSON.stringify(selected) === JSON.stringify(expected),
          detail: `selected scope kinds=${JSON.stringify(selected)}`,
        }
      },
    })
  }

  async function run(page) {
    await freshProject(page, 'scopes_actions', primaryMedia)
    await closeOverlays(page)
    await ensureReviewPanel(page)
    await installFixture(page)
    try {
      await reviewTab(page, 'scopes', '[data-cut-scopes]', 8_000)
      const scopes = page.locator('[data-cut-scopes]')
      await scopes.locator('[data-cut-scopes-at-ms]').fill('1250')

      await probe(page, {
        surface,
        name: 'include-rendered-scope-images',
        actionId: 'scopes-images',
        sel: scopes.locator('[data-cut-scopes-images]'),
        group: scopes.locator('[data-cut-scopes-bar]'),
        groupName: 'scopes-image-option',
        doClick: async () => {
          await scopes.locator('[data-cut-scopes-images]').check()
        },
        assertResult: async () => ({
          ok: await scopes.locator('[data-cut-scopes-images]').isChecked(),
          detail: `scope images checked=${await scopes.locator('[data-cut-scopes-images]').isChecked()}`,
        }),
      })

      await kindProbe(page, 'waveform', 'disable-waveform-scope', ['vectorscope', 'histogram'])
      await kindProbe(page, 'histogram', 'disable-histogram-scope', ['vectorscope'])
      await kindProbe(page, 'vectorscope', 'retain-last-selected-scope', ['vectorscope'])
      await kindProbe(page, 'waveform', 'enable-waveform-scope', ['vectorscope', 'waveform'])
      await kindProbe(page, 'vectorscope', 'disable-vectorscope-scope', ['waveform'])
      await kindProbe(page, 'histogram', 'enable-histogram-scope', ['waveform', 'histogram'])

      await probe(page, {
        surface,
        name: 'run-scopes-with-changed-options',
        actionId: 'scopes-run',
        sel: scopes.locator('[data-cut-action="scopes-run"]'),
        group: scopes,
        groupName: 'scopes-result',
        doClick: async () => {
          await scopes.locator('[data-cut-action="scopes-run"]').click()
          await scopes.locator('[data-cut-scopes-result="warn"]').waitFor({ state: 'visible', timeout: 5_000 })
          await scopes.locator('[data-cut-scopes-image]').first().waitFor({ state: 'visible', timeout: 5_000 })
          await page.waitForFunction(
            () => Array.from(document.querySelectorAll('[data-cut-scopes-image] img'))
              .every((node) => node.complete && node.naturalWidth > 0),
            null,
            { timeout: 5_000 },
          )
          await renderGroup(page, surface, 'scopes-result-completed', scopes)
          const lastImage = scopes.locator('[data-cut-scopes-image]').last()
          await lastImage.scrollIntoViewIfNeeded()
          await renderGroup(page, surface, 'scopes-result-image-access', lastImage)
        },
        assertResult: async () => {
          const fixture = await fixtureState(page)
          const call = fixture.calls[0]
          const exactCall = call?.at_ms === 1250
            && call?.scope_images === true
            && Array.isArray(call?.kinds)
            && call.kinds.length === 2
            && call.kinds[0] === 'waveform'
            && call.kinds[1] === 'histogram'
          const warnings = (await scopes.locator('[data-cut-scopes-warnings]').textContent()) || ''
          const images = scopes.locator('[data-cut-scopes-image]')
          const loaded = await images.locator('img').evaluateAll(
            (nodes) => nodes.every((node) => node.complete && node.naturalWidth > 0),
          )
          const scroll = await scopes.evaluate((scroller) => {
            const last = scroller.querySelector('[data-cut-scopes-image]:last-child')
            const scrollerBox = scroller.getBoundingClientRect()
            const lastBox = last?.getBoundingClientRect()
            return {
              clientHeight: scroller.clientHeight,
              scrollHeight: scroller.scrollHeight,
              scrollTop: scroller.scrollTop,
              lastVisible: Boolean(lastBox
                && lastBox.top >= scrollerBox.top
                && lastBox.bottom <= scrollerBox.bottom + 1),
            }
          })
          return {
            ok: fixture.calls.length === 1
              && exactCall
              && await images.count() === 2
              && loaded
              && scroll.scrollHeight > scroll.clientHeight
              && scroll.scrollTop > 0
              && scroll.lastVisible
              && warnings.includes('clipped highlights')
              && warnings.includes('broadcast levels outside range'),
            detail: `verify.scopes args=${JSON.stringify(fixture.calls[0])}; image links=${await images.count()}; loaded=${loaded}; scroll=${JSON.stringify(scroll)}; warnings="${warnings.replace(/\s+/g, ' ').trim()}"`,
          }
        },
      })
    } finally {
      await restoreFixture(page)
      await closeOverlays(page)
    }
  }

  return { run }
}
