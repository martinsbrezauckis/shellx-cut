// Deterministic UI-action coverage for the Record workspace.
//
// The recorder rig remains the release proof for real OS capture, permissions,
// system audio, and produced pixels. This lane owns the WebView control wiring:
// it records exact screen_record payloads and returns bounded fixture envelopes,
// so conditional Stop, Export, Raw-add, and output-path actions mount on every
// host. Native file-dialog transport is separately owned by the OS-action gate;
// here its Tauri invoke is narrowly intercepted, then restored.

export function createRecordActionCoverage({
  probe,
  sleep,
  freshProject,
  closeOverlays,
  nativeOsActionsEnabled = false,
  nativeOutputPath = '/fixture/chosen-recording.mp4',
}) {
  const surface = 'record-actions'
  const sameHostPath = (left, right) => {
    const normalize = (value) => String(value || '')
      .replace(/^[/\\]{2}[?][/\\]/, '')
      .replace(/\\/g, '/')
      .replace(/\/+/g, '/')
      // macOS resolves /var and /tmp through /private. Native save panels may
      // return the physical path even when the supplied path used the public
      // alias; both names identify the same file.
      .replace(/^\/private\/var(?=\/|$)/i, '/var')
      .replace(/^\/private\/tmp(?=\/|$)/i, '/tmp')
      .toLowerCase()
    return normalize(left) === normalize(right)
  }

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

  async function installFixture(page) {
    await page.evaluate((useNativeDialog) => {
      const target = window
      if (!target.__fcvRecordOriginalFetch) target.__fcvRecordOriginalFetch = window.fetch
      const originalFetch = target.__fcvRecordOriginalFetch
      target.__fcvRecordOriginalTauri = target.__TAURI__
      target.__fcvRecordOriginalInternals = target.__TAURI_INTERNALS__
      target.__fcvRecordOriginalInternalInvoke = target.__TAURI_INTERNALS__?.invoke
      target.__fcvRecordFixture = {
        doctorCalls: [],
        startCalls: [],
        studioCalls: [],
        stopCalls: [],
        polishCalls: [],
        exportCalls: [],
        importCalls: [],
        saveCalls: [],
        captureSeq: 0,
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
        const fixture = target.__fcvRecordFixture

        if (pathname === '/api/verb/screen_record.doctor') {
          const body = requestArgs(options)
          fixture.doctorCalls.push(body)
          return envelope({
            ok: true,
            result: {
              ready: true,
              cards: [
                { name: 'ffmpeg', status: 'ok', detail: 'fixture media tools ready' },
                { name: 'screen_capture', status: 'ok', detail: 'fixture screen ready' },
                { name: 'input_hook', status: 'ok', detail: 'fixture input ready' },
                { name: 'webcam', status: 'missing', detail: 'not available in this release' },
              ],
              monitors: [
                { index: 1, name: 'Fixture main', width: 1920, height: 1080, primary: true },
                { index: 2, name: 'Fixture second', width: 1280, height: 720, primary: false },
              ],
              windows: [
                { id: 41, title: 'Fixture Window', app: 'Fixture App' },
              ],
              mic_warm: body.warm_mic
                ? { live: true, device: 'Fixture microphone', supported: true }
                : undefined,
            },
          })
        }

        if (pathname === '/api/verb/screen_record.start') {
          const body = requestArgs(options)
          fixture.startCalls.push(body)
          fixture.captureSeq += 1
          return envelope({
            ok: true,
            result: {
              capture_id: `fixture-capture-${fixture.captureSeq}`,
              out_dir: '/fixture/capture',
              status: 'recording',
              open_ended: body.duration_ms == null,
            },
          })
        }

        if (pathname === '/api/verb/screen_record.studio_event') {
          const body = requestArgs(options)
          fixture.studioCalls.push(body)
          return envelope({ ok: true, result: { appended: true } })
        }

        if (pathname === '/api/verb/screen_record.stop') {
          const body = requestArgs(options)
          fixture.stopCalls.push(body)
          if (body.mux_raw) {
            return envelope({
              ok: true,
              result: {
                raw_path: body.raw_path || '/fixture/raw-recording.mp4',
                raw_has_mic: true,
                raw_has_system: true,
                raw_streams: {
                  screen: '/fixture/screen.mp4',
                  mic: '/fixture/mic.wav',
                  system: '/fixture/system.wav',
                  studio_events: '/fixture/studio-events.json',
                },
              },
            })
          }
          return envelope({
            ok: true,
            result: {
              source: '/fixture/source.mp4',
              plan: '/fixture/plan.json',
              raw_streams: {
                screen: '/fixture/screen.mp4',
                mic: '/fixture/mic.wav',
                system: '/fixture/system.wav',
                studio_events: '/fixture/studio-events.json',
              },
            },
          })
        }

        if (pathname === '/api/verb/screen_record.polish') {
          const body = requestArgs(options)
          fixture.polishCalls.push(body)
          return envelope({ ok: true, result: { clip_id: 'fixture-polished-clip' } })
        }

        if (pathname === '/api/verb/screen_record.export') {
          const body = requestArgs(options)
          fixture.exportCalls.push(body)
          return envelope({
            ok: true,
            result: { path: `/fixture/export.${body.format || 'mp4'}` },
          })
        }

        if (pathname === '/api/verb/media.import') {
          const body = requestArgs(options)
          fixture.importCalls.push(body)
          return envelope({ ok: true, result: { asset: 'fixture-raw-asset' } })
        }

        return originalFetch(...args)
      }

      if (!useNativeDialog) {
        const invoke = async (command, args, options) => {
          if (command === 'plugin:dialog|save') {
            target.__fcvRecordFixture.saveCalls.push(args)
            return '/fixture/chosen-recording.mp4'
          }
          const original = target.__fcvRecordOriginalInternalInvoke
          if (typeof original === 'function') return original(command, args, options)
          return null
        }
        if (target.__TAURI_INTERNALS__) {
          target.__TAURI_INTERNALS__.invoke = invoke
        } else {
          target.__TAURI_INTERNALS__ = { invoke }
        }
        if (!target.__TAURI__) {
          target.__TAURI__ = {
            core: { invoke },
            event: { listen: async () => () => {} },
          }
        }
      }
    }, nativeOsActionsEnabled)
  }

  async function fixtureState(page) {
    return page.evaluate(() => JSON.parse(JSON.stringify(window.__fcvRecordFixture)))
  }

  async function restoreFixture(page) {
    await page.evaluate(() => {
      const target = window
      if (target.__fcvRecordOriginalFetch) window.fetch = target.__fcvRecordOriginalFetch
      if (target.__fcvRecordOriginalInternals) {
        target.__fcvRecordOriginalInternals.invoke = target.__fcvRecordOriginalInternalInvoke
        target.__TAURI_INTERNALS__ = target.__fcvRecordOriginalInternals
      } else {
        delete target.__TAURI_INTERNALS__
      }
      if (target.__fcvRecordOriginalTauri) target.__TAURI__ = target.__fcvRecordOriginalTauri
      else delete target.__TAURI__
      delete target.__fcvRecordOriginalFetch
      delete target.__fcvRecordOriginalTauri
      delete target.__fcvRecordOriginalInternals
      delete target.__fcvRecordOriginalInternalInvoke
      delete target.__fcvRecordFixture
    })
  }

  async function clickState(page, panel, {
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
      group: panel,
      groupName: 'record-workspace',
      doClick: async () => {
        await control.click()
        await sleep(waitMs)
      },
      assertResult,
    })
  }

  async function run(page) {
    await freshProject(page, 'record-actions')
    await closeOverlays(page)
    await installFixture(page)

    try {
      await page.locator('[data-cut-mode="record"]').first().click()
      const panel = page.locator('[data-cut-panel="record"]').first()
      await panel.waitFor({ state: 'visible', timeout: 12_000 })
      await page.locator('[data-cut-rec-card="screen_capture"]').first().waitFor({
        state: 'visible',
        timeout: 8000,
      })

      const source = page.locator('[data-cut-rec-source]').first()
      for (const [value, expectedKind, expected] of [
        ['mon:2', 'monitor', '2'],
        ['win:Fixture Window', 'window', 'Fixture Window'],
        ['mon:1', 'monitor', '1'],
      ]) {
        await probe(page, {
          surface,
          name: `record-source-${value.startsWith('win:') ? 'window' : value.slice(4)}`,
          actionId: 'rec-source',
          sel: source,
          group: panel,
          groupName: 'record-workspace',
          doClick: async () => {
            await source.selectOption(value)
            await sleep(70)
          },
          assertResult: async () => {
            const attr = expectedKind === 'window' ? 'data-cut-rec-window' : 'data-cut-rec-monitor'
            return {
              ok: (await source.inputValue()) === value
                && (await source.getAttribute(attr)) === expected,
              detail: `source=${await source.inputValue()}; ${attr}=${await source.getAttribute(attr)}`,
            }
          },
        })
      }

      for (const preset of ['none', '10000', '30000', '60000', '120000']) {
        await clickState(page, panel, {
          name: `record-duration-${preset}`,
          actionId: 'rec-dur',
          selector: `[data-cut-rec-dur="${preset}"]`,
          assertResult: async () => ({
            ok: (await page.locator(`[data-cut-rec-dur="${preset}"]`).first().getAttribute('class') || '').includes('rec__seg-btn--on'),
            detail: `duration ${preset} selected`,
          }),
        })
      }
      // Return to open-ended capture for a deterministic manual Stop.
      await page.locator('[data-cut-rec-dur="none"]').first().click()

      for (const fps of ['24', '30', '60']) {
        await clickState(page, panel, {
          name: `record-fps-${fps}`,
          actionId: 'rec-fps',
          selector: `[data-cut-rec-fps="${fps}"]`,
          assertResult: async () => ({
            ok: (await page.locator(`[data-cut-rec-fps="${fps}"]`).first().getAttribute('class') || '').includes('rec__seg-btn--on'),
            detail: `fps ${fps} selected`,
          }),
        })
      }

      for (const [name, actionId, selector] of [
        ['record-microphone-toggle', 'rec-audio-toggle-input', '[data-cut-rec-audio-toggle-input]'],
        ['record-system-audio-toggle', 'rec-system-audio-toggle-input', '[data-cut-rec-system-audio-toggle-input]'],
        ['record-keycast-toggle', 'rec-keys-toggle-input', '[data-cut-rec-keys-toggle-input]'],
        ['record-autopolish-toggle', 'rec-autopolish-toggle-input', '[data-cut-rec-autopolish-toggle-input]'],
      ]) {
        const control = page.locator(selector).first()
        let before = false
        await probe(page, {
          surface,
          name,
          actionId,
          sel: control,
          group: panel,
          groupName: 'record-workspace',
          doClick: async () => {
            before = await control.isChecked()
            await control.click()
            await sleep(80)
          },
          assertResult: async () => ({
            ok: (await control.isChecked()) !== before,
            detail: `${name} ${before}→${await control.isChecked()}`,
          }),
        })
        // Use microphone/system/keycast/polish ON for the exact Start payload.
        if (!(await control.isChecked())) await control.click()
      }

      for (const background of ['none', 'blur_screen', 'solid', 'gradient']) {
        const control = page.locator('[data-cut-studio-background-select]').first()
        await probe(page, {
          surface,
          name: `record-background-${background}`,
          actionId: 'studio-background-select',
          sel: control,
          group: panel,
          groupName: 'record-workspace',
          doClick: async () => {
            await control.selectOption(background)
            await sleep(70)
          },
          assertResult: async () => ({
            ok: (await control.inputValue()) === background
              && (await page.locator('[data-cut-studio-preview]').first().getAttribute('data-cut-studio-background')) === background,
            detail: `background=${await control.inputValue()}`,
          }),
        })
      }

      await clickState(page, panel, {
        name: 'record-mode-raw',
        actionId: 'rec-mode',
        selector: '[data-cut-rec-mode="raw"]',
        assertResult: async () => ({
          ok: (await panel.getAttribute('data-cut-record-mode')) === 'quick'
            && (await page.locator('[data-cut-rec-keys-toggle-input]').count()) === 0,
          detail: `mode=${await panel.getAttribute('data-cut-record-mode')}; polish controls hidden`,
        }),
      })
      await clickState(page, panel, {
        name: 'record-mode-auto',
        actionId: 'rec-mode',
        selector: '[data-cut-rec-mode="auto"]',
        assertResult: async () => ({
          ok: (await panel.getAttribute('data-cut-record-mode')) === 'studio'
            && (await page.locator('[data-cut-rec-keys-toggle-input]').count()) === 1,
          detail: `mode=${await panel.getAttribute('data-cut-record-mode')}; polish controls visible`,
        }),
      })

      const pick = page.locator('[data-cut-action="record-output-pick"]').first()
      await probe(page, {
        surface,
        name: 'record-output-pick',
        actionId: 'record-output-pick',
        sel: pick,
        group: panel,
        groupName: 'record-output-path',
        nativeAction: nativeOsActionsEnabled ? {
          mode: 'select',
          path: nativeOutputPath,
          useDoClick: true,
          verifyResult: true,
        } : undefined,
        doClick: async () => {
          await pick.click()
          await waitFor(async () => (
            sameHostPath(
              await page.locator('[data-cut-rec-output-path]').first().getAttribute('data-cut-rec-output-path'),
              nativeOutputPath,
            )
          ))
        },
        assertResult: async () => {
          const fixture = await fixtureState(page)
          const path = await page.locator('[data-cut-rec-output-path]').first().getAttribute('data-cut-rec-output-path')
          return {
            ok: sameHostPath(path, nativeOutputPath)
              && (nativeOsActionsEnabled || fixture.saveCalls.length === 1),
            detail: `path=${path}; expected=${nativeOutputPath}; fixture save invocations=${fixture.saveCalls.length}; native=${nativeOsActionsEnabled}`,
          }
        },
      })
      await clickState(page, panel, {
        name: 'record-output-clear',
        actionId: 'record-output-clear',
        selector: '[data-cut-action="record-output-clear"]',
        assertResult: async () => {
          const path = await page.locator('[data-cut-rec-output-path]').first().getAttribute('data-cut-rec-output-path')
          const note = await page.locator('[data-cut-rec-output-note]').first().textContent().catch(() => '')
          return {
            ok: path === '' && /default export folder/i.test(note || ''),
            detail: `path="${path}"; note="${note || ''}"`,
          }
        },
      })

      await clickState(page, panel, {
        name: 'record-output-default-folder',
        actionId: 'record-output-default-folder',
        selector: '[data-cut-action="record-output-default-folder"]',
        waitMs: 120,
        assertResult: async () => {
          const open = await waitFor(() => page.locator('[data-cut-environment]').first().isVisible())
          const row = !!open && await page.locator('[data-cut-export-default-folder]').first().isVisible()
          await page.locator('[data-cut-environment-close]').first().click().catch(() => {})
          await sleep(80)
          return { ok: !!open && row, detail: `settings=${!!open}; default-folder row=${row}` }
        },
      })

      const start = page.locator('[data-cut-action="record-start"]').first()
      await probe(page, {
        surface,
        name: 'record-start-auto',
        actionId: 'record-start',
        sel: start,
        group: panel,
        groupName: 'record-transport',
        doClick: async () => {
          await start.click()
          await page.locator('[data-cut-action="record-stop"]').first().waitFor({
            state: 'visible',
            timeout: 8000,
          })
        },
        assertResult: async () => {
          const args = (await fixtureState(page)).startCalls.at(-1)
          return {
            ok: args?.fps === 60
              && args?.audio === true
              && args?.system_audio === true
              && args?.keys === true
              && args?.window == null
              && args?.monitor === 1
              && args?.duration_ms == null
              && args?.studio?.background === 'gradient',
            detail: `auto start=${JSON.stringify(args)}`,
          }
        },
      })

      // The same visible background control emits live Studio metadata only once
      // a capture exists; prove that conditional path as a separate action.
      const liveBackground = page.locator('[data-cut-studio-background-select]').first()
      await probe(page, {
        surface,
        name: 'record-background-live',
        actionId: 'studio-background-select',
        sel: liveBackground,
        group: panel,
        groupName: 'record-transport',
        doClick: async () => {
          await liveBackground.selectOption('solid')
          await waitFor(async () => (await fixtureState(page)).studioCalls.length > 0)
        },
        assertResult: async () => {
          const args = (await fixtureState(page)).studioCalls.at(-1)
          return {
            ok: args?.capture_id === 'fixture-capture-1'
              && args?.event?.source === 'background'
              && args?.event?.kind === 'style'
              && args?.event?.background === 'solid',
            detail: `studio event=${JSON.stringify(args)}`,
          }
        },
      })

      const stop = page.locator('[data-cut-action="record-stop"]').first()
      await probe(page, {
        surface,
        name: 'record-stop-auto',
        actionId: 'record-stop',
        sel: stop,
        group: panel,
        groupName: 'record-transport',
        doClick: async () => {
          await stop.click()
          await page.locator('[data-cut-rec-export]').first().waitFor({
            state: 'visible',
            timeout: 8000,
          })
        },
        assertResult: async () => {
          const fixture = await fixtureState(page)
          const stopArgs = fixture.stopCalls.at(-1)
          const polishArgs = fixture.polishCalls.at(-1)
          return {
            ok: stopArgs?.capture_id === 'fixture-capture-1'
              && stopArgs?.autoedit === true
              && polishArgs?.source === '/fixture/source.mp4'
              && polishArgs?.plan === '/fixture/plan.json'
              && polishArgs?.raw === false
              && await page.locator('[data-cut-rec-export]').first().isVisible(),
            detail: `stop=${JSON.stringify(stopArgs)}; polish=${JSON.stringify(polishArgs)}`,
          }
        },
      })

      for (const format of ['gif', 'mp4']) {
        await clickState(page, panel, {
          name: `record-export-format-${format}`,
          actionId: 'rec-export-fmt',
          selector: `[data-cut-rec-export-fmt="${format}"]`,
          assertResult: async () => ({
            ok: (await page.locator(`[data-cut-rec-export-fmt="${format}"]`).first().getAttribute('class') || '').includes('rec__seg-btn--on'),
            detail: `export format ${format} selected`,
          }),
        })
        const exportButton = page.locator('[data-cut-action="record-export"]').first()
        await probe(page, {
          surface,
          name: `record-export-${format}`,
          actionId: 'record-export',
          sel: exportButton,
          group: panel,
          groupName: 'record-export',
          doClick: async () => {
            const before = (await fixtureState(page)).exportCalls.length
            await exportButton.click()
            await waitFor(async () => (await fixtureState(page)).exportCalls.length > before)
            await waitFor(async () => new RegExp(`Saved ${format}`, 'i').test(
              await page.locator('[data-cut-rec-export-note]').first().textContent().catch(() => ''),
            ))
          },
          assertResult: async () => {
            const args = (await fixtureState(page)).exportCalls.at(-1)
            const note = await page.locator('[data-cut-rec-export-note]').first().textContent().catch(() => '')
            return {
              ok: args?.format === format
                && args?.source === '/fixture/source.mp4'
                && args?.plan === '/fixture/plan.json'
                && new RegExp(`Saved ${format}`, 'i').test(note || ''),
              detail: `export=${JSON.stringify(args)}; note="${note || ''}"`,
            }
          },
        })
      }

      await page.locator('[data-cut-rec-mode="raw"]').first().click()
      await page.locator('[data-cut-action="record-start"]').first().click()
      await page.locator('[data-cut-action="record-stop"]').first().waitFor({
        state: 'visible',
        timeout: 8000,
      })
      const rawStop = page.locator('[data-cut-action="record-stop"]').first()
      await probe(page, {
        surface,
        name: 'record-stop-raw',
        actionId: 'record-stop',
        sel: rawStop,
        group: panel,
        groupName: 'record-raw',
        doClick: async () => {
          await rawStop.click()
          await page.locator('[data-cut-rec-raw-done]').first().waitFor({
            state: 'visible',
            timeout: 8000,
          })
        },
        assertResult: async () => {
          const args = (await fixtureState(page)).stopCalls.at(-1)
          const audio = await page.locator('[data-cut-rec-raw-audio]').first().getAttribute('data-cut-rec-raw-audio')
          return {
            ok: args?.capture_id === 'fixture-capture-2'
              && args?.autoedit === false
              && args?.mux_raw === true
              && audio === 'mic+system',
            detail: `raw stop=${JSON.stringify(args)}; audio=${audio}`,
          }
        },
      })

      const addRaw = page.locator('[data-cut-action="record-add-raw"]').first()
      await probe(page, {
        surface,
        name: 'record-add-raw',
        actionId: 'record-add-raw',
        sel: addRaw,
        group: panel,
        groupName: 'record-raw',
        doClick: async () => {
          await addRaw.click()
          await waitFor(async () => (await fixtureState(page)).importCalls.length > 0)
          await waitFor(async () => /Added to the timeline/.test(
            await page.locator('[data-cut-rec-raw-note]').first().textContent().catch(() => ''),
          ))
        },
        assertResult: async () => {
          const args = (await fixtureState(page)).importCalls.at(-1)
          const note = await page.locator('[data-cut-rec-raw-note]').first().textContent().catch(() => '')
          return {
            ok: args?.path === '/fixture/raw-recording.mp4'
              && /Added to the timeline/.test(note || ''),
            detail: `media.import=${JSON.stringify(args)}; note="${note || ''}"`,
          }
        },
      })

      await page.locator('[data-cut-mode="edit"]').first().click()
      await sleep(120)
    } finally {
      await restoreFixture(page)
      await closeOverlays(page)
    }
  }

  return { run }
}
