// Deterministic coverage for Director choices and conditional completion/error
// actions. The provider-backed `director` section keeps the real CV/render
// proof; this product-mock companion makes every human control executable on
// every installed host, including hosts without the optional perception venv.

import { resolveCoverageAppUrl } from './fullCoverageAppUrl.mjs'

export function createDirectorActionCoverage({
  app,
  probe,
  sleep,
  closeOverlays,
}) {
  const surface = 'director-actions'

  function mockUrl(activeApp, error = false) {
    const url = new URL(activeApp)
    url.searchParams.set('mock', '1')
    if (error) url.searchParams.set('mockDirectorError', '1')
    return url.toString()
  }

  async function resetMock(page, activeApp, error = false) {
    await page.evaluate(() => {
      localStorage.setItem('cut.layout.v1', JSON.stringify({
        txFrac: 0.4,
        tlH: 280,
        railW: 420,
        railCollapsed: false,
        railPinned: false,
        leftCollapsed: false,
        leftTab: 'projects',
        findSurface: 'find-media',
        workspaceMode: 'edit',
        rightTab: 'inspector',
      }))
    })
    await page.goto(mockUrl(activeApp, error), { waitUntil: 'domcontentloaded' })
    await sleep(150)
    await closeOverlays(page)
  }

  async function calls(page) {
    return page.evaluate(() => JSON.parse(JSON.stringify(window.__cutMock?.calls?.() ?? [])))
  }

  async function openDirector(page) {
    await page.locator('[data-cut-render-opts]').click()
    await page.locator('[data-cut-render-menu]').waitFor({ state: 'visible', timeout: 5000 })
    await page.locator('[data-cut-render-aspect]').selectOption('9:16')
    await page.locator('[data-cut-director-open]').click()
    await page.locator('[data-cut-director]').waitFor({ state: 'visible', timeout: 5000 })
  }

  async function waitForPick(page) {
    await page.locator('[data-cut-director-pick]').waitFor({ state: 'visible', timeout: 5000 })
  }

  async function renderToDone(page) {
    await page.locator('[data-cut-director-render]').click()
    await page.locator('[data-cut-director-done]').waitFor({ state: 'visible', timeout: 5000 })
  }

  async function run(page) {
    const activeApp = await resolveCoverageAppUrl(page, app)
    try {
      await resetMock(page, activeApp)
      await openDirector(page)
      await probe(page, {
        surface,
        name: 'close-director-from-header',
        actionId: 'director-close',
        sel: page.locator('[data-cut-director-close]'),
        group: page.locator('[data-cut-director]'),
        groupName: 'director-header',
        doClick: async () => {
          await page.locator('[data-cut-director-close]').click()
          await page.locator('[data-cut-director]').waitFor({ state: 'detached', timeout: 5000 })
        },
        assertResult: async () => ({
          ok: await page.locator('[data-cut-director]').count() === 0,
          detail: `Director header Close detached modal=${await page.locator('[data-cut-director]').count() === 0}`,
        }),
      })
      await openDirector(page)
      await waitForPick(page)

      const scene = page.locator('[data-cut-director-scene="0"]')
      for (const choice of ['A', 'widen', 'auto', 'A']) {
        const choiceName = choice === 'A' ? 'subject-a' : choice
        await probe(page, {
          surface,
          name: `choose-director-${choiceName}`,
          actionId: 'pick',
          sel: scene.locator(`[data-cut-pick="${choice}"]`),
          group: scene,
          groupName: `director-scene-${choiceName}`,
          doClick: async () => {
            await scene.locator(`[data-cut-pick="${choice}"]`).click()
            await scene.locator(`[data-cut-pick="${choice}"][aria-pressed="true"]`).waitFor({
              state: 'visible',
              timeout: 5000,
            })
          },
          assertResult: async () => ({
            ok: await scene.locator(`[data-cut-pick="${choice}"]`).getAttribute('aria-pressed') === 'true',
            detail: `${choice} selected=${await scene.locator(`[data-cut-pick="${choice}"]`).getAttribute('aria-pressed')}`,
          }),
        })
      }

      const beforeFirstRender = (await calls(page)).length
      await renderToDone(page)
      const firstRender = (await calls(page)).slice(beforeFirstRender).find((entry) => entry.name === 'render.reframe')
      if (JSON.stringify(firstRender?.args?.direction) !== JSON.stringify({ 0: { cx: 0.35 } })) {
        throw new Error(`Director subject direction=${JSON.stringify(firstRender?.args?.direction)}`)
      }

      await page.locator('[data-cut-director-review]').click()
      await page.locator('[data-cut-director-qc]').waitFor({ state: 'visible', timeout: 5000 })
      await probe(page, {
        surface,
        name: 'repick-flagged-director-scene',
        actionId: 'director-repick',
        sel: page.locator('[data-cut-director-repick]'),
        group: page.locator('[data-cut-director-done]'),
        groupName: 'director-flagged-qc',
        doClick: async () => {
          await page.locator('[data-cut-director-repick]').click()
          await waitForPick(page)
        },
        assertResult: async () => ({
          ok: await page.locator('[data-cut-director-pick]').isVisible()
            && await scene.locator('[data-cut-pick="A"]').getAttribute('aria-pressed') === 'true',
          detail: `pick phase=${await page.locator('[data-cut-director-pick]').isVisible()}; prior subject preserved=${await scene.locator('[data-cut-pick="A"]').getAttribute('aria-pressed')}`,
        }),
      })

      await scene.locator('[data-cut-pick="widen"]').click()
      const beforeSecondRender = (await calls(page)).length
      await renderToDone(page)
      const secondRender = (await calls(page)).slice(beforeSecondRender).find((entry) => entry.name === 'render.reframe')
      if (JSON.stringify(secondRender?.args?.direction) !== JSON.stringify({ 0: { mode: 'widen' } })) {
        throw new Error(`Director widen direction=${JSON.stringify(secondRender?.args?.direction)}`)
      }

      await probe(page, {
        surface,
        name: 'close-completed-director',
        actionId: 'director-done-close',
        sel: page.locator('[data-cut-director-done-close]'),
        group: page.locator('[data-cut-director-done]'),
        groupName: 'director-completed',
        doClick: async () => {
          await page.locator('[data-cut-director-done-close]').click()
          await page.locator('[data-cut-director]').waitFor({ state: 'detached', timeout: 5000 })
        },
        assertResult: async () => ({
          ok: await page.locator('[data-cut-director]').count() === 0,
          detail: `Director modal count=${await page.locator('[data-cut-director]').count()}; second direction=${JSON.stringify(secondRender?.args?.direction)}`,
        }),
      })

      await resetMock(page, activeApp, true)
      await openDirector(page)
      await page.locator('[data-cut-director-error]').waitFor({ state: 'visible', timeout: 5000 })
      await probe(page, {
        surface,
        name: 'close-director-error',
        actionId: 'director-error-close',
        sel: page.locator('[data-cut-director-error-close]'),
        group: page.locator('[data-cut-director-error]'),
        groupName: 'director-error',
        doClick: async () => {
          await page.locator('[data-cut-director-error-close]').click()
          await page.locator('[data-cut-director]').waitFor({ state: 'detached', timeout: 5000 })
        },
        assertResult: async () => ({
          ok: await page.locator('[data-cut-director]').count() === 0,
          detail: `Director error dismissed; modal count=${await page.locator('[data-cut-director]').count()}`,
        }),
      })
    } finally {
      await page.goto(activeApp, { waitUntil: 'domcontentloaded' }).catch(() => {})
      await sleep(250)
    }
  }

  return { run }
}
