// Installed-WebView coverage for conditional topbar dialogs that otherwise
// depend on a native file picker or a particular preflight report. This keeps
// the shipped OTIO and export-warning UI intact while replacing only those
// external boundaries with deterministic responses.

export function createTopbarDialogActionCoverage({
  probe,
  sleep,
  freshProject,
  closeOverlays,
  primaryMedia,
  nativeOsActionsEnabled = false,
}) {
  const surface = 'topbar-dialog-actions'

  async function waitFor(check, timeoutMs = 8_000) {
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
      target.__fcvTopbarDialogsOriginalFetch = window.fetch
      target.__fcvTopbarDialogsOriginalOpen = window.open
      target.__fcvTopbarDialogsOriginalTauri = target.__TAURI__
      target.__fcvTopbarDialogsOriginalInternals = target.__TAURI_INTERNALS__
      target.__fcvTopbarDialogsOriginalInternalInvoke = target.__TAURI_INTERNALS__?.invoke
      target.__fcvTopbarDialogsFixture = {
        dialogCalls: [],
        importCalls: [],
        pregateCalls: [],
        renderCalls: [],
        opens: [],
      }
      const fixture = target.__fcvTopbarDialogsFixture
      const originalFetch = target.__fcvTopbarDialogsOriginalFetch
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
        const body = requestArgs(options)
        if (pathname === '/api/verb/import.otio') {
          fixture.importCalls.push(body)
          if (body.mode === 'preview') {
            return envelope({ ok: true, result: {
              status: 'preview',
              path: '/fixture/editorial-cut.otio',
              source_hash: 'sha256:fixture-otio',
              name: 'Editorial cut',
              tracks: [
                { name: 'Picture', kind: 'video', clips: 3, gaps: 1, duration_ms: 62_000 },
                { name: 'Dialogue', kind: 'audio', clips: 2, gaps: 0, duration_ms: 62_000 },
              ],
              track_count: 2,
              clips: 5,
              gaps: 1,
              media_references: 4,
              media_available: 3,
              media_missing: 1,
              missing_clips: 1,
              missing_sources: ['offline-broll.mov'],
              source_format: { width: 3840, height: 2160, fps: 23.976 },
              format_policy: 'preserve_project',
            } })
          }
          return envelope({ ok: true, result: {
            tracks_created: 2,
            clips_inserted: 5,
            missing_clips: 1,
          } })
        }
        if (pathname === '/api/verb/verify.pregate') {
          fixture.pregateCalls.push(body)
          return envelope({ ok: true, result: {
            pass: true,
            summary: 'One pacing warning is worth reviewing before export.',
            risks: [{
              kind: 'slideshow_risk',
              severity: 'med',
              detail: 'Only one visual cut was detected in the first 30 seconds.',
              range_ms: [0, 30_000],
            }],
            perception_assets: 1,
            uninstrumented_assets: ['a2'],
          } })
        }
        if (pathname === '/api/verb/render.final') {
          fixture.renderCalls.push(body)
          return envelope({ ok: true, result: { job_id: 'job_fcv_dialog_render' } })
        }
        return originalFetch(...args)
      }
      window.open = (...args) => {
        fixture.opens.push(args.map((value) => value == null ? null : String(value)))
        return null
      }
      const invoke = async (command, args, options) => {
        if (command === 'plugin:dialog|open') {
          fixture.dialogCalls.push({ command, args })
          return '/fixture/editorial-cut.otio'
        }
        const original = target.__fcvTopbarDialogsOriginalInternalInvoke
        if (typeof original === 'function') return original(command, args, options)
        return null
      }
      if (target.__TAURI_INTERNALS__) target.__TAURI_INTERNALS__.invoke = invoke
      else target.__TAURI_INTERNALS__ = { invoke }
      if (!target.__TAURI__) {
        target.__TAURI__ = {
          core: { invoke },
          event: { listen: async () => () => {} },
        }
      }
    })
  }

  async function fixtureState(page) {
    return page.evaluate(() => JSON.parse(JSON.stringify(window.__fcvTopbarDialogsFixture)))
  }

  async function restoreFixture(page) {
    await page.evaluate(() => {
      const target = window
      if (target.__fcvTopbarDialogsOriginalFetch) window.fetch = target.__fcvTopbarDialogsOriginalFetch
      if (target.__fcvTopbarDialogsOriginalOpen) window.open = target.__fcvTopbarDialogsOriginalOpen
      if (target.__fcvTopbarDialogsOriginalInternals) {
        target.__fcvTopbarDialogsOriginalInternals.invoke =
          target.__fcvTopbarDialogsOriginalInternalInvoke
        target.__TAURI_INTERNALS__ = target.__fcvTopbarDialogsOriginalInternals
      } else {
        delete target.__TAURI_INTERNALS__
      }
      if (target.__fcvTopbarDialogsOriginalTauri) target.__TAURI__ = target.__fcvTopbarDialogsOriginalTauri
      else delete target.__TAURI__
      delete target.__fcvTopbarDialogsOriginalFetch
      delete target.__fcvTopbarDialogsOriginalOpen
      delete target.__fcvTopbarDialogsOriginalTauri
      delete target.__fcvTopbarDialogsOriginalInternals
      delete target.__fcvTopbarDialogsOriginalInternalInvoke
      delete target.__fcvTopbarDialogsFixture
    })
  }

  async function openOtio(page) {
    await page.locator('[data-cut-left-tab="assets"]').first().click()
    await page.locator('[data-cut-import-otio]').first().click()
    await page.locator('[data-cut-otio-import]').waitFor({ state: 'visible', timeout: 8_000 })
  }

  async function triggerPreflight(page) {
    await page.locator('[data-cut-render-btn]').first().click()
    await page.locator('[data-cut-pregate-warning]').waitFor({ state: 'visible', timeout: 8_000 })
  }

  async function run(page) {
    await freshProject(page, 'topbar_dialog_actions', primaryMedia)
    await closeOverlays(page)
    await installFixture(page)
    try {
      // The exact native-OTIO lane owns picker + preview + cancel/close/confirm
      // on installed hosts. Repeating these through a monkey-patched Tauri
      // invoke is neither exact nor reliable once the plugin module is loaded.
      if (!nativeOsActionsEnabled) {
        await openOtio(page)
        await probe(page, {
        surface,
        name: 'cancel-otio-preview',
        actionId: 'otio-cancel',
        sel: page.locator('[data-cut-otio-cancel]'),
        group: page.locator('[data-cut-otio-import]'),
        groupName: 'otio-preview-cancel',
        doClick: async () => {
          await page.locator('[data-cut-otio-cancel]').click()
          await page.locator('[data-cut-otio-import]').waitFor({ state: 'detached', timeout: 5_000 })
        },
        assertResult: async () => {
          const fixture = await fixtureState(page)
          return {
            ok: await page.locator('[data-cut-otio-import]').count() === 0
              && fixture.importCalls.length === 1
              && fixture.importCalls[0]?.mode === 'preview',
            detail: `modal count=${await page.locator('[data-cut-otio-import]').count()}; import calls=${JSON.stringify(fixture.importCalls)}`,
          }
        },
        })

        await openOtio(page)
        await probe(page, {
        surface,
        name: 'close-otio-preview',
        actionId: 'otio-close',
        sel: page.locator('[data-cut-otio-close]'),
        group: page.locator('[data-cut-otio-import]'),
        groupName: 'otio-preview-close',
        doClick: async () => {
          await page.locator('[data-cut-otio-close]').click()
          await page.locator('[data-cut-otio-import]').waitFor({ state: 'detached', timeout: 5_000 })
        },
        assertResult: async () => ({
          ok: await page.locator('[data-cut-otio-import]').count() === 0,
          detail: `modal count=${await page.locator('[data-cut-otio-import]').count()}`,
        }),
        })

        await openOtio(page)
        await probe(page, {
        surface,
        name: 'confirm-otio-replacement',
        actionId: 'otio-confirm',
        sel: page.locator('[data-cut-otio-confirm]'),
        group: page.locator('[data-cut-otio-import]'),
        groupName: 'otio-preview-confirm',
        doClick: async () => {
          await page.locator('[data-cut-otio-confirm]').click()
          await page.locator('[data-cut-otio-import]').waitFor({ state: 'detached', timeout: 5_000 })
          await waitFor(async () =>
            (await page.locator('[data-cut-topbar-note]').textContent())?.includes('Imported timeline'))
        },
        assertResult: async () => {
          const fixture = await fixtureState(page)
          const replace = fixture.importCalls.at(-1)
          const exact = replace?.path === '/fixture/editorial-cut.otio'
            && replace?.mode === 'replace'
            && replace?.expected_hash === 'sha256:fixture-otio'
            && replace?.rationale === 'confirmed OTIO preview in the desktop app'
          const note = await page.locator('[data-cut-topbar-note]').textContent()
          return {
            ok: exact && note === 'Imported timeline — 5 clips on 2 tracks · 1 offline',
            detail: `exact replace=${exact}; note="${note}"; args=${JSON.stringify(replace)}`,
          }
        },
        })
      }

      await triggerPreflight(page)
      const warning = page.locator('[data-cut-pregate-warning]')
      await probe(page, {
        surface,
        name: 'open-preflight-details',
        actionId: 'pregate-details-toggle',
        sel: warning.locator('[data-cut-pregate-details-toggle]'),
        group: warning,
        groupName: 'pregate-warning-details',
        doClick: async () => {
          await warning.locator('[data-cut-pregate-details-toggle]').click()
          await warning.locator('[data-cut-pregate-details][open]').waitFor({ state: 'visible', timeout: 5_000 })
        },
        assertResult: async () => ({
          ok: await warning.locator('[data-cut-pregate-details]').getAttribute('open') !== null
            && (await warning.locator('[data-cut-pregate-details]').textContent())?.includes('slideshow_risk'),
          detail: `open=${await warning.locator('[data-cut-pregate-details]').getAttribute('open') !== null}; text="${(await warning.locator('[data-cut-pregate-details]').textContent())?.replace(/\s+/g, ' ').trim()}"`,
        }),
      })

      await probe(page, {
        surface,
        name: 'open-preflight-guide',
        actionId: 'pregate-guide',
        sel: warning.locator('[data-cut-pregate-guide]'),
        group: warning,
        groupName: 'pregate-warning-guide',
        doClick: async () => { await warning.locator('[data-cut-pregate-guide]').click() },
        assertResult: async () => {
          const fixture = await fixtureState(page)
          const open = fixture.opens.at(-1)
          const exact = open?.[0] === 'https://docs.theshellx.com/manual/cut/?feature=cut.export.preflight.pacing'
            && open?.[1] === '_blank'
          return { ok: exact, detail: `exact guide=${exact}; args=${JSON.stringify(open)}` }
        },
      })

      await probe(page, {
        surface,
        name: 'cancel-preflight',
        actionId: 'pregate-cancel',
        sel: warning.locator('[data-cut-pregate-cancel]'),
        group: warning,
        groupName: 'pregate-warning-cancel',
        doClick: async () => {
          await warning.locator('[data-cut-pregate-cancel]').click()
          await warning.waitFor({ state: 'detached', timeout: 5_000 })
        },
        assertResult: async () => {
          const fixture = await fixtureState(page)
          return {
            ok: await warning.count() === 0 && fixture.renderCalls.length === 0,
            detail: `warning count=${await warning.count()}; render calls=${fixture.renderCalls.length}`,
          }
        },
      })

      await triggerPreflight(page)
      await probe(page, {
        surface,
        name: 'close-preflight',
        actionId: 'pregate-close',
        sel: warning.locator('[data-cut-pregate-close]'),
        group: warning,
        groupName: 'pregate-warning-close',
        doClick: async () => {
          await warning.locator('[data-cut-pregate-close]').click()
          await warning.waitFor({ state: 'detached', timeout: 5_000 })
        },
        assertResult: async () => {
          const fixture = await fixtureState(page)
          return {
            ok: await warning.count() === 0 && fixture.renderCalls.length === 0,
            detail: `warning count=${await warning.count()}; render calls=${fixture.renderCalls.length}`,
          }
        },
      })

      // Complete one warning flow so the action stored behind the preflight is
      // also proven to survive two prior cancellation paths.
      await triggerPreflight(page)
      await warning.locator('[data-cut-pregate-continue]').click()
      await warning.waitFor({ state: 'detached', timeout: 5_000 })
      await waitFor(async () =>
        (await page.locator('[data-cut-topbar-note]').textContent()) === 'render · job_fcv_dialog_render')
      const fixture = await fixtureState(page)
      if (fixture.renderCalls.length !== 1 || fixture.pregateCalls.length !== 3) {
        throw new Error(`preflight continuation calls=${JSON.stringify(fixture)}`)
      }
    } finally {
      await restoreFixture(page)
      await closeOverlays(page)
    }
  }

  return { run }
}
