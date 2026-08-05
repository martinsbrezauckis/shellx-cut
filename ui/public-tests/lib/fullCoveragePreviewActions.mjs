// Direct action coverage for the always-mounted Preview monitor.
//
// This section owns every transport/view/export control individually, including
// the conditional FFmpeg setup strip. The latter is reached deterministically
// by intercepting only system.doctor inside the already-running test document;
// production state and the installed toolchain are restored before the section
// exits.

export function createPreviewActionCoverage({
  probe,
  verb,
  state,
  waitForState,
  captureVerbResp,
  sleep,
  freshProject,
  closeOverlays,
  selectClip,
}) {
  const surface = 'preview-actions'

  async function waitFor(check, timeoutMs = 8000) {
    const deadline = Date.now() + timeoutMs
    let last = null
    while (Date.now() < deadline) {
      try {
        last = await check()
        if (last) return last
      } catch {}
      await sleep(100)
    }
    return last
  }

  async function uiState() {
    const response = await verb('ui.state', {})
    return response?.result || {}
  }

  async function seekFromAgent(page, atMs) {
    const response = await verb('ui.playhead', { at_ms: atMs })
    if (!response?.ok) throw new Error(`ui.playhead(${atMs}) failed`)
    return waitFor(async () => {
      const current = Number((await uiState()).playhead_ms)
      return Math.abs(current - atMs) <= 2 ? current : null
    }, 8000)
  }

  async function pause(page) {
    const panel = page.locator('[data-cut-panel="preview"]').first()
    if ((await panel.getAttribute('data-cut-playing')) === 'true') {
      await page.locator('[data-cut-transport-btn="play"]').first().click()
      await waitFor(async () => (await panel.getAttribute('data-cut-playing')) === 'false')
    }
  }

  async function probeTransport(page, panel, name, key, assertResult) {
    const control = page.locator(`[data-cut-transport-btn="${key}"]`).first()
    await probe(page, {
      surface,
      name,
      actionId: 'transport-btn',
      sel: control,
      group: panel,
      groupName: 'preview-transport',
      doClick: async () => {
        await control.click()
        await sleep(180)
      },
      assertResult,
    })
  }

  async function installMissingDoctorFixture(page) {
    await page.evaluate(() => {
      const target = window
      if (!target.__fcvPreviewOriginalFetch) target.__fcvPreviewOriginalFetch = window.fetch
      target.__fcvPreviewDoctorRequests = 0
      const originalFetch = target.__fcvPreviewOriginalFetch
      window.fetch = async (...args) => {
        const input = args[0]
        const url = typeof input === 'string' ? input : input?.url || ''
        if (String(url).includes('/api/verb/system.doctor')) {
          target.__fcvPreviewDoctorRequests += 1
          return new Response(JSON.stringify({
            ok: true,
            result: {
              schema: 'shellx-cut/doctor-fixture@1',
              scanned_at: new Date().toISOString(),
              os: 'full-coverage',
              arch: 'fixture',
              app_version: 'fixture',
              essential_ok: false,
              cards: [{
                id: 'ffmpeg',
                kind: 'tool',
                status: 'missing',
                source: 'missing',
                hint: 'Install video processing',
                details: {},
              }],
            },
          }), {
            status: 200,
            headers: { 'content-type': 'application/json' },
          })
        }
        return originalFetch(...args)
      }
      document.dispatchEvent(new CustomEvent('cut:refresh-doctor'))
    })
    return waitFor(async () => (
      await page.locator('[data-cut-preview-ffmpeg-setup]').first().isVisible().catch(() => false)
    ), 8000)
  }

  async function restoreDoctorFixture(page) {
    await page.evaluate(() => {
      const target = window
      if (target.__fcvPreviewOriginalFetch) {
        window.fetch = target.__fcvPreviewOriginalFetch
        delete target.__fcvPreviewOriginalFetch
      }
      if (target.__fcvPreviewOriginalOpen) {
        window.open = target.__fcvPreviewOriginalOpen
        delete target.__fcvPreviewOriginalOpen
      }
      delete target.__fcvPreviewDoctorRequests
      delete target.__fcvPreviewManualUrl
      document.dispatchEvent(new CustomEvent('cut:refresh-doctor'))
    })
    await waitFor(async () => (
      (await page.locator('[data-cut-preview-ffmpeg-setup]').count()) === 0
    ), 8000)
  }

  async function run(page) {
    await freshProject(page, 'preview-actions')
    await closeOverlays(page)
    const panel = page.locator('[data-cut-panel="preview"]').first()
    await panel.waitFor({ state: 'visible', timeout: 12_000 })

    // Keep the exact-range render short while retaining a real imported video.
    const beforeSplit = await state()
    const videoTrack = (beforeSplit.tracks || []).find((track) => (
      track.kind === 'video' && (track.clips || []).some((clip) => clip.asset)
    ))
    if (!videoTrack?.id) throw new Error('Preview action fixture has no video track')
    const split = await verb('edit.split', { track: videoTrack.id, at_ms: 1800 })
    if (!split?.ok) throw new Error(`Preview action fixture split failed: ${split?.error?.message || 'unknown error'}`)
    const splitState = await waitForState((project) => (
      (project.tracks || []).find((track) => track.id === videoTrack.id)?.clips?.filter((clip) => clip.asset).length >= 2
    ), 12_000)
    const selectedClip = splitState?.tracks
      ?.find((track) => track.id === videoTrack.id)
      ?.clips?.find((clip) => clip.asset)
    if (!selectedClip?.id || !(await selectClip(page, selectedClip.id))) {
      throw new Error('Preview action fixture could not select the short first clip')
    }

    await seekFromAgent(page, 900)
    await probeTransport(page, panel, 'transport-to-start', 'start', async () => {
      const current = Number((await uiState()).playhead_ms)
      return { ok: current === 0, detail: `playhead=${current}ms` }
    })

    await probeTransport(page, panel, 'transport-to-end', 'end', async () => {
      const timecode = (await page.locator('[data-cut-tc]').first().textContent() || '')
        .split('/')
        .map((part) => part.trim())
      return {
        ok: timecode.length === 2 && timecode[0] === timecode[1],
        detail: `timecode=${timecode.join(' / ')}`,
      }
    })

    await seekFromAgent(page, 500)
    await probeTransport(page, panel, 'transport-play-pause', 'play', async () => {
      const playing = await panel.getAttribute('data-cut-playing')
      return { ok: playing === 'true', detail: `preview playing=${playing}` }
    })
    await pause(page)

    await seekFromAgent(page, 1200)
    await probeTransport(page, panel, 'transport-shuttle-back', 'back', async () => {
      const control = page.locator('[data-cut-transport-btn="back"]').first()
      const active = await control.evaluate((element) => element.classList.contains('pv-btn--active'))
      return {
        ok: active && (await panel.getAttribute('data-cut-playing')) === 'true',
        detail: `reverse shuttle active=${active}`,
      }
    })
    await pause(page)

    await seekFromAgent(page, 500)
    await probeTransport(page, panel, 'transport-shuttle-forward', 'fwd', async () => {
      const control = page.locator('[data-cut-transport-btn="fwd"]').first()
      const active = await control.evaluate((element) => element.classList.contains('pv-btn--active'))
      return {
        ok: active && (await panel.getAttribute('data-cut-playing')) === 'true',
        detail: `forward shuttle active=${active}`,
      }
    })
    await pause(page)

    const audio = page.locator('[data-cut-audio-toggle]').first()
    let audioBefore = ''
    await probe(page, {
      surface,
      name: 'preview-audio-toggle',
      actionId: 'audio-toggle',
      sel: audio,
      group: panel,
      groupName: 'preview-view-controls',
      doClick: async () => {
        audioBefore = await audio.getAttribute('data-cut-audio-on') || ''
        await audio.click()
        await sleep(120)
      },
      assertResult: async () => {
        const after = await audio.getAttribute('data-cut-audio-on') || ''
        return { ok: !!audioBefore && after !== audioBefore, detail: `audio ${audioBefore}→${after}` }
      },
    })
    await audio.click()

    const quality = page.locator('[data-cut-quality-toggle]').first()
    let qualityBefore = ''
    await probe(page, {
      surface,
      name: 'preview-composed-toggle',
      actionId: 'quality-toggle',
      sel: quality,
      group: panel,
      groupName: 'preview-view-controls',
      doClick: async () => {
        qualityBefore = await quality.getAttribute('data-cut-composed') || ''
        await quality.click()
        await sleep(300)
      },
      assertResult: async () => {
        const after = await quality.getAttribute('data-cut-composed') || ''
        return { ok: !!qualityBefore && after !== qualityBefore, detail: `composed ${qualityBefore}→${after}` }
      },
    })

    const guides = page.locator('[data-cut-action="cycle-guides"]').first()
    const guideOrder = ['off', 'thirds', 'safe', 'both']
    for (let index = 0; index < guideOrder.length; index++) {
      let before = ''
      await probe(page, {
        surface,
        name: `preview-guides-cycle-${index + 1}`,
        actionId: 'cycle-guides',
        sel: guides,
        group: panel,
        groupName: 'preview-view-controls',
        doClick: async () => {
          before = await guides.getAttribute('data-cut-guides') || ''
          await guides.click()
          await sleep(100)
        },
        assertResult: async () => {
          const after = await guides.getAttribute('data-cut-guides') || ''
          const expected = guideOrder[(guideOrder.indexOf(before) + 1) % guideOrder.length]
          return { ok: after === expected, detail: `guides ${before}→${after}` }
        },
      })
    }

    const fullscreen = page.locator('[data-cut-action="fullscreen-toggle"]').first()
    await probe(page, {
      surface,
      name: 'preview-fullscreen-enter',
      actionId: 'fullscreen-toggle',
      sel: fullscreen,
      group: panel,
      groupName: 'preview-fullscreen-entry',
      doClick: async () => {
        await fullscreen.click()
        await waitFor(async () => (await panel.getAttribute('data-cut-fullscreen')) === 'true')
      },
      assertResult: async () => {
        const full = await panel.getAttribute('data-cut-fullscreen')
        const box = await panel.boundingBox()
        const viewport = page.viewportSize()
        return {
          ok: full === 'true' && !!box && !!viewport
            && box.width >= viewport.width - 2
            && box.height >= viewport.height - 2,
          detail: `fullscreen=${full}; panel=${Math.round(box?.width || 0)}x${Math.round(box?.height || 0)}; viewport=${viewport?.width || 0}x${viewport?.height || 0}`,
        }
      },
    })
    await probe(page, {
      surface,
      name: 'preview-fullscreen-exit',
      actionId: 'fullscreen-toggle',
      sel: fullscreen,
      group: panel,
      groupName: 'preview-fullscreen-exit',
      doClick: async () => {
        await fullscreen.click()
        await waitFor(async () => (await panel.getAttribute('data-cut-fullscreen')) === 'false')
      },
      assertResult: async () => ({
        ok: (await panel.getAttribute('data-cut-fullscreen')) === 'false',
        detail: `fullscreen=${await panel.getAttribute('data-cut-fullscreen')}`,
      }),
    })

    await seekFromAgent(page, 500)
    const snapshot = page.locator('[data-cut-action="snapshot-frame"]').first()
    let snapshotResponse = null
    await probe(page, {
      surface,
      name: 'preview-snapshot-frame',
      actionId: 'snapshot-frame',
      sel: snapshot,
      group: panel,
      groupName: 'preview-export-controls',
      doClick: async () => {
        snapshotResponse = await captureVerbResp(page, 'export.frame', () => snapshot.click(), 60_000)
      },
      assertResult: async () => {
        const assetId = snapshotResponse?.result?.asset_id
        const changed = assetId
          ? await waitForState((project) => !!project.assets?.[assetId], 12_000)
          : null
        return {
          ok: !!snapshotResponse?.ok && !!assetId && !!changed,
          detail: `export.frame ok=${snapshotResponse?.ok}; asset=${assetId || 'none'}`,
        }
      },
    })

    const renderSection = page.locator('[data-cut-action="render-section"]').first()
    let rangeResponse = null
    await probe(page, {
      surface,
      name: 'preview-render-selection',
      actionId: 'render-section',
      sel: renderSection,
      group: panel,
      groupName: 'preview-export-controls',
      doClick: async () => {
        rangeResponse = await captureVerbResp(page, 'export.range', () => renderSection.click(), 120_000)
        await page.locator('[data-cut-exact]').first().waitFor({ state: 'visible', timeout: 12_000 })
      },
      assertResult: async () => ({
        ok: !!rangeResponse?.ok
          && !!rangeResponse?.result?.path
          && (await page.locator('[data-cut-exact]').count()) === 1,
        detail: `export.range ok=${rangeResponse?.ok}; exact review=${await page.locator('[data-cut-exact]').count()}`,
      }),
    })

    const exact = page.locator('[data-cut-exact]').first()
    const saveSection = page.locator('[data-cut-action="save-section"]').first()
    let saveResponse = null
    await probe(page, {
      surface,
      name: 'preview-save-selection-to-assets',
      actionId: 'save-section',
      sel: saveSection,
      group: exact,
      groupName: 'preview-exact-review',
      doClick: async () => {
        saveResponse = await captureVerbResp(page, 'media.import', () => saveSection.click(), 60_000)
      },
      assertResult: async () => {
        const assetId = saveResponse?.result?.asset_id
        const changed = assetId
          ? await waitForState((project) => !!project.assets?.[assetId], 12_000)
          : null
        return {
          ok: !!saveResponse?.ok && !!assetId && !!changed,
          detail: `media.import ok=${saveResponse?.ok}; saved asset=${assetId || 'none'}`,
        }
      },
    })

    const exitExact = page.locator('[data-cut-action="exit-exact"]').first()
    await probe(page, {
      surface,
      name: 'preview-exit-exact-review',
      actionId: 'exit-exact',
      sel: exitExact,
      group: exact,
      groupName: 'preview-exact-review',
      doClick: async () => {
        await exitExact.click()
        await sleep(120)
      },
      assertResult: async () => ({
        ok: (await page.locator('[data-cut-exact]').count()) === 0,
        detail: `exact review count=${await page.locator('[data-cut-exact]').count()}`,
      }),
    })

    // Conditional setup strip: force only system.doctor to report FFmpeg
    // missing, drive all three actions, then restore the real doctor.
    if (!(await installMissingDoctorFixture(page))) {
      throw new Error('Preview FFmpeg setup fixture did not mount')
    }
    const setup = page.locator('[data-cut-preview-ffmpeg-setup]').first()
    const install = page.locator('[data-cut-preview-install-ffmpeg]').first()
    await probe(page, {
      surface,
      name: 'preview-install-ffmpeg-route',
      actionId: 'preview-install-ffmpeg',
      sel: install,
      group: setup,
      groupName: 'preview-ffmpeg-setup',
      doClick: async () => {
        await install.click()
        await sleep(450)
      },
      assertResult: async () => {
        const environment = await page.locator('[data-cut-environment]').first().isVisible().catch(() => false)
        const highlight = (await page.locator('[data-cut-highlight]').count()) > 0
        return { ok: environment && highlight, detail: `Environment visible=${environment}; FFmpeg highlight=${highlight}` }
      },
    })
    await page.locator('[data-cut-environment-close]').first().click().catch(() => {})
    await page.locator('[data-cut-highlight-close]').first().click().catch(() => {})
    await sleep(160)

    await page.evaluate(() => {
      const target = window
      if (!target.__fcvPreviewOriginalOpen) target.__fcvPreviewOriginalOpen = window.open
      window.open = (url) => {
        target.__fcvPreviewManualUrl = String(url || '')
        return null
      }
    })
    const guide = page.locator('[data-cut-preview-ffmpeg-guide]').first()
    await probe(page, {
      surface,
      name: 'preview-ffmpeg-guide',
      actionId: 'preview-ffmpeg-guide',
      sel: guide,
      group: setup,
      groupName: 'preview-ffmpeg-setup',
      doClick: async () => {
        await guide.click()
        await sleep(80)
      },
      assertResult: async () => {
        const url = await page.evaluate(() => window.__fcvPreviewManualUrl || '')
        return {
          ok: url.includes('docs.theshellx.com/manual/cut/')
            && url.includes('feature=cut.preview.ffmpeg_setup'),
          detail: `manual=${url}`,
        }
      },
    })

    const recheck = page.locator('[data-cut-preview-ffmpeg-recheck]').first()
    let doctorBefore = 0
    await probe(page, {
      surface,
      name: 'preview-ffmpeg-recheck',
      actionId: 'preview-ffmpeg-recheck',
      sel: recheck,
      group: setup,
      groupName: 'preview-ffmpeg-setup',
      doClick: async () => {
        doctorBefore = await page.evaluate(() => window.__fcvPreviewDoctorRequests || 0)
        await recheck.click()
        await waitFor(async () => (
          (await page.evaluate(() => window.__fcvPreviewDoctorRequests || 0)) > doctorBefore
        ))
      },
      assertResult: async () => {
        const after = await page.evaluate(() => window.__fcvPreviewDoctorRequests || 0)
        return { ok: after > doctorBefore, detail: `system.doctor requests ${doctorBefore}→${after}` }
      },
    })
    await restoreDoctorFixture(page)
  }

  return { run }
}
