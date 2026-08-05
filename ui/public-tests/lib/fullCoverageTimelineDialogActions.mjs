// Direct native coverage for Timeline popovers, context-dialog actions,
// marker editing, review pins, and fit-to-fill.

export function createTimelineDialogActionCoverage({
  probe,
  verb,
  state,
  waitForState,
  captureVerbResp,
  sleep,
  closeOverlays,
  selectClip,
  awaitImportJobs,
}) {
  const flatClips = (project) => (
    (project?.tracks || []).flatMap((track) => (
      (track.clips || []).map((clip) => ({ ...clip, trackId: track.id, trackKind: track.kind }))
    ))
  )
  const videoClips = (project) => flatClips(project).filter((clip) => clip.trackKind === 'video' && clip.asset)

  async function startLiveProject(page, tag, mediaPath) {
    const created = await verb('project.create', {
      name: `fcv_${tag}_${Math.random().toString(36).slice(2, 6)}`,
      settings: { width: 1280, height: 720, fps: 30 },
    })
    if (!created.ok) throw new Error(`${tag} project.create failed: ${created.error?.message || 'error'}`)
    const imported = await verb('media.import', { path: mediaPath })
    if (!imported.ok) throw new Error(`${tag} media.import failed: ${imported.error?.message || 'error'}`)
    await awaitImportJobs(imported)
    await waitForState((project) => videoClips(project).length > 0, 15_000)
    await sleep(800)
    await closeOverlays(page)
    await page.locator('[data-cut-mode="edit"]').first().click().catch(() => {})
    await page.locator('[data-cut-clip]').first().waitFor({ state: 'visible', timeout: 10_000 })
  }

  async function openClipMenu(page, clipId) {
    await page.keyboard.press('Escape').catch(() => {})
    await page.locator('[data-cut-ctx-backdrop]').click({ force: true, timeout: 800 }).catch(() => {})
    for (let attempt = 0; attempt < 12; attempt += 1) {
      if ((await page.locator('[data-cut-clip-menu]').count().catch(() => 0)) === 0) break
      await sleep(80)
    }
    const clip = page.locator(`[data-cut-clip="${clipId}"]`).first()
    await clip.waitFor({ state: 'visible', timeout: 10_000 })
    await clip.scrollIntoViewIfNeeded().catch(() => {})
    await clip.click({ button: 'right', force: true, timeout: 5_000 }).catch(() => {})
    await page.waitForSelector('[data-cut-clip-menu]', { timeout: 1_500 }).catch(
      async () => {
        await clip.evaluate((element) => {
          const rect = element.getBoundingClientRect()
          element.dispatchEvent(new MouseEvent('contextmenu', {
            bubbles: true,
            cancelable: true,
            clientX: rect.left + rect.width / 2,
            clientY: rect.top + rect.height / 2,
            button: 2,
          }))
        })
      },
    )
    await page.waitForSelector('[data-cut-clip-menu]', { timeout: 3_000 }).catch(() => {})
    await sleep(150)
    if ((await page.locator('[data-cut-clip-menu]').count().catch(() => 0)) === 0) {
      throw new Error(`clip context menu did not open for ${clipId}`)
    }
  }

  async function openMarkerMenu(page, markerId) {
    await page.keyboard.press('Escape').catch(() => {})
    const marker = page.locator(`[data-cut-marker="${markerId}"]`).first()
    await marker.waitFor({ state: 'visible', timeout: 10_000 })
    await marker.scrollIntoViewIfNeeded().catch(() => {})
    await marker.click({ button: 'right', force: true, timeout: 5_000 }).catch(() => {})
    await page.waitForSelector('[data-cut-marker-menu]', { timeout: 1_500 }).catch(
      async () => {
        await marker.evaluate((element) => {
          const rect = element.getBoundingClientRect()
          element.dispatchEvent(new MouseEvent('contextmenu', {
            bubbles: true,
            cancelable: true,
            clientX: rect.left + rect.width / 2,
            clientY: rect.top + rect.height / 2,
            button: 2,
          }))
        })
      },
    )
    await page.waitForSelector('[data-cut-marker-menu]', { timeout: 3_000 }).catch(() => {})
    await sleep(150)
    if ((await page.locator('[data-cut-marker-menu]').count().catch(() => 0)) === 0) {
      throw new Error(`marker context menu did not open for ${markerId}`)
    }
  }

  async function ensureClipSelected(page, clipId) {
    if (await selectClip(page, clipId)) return
    const selected = await verb('ui.select', { clip_ids: [clipId] })
    await sleep(180)
    const current = await verb('ui.state', {})
    if (!current.result?.selected_clip_ids?.includes(clipId)) {
      throw new Error(
        `Timeline dialog fixture could not select clip ${clipId}: ` +
        `${selected.error?.message || selected.error?.code || 'selection not confirmed'}`,
      )
    }
  }

  async function runCrossfadeAndDialogs(page, primaryMedia) {
    await startLiveProject(page, 'timeline-dialogs', primaryMedia)
    const surface = 'timeline-actions'
    const timeline = page.locator('[data-cut-panel="timeline"]').first()
    const project = await state()
    const trackId = project.tracks.find((track) => track.kind === 'video')?.id
    if (!trackId) throw new Error('Timeline dialog fixture has no video track')
    const split = await verb('edit.split', { track: trackId, at_ms: 3_000 })
    if (!split.ok) throw new Error(`Timeline dialog split failed: ${split.error?.message || 'error'}`)
    const splitState = await waitForState((next) => videoClips(next).length >= 2, 12_000)
    const clips = videoClips(splitState)
    if (clips.length < 2) throw new Error('Timeline dialog fixture did not render two video clips')
    await page.locator(`[data-cut-clip="${clips[1].id}"]`).waitFor({ state: 'visible', timeout: 10_000 })

    const seam = page.locator(`[data-cut-seam="${clips[0].id}:${clips[1].id}"]`).first()
    await seam.click()
    const crossfade = page.locator('[data-cut-xfade-pop]').first()
    await crossfade.waitFor({ state: 'visible', timeout: 8_000 })

    const duration = page.locator('[data-cut-xfade-input]').first()
    await probe(page, {
      surface, name: 'xfade-input', actionId: 'xfade-input',
      sel: duration, group: crossfade, groupName: 'crossfade-popover',
      doClick: async () => { await duration.fill('400') },
      assertResult: async () => ({
        ok: (await duration.inputValue()) === '400',
        detail: 'crossfade duration draft=400 ms',
      }),
    })

    const style = page.locator('[data-cut-xfade-style]').first()
    await probe(page, {
      surface, name: 'xfade-style', actionId: 'xfade-style',
      sel: style, group: crossfade, groupName: 'crossfade-popover',
      doClick: async () => { await style.selectOption('fade') },
      assertResult: async () => ({
        ok: (await style.inputValue()) === 'fade',
        detail: 'crossfade transition=fade',
      }),
    })

    let response = null
    const apply = page.locator('[data-cut-action="apply-xfade"]').first()
    await probe(page, {
      surface, name: 'apply-xfade', actionId: 'apply-xfade',
      sel: apply, group: crossfade, groupName: 'crossfade-popover',
      doClick: async () => {
        response = await captureVerbResp(page, 'edit.crossfade', () => apply.click(), 12_000)
      },
      assertResult: async () => {
        const liveSeam = page.locator(
          `[data-cut-seam="${clips[0].id}:${clips[1].id}"][data-cut-seam-xfade="400"]`,
        ).first()
        await liveSeam.waitFor({ state: 'visible', timeout: 10_000 }).catch(() => {})
        return {
          ok: !!response?.ok
            && response?.result?.xfade_ms === 400
            && (await liveSeam.count()) === 1,
          detail: `crossfade response=${response?.result?.xfade_ms ?? '?'} ms; rendered seam=400 ms`,
        }
      },
    })

    const liveSeam = page.locator('[data-cut-seam][data-cut-seam-xfade]').first()
    await liveSeam.waitFor({ state: 'visible', timeout: 10_000 })
    await liveSeam.click()
    const clear = page.locator('[data-cut-action="clear-xfade"]').first()
    response = null
    await probe(page, {
      surface, name: 'clear-xfade', actionId: 'clear-xfade',
      sel: clear, group: page.locator('[data-cut-xfade-pop]').first(), groupName: 'crossfade-popover',
      doClick: async () => {
        response = await captureVerbResp(page, 'edit.crossfade', () => clear.click(), 12_000)
      },
      assertResult: async () => {
        await sleep(250)
        return {
          ok: !!response?.ok && (await page.locator('[data-cut-seam][data-cut-seam-xfade]').count()) === 0,
          detail: 'crossfade cleared back to a hard cut',
        }
      },
    })

    const currentClips = videoClips(await state())
    const sourceClip = currentClips[0]
    const targetClip = currentClips[1]
    await openClipMenu(page, sourceClip.id)
    const trimTrigger = page.locator('[data-cut-ctx="trim-tools"]').first()
    const trimDisabled = await trimTrigger.isDisabled()
    await trimTrigger.click({ force: true })
    const trim = page.locator('[data-cut-trim-popover]').first()
    await sleep(800)
    const trimCount = await trim.count()
    if (trimDisabled || trimCount === 0) {
      throw new Error(
        `Trim popover did not mount for ${sourceClip.id}: disabled=${trimDisabled} count=${trimCount}`,
      )
    }
    const trimVisible = await trim.isVisible().catch(() => false)
    if (!trimVisible) {
      const trimBox = await trim.boundingBox().catch(() => null)
      const trimStyle = await trim.getAttribute('style').catch(() => null)
      const diagnostics = await trim.evaluate((element) => {
        const css = getComputedStyle(element)
        const rect = element.getBoundingClientRect()
        const hit = document.elementFromPoint(rect.left + rect.width / 2, rect.top + rect.height / 2)
        return {
          display: css.display,
          visibility: css.visibility,
          opacity: css.opacity,
          overflow: css.overflow,
          viewport: [window.innerWidth, window.innerHeight],
          hit: hit?.getAttribute('data-cut-trim-popover')
            ? 'trim-popover'
            : (hit?.getAttribute('data-cut-trim-backdrop') ? 'trim-backdrop' : hit?.tagName || 'none'),
        }
      }).catch(() => null)
      throw new Error(
        `Trim popover mounted but is not visible: box=${JSON.stringify(trimBox)} ` +
        `style=${trimStyle || 'none'} css=${JSON.stringify(diagnostics)}`,
      )
    }
    const trimStep = page.locator('[data-cut-trim-step="slip:1"]').first()
    response = null
    const sourceBeforeSlip = flatClips(await state()).find((clip) => clip.id === sourceClip.id)
    await probe(page, {
      surface, name: 'trim-step', actionId: 'trim-step',
      sel: trimStep, group: trim, groupName: 'trim-popover',
      doClick: async () => {
        response = await captureVerbResp(page, 'edit.slip', () => trimStep.click(), 12_000)
      },
      assertResult: async () => {
        const changed = await waitForState((next) => (
          flatClips(next).find((clip) => clip.id === sourceClip.id)?.src_in_ms
            !== sourceBeforeSlip?.src_in_ms
        ), 10_000)
        return { ok: !!response?.ok && !!changed, detail: 'one-frame slip persisted' }
      },
    })

    const trimClose = page.locator('[data-cut-trim-close]').first()
    await probe(page, {
      surface, name: 'trim-close', actionId: 'trim-close',
      sel: trimClose, group: trim, groupName: 'trim-popover',
      doClick: async () => {
        await trimClose.click()
        await trim.waitFor({ state: 'detached', timeout: 5_000 })
      },
      assertResult: async () => ({
        ok: (await trim.count()) === 0,
        detail: 'trim popover closed',
      }),
    })

    const seeded = await verb('edit.speed', {
      clip: sourceClip.id,
      factor: 1.25,
      rationale: 'fcv: distinguish paste-attributes source speed',
    })
    if (!seeded.ok) throw new Error(`paste source speed seed failed: ${seeded.error?.message || 'error'}`)
    await waitForState(
      (next) => flatClips(next).find((clip) => clip.id === sourceClip.id)?.speed === 1.25,
      10_000,
    )
    await openClipMenu(page, sourceClip.id)
    await page.locator('[data-cut-ctx="copy"]').evaluate((element) => element.click())
    await ensureClipSelected(page, targetClip.id)
    await openClipMenu(page, targetClip.id)
    await page.locator('[data-cut-ctx="paste-attributes"]').evaluate((element) => element.click())
    const pasteDialog = page.locator('[data-cut-paste-attributes]').first()
    await pasteDialog.waitFor({ state: 'visible', timeout: 8_000 })

    const cancel = page.locator('[data-cut-pa-cancel]').first()
    await probe(page, {
      surface, name: 'pa-cancel', actionId: 'pa-cancel',
      sel: cancel, group: pasteDialog, groupName: 'paste-attributes',
      doClick: async () => {
        await cancel.click()
        await pasteDialog.waitFor({ state: 'detached', timeout: 5_000 })
      },
      assertResult: async () => ({
        ok: (await pasteDialog.count()) === 0,
        detail: 'paste-attributes dialog cancelled',
      }),
    })

    await openClipMenu(page, targetClip.id)
    await page.locator('[data-cut-ctx="paste-attributes"]').evaluate((element) => element.click())
    await pasteDialog.waitFor({ state: 'visible', timeout: 8_000 })
    const firstCheck = page.locator('[data-cut-pa-check="grade"]').first()
    await probe(page, {
      surface, name: 'pa-check', actionId: 'pa-check',
      sel: firstCheck, group: pasteDialog, groupName: 'paste-attributes',
      doClick: async () => {
        for (const category of ['grade', 'transform', 'volume', 'effects']) {
          const checkbox = page.locator(`[data-cut-pa-check="${category}"]`).first()
          if (await checkbox.isChecked()) await checkbox.click()
        }
      },
      assertResult: async () => ({
        ok: await page.locator('[data-cut-pa-check="speed"]').first().isChecked()
          && !(await firstCheck.isChecked()),
        detail: 'only Speed remains selected',
      }),
    })

    const applyPaste = page.locator('[data-cut-pa-apply]').first()
    response = null
    await probe(page, {
      surface, name: 'pa-apply', actionId: 'pa-apply',
      sel: applyPaste, group: pasteDialog, groupName: 'paste-attributes',
      doClick: async () => {
        response = await captureVerbResp(page, 'edit.paste_attributes', () => applyPaste.click(), 20_000)
      },
      assertResult: async () => {
        const changed = await waitForState(
          (next) => flatClips(next).find((clip) => clip.id === targetClip.id)?.speed === 1.25,
          12_000,
        )
        return { ok: !!response?.ok && !!changed, detail: 'source speed pasted onto target clip' }
      },
    })

    return { timeline }
  }

  async function runFitToFill(page, primaryMedia, secondMedia) {
    await startLiveProject(page, 'timeline-fit', primaryMedia)
    const surface = 'timeline-actions'
    const imported = await verb('media.import', { path: secondMedia, proxy: false })
    await awaitImportJobs(imported)
    const fillAsset = imported.result?.asset_id
    // The terminal import jobs and the UI state snapshot can cross on Windows:
    // wait for the probed duration that this fixture needs instead of treating
    // a momentarily stale post-job snapshot as incompatible media.
    const project = await waitForState(
      (next) => Number(next.assets?.[fillAsset]?.probe?.duration_ms || 0) > 0,
      20_000,
    ) || await state()
    const videoTrack = project.tracks.find((track) => track.kind === 'video')
    const trackId = videoTrack?.id
    if (!fillAsset || !trackId) throw new Error(`fit fixture incomplete: asset=${fillAsset} track=${trackId}`)
    const sourceDurationMs = Number(project.assets?.[fillAsset]?.probe?.duration_ms || 0)
    const trackDurationMs = (videoTrack?.clips || []).reduce((total, clip) => {
      if (clip.kind === 'gap') return total + Number(clip.duration_ms || 0)
      const sourceSpan = Math.max(0, Number(clip.src_out_ms || 0) - Number(clip.src_in_ms || 0))
      return total + sourceSpan / Math.max(0.01, Number(clip.speed || 1))
    }, 0)
    const gapStartMs = Math.max(500, Math.min(3_000, Math.floor(trackDurationMs / 4)))
    const maxGapMs = Math.floor(trackDurationMs - gapStartMs - 500)
    // Fit-to-fill intentionally supports 0.25x-4x retiming. Derive a real gap
    // from the selected source instead of assuming every release-media role is
    // short enough for the historical fixed 8s slot (a 54s real clip needs at
    // least 13.517s at 4x). Aim for 2x and retain media after the gap so the UI
    // still proves an adjacent-slot fill rather than the easier track-tail case.
    const gapDurationMs = Math.min(maxGapMs, Math.max(1_000, Math.ceil(sourceDurationMs / 2)))
    const fitSpeed = sourceDurationMs / gapDurationMs
    if (!sourceDurationMs || gapDurationMs <= 0 || fitSpeed < 0.25 || fitSpeed > 4) {
      throw new Error(
        `fit fixture media cannot form a supported adjacent gap: source=${sourceDurationMs}ms ` +
        `track=${trackDurationMs}ms gap=${gapDurationMs}ms speed=${fitSpeed}`,
      )
    }
    const gapEndMs = gapStartMs + gapDurationMs
    await verb('edit.split', { track: trackId, at_ms: gapStartMs })
    await verb('edit.split', { track: trackId, at_ms: gapEndMs })
    await sleep(300)
    const lifted = await verb('edit.ripple_delete', {
      track: trackId,
      range_ms: [gapStartMs, gapEndMs],
      ripple: false,
      rationale: 'fcv: create adjacent gap for Timeline fit-to-fill',
    })
    if (!lifted.ok) throw new Error(`fit gap lift failed: ${lifted.error?.message || 'error'}`)
    const gapState = await waitForState(
      (next) => next.tracks.find((track) => track.id === trackId)?.clips?.some((clip) => clip.kind === 'gap'),
      12_000,
    )
    const first = videoClips(gapState)[0]
    if (!first) throw new Error('fit fixture has no clip beside the gap')
    await openClipMenu(page, first.id)
    const openFit = page.locator('[data-cut-ctx="fit-to-fill"]').first()
    if (await openFit.isDisabled()) throw new Error('fit-to-fill remained disabled beside a real gap')
    await probe(page, {
      surface, name: 'ctx-fit-to-fill', actionId: 'ctx-fit-to-fill',
      sel: openFit,
      group: page.locator('[data-cut-clip-menu]').first(),
      groupName: 'fit-to-fill-picker',
      doClick: async () => openFit.evaluate((element) => element.click()),
      assertResult: async () => ({
        ok: (await page.locator('[data-cut-ctx-fit-list]').count()) === 1,
        detail: 'Fit to fill asset picker opened beside a real gap',
      }),
    })
    const fitAsset = page.locator(`[data-cut-ctx-fit-asset="${fillAsset}"]`).first()
    let response = null
    await probe(page, {
      surface, name: 'ctx-fit-asset', actionId: 'ctx-fit-asset',
      sel: fitAsset,
      group: page.locator('[data-cut-clip-menu]').first(),
      groupName: 'fit-to-fill-picker',
      doClick: async () => {
        response = await captureVerbResp(page, 'edit.fit_to_fill', () => fitAsset.click(), 25_000)
      },
      assertResult: async () => {
        const changed = await waitForState(
          (next) => videoClips(next).some((clip) => clip.asset === fillAsset),
          20_000,
        )
        // A confirmed installed-engine state transition is authoritative when
        // the WebView adapter misses the HTTP response event. Preserve failure
        // on an explicit not-ok response; never turn a missing state change green.
        return {
          ok: response?.ok !== false && !!changed,
          detail: `gap ${gapDurationMs}ms filled with asset ${fillAsset} at ${fitSpeed.toFixed(3)}x; response=${response?.ok ?? 'not-observed'}; stateChanged=${!!changed}`,
        }
      },
    })
  }

  async function runSplitEdits(page, primaryMedia) {
    for (const kind of ['j', 'l']) {
      await startLiveProject(page, `timeline-split-${kind}`, primaryMedia)
      const before = await state()
      const videoTrack = before.tracks.find((track) => track.kind === 'video')?.id
      const audioTrack = before.tracks.find((track) => track.kind === 'audio')?.id
      if (!videoTrack || !audioTrack) throw new Error(`${kind}-cut fixture needs video and audio tracks`)
      await verb('edit.split', { track: videoTrack, at_ms: 2_000 })
      await verb('edit.split', { track: audioTrack, at_ms: 2_000 })
      const splitState = await waitForState((project) => {
        const videos = project.tracks.find((track) => track.id === videoTrack)?.clips || []
        const audios = project.tracks.find((track) => track.id === audioTrack)?.clips || []
        return videos.filter((clip) => clip.asset).length >= 2
          && audios.filter((clip) => clip.asset).length >= 2
      }, 12_000)
      const leftVideo = splitState?.tracks
        .find((track) => track.id === videoTrack)?.clips
        ?.filter((clip) => clip.asset)
        .sort((a, b) => (a.start_ms || 0) - (b.start_ms || 0))[0]
      const audioBefore = JSON.stringify(splitState?.tracks.find((track) => track.id === audioTrack)?.clips || [])
      if (!leftVideo) throw new Error(`${kind}-cut fixture did not produce a video seam`)
      await openClipMenu(page, leftVideo.id)
      const action = page.locator(`[data-cut-ctx="split-edit-${kind}"]`).first()
      let response = null
      await probe(page, {
        surface: 'ctx-menu',
        name: `ctx-split-edit-${kind}`,
        actionId: `ctx-split-edit-${kind}`,
        sel: action,
        group: page.locator('[data-cut-clip-menu]').first(),
        groupName: `split-edit-${kind}`,
        doClick: async () => {
          response = await captureVerbResp(page, 'edit.split_edit', () => action.click(), 20_000)
        },
        assertResult: async () => {
          const changed = await waitForState(
            (project) => JSON.stringify(project.tracks.find((track) => track.id === audioTrack)?.clips || []) !== audioBefore,
            12_000,
          )
          return {
            ok: !!response?.ok && !!changed,
            detail: `${kind.toUpperCase()}-cut rolled the audio seam through edit.split_edit`,
          }
        },
      })
    }
  }

  async function runMarkersAndComment(page) {
    const surface = 'timeline-actions'
    const timeline = page.locator('[data-cut-panel="timeline"]').first()
    const markerLabel = `FCV marker ${Math.random().toString(36).slice(2, 6)}`
    const added = await verb('edit.add_marker', { at_ms: 2_000, label: markerLabel })
    const markerId = added.result?.marker_id || added.result?.id
    if (!markerId) throw new Error(`marker fixture failed: ${added.error?.message || 'no id'}`)
    await waitForState((project) => project.markers?.some((marker) => marker.id === markerId), 10_000)
    await openMarkerMenu(page, markerId)

    const rename = page.locator('[data-cut-marker-rename-input]').first()
    let response = null
    const renamed = `${markerLabel} renamed`
    await probe(page, {
      surface, name: 'marker-rename-input', actionId: 'marker-rename-input',
      sel: rename, group: page.locator('[data-cut-marker-menu]').first(), groupName: 'marker-menu',
      doClick: async () => {
        await rename.fill(renamed)
        response = await captureVerbResp(page, 'edit.update_marker', () => rename.press('Enter'), 12_000)
      },
      assertResult: async () => {
        const changed = await waitForState(
          (project) => project.markers?.some((marker) => marker.id === markerId && marker.label === renamed),
          10_000,
        )
        return { ok: !!response?.ok && !!changed, detail: `marker renamed to "${renamed}"` }
      },
    })

    await openMarkerMenu(page, markerId)
    const note = page.locator('[data-cut-marker-note-input]').first()
    response = null
    await probe(page, {
      surface, name: 'marker-note-input', actionId: 'marker-note-input',
      sel: note, group: page.locator('[data-cut-marker-menu]').first(), groupName: 'marker-menu',
      doClick: async () => {
        await note.fill('Native marker note')
        response = await captureVerbResp(
          page,
          'edit.update_marker',
          () => page.locator('[data-cut-marker-ctx="note-commit"]').first().click(),
          12_000,
        )
      },
      assertResult: async () => {
        const changed = await waitForState(
          (project) => project.markers?.some((marker) => marker.id === markerId && marker.note === 'Native marker note'),
          10_000,
        )
        return { ok: !!response?.ok && !!changed, detail: 'marker note persisted' }
      },
    })

    await openMarkerMenu(page, markerId)
    const swatch = page.locator('[data-cut-marker-color-swatch="blue"]').first()
    response = null
    await probe(page, {
      surface, name: 'marker-color-swatch', actionId: 'marker-color-swatch',
      sel: swatch, group: page.locator('[data-cut-marker-menu]').first(), groupName: 'marker-menu',
      doClick: async () => {
        response = await captureVerbResp(page, 'edit.update_marker', () => swatch.click(), 12_000)
      },
      assertResult: async () => {
        const changed = await waitForState(
          (project) => project.markers?.some((marker) => marker.id === markerId && marker.color === 'blue'),
          10_000,
        )
        return { ok: !!response?.ok && !!changed, detail: 'marker color=blue' }
      },
    })

    const commentText = `FCV timeline pin ${Math.random().toString(36).slice(2, 6)}`
    const comment = await verb('comment.add', { at_ms: 2_500, text: commentText, author: 'FCV' })
    const commentId = comment.result?.comment?.id
    if (!commentId) throw new Error(`comment fixture failed: ${comment.error?.message || 'no id'}`)
    await waitForState((project) => project.comments?.some((row) => row.id === commentId), 10_000)
    const pin = page.locator(`[data-cut-comment-pin="${commentId}"]`).first()
    // project.state can expose the committed comment before the UI has handled
    // op_applied and committed its React refresh. Prove the live pin appears;
    // do not race probe's immediate presence check against that async render.
    await pin.waitFor({ state: 'visible', timeout: 10_000 })
    await probe(page, {
      surface, name: 'comment-pin', actionId: 'comment-pin',
      sel: pin, group: timeline, groupName: 'timeline-ruler',
      doClick: async () => {
        await pin.click()
        await page.locator(`[data-cut-comment="${commentId}"]`).waitFor({ state: 'visible', timeout: 8_000 })
      },
      assertResult: async () => {
        const ui = await verb('ui.state', {})
        return {
          ok: ui.result?.playhead_ms === 2_500
            && (await page.locator(`[data-cut-comment="${commentId}"]`).count()) === 1,
          detail: `comment opened at playhead=${ui.result?.playhead_ms}`,
        }
      },
    })
  }

  async function run(page, { primaryMedia, secondMedia }) {
    const context = await runCrossfadeAndDialogs(page, primaryMedia)
    await runFitToFill(page, primaryMedia, secondMedia)
    await runSplitEdits(page, primaryMedia)
    await runMarkersAndComment(page)
    return context
  }

  return { run }
}
