// Direct native coverage for Timeline toolbar actions. Each mutation is proven
// from the response plus project/UI state, including the formerly misclassified
// export.range "Save to Assets" control.

export function createTimelineToolbarActionCoverage({
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
  const mediaClip = (project, kind = 'video') => (
    project?.tracks?.find((track) => track.kind === kind)?.clips?.find((clip) => clip.asset)
  )
  const kindCount = (project, kind) => (
    project?.tracks?.filter((track) => track.kind === kind).length || 0
  )

  async function setPlayhead(page, atMs) {
    const ruler = page.locator('.tl-ruler-content').first()
    const zoomText = await page.locator('.tl-zoom-chip span').first().textContent()
    const zoom = Number.parseFloat(zoomText || '')
    const box = await ruler.boundingBox()
    if (!box || !Number.isFinite(zoom)) {
      throw new Error(`Timeline ruler fixture unavailable: zoom="${zoomText || ''}"`)
    }
    let actual = null
    for (let attempt = 0; attempt < 2; attempt += 1) {
      await page.mouse.click(
        box.x + ((atMs / 1000) * 50 * zoom),
        box.y + Math.min(8, box.height / 2),
      )
      await sleep(250)
      const current = await verb('ui.state', {})
      actual = current.result?.playhead_ms
      // The chip intentionally rounds the zoom display to two decimals, so a
      // pixel computed from it can differ by a few frames from the internal
      // zoom. This setup only needs a stable interior edit point.
      if (Number.isFinite(actual) && Math.abs(actual - atMs) <= 100) return
    }
    throw new Error(`Timeline ruler click targeted ${atMs} ms but reached ${actual ?? 'unknown'} ms`)
  }

  async function ensureClipSelected(page, clipId) {
    if (await selectClip(page, clipId)) return
    const selected = await verb('ui.select', { clip_ids: [clipId] })
    await sleep(180)
    const current = await verb('ui.state', {})
    if (!current.result?.selected_clip_ids?.includes(clipId)) {
      throw new Error(
        `Timeline toolbar fixture could not select video clip ${clipId}: ` +
        `${selected.error?.message || selected.error?.code || 'selection not confirmed'}`,
      )
    }
  }

  async function run(page) {
    await freshProject(page, 'timeline-toolbar')
    await closeOverlays(page)
    const surface = 'timeline-actions'
    const panel = page.locator('[data-cut-panel="timeline"]').first()
    const toolbar = page.locator('[data-cut-timeline-toolbar]').first()

    const timecode = page.locator('[data-cut-tc-readout]').first()
    const timeModeBefore = await timecode.getAttribute('data-cut-time-display')
    await probe(page, {
      surface, name: 'tc-readout', actionId: 'tc-readout',
      sel: timecode, group: toolbar, groupName: 'timeline-toolbar',
      doClick: async () => { await timecode.click(); await sleep(100) },
      assertResult: async () => ({
        ok: (await timecode.getAttribute('data-cut-time-display')) !== timeModeBefore,
        detail: `time display cycled from ${timeModeBefore}`,
      }),
    })

    const zoomLabel = page.locator('.tl-zoom-chip span').first()
    const zoomBefore = await zoomLabel.textContent()
    const zoomOut = page.locator('[data-cut-zoom-out]').first()
    await probe(page, {
      surface, name: 'zoom-out', actionId: 'zoom-out',
      sel: zoomOut, group: toolbar, groupName: 'timeline-toolbar',
      doClick: async () => { await zoomOut.click(); await sleep(120) },
      assertResult: async () => ({
        ok: (await zoomLabel.textContent()) !== zoomBefore,
        detail: `zoom ${zoomBefore} -> ${await zoomLabel.textContent()}`,
      }),
    })
    const zoomAfterOut = await zoomLabel.textContent()
    const zoomIn = page.locator('[data-cut-zoom-in]').first()
    await probe(page, {
      surface, name: 'zoom-in', actionId: 'zoom-in',
      sel: zoomIn, group: toolbar, groupName: 'timeline-toolbar',
      doClick: async () => { await zoomIn.click(); await sleep(120) },
      assertResult: async () => ({
        ok: (await zoomLabel.textContent()) !== zoomAfterOut,
        detail: `zoom ${zoomAfterOut} -> ${await zoomLabel.textContent()}`,
      }),
    })

    const addTrackControls = [
      { kind: 'video', actionId: 'add-video-track', selector: '[data-cut-action="add-video-track"]' },
      { kind: 'audio', actionId: 'add-audio-track', selector: '[data-cut-action="add-audio-track"]' },
    ]
    for (const { kind, actionId, selector } of addTrackControls) {
      const control = page.locator(selector).first()
      const before = kindCount(await state(), kind)
      let response = null
      await probe(page, {
        surface, name: actionId, actionId,
        sel: control,
        group: toolbar,
        groupName: 'timeline-toolbar',
        doClick: async () => {
          response = await captureVerbResp(
            page,
            'edit.add_track',
            () => control.click(),
            12_000,
          )
        },
        assertResult: async () => {
          const changed = await waitForState((project) => kindCount(project, kind) > before, 10_000)
          return {
            ok: !!response?.ok && !!changed,
            detail: `${kind} tracks ${before} -> ${kindCount(changed, kind)}`,
          }
        },
      })
    }

    let project = await state()
    const clipId = mediaClip(project, 'video')?.id
    if (!clipId) throw new Error('Timeline toolbar fixture has no video clip')
    await ensureClipSelected(page, clipId)

    const trimStart = page.locator('[data-cut-action="ripple-trim-start"]').first()
    const beforeStart = mediaClip(await state(), 'video')
    await setPlayhead(page, 1_000)
    let trimResponse = null
    await probe(page, {
      surface, name: 'ripple-trim-start', actionId: 'ripple-trim-start',
      sel: trimStart, group: toolbar, groupName: 'timeline-toolbar',
      doClick: async () => {
        trimResponse = await captureVerbResp(page, 'edit.trim', () => trimStart.click(), 12_000)
      },
      assertResult: async () => {
        const changed = await waitForState((next) => {
          const clip = mediaClip(next, 'video')
          return clip?.id === clipId && clip.src_in_ms > (beforeStart?.src_in_ms || 0)
        }, 10_000)
        return {
          ok: !!trimResponse?.ok && !!changed,
          detail: `src_in ${beforeStart?.src_in_ms ?? '?'} -> ${mediaClip(changed, 'video')?.src_in_ms ?? '?'}`,
        }
      },
    })

    await ensureClipSelected(page, clipId)
    const trimEnd = page.locator('[data-cut-action="ripple-trim-end"]').first()
    const beforeEnd = mediaClip(await state(), 'video')
    await setPlayhead(page, 3_000)
    trimResponse = null
    await probe(page, {
      surface, name: 'ripple-trim-end', actionId: 'ripple-trim-end',
      sel: trimEnd, group: toolbar, groupName: 'timeline-toolbar',
      doClick: async () => {
        trimResponse = await captureVerbResp(page, 'edit.trim', () => trimEnd.click(), 12_000)
      },
      assertResult: async () => {
        const changed = await waitForState((next) => {
          const clip = mediaClip(next, 'video')
          return clip?.id === clipId && clip.src_out_ms < (beforeEnd?.src_out_ms || Number.MAX_SAFE_INTEGER)
        }, 10_000)
        return {
          ok: !!trimResponse?.ok && !!changed,
          detail: `src_out ${beforeEnd?.src_out_ms ?? '?'} -> ${mediaClip(changed, 'video')?.src_out_ms ?? '?'}`,
        }
      },
    })

    await ensureClipSelected(page, clipId)
    const speedPreset = page.locator('[data-cut-speed-preset="2"]').first()
    let speedResponse = null
    await probe(page, {
      surface, name: 'speed-preset', actionId: 'speed-preset',
      sel: speedPreset, group: toolbar, groupName: 'timeline-toolbar',
      doClick: async () => {
        speedResponse = await captureVerbResp(page, 'edit.speed', () => speedPreset.click(), 12_000)
      },
      assertResult: async () => {
        const changed = await waitForState((next) => mediaClip(next, 'video')?.speed === 2, 10_000)
        return { ok: !!speedResponse?.ok && !!changed, detail: `clip speed=${mediaClip(changed, 'video')?.speed}` }
      },
    })

    const speedInput = page.locator('[data-cut-speed-input]').first()
    speedResponse = null
    await probe(page, {
      surface, name: 'speed-input', actionId: 'speed-input',
      sel: speedInput, group: toolbar, groupName: 'timeline-toolbar',
      doClick: async () => {
        await speedInput.fill('1.5')
        speedResponse = await captureVerbResp(page, 'edit.speed', () => speedInput.press('Enter'), 12_000)
      },
      assertResult: async () => {
        const changed = await waitForState((next) => mediaClip(next, 'video')?.speed === 1.5, 10_000)
        return { ok: !!speedResponse?.ok && !!changed, detail: `custom speed=${mediaClip(changed, 'video')?.speed}` }
      },
    })

    const saveRange = page.locator('[data-cut-action="save-range"]').first()
    let saveResponse = null
    const assetsBeforeRange = Object.keys((await state()).assets || {}).length
    await probe(page, {
      surface, name: 'save-range', actionId: 'save-range',
      sel: saveRange, group: toolbar, groupName: 'timeline-toolbar',
      doClick: async () => {
        saveResponse = await captureVerbResp(page, 'export.range', () => saveRange.click(), 120_000)
      },
      assertResult: async () => {
        const changed = await waitForState(
          (next) => Object.keys(next.assets || {}).length > assetsBeforeRange,
          15_000,
        )
        return {
          ok: !!saveResponse?.ok && !!saveResponse?.result?.path && !!changed,
          detail: `export.range path=${saveResponse?.result?.path || 'none'} assets ${assetsBeforeRange} -> ${Object.keys(changed?.assets || {}).length}`,
        }
      },
    })

    await ensureClipSelected(page, clipId)
    const saveGif = page.locator('[data-cut-action="save-gif"]').first()
    saveResponse = null
    const assetsBeforeGif = Object.keys((await state()).assets || {}).length
    await probe(page, {
      surface, name: 'save-gif', actionId: 'save-gif',
      sel: saveGif, group: toolbar, groupName: 'timeline-toolbar',
      doClick: async () => {
        saveResponse = await captureVerbResp(page, 'export.gif', () => saveGif.click(), 120_000)
      },
      assertResult: async () => {
        const changed = await waitForState(
          (next) => Object.keys(next.assets || {}).length > assetsBeforeGif,
          15_000,
        )
        return {
          ok: !!saveResponse?.ok && String(saveResponse?.result?.path || '').endsWith('.gif') && !!changed,
          detail: `export.gif path=${saveResponse?.result?.path || 'none'} assets ${assetsBeforeGif} -> ${Object.keys(changed?.assets || {}).length}`,
        }
      },
    })

    await ensureClipSelected(page, clipId)
    const openGrade = page.locator('[data-cut-action="open-grade"]').first()
    await probe(page, {
      surface, name: 'open-grade', actionId: 'open-grade',
      sel: openGrade, group: toolbar, groupName: 'timeline-toolbar',
      doClick: async () => {
        await openGrade.click()
        // Native WebViews can finish the preceding GIF import after the Grade
        // panel is already mounted. Keep this bounded but allow that state
        // refresh to settle instead of recording a false click failure.
        await page.locator('[data-cut-grade-embed]').waitFor({ state: 'visible', timeout: 20_000 })
      },
      assertResult: async () => ({
        ok: (await page.locator('[data-cut-grade-embed]').count()) === 1,
        detail: 'Color grade surface opened for the selected video',
      }),
    })

    project = await state()
    return {
      videoTrackIds: project.tracks.filter((track) => track.kind === 'video').map((track) => track.id),
      audioTrackIds: project.tracks.filter((track) => track.kind === 'audio').map((track) => track.id),
      panel,
    }
  }

  return { run }
}
