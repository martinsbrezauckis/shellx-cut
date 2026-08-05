// Exact installed-app OTIO coverage. Unlike the deterministic dialog-state
// module, this path keeps both the host file picker and import engine real.

export function createNativeOtioActionCoverage({
  probe,
  verb,
  state,
  captureVerbResp,
  freshProject,
  closeOverlays,
  primaryMedia,
  nativeOsActionsEnabled,
}) {
  const surface = 'native-otio-actions'
  const sameHostPath = (left, right) => {
    const normalize = (value) => String(value || '')
      .replace(/^[/\\]{2}[?][/\\]/, '')
      .replace(/\\/g, '/')
      .replace(/\/+/g, '/')
    const a = normalize(left)
    const b = normalize(right)
    return a === b || a.toLowerCase() === b.toLowerCase()
  }

  async function captureRequest(page, pathname, action) {
    let payload = null
    const onRequest = (request) => {
      let requestPath = ''
      try { requestPath = new URL(request.url()).pathname } catch { return }
      if (requestPath !== pathname) return
      try { payload = request.postDataJSON() } catch {}
    }
    page.on('request', onRequest)
    try {
      return { response: await action(), request: payload }
    } finally {
      page.off('request', onRequest)
    }
  }

  async function openPreview(page, filePath, suffix) {
    await page.locator('[data-cut-left-tab="assets"]').first().click()
    const trigger = page.locator('[data-cut-import-otio]').first()
    let preview = null
    await probe(page, {
      surface,
      name: `native-otio-open-${suffix}`,
      actionId: 'import-otio',
      sel: trigger,
      group: page.locator('[data-cut-panel="assets"]').first(),
      groupName: `native-otio-picker-${suffix}`,
      nativeAction: {
        mode: 'select',
        path: filePath,
        useDoClick: true,
        verifyResult: true,
      },
      doClick: async () => {
        preview = await captureVerbResp(
          page,
          'import.otio',
          () => trigger.click(),
          30_000,
        )
        await page.locator('[data-cut-otio-import]').first()
          .waitFor({ state: 'visible', timeout: 12_000 })
      },
      assertResult: async () => {
        const result = preview?.result
        return {
          ok: preview?.ok
            && result?.status === 'preview'
            && sameHostPath(result?.path, filePath)
            && typeof result?.source_hash === 'string'
            && result.source_hash.length > 16
            && Number.isInteger(result?.track_count)
            && Number.isInteger(result?.clips)
            && await page.locator('[data-cut-otio-import]').first().isVisible(),
          detail: `preview=${preview?.ok}; status=${result?.status || 'missing'}; pathExact=${sameHostPath(result?.path, filePath)}; tracks=${result?.track_count}; clips=${result?.clips}; hash=${result?.source_hash ? 'present' : 'missing'}`,
        }
      },
    })
    return preview
  }

  async function run(page) {
    if (!nativeOsActionsEnabled) return
    await freshProject(page, 'native_otio_actions', primaryMedia)
    await closeOverlays(page)
    const exported = await verb('export.otio', {
      rationale: 'fcv: exact installed OTIO picker and import round-trip',
    })
    const filePath = exported.result?.path || ''
    if (!exported.ok || !filePath) {
      throw new Error(`native OTIO source export failed: ${exported.error?.message || exported.error?.code || 'missing path'}`)
    }
    // Prove the just-exported file exists and is readable before opening an OS
    // picker. Otherwise a missing/stale path produces a confusing Explorer
    // filename error that looks like dialog automation failed.
    const sourceProbe = await verb('import.otio', {
      path: filePath,
      mode: 'preview',
    })
    if (!sourceProbe.ok
      || sourceProbe.result?.status !== 'preview'
      || !sameHostPath(sourceProbe.result?.path, filePath)) {
      throw new Error(
        `native OTIO source preflight failed before opening the picker: ${
          sourceProbe.error?.message
          || sourceProbe.error?.code
          || `status=${sourceProbe.result?.status || 'missing'} path=${sourceProbe.result?.path || 'missing'}`
        }`,
      )
    }

    let preview = await openPreview(page, filePath, 'cancel')
    await probe(page, {
      surface,
      name: 'native-otio-cancel',
      actionId: 'otio-cancel',
      sel: page.locator('[data-cut-otio-cancel]').first(),
      group: page.locator('[data-cut-otio-import]').first(),
      groupName: 'native-otio-cancel',
      doClick: async () => {
        await page.locator('[data-cut-otio-cancel]').first().click()
        await page.locator('[data-cut-otio-import]').first()
          .waitFor({ state: 'detached', timeout: 8_000 })
      },
      assertResult: async () => ({
        ok: preview?.ok && await page.locator('[data-cut-otio-import]').count() === 0,
        detail: `real preview=${preview?.ok}; modal count=${await page.locator('[data-cut-otio-import]').count()}`,
      }),
    })

    preview = await openPreview(page, filePath, 'close')
    await probe(page, {
      surface,
      name: 'native-otio-close',
      actionId: 'otio-close',
      sel: page.locator('[data-cut-otio-close]').first(),
      group: page.locator('[data-cut-otio-import]').first(),
      groupName: 'native-otio-close',
      doClick: async () => {
        await page.locator('[data-cut-otio-close]').first().click()
        await page.locator('[data-cut-otio-import]').first()
          .waitFor({ state: 'detached', timeout: 8_000 })
      },
      assertResult: async () => ({
        ok: preview?.ok && await page.locator('[data-cut-otio-import]').count() === 0,
        detail: `real preview=${preview?.ok}; modal count=${await page.locator('[data-cut-otio-import]').count()}`,
      }),
    })

    preview = await openPreview(page, filePath, 'confirm')
    let replace = null
    await probe(page, {
      surface,
      name: 'native-otio-confirm',
      actionId: 'otio-confirm',
      sel: page.locator('[data-cut-otio-confirm]').first(),
      group: page.locator('[data-cut-otio-import]').first(),
      groupName: 'native-otio-confirm',
      doClick: async () => {
        replace = await captureRequest(page, '/api/verb/import.otio', () =>
          captureVerbResp(
            page,
            'import.otio',
            () => page.locator('[data-cut-otio-confirm]').first().click(),
            45_000,
          ))
        await page.locator('[data-cut-otio-import]').first()
          .waitFor({ state: 'detached', timeout: 12_000 })
      },
      assertResult: async () => {
        const request = replace?.request
        const result = replace?.response?.result
        const exact = request?.path === preview?.result?.path
          && request?.mode === 'replace'
          && request?.expected_hash === preview?.result?.source_hash
          && request?.rationale === 'confirmed OTIO preview in the desktop app'
        const project = await state()
        const clips = (project.tracks || [])
          .reduce((total, track) => total + (track.clips || []).length, 0)
        const note = await page.locator('[data-cut-topbar-note]').textContent()
        return {
          ok: replace?.response?.ok
            && exact
            && result?.tracks_created > 0
            && result?.clips_inserted > 0
            && clips >= result.clips_inserted
            && note?.startsWith('Imported timeline —'),
          detail: `replace=${replace?.response?.ok}; exactRequest=${exact}; tracks=${result?.tracks_created}; inserted=${result?.clips_inserted}; projectClips=${clips}; note="${note || ''}"`,
        }
      },
    })
  }

  return { run }
}
