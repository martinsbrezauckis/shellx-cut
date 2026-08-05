// Deterministic native-WebView coverage for Inspector actions that only mount
// after a capability or content precondition:
//   - missing stabilization support → Open video setup
//   - successful engagement score → Re-score
//   - linked Motion package → edit/refresh/relink plus tracking/stabilization
//
// The product UI remains real. Narrow fetch/native-picker fixtures supply only
// the unavailable external capability responses, record exact requests, and are
// restored after each scenario. Engine contracts retain their Rust/API gates.

export function createInspectorConditionalActionCoverage({
  probe,
  verb,
  state,
  sleep,
  freshProject,
  closeOverlays,
  selectClip,
  propertiesTab,
  expandInspectorSection,
  primaryMedia,
  nativeOsActionsEnabled,
}) {
  const surface = 'inspector-conditional-actions'
  const sameHostPath = (left, right) => {
    const normalize = (value) => String(value || '')
      .replace(/^[/\\]{2}[?][/\\]/, '')
      .replace(/\\/g, '/')
      .replace(/\/+/g, '/')
    const a = normalize(left)
    const b = normalize(right)
    return a === b || a.toLowerCase() === b.toLowerCase()
  }

  async function waitFor(check, timeoutMs = 12_000) {
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

  async function installDoctorFixture(page) {
    await page.evaluate(() => {
      const target = window
      target.__fcvInspectorDoctorOriginalFetch = window.fetch
      target.__fcvInspectorDoctorFixture = { calls: [] }
      const originalFetch = target.__fcvInspectorDoctorOriginalFetch
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
        if (pathname === '/api/verb/system.doctor') {
          target.__fcvInspectorDoctorFixture.calls.push(requestArgs(options))
          return envelope({
            ok: true,
            result: {
              schema: 'shellx-cut/doctor/1',
              scanned_at: '2026-07-29T00:00:00Z',
              os: 'fixture',
              arch: 'fixture',
              app_version: 'fixture',
              essential_ok: true,
              cards: [{
                id: 'ffmpeg',
                kind: 'tool',
                status: 'ok',
                source: 'path',
                version: 'fixture',
                hint: 'Use Settings > Video performance.',
                details: { can_stabilize: false },
              }],
            },
          })
        }
        return originalFetch(...args)
      }
    })
  }

  async function restoreDoctorFixture(page) {
    await page.evaluate(() => {
      const target = window
      if (target.__fcvInspectorDoctorOriginalFetch) {
        window.fetch = target.__fcvInspectorDoctorOriginalFetch
      }
      delete target.__fcvInspectorDoctorOriginalFetch
      delete target.__fcvInspectorDoctorFixture
      document.dispatchEvent(new CustomEvent('cut:refresh-doctor'))
    })
  }

  async function runVideoSetup(page) {
    await freshProject(page, 'inspector_video_setup', primaryMedia)
    await closeOverlays(page)
    const clipId = (await state()).tracks.find((track) => track.kind === 'video')
      ?.clips.find((clip) => clip.asset)?.id
    if (!clipId) throw new Error('conditional Inspector fixture has no video clip')
    await selectClip(page, clipId)
    await propertiesTab(page)
    await expandInspectorSection(page, 'video-motion')
    await installDoctorFixture(page)
    try {
      await page.evaluate(() => document.dispatchEvent(new CustomEvent('cut:refresh-doctor')))
      const setup = page.locator('[data-cut-inspector-open-video-setup]').first()
      await setup.waitFor({ state: 'visible', timeout: 8_000 })
      await probe(page, {
        surface,
        name: 'inspector-open-video-setup',
        actionId: 'inspector-open-video-setup',
        sel: setup,
        group: page.locator('[data-cut-section="video-motion"]').first(),
        groupName: 'inspector-video-setup-blocker',
        doClick: async () => {
          await setup.click()
          await page.locator('[data-cut-settings-body="video-performance"]').first()
            .waitFor({ state: 'visible', timeout: 8_000 })
        },
        assertResult: async () => {
          const fixture = await page.evaluate(() =>
            JSON.parse(JSON.stringify(window.__fcvInspectorDoctorFixture)))
          const exactRefresh = fixture.calls.some((args) => args.refresh === true)
          const routed = await page.locator('[data-cut-settings-body="video-performance"]').first()
            .isVisible()
          await page.locator('[data-cut-environment-close]').first().click().catch(() => {})
          return {
            ok: exactRefresh && routed,
            detail: `doctor refresh:true=${exactRefresh}; Settings > Video performance=${routed}`,
          }
        },
      })
    } finally {
      await page.locator('[data-cut-environment-close]').first().click().catch(() => {})
      await restoreDoctorFixture(page)
      await sleep(250)
    }
  }

  async function installScoreFixture(page) {
    await page.evaluate(() => {
      const target = window
      target.__fcvInspectorScoreOriginalFetch = window.fetch
      target.__fcvInspectorScoreFixture = { calls: [] }
      const originalFetch = target.__fcvInspectorScoreOriginalFetch
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
        if (pathname === '/api/verb/score.clip') {
          const body = requestArgs(options)
          target.__fcvInspectorScoreFixture.calls.push(body)
          const score = target.__fcvInspectorScoreFixture.calls.length === 1 ? 81 : 92
          return envelope({
            ok: true,
            result: {
              score,
              duration_ms: 5_000,
              factors: { speech_density: 0.81, energy: 0.76, visual_dynamics: 0.64 },
              signals: { words: 18, scenes: 3, silence_ms: 120, dead_ms: 0 },
            },
          })
        }
        return originalFetch(...args)
      }
    })
  }

  async function restoreScoreFixture(page) {
    await page.evaluate(() => {
      const target = window
      if (target.__fcvInspectorScoreOriginalFetch) {
        window.fetch = target.__fcvInspectorScoreOriginalFetch
      }
      delete target.__fcvInspectorScoreOriginalFetch
      delete target.__fcvInspectorScoreFixture
    })
  }

  async function runScoreAgain(page) {
    await freshProject(page, 'inspector_score_again', primaryMedia)
    await closeOverlays(page)
    const clipId = (await state()).tracks.find((track) => track.kind === 'video')
      ?.clips.find((clip) => clip.asset)?.id
    if (!clipId) throw new Error('engagement-score fixture has no video clip')
    await selectClip(page, clipId)
    await propertiesTab(page)
    await expandInspectorSection(page, 'engagement')
    await installScoreFixture(page)
    try {
      await page.locator('[data-cut-action="score-clip"]').first().click()
      await page.locator('[data-cut-inspector-score="81"]').first()
        .waitFor({ state: 'visible', timeout: 8_000 })
      const rescore = page.locator('[data-cut-action="score-clip-again"]').first()
      await probe(page, {
        surface,
        name: 'score-clip-again',
        actionId: 'score-clip-again',
        sel: rescore,
        group: page.locator('[data-cut-inspector-group="engagement"]').first(),
        groupName: 'inspector-engagement-score',
        doClick: async () => {
          await rescore.click()
          await page.locator('[data-cut-inspector-score="92"]').first()
            .waitFor({ state: 'visible', timeout: 8_000 })
        },
        assertResult: async () => {
          const fixture = await page.evaluate(() =>
            JSON.parse(JSON.stringify(window.__fcvInspectorScoreFixture)))
          const exact = fixture.calls.length === 2
            && fixture.calls.every((args) => args.clip === clipId)
          const updated = await page.locator('[data-cut-inspector-score="92"]').first().isVisible()
          return {
            ok: exact && updated,
            detail: `two exact score.clip calls=${exact}; visible score 81→92=${updated}`,
          }
        },
      })
    } finally {
      await restoreScoreFixture(page)
    }
  }

  async function installMotionFixture(page, clipId) {
    await page.evaluate(({ fixtureClipId, mockPicker }) => {
      const target = window
      target.__fcvInspectorMotionOriginalFetch = window.fetch
      target.__fcvInspectorMotionOriginalTauri = target.__TAURI__
      target.__fcvInspectorMotionOriginalInternals = target.__TAURI_INTERNALS__
      target.__fcvInspectorMotionOriginalInternalInvoke = target.__TAURI_INTERNALS__?.invoke
      target.__fcvInspectorMotionFixture = {
        clipId: fixtureClipId,
        attached: false,
        inventoryCalls: [],
        editCalls: [],
        refreshCalls: [],
        relinkCalls: [],
        requestCalls: [],
        inspectCalls: [],
        applyCalls: [],
        verifyCalls: [],
        detachCalls: [],
        openCalls: [],
      }
      const fixture = target.__fcvInspectorMotionFixture
      const originalFetch = target.__fcvInspectorMotionOriginalFetch
      const envelope = (body) => new Response(JSON.stringify(body), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      })
      const requestArgs = (options) => {
        try { return JSON.parse(options?.body || '{}') } catch { return {} }
      }
      const inventory = () => ({
        packageId: 'pkg-fcv-motion',
        motionId: 'motion-fcv',
        width: 1920,
        height: 1080,
        durationMs: 5_000,
        fps: 30,
        videoAssets: [
          { id: 'footage_a', name: 'Primary footage', available: true },
          { id: 'footage_b', name: 'Alternate footage', available: true },
        ],
        targetLayers: [
          { id: 'layer_a', name: 'Subject', kind: 'video', trackingAttached: false },
          { id: 'layer_b', name: 'Screen', kind: 'image', trackingAttached: fixture.attached },
        ],
        analyses: fixture.requestCalls.length
          ? [{ analysisId: 'analysis_custom', state: 'succeeded', assetId: 'footage_b' }]
          : [],
      })
      const linkedState = (clip) => ({
        schema: 'shellx-cut/motion-link@1',
        clipId: fixtureClipId,
        assetId: clip.asset,
        motionSourceId: 'motion-source-fcv',
        packageId: 'pkg-fcv-motion',
        motionId: 'motion-fcv',
        sourceRevision: 'sha256:fcv-source-revision',
        sourceRevisionKind: 'motion-package',
        sourcePath: '/fixture/motion-package',
        planPath: '/fixture/cut-import-plan.json',
        mode: 'rendered_media',
        state: 'linked-current',
        render: {
          path: '/fixture/render.mp4',
          sha256: 'sha256:fcv-render',
          byteLength: 4096,
          artifactHandleId: 'artifact-fcv',
        },
        fallbackPath: '/fixture/render.mp4',
        availability: {
          source: true,
          plan: true,
          render: true,
          fallback: true,
          canRefresh: true,
          canRelink: true,
          canEditInMotion: true,
        },
      })
      window.fetch = async (...args) => {
        const input = args[0]
        const options = args[1]
        const url = typeof input === 'string' ? input : input?.url || ''
        let pathname = ''
        try { pathname = new URL(String(url), window.location.href).pathname } catch {}
        const body = requestArgs(options)

        if (pathname === '/api/verb/project.state') {
          const response = await originalFetch(...args)
          const payload = await response.clone().json()
          if (payload?.ok && payload?.result?.tracks) {
            for (const track of payload.result.tracks) {
              for (const clip of track.clips || []) {
                if (clip?.id === fixtureClipId && clip?.asset) {
                  clip.motion_link = linkedState(clip)
                }
              }
            }
          }
          return envelope(payload)
        }
        if (pathname === '/api/verb/motion.link.edit') {
          fixture.editCalls.push(body)
          return envelope({ ok: true, result: {
            clip: fixtureClipId,
            packageId: 'pkg-fcv-motion',
            motionId: 'motion-fcv',
            launched: true,
            localOnly: true,
            remotePublish: false,
          } })
        }
        if (pathname === '/api/verb/motion.link.refresh') {
          fixture.refreshCalls.push(body)
          return envelope({ ok: true, result: {
            clip: fixtureClipId,
            state: 'linked-current',
            render: { path: '/fixture/refreshed.mp4', sha256: 'sha256:refreshed', byteLength: 8192, preset: body.preset },
          } })
        }
        if (pathname === '/api/verb/motion.link.relink') {
          fixture.relinkCalls.push(body)
          return envelope({ ok: true, result: {
            clip: fixtureClipId,
            packageDir: body.package_dir,
            packageId: 'pkg-fcv-motion',
            motionId: 'motion-fcv',
            sourceRevision: 'sha256:relinked',
            state: 'source-dirty',
          } })
        }
        if (pathname === '/api/verb/motion.link.tracking.inventory') {
          fixture.inventoryCalls.push(body)
          return envelope({ ok: true, result: {
            ok: true,
            schema: 'shellx-cut/motion-tracking-inventory@1',
            clip: fixtureClipId,
            inventory: inventory(),
            localOnly: true,
          } })
        }
        if (pathname === '/api/verb/motion.link.tracking.request') {
          fixture.requestCalls.push(body)
          return envelope({ ok: true, result: {
            ok: true,
            clip: fixtureClipId,
            analysisId: body.analysis_id,
            lifecycle: { analysisId: body.analysis_id, state: 'succeeded' },
            receipt: { id: 'receipt-analysis', operation: 'analysis.tracking.request' },
            warnings: [],
            state: 'linked-current',
          } })
        }
        if (pathname === '/api/verb/motion.link.tracking.inspect') {
          fixture.inspectCalls.push(body)
          return envelope({ ok: true, result: {
            ok: true,
            clip: fixtureClipId,
            analysisId: body.analysis_id,
            lifecycle: { analysisId: body.analysis_id, state: 'succeeded' },
            source: { assetId: 'footage_b', current: true, sha256: 'sha256:footage', byteLength: 1024 },
            current: true,
            receipt: { id: 'receipt-analysis', operation: 'analysis.tracking.request' },
            warnings: [],
            localOnly: true,
          } })
        }
        if (pathname === '/api/verb/motion.link.tracking.apply') {
          fixture.applyCalls.push(body)
          fixture.attached = true
          return envelope({ ok: true, result: {
            ok: true,
            clip: fixtureClipId,
            analysisId: body.analysis_id,
            layerId: body.layer_id,
            plan: { status: 'applied', fidelity: 'exact', segmentCount: 1, warnings: [] },
            changedPaths: ['motion.json'],
            receipt: { id: 'receipt-apply', operation: 'analysis.tracking.apply' },
            warnings: [],
            state: 'source-dirty',
            refreshRequired: true,
          } })
        }
        if (pathname === '/api/verb/motion.link.tracking.verify') {
          fixture.verifyCalls.push(body)
          return envelope({ ok: true, result: {
            ok: true,
            clip: fixtureClipId,
            layerId: body.layer_id,
            analysisId: body.analysis_id,
            verification: { attached: true, current: true, reasons: [] },
            lifecycle: { analysisId: body.analysis_id, state: 'succeeded' },
            source: { assetId: 'footage_b', current: true },
            receipt: { id: 'receipt-verify', operation: 'analysis.tracking.verify' },
            warnings: [],
            localOnly: true,
          } })
        }
        if (pathname === '/api/verb/motion.link.tracking.detach') {
          fixture.detachCalls.push(body)
          fixture.attached = false
          return envelope({ ok: true, result: {
            ok: true,
            clip: fixtureClipId,
            layerId: body.layer_id,
            analysisId: 'analysis_custom',
            restoredPreviousKeyframes: true,
            changedPaths: ['motion.json'],
            receipt: { id: 'receipt-detach', operation: 'analysis.tracking.detach' },
            warnings: [],
            state: 'source-dirty',
            refreshRequired: true,
          } })
        }
        return originalFetch(...args)
      }

      if (mockPicker) {
        const invoke = async (command, args, options) => {
          if (command === 'plugin:dialog|open') {
            fixture.openCalls.push({ command, args })
            return '/fixture/motion-package'
          }
          const original = target.__fcvInspectorMotionOriginalInternalInvoke
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
      }
    }, { fixtureClipId: clipId, mockPicker: !nativeOsActionsEnabled })
  }

  async function motionFixtureState(page) {
    return page.evaluate(() =>
      JSON.parse(JSON.stringify(window.__fcvInspectorMotionFixture)))
  }

  async function restoreMotionFixture(page) {
    await page.evaluate(() => {
      const target = window
      if (target.__fcvInspectorMotionOriginalFetch) {
        window.fetch = target.__fcvInspectorMotionOriginalFetch
      }
      if (target.__fcvInspectorMotionOriginalInternals) {
        target.__fcvInspectorMotionOriginalInternals.invoke =
          target.__fcvInspectorMotionOriginalInternalInvoke
        target.__TAURI_INTERNALS__ = target.__fcvInspectorMotionOriginalInternals
      } else {
        delete target.__TAURI_INTERNALS__
      }
      if (target.__fcvInspectorMotionOriginalTauri) {
        target.__TAURI__ = target.__fcvInspectorMotionOriginalTauri
      } else {
        delete target.__TAURI__
      }
      delete target.__fcvInspectorMotionOriginalFetch
      delete target.__fcvInspectorMotionOriginalTauri
      delete target.__fcvInspectorMotionOriginalInternals
      delete target.__fcvInspectorMotionOriginalInternalInvoke
      delete target.__fcvInspectorMotionFixture
    })
  }

  async function runMotion(page) {
    const { projectPath } = await freshProject(page, 'inspector_motion', primaryMedia)
    await closeOverlays(page)
    const clipId = (await state()).tracks.find((track) => track.kind === 'video')
      ?.clips.find((clip) => clip.asset)?.id
    if (!clipId) throw new Error('linked-Motion fixture has no video clip')
    if (nativeOsActionsEnabled && !projectPath) {
      throw new Error('linked-Motion native picker coverage has no selectable project directory')
    }
    await selectClip(page, clipId)
    await propertiesTab(page)
    await installMotionFixture(page, clipId)
    try {
      await verb('edit.transform', {
        clip: clipId,
        x: 0.013,
        rationale: 'fcv: refresh selected clip into linked Motion fixture',
      })
      const motion = page.locator('[data-cut-inspector-group="motion-link"]').first()
      await motion.waitFor({ state: 'visible', timeout: 12_000 })
      const actionGroup = motion.locator('.insp__motion-actions').first()

      await probe(page, {
        surface,
        name: 'motion-edit',
        actionId: 'motion-edit',
        sel: page.locator(`[data-cut-motion-edit="${clipId}"]`).first(),
        group: actionGroup,
        groupName: 'inspector-motion-actions',
        doClick: async () => {
          await page.locator(`[data-cut-motion-edit="${clipId}"]`).first().click()
          await waitFor(async () => (await page.locator('[data-cut-motion-action-status]').textContent())?.includes('Opened in Canvas'))
        },
        assertResult: async () => {
          const fixture = await motionFixtureState(page)
          const call = fixture.editCalls[0]
          const status = await page.locator('[data-cut-motion-action-status]').textContent()
          return {
            ok: fixture.editCalls.length === 1 && call?.clip === clipId
              && status?.includes('Opened in Canvas Motion Studio.'),
            detail: `motion.link.edit=${JSON.stringify(call)}; status=${status}`,
          }
        },
      })

      await probe(page, {
        surface,
        name: 'motion-refresh',
        actionId: 'motion-refresh',
        sel: page.locator(`[data-cut-motion-refresh="${clipId}"]`).first(),
        group: actionGroup,
        groupName: 'inspector-motion-actions',
        doClick: async () => {
          await page.locator(`[data-cut-motion-refresh="${clipId}"]`).first().click()
          await waitFor(async () => (await page.locator('[data-cut-motion-action-status]').textContent())?.includes('refreshed'))
        },
        assertResult: async () => {
          const fixture = await motionFixtureState(page)
          const call = fixture.refreshCalls[0]
          const exact = call?.clip === clipId
            && call?.preset === 'mp4-h264'
            && call?.rationale === 'inspector: refresh linked Motion clip'
          return {
            ok: fixture.refreshCalls.length === 1 && exact,
            detail: `exact motion.link.refresh=${exact}; args=${JSON.stringify(call)}`,
          }
        },
      })

      await probe(page, {
        surface,
        name: 'motion-relink',
        actionId: 'motion-relink',
        sel: page.locator(`[data-cut-motion-relink="${clipId}"]`).first(),
        group: actionGroup,
        groupName: 'inspector-motion-actions',
        nativeAction: {
          mode: 'select',
          path: projectPath,
          useDoClick: true,
          verifyResult: true,
        },
        doClick: async () => {
          await page.locator(`[data-cut-motion-relink="${clipId}"]`).first().click()
          await waitFor(async () => (await page.locator('[data-cut-motion-action-status]').textContent())?.includes('Source relinked'))
        },
        assertResult: async () => {
          const fixture = await motionFixtureState(page)
          const call = fixture.relinkCalls[0]
          const exact = call?.clip === clipId
            && sameHostPath(
              call?.package_dir,
              nativeOsActionsEnabled ? projectPath : '/fixture/motion-package',
            )
            && call?.rationale === 'inspector: relink Motion package'
          const picker = nativeOsActionsEnabled || fixture.openCalls.length === 1
          return {
            ok: exact && picker,
            detail: `native package picker=${picker}; exact motion.link.relink=${exact}; args=${JSON.stringify(call)}`,
          }
        },
      })

      const tracking = page.locator('[data-cut-motion-tracking]').first()
      await tracking.waitFor({ state: 'visible', timeout: 8_000 })
      await page.locator('[data-cut-motion-tracking-asset]').first()
        .waitFor({ state: 'visible', timeout: 8_000 })

      const localControl = async ({ name, actionId, selector, act, expected }) => {
        const control = page.locator(selector).first()
        await probe(page, {
          surface,
          name,
          actionId,
          sel: control,
          group: tracking,
          groupName: 'inspector-motion-tracking',
          doClick: act,
          assertResult: async () => {
            const value = await control.inputValue()
            return { ok: value === expected, detail: `value=${value}` }
          },
        })
      }

      await localControl({
        name: 'motion-tracking-analysis',
        actionId: 'motion-tracking-analysis',
        selector: '[data-cut-motion-tracking-analysis]',
        act: () => page.locator('[data-cut-motion-tracking-analysis]').first().fill('analysis_custom'),
        expected: 'analysis_custom',
      })
      await localControl({
        name: 'motion-tracking-asset',
        actionId: 'motion-tracking-asset',
        selector: '[data-cut-motion-tracking-asset]',
        act: () => page.locator('[data-cut-motion-tracking-asset]').first().selectOption('footage_b'),
        expected: 'footage_b',
      })
      await localControl({
        name: 'motion-tracking-layer',
        actionId: 'motion-tracking-layer',
        selector: '[data-cut-motion-tracking-layer]',
        act: () => page.locator('[data-cut-motion-tracking-layer]').first().selectOption('layer_b'),
        expected: 'layer_b',
      })
      await localControl({
        name: 'motion-tracking-mode',
        actionId: 'motion-tracking-mode',
        selector: '[data-cut-motion-tracking-mode]',
        act: () => page.locator('[data-cut-motion-tracking-mode]').first().selectOption('planar'),
        expected: 'planar',
      })
      await localControl({
        name: 'motion-tracking-sample',
        actionId: 'motion-tracking-sample',
        selector: '[data-cut-motion-tracking-sample]',
        act: () => page.locator('[data-cut-motion-tracking-sample]').first().selectOption('200'),
        expected: '200',
      })

      const firstRegion = page.locator('[data-cut-motion-tracking-region-field]').first()
      await probe(page, {
        surface,
        name: 'motion-tracking-region-field',
        actionId: 'motion-tracking-region-field',
        sel: firstRegion,
        group: tracking,
        groupName: 'inspector-motion-tracking',
        doClick: async () => {
          for (const [field, value] of [['x', '10'], ['y', '20'], ['width', '40'], ['height', '30']]) {
            await page.locator(`[data-cut-motion-tracking-region-field="${field}"]`).first().fill(value)
          }
        },
        assertResult: async () => {
          const values = {}
          for (const field of ['x', 'y', 'width', 'height']) {
            values[field] = await page.locator(`[data-cut-motion-tracking-region-field="${field}"]`).first().inputValue()
          }
          const exact = values.x === '10' && values.y === '20'
            && values.width === '40' && values.height === '30'
          return { ok: exact, detail: `region=${JSON.stringify(values)}` }
        },
      })

      await probe(page, {
        surface,
        name: 'motion-tracking-analyze',
        actionId: 'motion-tracking-analyze',
        sel: page.locator('[data-cut-motion-tracking-analyze]').first(),
        group: tracking,
        groupName: 'inspector-motion-tracking',
        doClick: async () => {
          await page.locator('[data-cut-motion-tracking-analyze]').first().click()
          await waitFor(async () => (await page.locator('[data-cut-motion-tracking-status]').textContent()) === 'Analysis succeeded.')
        },
        assertResult: async () => {
          const fixture = await motionFixtureState(page)
          const call = fixture.requestCalls[0]
          const region = call?.region
          const exact = call?.clip === clipId
            && call?.analysis_id === 'analysis_custom'
            && call?.asset_id === 'footage_b'
            && call?.mode === 'planar'
            && call?.model === 'homography'
            && call?.every_ms === 200
            && region?.x === 0.1
            && region?.y === 0.2
            && region?.width === 0.4
            && region?.height === 0.3
            && call?.rationale === 'inspector: analyze linked Motion footage'
          const status = await page.locator('[data-cut-motion-tracking-status]').textContent()
          return {
            ok: fixture.requestCalls.length === 1 && exact && status === 'Analysis succeeded.',
            detail: `exact tracking request=${exact}; preserved status=${status}; args=${JSON.stringify(call)}`,
          }
        },
      })

      await probe(page, {
        surface,
        name: 'motion-tracking-inspect',
        actionId: 'motion-tracking-inspect',
        sel: page.locator('[data-cut-motion-tracking-inspect]').first(),
        group: tracking,
        groupName: 'inspector-motion-tracking',
        doClick: async () => {
          await page.locator('[data-cut-motion-tracking-inspect]').first().click()
          await waitFor(async () => (await page.locator('[data-cut-motion-tracking-status]').textContent())?.includes('source bytes are current'))
        },
        assertResult: async () => {
          const fixture = await motionFixtureState(page)
          const call = fixture.inspectCalls[0]
          const exact = call?.clip === clipId && call?.analysis_id === 'analysis_custom'
          return { ok: fixture.inspectCalls.length === 1 && exact, detail: `exact tracking inspect=${exact}; args=${JSON.stringify(call)}` }
        },
      })

      await probe(page, {
        surface,
        name: 'motion-tracking-apply',
        actionId: 'motion-tracking-apply',
        sel: page.locator('[data-cut-motion-tracking-apply]').first(),
        group: tracking,
        groupName: 'inspector-motion-tracking',
        doClick: async () => {
          await page.locator('[data-cut-motion-tracking-apply]').first().click()
          await waitFor(async () => !(await page.locator('[data-cut-motion-tracking-verify]').first().isDisabled()))
        },
        assertResult: async () => {
          const fixture = await motionFixtureState(page)
          const call = fixture.applyCalls[0]
          const exact = call?.clip === clipId
            && call?.analysis_id === 'analysis_custom'
            && call?.layer_id === 'layer_b'
            && call?.rationale === 'inspector: apply linked Motion stabilization'
          const status = await page.locator('[data-cut-motion-tracking-status]').textContent()
          return {
            ok: fixture.applyCalls.length === 1 && exact
              && status?.includes('Stabilization applied (exact).'),
            detail: `exact tracking apply=${exact}; preserved status=${status}; args=${JSON.stringify(call)}`,
          }
        },
      })

      await probe(page, {
        surface,
        name: 'motion-tracking-verify',
        actionId: 'motion-tracking-verify',
        sel: page.locator('[data-cut-motion-tracking-verify]').first(),
        group: tracking,
        groupName: 'inspector-motion-tracking-attached',
        doClick: async () => {
          await page.locator('[data-cut-motion-tracking-verify]').first().click()
          await waitFor(async () => (await page.locator('[data-cut-motion-tracking-status]').textContent())?.startsWith('Verified:'))
        },
        assertResult: async () => {
          const fixture = await motionFixtureState(page)
          const call = fixture.verifyCalls[0]
          const exact = call?.clip === clipId
            && call?.layer_id === 'layer_b'
            && call?.analysis_id === 'analysis_custom'
          const status = await page.locator('[data-cut-motion-tracking-status]').textContent()
          return {
            ok: fixture.verifyCalls.length === 1 && exact
              && status === 'Verified: stabilization and source are current.',
            detail: `exact tracking verify=${exact}; status=${status}; args=${JSON.stringify(call)}`,
          }
        },
      })

      await probe(page, {
        surface,
        name: 'motion-tracking-detach',
        actionId: 'motion-tracking-detach',
        sel: page.locator('[data-cut-motion-tracking-detach]').first(),
        group: tracking,
        groupName: 'inspector-motion-tracking-attached',
        doClick: async () => {
          await page.locator('[data-cut-motion-tracking-detach]').first().click()
          await waitFor(async () =>
            (await page.locator('[data-cut-motion-tracking-status]').textContent())
              ?.includes('Stabilization detached.'))
        },
        assertResult: async () => {
          const fixture = await motionFixtureState(page)
          const call = fixture.detachCalls[0]
          const exact = call?.clip === clipId
            && call?.layer_id === 'layer_b'
            && call?.rationale === 'inspector: detach linked Motion stabilization'
          const status = await page.locator('[data-cut-motion-tracking-status]').textContent()
          const detached = await page.locator('[data-cut-motion-tracking-detach]').first().isDisabled()
          return {
            ok: fixture.detachCalls.length === 1 && exact && detached
              && status?.includes('Stabilization detached.'),
            detail: `exact tracking detach=${exact}; detached=${detached}; preserved status=${status}; args=${JSON.stringify(call)}`,
          }
        },
      })
    } finally {
      await restoreMotionFixture(page)
      await verb('edit.transform', {
        clip: clipId,
        x: 0.014,
        rationale: 'fcv: restore real selected clip state after linked Motion fixture',
      })
      await waitFor(async () => (await page.locator('[data-cut-inspector-group="motion-link"]').count()) === 0)
    }
  }

  async function run(page) {
    await runVideoSetup(page)
    await runScoreAgain(page)
    await runMotion(page)
  }

  return { run }
}
