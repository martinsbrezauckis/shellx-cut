// Deterministic installed-WebView coverage for every action in the Matte drawer.
//
// The shipping integration section still runs a real edit.matte bake when the
// local runtime exists. This lane owns the human controls independent of machine
// provisioning: it intercepts only the four Matte-related verb endpoints in the
// current test document, records their exact payloads, and restores fetch before
// returning. That lets all three release hosts exercise missing, failed, base,
// and premium states without downloading models during every release gate.

export function createMatteActionCoverage({
  probe,
  sleep,
  freshProject,
  closeOverlays,
  clipOfKind,
  selectClip,
}) {
  const surface = 'matte-actions'

  async function waitFor(check, timeoutMs = 8000) {
    const deadline = Date.now() + timeoutMs
    let last = null
    while (Date.now() < deadline) {
      try {
        last = await check()
        if (last) return last
      } catch {}
      await sleep(80)
    }
    return last
  }

  async function installFixture(page, doctor = 'error') {
    await page.evaluate((initialDoctor) => {
      const target = window
      if (!target.__fcvMatteOriginalFetch) target.__fcvMatteOriginalFetch = window.fetch
      const originalFetch = target.__fcvMatteOriginalFetch
      target.__fcvMatteFixture = {
        doctor: initialDoctor,
        doctorCalls: 0,
        setupCalls: [],
        jobCalls: [],
        editCalls: [],
        setupFailure: false,
        pendingTier: null,
      }
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
        const fixture = target.__fcvMatteFixture

        if (pathname === '/api/verb/system.doctor') {
          fixture.doctorCalls += 1
          if (fixture.doctor === 'error') {
            return envelope({
              ok: false,
              error: { code: 'fixture_probe_failed', message: 'deterministic doctor failure' },
            })
          }
          const cards = []
          if (fixture.doctor === 'absent') {
            cards.push({
              id: 'matte',
              status: 'missing',
              hint: 'Install the on-device background-removal model.',
              details: {},
            })
            cards.push({ id: 'matte_premium', status: 'missing', details: {} })
          } else if (fixture.doctor === 'base') {
            cards.push({ id: 'matte', status: 'ok', details: {} })
            cards.push({ id: 'matte_premium', status: 'missing', details: {} })
          } else {
            cards.push({
              id: 'matte',
              status: fixture.doctor === 'premium-only' ? 'missing' : 'ok',
              details: {},
            })
            cards.push({ id: 'matte_premium', status: 'ok', details: {} })
          }
          return envelope({
            ok: true,
            result: {
              schema: 'shellx-cut/doctor-matte-fixture@1',
              cards,
            },
          })
        }

        if (pathname === '/api/verb/system.setup_matte') {
          const body = requestArgs(options)
          fixture.setupCalls.push(body)
          if (fixture.setupFailure) {
            return envelope({
              ok: false,
              error: { code: 'fixture_setup_failed', message: 'deterministic setup failure' },
            })
          }
          const tier = body.model === 'matanyone' ? 'matanyone' : 'rvm'
          fixture.pendingTier = tier
          return envelope({ ok: true, result: { job_id: `fixture-${tier}` } })
        }

        if (pathname === '/api/verb/jobs.status') {
          const body = requestArgs(options)
          fixture.jobCalls.push(body)
          if (fixture.pendingTier === 'matanyone') {
            fixture.doctor = fixture.doctor === 'base' ? 'premium' : 'premium-only'
          } else if (fixture.pendingTier === 'rvm') {
            fixture.doctor = 'base'
          }
          fixture.pendingTier = null
          return envelope({ ok: true, result: { state: 'done', progress: 1 } })
        }

        if (pathname === '/api/verb/edit.matte') {
          const body = requestArgs(options)
          fixture.editCalls.push(body)
          return envelope({
            ok: true,
            result: {
              clip: body.clip,
              enabled: body.enabled !== false,
              model: body.model,
              mode: body.mode,
            },
          })
        }

        return originalFetch(...args)
      }
    }, doctor)
  }

  async function setFixture(page, patch) {
    await page.evaluate((next) => Object.assign(window.__fcvMatteFixture, next), patch)
  }

  async function fixtureState(page) {
    return page.evaluate(() => JSON.parse(JSON.stringify(window.__fcvMatteFixture)))
  }

  async function restoreFixture(page) {
    await page.evaluate(() => {
      const target = window
      if (target.__fcvMatteOriginalFetch) {
        window.fetch = target.__fcvMatteOriginalFetch
        delete target.__fcvMatteOriginalFetch
      }
      delete target.__fcvMatteFixture
    })
  }

  async function openDrawer(page, clipId) {
    if (!(await selectClip(page, clipId))) {
      throw new Error(`Matte fixture could not select ${clipId}`)
    }
    await page.locator('[data-cut-action="open-matte"]').first().click()
    const drawer = page.locator('[data-cut-matte]').first()
    await drawer.waitFor({ state: 'visible', timeout: 12_000 })
    return drawer
  }

  async function clickState(page, drawer, {
    name,
    actionId,
    selector,
    assertResult,
    waitMs = 70,
  }) {
    const control = page.locator(selector).first()
    await probe(page, {
      surface,
      name,
      actionId,
      sel: control,
      group: drawer,
      groupName: 'matte-controls',
      doClick: async () => {
        await control.click()
        await sleep(waitMs)
      },
      assertResult,
    })
  }

  async function fillState(page, drawer, {
    name,
    actionId,
    selector,
    value,
  }) {
    const control = page.locator(selector).first()
    await probe(page, {
      surface,
      name,
      actionId,
      sel: control,
      group: drawer,
      groupName: 'matte-fields',
      doClick: async () => {
        await control.fill(String(value))
        await sleep(60)
      },
      assertResult: async () => ({
        ok: (await control.inputValue()) === String(value),
        detail: `${name}=${await control.inputValue()}`,
      }),
    })
  }

  async function run(page) {
    await freshProject(page, 'matte-actions')
    await closeOverlays(page)
    const clipId = await clipOfKind('video')
    if (!clipId) throw new Error('Matte action fixture has no video clip')
    await installFixture(page, 'error')

    try {
      let drawer = await openDrawer(page, clipId)
      await page.locator('[data-cut-matte-probe-error]').first().waitFor({
        state: 'visible',
        timeout: 8000,
      })

      let doctorCalls = 0
      await clickState(page, drawer, {
        name: 'matte-recheck-after-probe-error',
        actionId: 'matte-recheck',
        selector: '[data-cut-matte-recheck]',
        waitMs: 130,
        assertResult: async () => {
          const fixture = await fixtureState(page)
          return {
            ok: fixture.doctorCalls > doctorCalls
              && await page.locator('[data-cut-matte-probe-error]').first().isVisible(),
            detail: `doctor calls ${doctorCalls}→${fixture.doctorCalls}; probe error remains visible`,
          }
        },
      })

      await setFixture(page, { doctor: 'absent' })
      doctorCalls = (await fixtureState(page)).doctorCalls
      await clickState(page, drawer, {
        name: 'matte-recheck-to-requirements',
        actionId: 'matte-recheck',
        selector: '[data-cut-matte-recheck]',
        waitMs: 130,
        assertResult: async () => {
          const fixture = await fixtureState(page)
          const visible = await waitFor(() => (
            page.locator('[data-cut-matte-requirements]').first().isVisible()
          ))
          return {
            ok: fixture.doctorCalls > doctorCalls && !!visible,
            detail: `doctor calls ${doctorCalls}→${fixture.doctorCalls}; requirements=${!!visible}`,
          }
        },
      })

      // A failed install must stop at its error. It must not immediately run a
      // second doctor request that can obscure the setup failure.
      await setFixture(page, { setupFailure: true })
      doctorCalls = (await fixtureState(page)).doctorCalls
      await clickState(page, drawer, {
        name: 'matte-install-rvm-failure',
        actionId: 'matte-install-rvm',
        selector: '[data-cut-matte-install-rvm]',
        waitMs: 150,
        assertResult: async () => {
          const fixture = await fixtureState(page)
          const error = await page.locator('[data-cut-matte-error]').first().textContent().catch(() => '')
          return {
            ok: fixture.setupCalls.at(-1)?.model === 'rvm'
              && fixture.doctorCalls === doctorCalls
              && /deterministic setup failure/.test(error || ''),
            detail: `rvm failure surfaced; doctor calls stayed ${doctorCalls}; error="${error || ''}"`,
          }
        },
      })

      await setFixture(page, { setupFailure: false })
      await clickState(page, drawer, {
        name: 'matte-install-rvm-success',
        actionId: 'matte-install-rvm',
        selector: '[data-cut-matte-install-rvm]',
        waitMs: 180,
        assertResult: async () => {
          const fixture = await fixtureState(page)
          const ready = await waitFor(() => (
            page.locator('[data-cut-matte-ready="true"] [data-cut-matte-model]').first().isVisible()
          ))
          return {
            ok: fixture.setupCalls.at(-1)?.model === 'rvm'
              && fixture.jobCalls.at(-1)?.job_id === 'fixture-rvm'
              && !!ready,
            detail: `rvm setup→job→ready=${!!ready}`,
          }
        },
      })

      await clickState(page, drawer, {
        name: 'matte-model-rvm',
        actionId: 'matte-model-rvm',
        selector: '[data-cut-matte-model-rvm]',
        assertResult: async () => ({
          ok: (await page.locator('[data-cut-matte-model-rvm]').first().getAttribute('aria-selected')) === 'true',
          detail: 'Standard RVM selected',
        }),
      })
      await clickState(page, drawer, {
        name: 'matte-mode-remove',
        actionId: 'matte-mode-remove',
        selector: '[data-cut-matte-mode-remove]',
        assertResult: async () => ({
          ok: (await page.locator('[data-cut-matte-mode-remove]').first().getAttribute('aria-selected')) === 'true',
          detail: 'remove mode selected',
        }),
      })
      await clickState(page, drawer, {
        name: 'matte-mode-replace',
        actionId: 'matte-mode-replace',
        selector: '[data-cut-matte-mode-replace]',
        assertResult: async () => ({
          ok: (await page.locator('[data-cut-matte-mode-replace]').first().getAttribute('aria-selected')) === 'true',
          detail: 'replace mode selected',
        }),
      })
      await fillState(page, drawer, {
        name: 'matte-background-color',
        actionId: 'matte-bg',
        selector: '[data-cut-matte-bg]',
        value: '#123ABC',
      })
      await clickState(page, drawer, {
        name: 'matte-quality-fast',
        actionId: 'matte-quality-fast',
        selector: '[data-cut-matte-quality-fast]',
        assertResult: async () => ({
          ok: (await page.locator('[data-cut-matte-quality-fast]').first().getAttribute('aria-selected')) === 'true',
          detail: 'fast quality selected',
        }),
      })
      await clickState(page, drawer, {
        name: 'matte-quality-good',
        actionId: 'matte-quality-good',
        selector: '[data-cut-matte-quality-good]',
        assertResult: async () => ({
          ok: (await page.locator('[data-cut-matte-quality-good]').first().getAttribute('aria-selected')) === 'true',
          detail: 'good quality selected',
        }),
      })

      await clickState(page, drawer, {
        name: 'matte-model-premium-reveal-consent',
        actionId: 'matte-model-premium',
        selector: '[data-cut-matte-model-premium]',
        waitMs: 100,
        assertResult: async () => ({
          ok: await page.locator('[data-cut-matte-premium-consent]').first().isVisible(),
          detail: 'Premium consent and install block visible',
        }),
      })
      doctorCalls = (await fixtureState(page)).doctorCalls
      await clickState(page, drawer, {
        name: 'matte-premium-recheck',
        actionId: 'matte-premium-recheck',
        selector: '[data-cut-matte-premium-recheck]',
        waitMs: 140,
        assertResult: async () => {
          const fixture = await fixtureState(page)
          return {
            ok: fixture.doctorCalls > doctorCalls
              && await page.locator('[data-cut-matte-premium-consent]').first().isVisible(),
            detail: `premium recheck doctor calls ${doctorCalls}→${fixture.doctorCalls}`,
          }
        },
      })
      await clickState(page, drawer, {
        name: 'matte-install-premium-from-controls',
        actionId: 'matte-install-premium',
        selector: '[data-cut-matte-premium-consent] [data-cut-matte-install-premium]',
        waitMs: 180,
        assertResult: async () => {
          const fixture = await fixtureState(page)
          const premiumReady = await waitFor(async () => (
            (await page.locator('[data-cut-matte-model-premium]').first().textContent())?.trim() === 'Premium'
          ))
          const args = fixture.setupCalls.at(-1)
          return {
            ok: args?.model === 'matanyone'
              && args?.accept_noncommercial === true
              && fixture.jobCalls.at(-1)?.job_id === 'fixture-matanyone'
              && !!premiumReady,
            detail: `premium consent=${args?.accept_noncommercial}; job completed; ready=${!!premiumReady}`,
          }
        },
      })
      await clickState(page, drawer, {
        name: 'matte-model-premium',
        actionId: 'matte-model-premium',
        selector: '[data-cut-matte-model-premium]',
        assertResult: async () => ({
          ok: (await page.locator('[data-cut-matte-model-premium]').first().getAttribute('aria-selected')) === 'true',
          detail: 'MatAnyone2 selected after install',
        }),
      })
      await clickState(page, drawer, {
        name: 'matte-pick-subject',
        actionId: 'matte-pick',
        selector: '[data-cut-matte-pick]',
        assertResult: async () => ({
          ok: await page.locator('[data-cut-matte-pick]').first().isChecked()
            && await page.locator('[data-cut-matte-pick-xy]').first().isVisible(),
          detail: 'subject point enabled and coordinates visible',
        }),
      })
      await fillState(page, drawer, {
        name: 'matte-pick-x',
        actionId: 'matte-pick-x',
        selector: '[data-cut-matte-pick-x]',
        value: 0.33,
      })
      await fillState(page, drawer, {
        name: 'matte-pick-y',
        actionId: 'matte-pick-y',
        selector: '[data-cut-matte-pick-y]',
        value: 0.67,
      })

      const apply = page.locator('[data-cut-matte-apply]').first()
      await probe(page, {
        surface,
        name: 'matte-apply-premium-replace',
        actionId: 'matte-apply',
        sel: apply,
        group: drawer,
        groupName: 'matte-apply',
        doClick: async () => {
          await apply.click()
          await waitFor(() => page.locator('[data-cut-matte-result]').first().isVisible())
        },
        assertResult: async () => {
          const args = (await fixtureState(page)).editCalls.at(-1)
          const result = await page.locator('[data-cut-matte-result-state]').first().textContent().catch(() => '')
          return {
            ok: args?.clip === clipId
              && args?.enabled === true
              && args?.model === 'matanyone'
              && args?.mode === 'replace'
              && args?.quality === 'good'
              && args?.bg?.color === '#123ABC'
              && args?.seed?.point?.[0] === 0.33
              && args?.seed?.point?.[1] === 0.67
              && /matte applied/.test(result || ''),
            detail: `apply payload=${JSON.stringify(args)}; result="${result || ''}"`,
          }
        },
      })

      const clear = page.locator('[data-cut-matte-remove]').first()
      await probe(page, {
        surface,
        name: 'matte-clear',
        actionId: 'matte-remove',
        sel: clear,
        group: drawer,
        groupName: 'matte-clear',
        doClick: async () => {
          await clear.click()
          await waitFor(async () => /matte cleared/.test(
            await page.locator('[data-cut-matte-result-state]').first().textContent().catch(() => ''),
          ))
        },
        assertResult: async () => {
          const args = (await fixtureState(page)).editCalls.at(-1)
          const result = await page.locator('[data-cut-matte-result-state]').first().textContent().catch(() => '')
          return {
            ok: args?.clip === clipId && args?.enabled === false && /matte cleared/.test(result || ''),
            detail: `clear payload=${JSON.stringify(args)}; result="${result || ''}"`,
          }
        },
      })

      // The requirements card and the controls consent block are separate DOM
      // locations. Re-mount from a clean absent state and exercise the former.
      await page.locator('[data-cut-matte-close]').first().click()
      await waitFor(() => page.locator('[data-cut-matte]').count().then((count) => count === 0))
      await setFixture(page, {
        doctor: 'absent',
        doctorCalls: 0,
        setupCalls: [],
        jobCalls: [],
        editCalls: [],
        setupFailure: false,
        pendingTier: null,
      })
      drawer = await openDrawer(page, clipId)
      await page.locator('[data-cut-matte-requirements]').first().waitFor({
        state: 'visible',
        timeout: 8000,
      })
      await clickState(page, drawer, {
        name: 'matte-install-premium-from-requirements',
        actionId: 'matte-install-premium',
        selector: '[data-cut-matte-requirements] [data-cut-matte-install-premium]',
        waitMs: 180,
        assertResult: async () => {
          const fixture = await fixtureState(page)
          const controls = await waitFor(() => (
            page.locator('[data-cut-matte-ready="true"] [data-cut-matte-model]').first().isVisible()
          ))
          const args = fixture.setupCalls.at(-1)
          return {
            ok: args?.model === 'matanyone'
              && args?.accept_noncommercial === true
              && fixture.jobCalls.at(-1)?.job_id === 'fixture-matanyone'
              && !!controls,
            detail: `requirements premium consent=${args?.accept_noncommercial}; controls=${!!controls}`,
          }
        },
      })

      const close = page.locator('[data-cut-matte-close]').first()
      await probe(page, {
        surface,
        name: 'matte-close',
        actionId: 'matte-close',
        sel: close,
        group: drawer,
        groupName: 'matte-close',
        doClick: async () => {
          await close.click()
          await sleep(90)
        },
        assertResult: async () => ({
          ok: (await page.locator('[data-cut-matte]').count()) === 0,
          detail: `drawers=${await page.locator('[data-cut-matte]').count()}`,
        }),
      })
    } finally {
      await restoreFixture(page)
      await closeOverlays(page)
    }
  }

  return { run }
}
