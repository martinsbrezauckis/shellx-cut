// Installed-WebView coverage for the remaining topbar navigation, delivery
// choices, render action, and the conditional missing-FFmpeg recovery strip.
// A narrow fixture prevents a real render and external browser launch while
// preserving the shipped controls, exact requests, and UI state transitions.

export function createTopbarActionCoverage({
  probe,
  sleep,
  freshProject,
  closeOverlays,
  primaryMedia,
}) {
  const surface = 'topbar-actions'

  async function waitFor(check, timeoutMs = 10_000) {
    const deadline = Date.now() + timeoutMs
    let value = null
    while (Date.now() < deadline) {
      try {
        value = await check()
        if (value) return value
      } catch {}
      await sleep(80)
    }
    return value
  }

  async function installFixture(page) {
    await page.evaluate(() => {
      const target = window
      target.__fcvTopbarOriginalFetch = window.fetch
      target.__fcvTopbarOriginalOpen = window.open
      target.__fcvTopbarFixture = {
        doctorCalls: [],
        pregateCalls: [],
        renderCalls: [],
        opens: [],
      }
      const fixture = target.__fcvTopbarFixture
      const originalFetch = target.__fcvTopbarOriginalFetch
      const envelope = (body) => new Response(JSON.stringify(body), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      })
      const requestArgs = (options) => {
        try { return JSON.parse(options?.body || '{}') } catch { return {} }
      }
      const doctor = (missing) => ({
        schema: 'shellx-cut/doctor/1',
        scanned_at: '2026-07-29T00:00:00Z',
        os: 'fixture',
        arch: 'fixture',
        app_version: 'fixture',
        essential_ok: !missing,
        cards: [{
          id: 'ffmpeg',
          kind: 'tool',
          status: missing ? 'missing' : 'ok',
          source: missing ? 'missing' : 'path',
          version: missing ? undefined : 'fixture',
          hint: missing ? 'Install video processing.' : undefined,
          details: { can_stabilize: !missing },
        }],
      })
      window.fetch = async (...args) => {
        const input = args[0]
        const options = args[1]
        const url = typeof input === 'string' ? input : input?.url || ''
        let pathname = ''
        try { pathname = new URL(String(url), window.location.href).pathname } catch {}
        const body = requestArgs(options)
        if (pathname === '/api/verb/system.doctor') {
          fixture.doctorCalls.push(body)
          return envelope({
            ok: true,
            result: doctor(fixture.doctorCalls.length === 1),
          })
        }
        if (pathname === '/api/verb/verify.pregate') {
          fixture.pregateCalls.push(body)
          return envelope({
            ok: true,
            result: {
              pass: true,
              risks: [],
              summary: 'Fixture preflight passed.',
              perception_assets: 1,
              uninstrumented_assets: [],
            },
          })
        }
        if (pathname === '/api/verb/render.final') {
          fixture.renderCalls.push(body)
          return envelope({
            ok: true,
            result: { job_id: 'job_fcv_topbar_render' },
          })
        }
        return originalFetch(...args)
      }
      window.open = (...args) => {
        fixture.opens.push(args.map((value) => value == null ? null : String(value)))
        return null
      }
    })
  }

  async function fixtureState(page) {
    return page.evaluate(() => JSON.parse(JSON.stringify(window.__fcvTopbarFixture)))
  }

  async function restoreFixture(page) {
    await page.evaluate(() => {
      const target = window
      if (target.__fcvTopbarOriginalFetch) window.fetch = target.__fcvTopbarOriginalFetch
      if (target.__fcvTopbarOriginalOpen) window.open = target.__fcvTopbarOriginalOpen
      delete target.__fcvTopbarOriginalFetch
      delete target.__fcvTopbarOriginalOpen
      delete target.__fcvTopbarFixture
      document.dispatchEvent(new CustomEvent('cut:refresh-doctor'))
    })
  }

  async function run(page) {
    await freshProject(page, 'topbar_actions', primaryMedia)
    await closeOverlays(page)
    await installFixture(page)
    try {
      // The topbar Projects action is a toggle. Normalize to the neighboring
      // Assets destination so this probe always proves its open/reveal branch,
      // independent of persisted layout from a previous section.
      await page.evaluate(() => document.dispatchEvent(new CustomEvent('cut:open-ui-surface', {
        detail: { id: 'assets' },
      })))
      await page.locator('[data-cut-left-tab="assets"][aria-selected="true"]').first()
        .waitFor({ state: 'visible', timeout: 8_000 })
      const projects = page.locator('[data-cut-projects-btn]').first()
      await probe(page, {
        surface,
        name: 'projects-btn',
        actionId: 'projects-btn',
        sel: projects,
        group: page.locator('[data-cut-panel="topbar"]').first(),
        groupName: 'topbar-navigation',
        doClick: async () => {
          await projects.click()
          await page.locator('[data-cut-left-tab="projects"][aria-selected="true"]').first()
            .waitFor({ state: 'visible', timeout: 8_000 })
        },
        assertResult: async () => ({
          ok: await page.locator('[data-cut-panel="projects"]').first().isVisible()
            && await page.locator('[data-cut-left-tab="projects"]').first().getAttribute('aria-selected') === 'true',
          detail: `Projects selected=${await page.locator('[data-cut-left-tab="projects"]').first().getAttribute('aria-selected')}; panel visible=${await page.locator('[data-cut-panel="projects"]').first().isVisible()}`,
        }),
      })

      const manual = page.locator('[data-cut-manual-link]').first()
      await probe(page, {
        surface,
        name: 'manual-link',
        actionId: 'manual-link',
        sel: manual,
        group: page.locator('[data-cut-panel="topbar"]').first(),
        groupName: 'topbar-navigation',
        doClick: async () => { await manual.click() },
        assertResult: async () => {
          const fixture = await fixtureState(page)
          const call = fixture.opens[0]
          const exact = call?.[0] === 'https://docs.theshellx.com/manual/cut/'
            && call?.[1] === '_blank'
            && call?.[2] === 'noopener,noreferrer'
          return { ok: fixture.opens.length === 1 && exact, detail: `exact manual launch=${exact}; args=${JSON.stringify(call)}` }
        },
      })

      const gpu = page.locator('[data-cut-gpu-toggle]').first()
      await probe(page, {
        surface,
        name: 'gpu-toggle',
        actionId: 'gpu-toggle',
        sel: gpu,
        group: page.locator('.tb-render').first(),
        groupName: 'topbar-render-zone',
        doClick: async () => {
          if ((await gpu.getAttribute('aria-pressed')) === 'true') await gpu.click()
        },
        assertResult: async () => {
          const pressed = await gpu.getAttribute('aria-pressed')
          const label = await gpu.textContent()
          return { ok: pressed === 'false' && label?.includes('Faster OFF'), detail: `pressed=${pressed}; label=${label}` }
        },
      })

      await page.locator('[data-cut-render-opts]').first().click()
      const renderMenu = page.locator('[data-cut-render-menu]').first()
      await renderMenu.waitFor({ state: 'visible', timeout: 5_000 })

      const renderGpu = page.locator('[data-cut-render-gpu]').first()
      await probe(page, {
        surface,
        name: 'render-gpu',
        actionId: 'render-gpu',
        sel: renderGpu,
        group: renderMenu,
        groupName: 'topbar-render-options',
        doClick: async () => { await renderGpu.click() },
        assertResult: async () => {
          const checked = await renderGpu.isChecked()
          const synced = await gpu.getAttribute('aria-pressed')
          return { ok: checked && synced === 'true', detail: `option checked=${checked}; header pressed=${synced}` }
        },
      })

      const profile = page.locator('[data-cut-render-profile]').first()
      await probe(page, {
        surface,
        name: 'render-profile',
        actionId: 'render-profile',
        sel: profile,
        group: renderMenu,
        groupName: 'topbar-render-options-gpu',
        doClick: async () => { await profile.selectOption('talking_head') },
        assertResult: async () => ({
          ok: await profile.inputValue() === 'talking_head',
          detail: `profile=${await profile.inputValue()}`,
        }),
      })

      const render = page.locator('[data-cut-render-btn]').first()
      await probe(page, {
        surface,
        name: 'render-btn',
        actionId: 'render-btn',
        sel: render,
        group: renderMenu,
        groupName: 'topbar-render-options-selected',
        doClick: async () => {
          await render.click()
          await waitFor(async () =>
            (await page.locator('[data-cut-topbar-note]').textContent())?.includes('job_fcv_topbar_render'))
        },
        assertResult: async () => {
          const fixture = await fixtureState(page)
          const call = fixture.renderCalls[0]
          const exact = fixture.pregateCalls.length === 1
            && Object.keys(fixture.pregateCalls[0] || {}).length === 0
            && call?.preset === 'standard'
            && call?.format === 'h264'
            && call?.hardware === 'auto'
            && call?.profile === 'talking_head'
            && !('normalize_loudness' in (call || {}))
          const note = await page.locator('[data-cut-topbar-note]').textContent()
          return {
            ok: fixture.renderCalls.length === 1 && exact
              && note === 'render · job_fcv_topbar_render',
            detail: `exact pregate+render=${exact}; note=${note}; args=${JSON.stringify(call)}`,
          }
        },
      })

      await page.evaluate(() => document.dispatchEvent(new CustomEvent('cut:refresh-doctor')))
      const setup = page.locator('[data-cut-export-ffmpeg-setup]').first()
      await setup.waitFor({ state: 'visible', timeout: 8_000 })

      const install = page.locator('[data-cut-export-install-ffmpeg]').first()
      await probe(page, {
        surface,
        name: 'export-install-ffmpeg',
        actionId: 'export-install-ffmpeg',
        sel: install,
        group: setup,
        groupName: 'topbar-ffmpeg-setup',
        doClick: async () => {
          await install.click()
          await page.locator('[data-cut-settings-body="video-performance"]').first()
            .waitFor({ state: 'visible', timeout: 8_000 })
        },
        assertResult: async () => {
          const routed = await page.locator('[data-cut-settings-body="video-performance"]').first().isVisible()
          await page.locator('[data-cut-environment-close]').first().click().catch(() => {})
          return { ok: routed, detail: `Settings > Video performance=${routed}` }
        },
      })

      const guide = page.locator('[data-cut-export-ffmpeg-guide]').first()
      await probe(page, {
        surface,
        name: 'export-ffmpeg-guide',
        actionId: 'export-ffmpeg-guide',
        sel: guide,
        group: setup,
        groupName: 'topbar-ffmpeg-setup',
        doClick: async () => { await guide.click() },
        assertResult: async () => {
          const fixture = await fixtureState(page)
          const call = fixture.opens[1]
          const exact = call?.[0] === 'https://docs.theshellx.com/manual/cut/?feature=cut.preview.ffmpeg_setup'
            && call?.[1] === '_blank'
          return { ok: fixture.opens.length === 2 && exact, detail: `exact FFmpeg guide=${exact}; args=${JSON.stringify(call)}` }
        },
      })

      const recheck = page.locator('[data-cut-export-ffmpeg-recheck]').first()
      await probe(page, {
        surface,
        name: 'export-ffmpeg-recheck',
        actionId: 'export-ffmpeg-recheck',
        sel: recheck,
        group: setup,
        groupName: 'topbar-ffmpeg-setup',
        doClick: async () => {
          await recheck.click()
          await page.locator('[data-cut-export-ffmpeg-setup]').first()
            .waitFor({ state: 'detached', timeout: 8_000 })
        },
        assertResult: async () => {
          const fixture = await fixtureState(page)
          const exact = fixture.doctorCalls.length === 2
            && fixture.doctorCalls.every((args) => args.refresh === true)
          const cleared = await page.locator('[data-cut-export-ffmpeg-setup]').count() === 0
          return { ok: exact && cleared, detail: `two refresh:true checks=${exact}; setup cleared=${cleared}` }
        },
      })
    } finally {
      await page.locator('[data-cut-environment-close]').first().click().catch(() => {})
      await restoreFixture(page)
      await sleep(250)
    }
  }

  return { run }
}
