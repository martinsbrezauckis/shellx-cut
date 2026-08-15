// Canonical native-matrix coverage for context owners outside the clip menu.
// Each row opens the real menu on the exact Timeline target, actuates its UI
// route, and proves the resulting project or UI state.

export function createTimelineContextActionCoverage({
  probe,
  verb,
  state,
  waitForState,
  captureVerbResp,
  sleep,
  freshProject,
  closeOverlays,
  nativeOsActionsEnabled = false,
  rec,
}) {
  const surface = 'ctx-menu'
  const trackOf = (project, id) => project?.tracks?.find((track) => track.id === id)
  const trackCount = (project, kind) => project?.tracks?.filter((track) => track.kind === kind).length || 0

  async function dismiss(page) {
    await page.keyboard.press('Escape').catch(() => {})
    await page.locator('[data-cut-timeline-ctx-backdrop]').click({ timeout: 500 }).catch(() => {})
    await sleep(80)
  }

  async function beginKeyboardMenuDiagnostics(page, menuSelector) {
    return page.evaluate((selector) => {
      const describe = (node) => {
        if (!(node instanceof Element)) return null
        const rect = node.getBoundingClientRect()
        return {
          tag: node.tagName.toLowerCase(),
          id: node.id || '',
          trackHeader: node.getAttribute('data-cut-track-header') || '',
          track: node.getAttribute('data-cut-track') || '',
          role: node.getAttribute('role') || '',
          label: node.getAttribute('aria-label') || '',
          visible: rect.width > 0 && rect.height > 0,
        }
      }
      const previous = globalThis.__shellxCutFcvKeyboardMenuDiagnostics
      previous?.stop?.()
      const keys = []
      const capture = (event) => {
        keys.push({
          type: event.type,
          key: event.key,
          code: event.code,
          shiftKey: event.shiftKey,
          target: describe(event.target),
          activeElement: describe(document.activeElement),
        })
      }
      document.addEventListener('keydown', capture, true)
      document.addEventListener('keyup', capture, true)
      globalThis.__shellxCutFcvKeyboardMenuDiagnostics = {
        menuSelector: selector,
        keys,
        stop: () => {
          document.removeEventListener('keydown', capture, true)
          document.removeEventListener('keyup', capture, true)
        },
      }
      return {
        activeElement: describe(document.activeElement),
        menu: describe(document.querySelector(selector)),
      }
    }, menuSelector).catch((error) => ({ captureError: String(error?.message || error) }))
  }

  async function endKeyboardMenuDiagnostics(page, menuSelector) {
    return page.evaluate((selector) => {
      const describe = (node) => {
        if (!(node instanceof Element)) return null
        const rect = node.getBoundingClientRect()
        return {
          tag: node.tagName.toLowerCase(),
          id: node.id || '',
          trackHeader: node.getAttribute('data-cut-track-header') || '',
          track: node.getAttribute('data-cut-track') || '',
          role: node.getAttribute('role') || '',
          label: node.getAttribute('aria-label') || '',
          visible: rect.width > 0 && rect.height > 0,
        }
      }
      const diagnostics = globalThis.__shellxCutFcvKeyboardMenuDiagnostics
      const keyEvents = diagnostics?.keys?.slice(-8) || []
      diagnostics?.stop?.()
      if (globalThis.__shellxCutFcvKeyboardMenuDiagnostics === diagnostics) {
        delete globalThis.__shellxCutFcvKeyboardMenuDiagnostics
      }
      return {
        captureInstalled: diagnostics?.menuSelector === selector,
        activeElement: describe(document.activeElement),
        menu: describe(document.querySelector(selector)),
        keyEvents,
      }
    }, menuSelector).catch((error) => ({ captureError: String(error?.message || error) }))
  }

  async function openKeyboardMenu(page, target, menuSelector) {
    await dismiss(page)
    await target.waitFor({ state: 'visible', timeout: 8_000 })
    await target.scrollIntoViewIfNeeded().catch(() => {})
    const before = await beginKeyboardMenuDiagnostics(page, menuSelector)
    const headerBefore = await target.evaluate((element) => {
      const rect = element.getBoundingClientRect()
      return {
        visible: rect.width > 0 && rect.height > 0,
        locked: element.closest('[data-cut-track]')?.getAttribute('data-cut-track-locked') || 'false',
        active: document.activeElement === element,
      }
    }).catch((error) => ({ captureError: String(error?.message || error) }))
    let pressError = ''
    try {
      await target.focus()
      await page.keyboard.press('Shift+F10')
    } catch (error) {
      pressError = String(error?.message || error)
    }
    const menu = page.locator(menuSelector).first()
    let menuWaitError = ''
    try {
      await menu.waitFor({ state: 'visible', timeout: 2_500 })
    } catch (error) {
      menuWaitError = String(error?.message || error)
    }
    const headerAfterFocus = await target.evaluate((element) => ({
      active: document.activeElement === element,
      locked: element.closest('[data-cut-track]')?.getAttribute('data-cut-track-locked') || 'false',
    })).catch((error) => ({ captureError: String(error?.message || error) }))
    const after = await endKeyboardMenuDiagnostics(page, menuSelector)
    const opened = !menuWaitError && await menu.isVisible().catch(() => false)
    return {
      menu: opened ? menu : null,
      diagnostics: { before, headerBefore, headerAfterFocus, pressError, menuWaitError, after },
    }
  }

  async function openMenu(page, target, menuSelector, { ratio = 0.5 } = {}) {
    await dismiss(page)
    await target.waitFor({ state: 'visible', timeout: 8_000 })
    await target.scrollIntoViewIfNeeded().catch(() => {})
    const options = { button: 'right' }
    if (ratio !== 0.5) {
      const box = await target.boundingBox()
      if (!box || box.width <= 0 || box.height <= 0) {
        throw new Error(`right-click target has no usable box: ${menuSelector}`)
      }
      options.position = {
        x: Math.max(8, Math.min(box.width - 8, box.width * ratio)),
        y: Math.max(6, Math.min(14, box.height / 2)),
      }
    }
    // Do not force this through a DOM event: native matrix action rows need a
    // real WebDriver pointer context-menu request after any keyboard outcome.
    await target.click(options)
    const menu = page.locator(menuSelector).first()
    try {
      await menu.waitFor({ state: 'visible', timeout: 4_000 })
    } catch (firstError) {
      // WebKitGTK can acknowledge a W3C right-button action without emitting
      // contextmenu. Retry the same native pointer gesture once; never replace
      // it with a synthetic DOM event or hide a second delivery failure.
      await dismiss(page)
      await target.waitFor({ state: 'visible', timeout: 8_000 })
      await target.scrollIntoViewIfNeeded().catch(() => {})
      await target.click(options)
      try {
        await menu.waitFor({ state: 'visible', timeout: 4_000 })
      } catch (secondError) {
        throw new Error(
          `native context menu did not open after one bounded retry: ${menuSelector}; `
          + `first=${String(firstError?.message || firstError)}; `
          + `second=${String(secondError?.message || secondError)}`,
        )
      }
    }
    return menu
  }

  async function openTrack(page, trackId, menuSelector = '[data-cut-track-menu]') {
    await dismiss(page)
    const trigger = page.locator(`[data-cut-track-menu-button="${trackId}"]`).first()
    await trigger.waitFor({ state: 'visible', timeout: 8_000 })
    await trigger.scrollIntoViewIfNeeded().catch(() => {})
    await trigger.click()
    const menu = page.locator(menuSelector).first()
    await menu.waitFor({ state: 'visible', timeout: 4_000 })
    return menu
  }

  async function openTrackKeyboard(page, trackId) {
    return openKeyboardMenu(
      page,
      page.locator(`[data-cut-track-header="${trackId}"]`).first(),
      '[data-cut-track-menu]',
    )
  }

  function recordKeyboardEntry(trackId, keyboardMenu) {
    const diagnostics = keyboardMenu.diagnostics || {}
    const keyEvents = diagnostics.after?.keyEvents || []
    const sawShiftF10 = keyEvents.some((event) => event?.key === 'F10' && event?.shiftKey)
    const transportUnsupported = !keyboardMenu.menu && (!sawShiftF10 || !!diagnostics.pressError)
    const outcome = keyboardMenu.menu ? 'pass' : transportUnsupported ? 'unsupported' : 'fail'
    const headerVisible = diagnostics.headerBefore?.visible === true
    const headerUnlocked = diagnostics.headerBefore?.locked === 'false'
    rec(surface, 'track-context-keyboard-entry', {
      rowKind: 'support',
      actionId: 'track-ctx-keyboard-entry',
      present: headerVisible ? 'pass' : 'fail',
      render: 'na',
      click: outcome === 'pass' ? 'pass' : outcome === 'unsupported' ? 'na' : 'fail',
      result: outcome === 'pass' ? 'pass' : outcome === 'unsupported' ? 'na' : 'fail',
    }, [
      `track=${trackId}`,
      `outcome=${outcome}`,
      `header visible=${headerVisible} unlocked=${headerUnlocked}`,
      `active before=${JSON.stringify(diagnostics.before?.activeElement || null)}`,
      `active after=${JSON.stringify(diagnostics.after?.activeElement || null)}`,
      `keys=${JSON.stringify(keyEvents)}`,
      `menu=${JSON.stringify(diagnostics.after?.menu || null)}`,
      diagnostics.pressError ? `pressError=${diagnostics.pressError}` : '',
      diagnostics.menuWaitError ? `menuWaitError=${diagnostics.menuWaitError}` : '',
    ].filter(Boolean).join('; '))
    return outcome
  }

  async function openLockedClip(page, trackId) {
    return openMenu(
      page,
      page.locator(`[data-cut-track="${trackId}"] [data-cut-clip]`).first(),
      '[data-cut-locked-track-menu]',
    )
  }

  async function waitForTrackLockDom(page, trackId, locked) {
    const value = locked ? 'true' : 'false'
    await page.locator(
      `[data-cut-track="${trackId}"][data-cut-track-locked="${value}"]`,
    ).first().waitFor({ state: 'visible', timeout: 8_000 })
  }

  async function openEmpty(page, trackId) {
    return openMenu(
      page,
      page.locator(`[data-cut-track="${trackId}"] .tl-lane`).first(),
      '[data-cut-timeline-empty-menu]',
      { ratio: 0.72 },
    )
  }

  async function openGap(page, gapId) {
    return openMenu(page, page.locator(`[data-cut-gap="${gapId}"]`).first(), '[data-cut-gap-menu]')
  }

  async function copyVideoClip(page, trackId) {
    const clip = page.locator(`[data-cut-track="${trackId}"] [data-cut-clip]`).first()
    const menu = await openMenu(page, clip, '[data-cut-clip-menu]')
    const copy = menu.locator('[data-cut-ctx="copy"]')
    await copy.waitFor({ state: 'visible', timeout: 4_000 })
    await copy.click()
    await sleep(120)
  }

  async function addTrackFixture(kind, rationale) {
    const response = await verb('edit.add_track', { kind, rationale })
    const id = response?.result?.track_id || response?.result?.id || ''
    if (!response?.ok || !id) throw new Error(`could not add ${kind} track fixture`)
    await waitForState((project) => !!trackOf(project, id), 8_000)
    return id
  }

  async function runTrackAndEmptyMenus(page) {
    await freshProject(page, 'ctx-surface-track')
    await closeOverlays(page)
    let project = await state()
    const baseVideo = project.tracks.find((track) => track.kind === 'video')?.id
    const baseAudio = project.tracks.find((track) => track.kind === 'audio')?.id
    if (!baseVideo || !baseAudio) throw new Error('context surface fixture lacks base video/audio tracks')
    const overlay = await addTrackFixture('video', 'fcv: empty-lane and removable-track context fixture')

    const keyboardMenu = await openTrackKeyboard(page, baseVideo)
    const keyboardOutcome = recordKeyboardEntry(baseVideo, keyboardMenu)
    // Keep the Shift+F10 row self-contained. A failed/unsupported keyboard
    // transport must remain visible in its own receipt row; only then can the
    // independent menu actions proceed through a real pointer right-click.
    let menu = keyboardMenu.menu || await openTrack(page, baseVideo)
    const menuEntry = keyboardOutcome === 'pass' ? 'Shift+F10' : 'WebDriver right-click after keyboard entry did not pass'
    let response = null
    await probe(page, {
      surface, name: 'track-context-lock', actionId: 'track-ctx',
      sel: menu.locator('[data-cut-track-ctx="lock"]'), group: menu, groupName: 'ctx-track-video',
      doClick: async () => {
        response = await captureVerbResp(page, 'edit.track_lock', () => menu.locator('[data-cut-track-ctx="lock"]').click(), 12_000)
      },
      assertResult: async () => ({
        ok: !!response?.ok && !!(await waitForState((next) => trackOf(next, baseVideo)?.locked === true, 8_000)),
        detail: `${baseVideo} locked through ${menuEntry}; response=${response?.ok}`,
      }),
    })

    // Engine state is authoritative, but React may publish the matching track
    // attribute one render later. The locked-menu branch reads that DOM state,
    // so do not race it on slower native WebKitGTK hosts.
    await waitForTrackLockDom(page, baseVideo, true)
    menu = await openLockedClip(page, baseVideo)
    const lockedClipId = await page.locator(`[data-cut-track="${baseVideo}"] [data-cut-clip]`).first().getAttribute('data-cut-clip')
    await probe(page, {
      surface, name: 'locked-track-context-inspect', actionId: 'track-ctx',
      sel: menu.locator('[data-cut-track-ctx="inspect"]'), group: menu, groupName: 'ctx-track-locked',
      doClick: async () => { await menu.locator('[data-cut-track-ctx="inspect"]').click(); await sleep(220) },
      assertResult: async () => {
        const ui = await verb('ui.state', {})
        return {
          ok: !!lockedClipId && ui.result?.selected_clip_ids?.includes(lockedClipId)
            && ui.result?.open_surface_ids?.includes('properties'),
          detail: `selected=${ui.result?.selected_clip_ids?.join(',') || 'none'} properties=${ui.result?.open_surface_ids?.includes('properties')}`,
        }
      },
    })

    menu = await openLockedClip(page, baseVideo)
    response = null
    await probe(page, {
      surface, name: 'locked-track-context-unlock', actionId: 'track-ctx',
      sel: menu.locator('[data-cut-track-ctx="lock"]'), group: menu, groupName: 'ctx-track-locked',
      doClick: async () => {
        response = await captureVerbResp(page, 'edit.track_lock', () => menu.locator('[data-cut-track-ctx="lock"]').click(), 12_000)
      },
      assertResult: async () => ({
        ok: !!response?.ok && !!(await waitForState((next) => trackOf(next, baseVideo)?.locked !== true, 8_000)),
        detail: `${baseVideo} unlocked; response=${response?.ok}`,
      }),
    })

    await waitForTrackLockDom(page, baseVideo, false)
    menu = await openTrack(page, baseVideo)
    response = null
    await probe(page, {
      surface, name: 'track-context-visibility', actionId: 'track-ctx',
      sel: menu.locator('[data-cut-track-ctx="visibility"]'), group: menu, groupName: 'ctx-track-video',
      doClick: async () => {
        response = await captureVerbResp(page, 'edit.track_visible', () => menu.locator('[data-cut-track-ctx="visibility"]').click(), 12_000)
      },
      assertResult: async () => ({
        ok: !!response?.ok && !!(await waitForState((next) => trackOf(next, baseVideo)?.visible === false, 8_000)),
        detail: `${baseVideo} visible=false; response=${response?.ok}`,
      }),
    })

    for (const [action, verbName, field] of [['mute', 'edit.mute', 'muted'], ['solo', 'edit.solo', 'solo']]) {
      menu = await openTrack(page, baseAudio)
      response = null
      await probe(page, {
        surface, name: `track-context-${action}`, actionId: 'track-ctx',
        sel: menu.locator(`[data-cut-track-ctx="${action}"]`), group: menu, groupName: 'ctx-track-audio',
        doClick: async () => {
          response = await captureVerbResp(page, verbName, () => menu.locator(`[data-cut-track-ctx="${action}"]`).click(), 12_000)
        },
        assertResult: async () => ({
          ok: !!response?.ok && !!(await waitForState((next) => trackOf(next, baseAudio)?.[field] === true, 8_000)),
          detail: `${baseAudio} ${field}=true; response=${response?.ok}`,
        }),
      })
    }

    menu = await openEmpty(page, overlay)
    let emptyAt = null
    await probe(page, {
      surface, name: 'empty-context-seek', actionId: 'timeline-ctx',
      sel: menu.locator('[data-cut-timeline-ctx="empty-seek"]'), group: menu, groupName: 'ctx-empty-lane',
      doClick: async () => { await menu.locator('[data-cut-timeline-ctx="empty-seek"]').click(); await sleep(180) },
      assertResult: async () => {
        const ui = await verb('ui.state', {})
        emptyAt = Number(ui.result?.playhead_ms)
        return { ok: Number.isFinite(emptyAt) && emptyAt > 0, detail: `playhead=${emptyAt}ms` }
      },
    })

    menu = await openEmpty(page, overlay)
    const markersBefore = (await state()).markers?.length || 0
    response = null
    await probe(page, {
      surface, name: 'empty-context-marker', actionId: 'timeline-ctx',
      sel: menu.locator('[data-cut-timeline-ctx="empty-marker"]'), group: menu, groupName: 'ctx-empty-lane',
      doClick: async () => {
        response = await captureVerbResp(page, 'edit.add_marker', () => menu.locator('[data-cut-timeline-ctx="empty-marker"]').click(), 12_000)
      },
      assertResult: async () => ({
        ok: !!response?.ok && !!(await waitForState((next) => (next.markers?.length || 0) > markersBefore, 8_000)),
        detail: `marker count increased from ${markersBefore}; response=${response?.ok}`,
      }),
    })

    for (const [action, endpoint] of [['empty-mark-in', 0], ['empty-mark-out', 1]]) {
      menu = await openEmpty(page, overlay)
      await probe(page, {
        surface, name: `empty-context-${action === 'empty-mark-in' ? 'mark-in' : 'mark-out'}`, actionId: 'timeline-ctx',
        sel: menu.locator(`[data-cut-timeline-ctx="${action}"]`), group: menu, groupName: 'ctx-empty-lane',
        doClick: async () => { await menu.locator(`[data-cut-timeline-ctx="${action}"]`).click(); await sleep(120) },
        assertResult: async () => {
          const raw = await page.locator('[data-cut-range]').first().getAttribute('data-cut-range') || ''
          const range = raw.split(',').map(Number)
          const exact = Number.isFinite(emptyAt) && Math.abs(range[endpoint] - emptyAt) <= 100
          return { ok: range.length === 2 && exact && range[1] > range[0], detail: `range=${raw}; expected endpoint≈${emptyAt}` }
        },
      })
    }

    for (const kind of ['video', 'audio']) {
      menu = await openEmpty(page, overlay)
      const before = trackCount(await state(), kind)
      response = null
      await probe(page, {
        surface, name: `empty-context-add-${kind}-track`, actionId: 'timeline-ctx',
        sel: menu.locator(`[data-cut-timeline-ctx="empty-add-${kind}-track"]`), group: menu, groupName: 'ctx-empty-lane',
        doClick: async () => {
          response = await captureVerbResp(page, 'edit.add_track', () => menu.locator(`[data-cut-timeline-ctx="empty-add-${kind}-track"]`).click(), 12_000)
        },
        assertResult: async () => ({
          ok: !!response?.ok && !!(await waitForState((next) => trackCount(next, kind) > before, 8_000)),
          detail: `${kind} track count increased from ${before}; response=${response?.ok}`,
        }),
      })
    }

    await copyVideoClip(page, baseVideo)
    menu = await openEmpty(page, overlay)
    const overlayBefore = trackOf(await state(), overlay)?.clips?.length || 0
    response = null
    await probe(page, {
      surface, name: 'empty-context-paste', actionId: 'timeline-ctx',
      sel: menu.locator('[data-cut-timeline-ctx="empty-paste"]'), group: menu, groupName: 'ctx-empty-lane',
      doClick: async () => {
        response = await captureVerbResp(page, 'edit.paste', () => menu.locator('[data-cut-timeline-ctx="empty-paste"]').click(), 12_000)
      },
      assertResult: async () => ({
        ok: !!response?.ok && !!(await waitForState((next) => (trackOf(next, overlay)?.clips?.length || 0) > overlayBefore, 8_000)),
        detail: `${overlay} gained pasted clip; response=${response?.ok}`,
      }),
    })

    menu = await openTrack(page, overlay)
    const accept = (dialog) => { dialog.accept().catch(() => {}) }
    if (!nativeOsActionsEnabled) page.on('dialog', accept)
    response = null
    await probe(page, {
      surface, name: 'track-context-remove', actionId: 'track-ctx',
      sel: menu.locator('[data-cut-track-ctx="remove"]'), group: menu, groupName: 'ctx-track-overlay',
      nativeAction: { mode: 'accept', useDoClick: true, verifyResult: true },
      doClick: async () => {
        response = await captureVerbResp(page, 'edit.remove_track', () => menu.locator('[data-cut-track-ctx="remove"]').click(), 20_000)
      },
      assertResult: async () => ({
        ok: !!response?.ok && !!(await waitForState((next) => !trackOf(next, overlay), 8_000)),
        detail: `${overlay} removed after confirmation; response=${response?.ok}`,
      }),
    })
    if (!nativeOsActionsEnabled) page.off('dialog', accept)
  }

  async function buildGapFixture(page, tag) {
    await freshProject(page, tag)
    await closeOverlays(page)
    const videoTrack = (await state()).tracks.find((track) => track.kind === 'video')?.id
    if (!videoTrack) throw new Error('gap context fixture lacks a video track')
    const lifted = await verb('edit.ripple_delete', {
      track: videoTrack,
      range_ms: [900, 1500],
      ripple: false,
      rationale: `fcv: ${tag} real gap context fixture`,
    })
    if (!lifted?.ok) throw new Error(`gap fixture lift failed: ${lifted?.error?.message || 'unknown error'}`)
    const gap = page.locator(`[data-cut-track="${videoTrack}"] [data-cut-gap]`).first()
    await gap.waitFor({ state: 'visible', timeout: 8_000 })
    const gapId = await gap.getAttribute('data-cut-gap')
    if (!gapId) throw new Error('gap fixture rendered without an id')
    await copyVideoClip(page, videoTrack)
    return { videoTrack, gapId }
  }

  async function runGapMenus(page) {
    let fixture = await buildGapFixture(page, 'ctx-surface-gap-fit')
    let menu = await openGap(page, fixture.gapId)
    await probe(page, {
      surface, name: 'gap-context-seek', actionId: 'timeline-ctx',
      sel: menu.locator('[data-cut-timeline-ctx="gap-seek"]'), group: menu, groupName: 'ctx-gap',
      doClick: async () => { await menu.locator('[data-cut-timeline-ctx="gap-seek"]').click(); await sleep(160) },
      assertResult: async () => {
        const ui = await verb('ui.state', {})
        const at = Number(ui.result?.playhead_ms)
        return { ok: Math.abs(at - 900) <= 100, detail: `gap seek playhead=${at}ms` }
      },
    })

    menu = await openGap(page, fixture.gapId)
    await probe(page, {
      surface, name: 'gap-context-select-range', actionId: 'timeline-ctx',
      sel: menu.locator('[data-cut-timeline-ctx="gap-select-range"]'), group: menu, groupName: 'ctx-gap',
      doClick: async () => { await menu.locator('[data-cut-timeline-ctx="gap-select-range"]').click(); await sleep(120) },
      assertResult: async () => {
        const raw = await page.locator('[data-cut-range]').first().getAttribute('data-cut-range') || ''
        const range = raw.split(',').map(Number)
        return { ok: range.length === 2 && Math.abs(range[0] - 900) <= 100 && Math.abs(range[1] - 1500) <= 100, detail: `gap range=${raw}` }
      },
    })

    menu = await openGap(page, fixture.gapId)
    let response = null
    await probe(page, {
      surface, name: 'gap-context-fit-clipboard', actionId: 'timeline-ctx',
      sel: menu.locator('[data-cut-timeline-ctx="gap-fit-clipboard"]'), group: menu, groupName: 'ctx-gap',
      doClick: async () => {
        response = await captureVerbResp(page, 'edit.fit_to_fill', () => menu.locator('[data-cut-timeline-ctx="gap-fit-clipboard"]').click(), 20_000)
        await sleep(300)
      },
      assertResult: async () => {
        const gone = !!(await waitForState((next) => (
          !(next.tracks || []).some((track) => (track.clips || []).some((clip) => clip.id === fixture.gapId))
        ), 8_000))
        const detached = (await page.locator(`[data-cut-gap="${fixture.gapId}"]`).count()) === 0
        return { ok: !!response?.ok && gone && detached, detail: `fit response=${response?.ok}; state gap gone=${gone}; rendered gap detached=${detached}` }
      },
    })

    fixture = await buildGapFixture(page, 'ctx-surface-gap-paste')
    menu = await openGap(page, fixture.gapId)
    const clipsBefore = trackOf(await state(), fixture.videoTrack)?.clips?.length || 0
    response = null
    await probe(page, {
      surface, name: 'gap-context-paste', actionId: 'timeline-ctx',
      sel: menu.locator('[data-cut-timeline-ctx="gap-paste"]'), group: menu, groupName: 'ctx-gap',
      doClick: async () => {
        response = await captureVerbResp(page, 'edit.paste', () => menu.locator('[data-cut-timeline-ctx="gap-paste"]').click(), 12_000)
      },
      assertResult: async () => ({
        ok: !!response?.ok && !!(await waitForState((next) => (trackOf(next, fixture.videoTrack)?.clips?.length || 0) > clipsBefore, 8_000)),
        detail: `gap paste response=${response?.ok}; clip count increased from ${clipsBefore}`,
      }),
    })
  }

  async function run(page) {
    await runTrackAndEmptyMenus(page)
    await runGapMenus(page)
  }

  return { run }
}
