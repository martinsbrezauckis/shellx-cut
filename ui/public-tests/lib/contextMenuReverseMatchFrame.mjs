function sourceTimeLabel(ms) {
  const total = Math.max(0, Math.round(ms))
  const minutes = Math.floor(total / 60_000)
  const seconds = Math.floor((total % 60_000) / 1000)
  const millis = total % 1000
  return `${minutes}:${String(seconds).padStart(2, '0')}.${String(millis).padStart(3, '0')}`
}

function clipById(project, clipId) {
  return project?.tracks?.flatMap((track) => track.clips ?? []).find((clip) => clip.id === clipId) ?? null
}

export function isReverseMatchFrameFixtureRestored(reverseDisabled, clip, uiState) {
  // `reverse: false` is intentionally omitted from serialized clips for compatibility.
  return reverseDisabled && clip?.reverse !== true && uiState?.playhead_ms === 0
}

export function isReverseMatchFrameFixtureAnchored(anchored, uiState) {
  const applied = anchored?.ok === true
  const alreadyAtStart = anchored?.error?.code === 'conflict'
  return (applied || alreadyAtStart) && uiState?.ok === true && uiState.result?.playhead_ms === 0
}

/**
 * Keep the reverse Match Frame boundary proof in the maintained context-menu
 * browser verifier without allowing its temporary edit to leak into later rows.
 */
export async function verifyReverseMatchFrameAtStart({ page, verb, state, check, openRightClick, videoClip, assetId }) {
  const sourceInMs = Number(videoClip.src_in_ms)
  const sourceOutMs = Number(videoClip.src_out_ms)
  if (!Number.isFinite(sourceInMs) || !Number.isFinite(sourceOutMs) || sourceOutMs <= sourceInMs) {
    throw new Error('reverse Match Frame fixture lacks a positive source window')
  }
  const expectedSourceMs = sourceOutMs - 1
  let reversed = false
  const source = page.locator(`[data-cut-source-monitor="${assetId}"]`)

  try {
    const enabled = await verb('edit.reverse', {
      clip: videoClip.id,
      enabled: true,
      rationale: 'context-menu browser fixture reverse Match Frame boundary',
    })
    if (!enabled.ok) throw new Error(`edit.reverse fixture setup failed: ${enabled.error?.message ?? 'unknown'}`)
    reversed = true
    const anchored = await verb('ui.playhead', { at_ms: 0 })
    const view = await verb('ui.state', {})
    if (!isReverseMatchFrameFixtureAnchored(anchored, view)) {
      throw new Error(`reverse Match Frame could not anchor the first timeline instant: ${JSON.stringify({ anchored, view })}`)
    }
    await page.reload({ waitUntil: 'domcontentloaded' })
    await page.locator('[data-cut-mode="edit"]').first().waitFor({ state: 'visible', timeout: 15_000 })

    const clip = page.locator(`[data-cut-clip="${videoClip.id}"]`).first()
    const menu = await openRightClick(clip, '[data-cut-clip-menu]')
    const matchFrame = menu.locator('[data-cut-action="match-frame"]')
    check('Reversed footage clip exposes Match Frame at its first timeline instant',
      await matchFrame.count() === 1 && !(await matchFrame.isDisabled()))
    await matchFrame.focus()
    await page.keyboard.press('Enter')
    await source.waitFor({ state: 'visible', timeout: 5_000 })
    await page.waitForFunction((expectedAsset) => {
      const media = document.querySelector(`[data-cut-source-monitor="${expectedAsset}"] video`)
      return media instanceof HTMLVideoElement && Number.isFinite(media.duration) && media.duration > 0
    }, assetId)
    const current = await source.locator('[data-cut-source-current]').textContent()
    check('Reverse Match Frame opens the final included source millisecond',
      current === sourceTimeLabel(expectedSourceMs),
      `source=${current ?? 'none'} expected=${sourceTimeLabel(expectedSourceMs)}`)
  } finally {
    const close = source.locator('[data-cut-source-monitor-close]')
    if (await close.count()) await close.click()
    await source.waitFor({ state: 'detached', timeout: 5_000 }).catch(() => {})

    if (reversed) {
      const disabled = await verb('edit.reverse', {
        clip: videoClip.id,
        enabled: false,
        rationale: 'restore context-menu reverse Match Frame fixture',
      })
      const restored = clipById(await state(), videoClip.id)
      const view = await verb('ui.state', {})
      const restoredOk = isReverseMatchFrameFixtureRestored(disabled.ok, restored, view.result)
      check('Reverse Match Frame fixture restores the normal clip before later rows', restoredOk,
        `reverse=${String(restored?.reverse)} playhead=${String(view.result?.playhead_ms)}`)
      if (!restoredOk) throw new Error('reverse Match Frame fixture did not restore its normal clip state')
    }
    await page.reload({ waitUntil: 'domcontentloaded' })
    await page.locator('[data-cut-mode="edit"]').first().waitFor({ state: 'visible', timeout: 15_000 })
  }
}
