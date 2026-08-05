// Direct native coverage for per-track header actions.

export function createTimelineTrackActionCoverage({
  probe,
  state,
  waitForState,
  captureVerbResp,
  sleep,
  closeOverlays,
}) {
  const trackOf = (project, id) => project?.tracks?.find((track) => track.id === id)
  const sameKindIndex = (project, id) => {
    const target = trackOf(project, id)
    return target
      ? project.tracks.filter((track) => track.kind === target.kind).findIndex((track) => track.id === id)
      : -1
  }

  async function run(page, { videoTrackIds, audioTrackIds }) {
    await closeOverlays(page)
    const surface = 'timeline-actions'
    const panel = page.locator('[data-cut-panel="timeline"]').first()
    const baseVideo = videoTrackIds[0]
    const addedVideo = videoTrackIds.at(-1)
    const baseAudio = audioTrackIds[0]
    if (!baseVideo || !addedVideo || baseVideo === addedVideo || !baseAudio) {
      throw new Error(`Timeline track fixtures incomplete: video=${videoTrackIds} audio=${audioTrackIds}`)
    }

    const visibility = page.locator(
      `[data-cut-visibility-track="${baseVideo}"][data-cut-action="toggle-track-visibility"]`,
    ).first()
    let response = null
    await probe(page, {
      surface, name: 'toggle-track-visibility', actionId: 'toggle-track-visibility',
      sel: visibility, group: panel, groupName: 'timeline-track-headers',
      doClick: async () => {
        response = await captureVerbResp(page, 'edit.track_visible', () => visibility.click(), 12_000)
      },
      assertResult: async () => {
        const changed = await waitForState((project) => trackOf(project, baseVideo)?.visible === false, 10_000)
        return { ok: !!response?.ok && !!changed, detail: `${baseVideo} visible=false` }
      },
    })
    await visibility.click()
    await waitForState((project) => trackOf(project, baseVideo)?.visible !== false, 10_000)

    const lock = page.locator(
      `[data-cut-lock-track="${baseVideo}"][data-cut-action="toggle-track-lock"]`,
    ).first()
    response = null
    await probe(page, {
      surface, name: 'toggle-track-lock', actionId: 'toggle-track-lock',
      sel: lock, group: panel, groupName: 'timeline-track-headers',
      doClick: async () => {
        response = await captureVerbResp(page, 'edit.track_lock', () => lock.click(), 12_000)
      },
      assertResult: async () => {
        const changed = await waitForState((project) => trackOf(project, baseVideo)?.locked === true, 10_000)
        return { ok: !!response?.ok && !!changed, detail: `${baseVideo} locked=true` }
      },
    })
    await lock.click()
    await waitForState((project) => trackOf(project, baseVideo)?.locked !== true, 10_000)

    const sendBack = page.locator(
      `[data-cut-track-order="${addedVideo}"] [data-cut-action="track-send-back"]`,
    ).first()
    const beforeBack = sameKindIndex(await state(), addedVideo)
    response = null
    await probe(page, {
      surface, name: 'track-send-back', actionId: 'track-send-back',
      sel: sendBack, group: panel, groupName: 'timeline-track-headers',
      doClick: async () => {
        response = await captureVerbResp(page, 'edit.reorder_track', () => sendBack.click(), 12_000)
      },
      assertResult: async () => {
        const changed = await waitForState(
          (project) => sameKindIndex(project, addedVideo) < beforeBack,
          10_000,
        )
        return {
          ok: !!response?.ok && !!changed,
          detail: `${addedVideo} index ${beforeBack} -> ${sameKindIndex(changed, addedVideo)}`,
        }
      },
    })

    const bringForward = page.locator(
      `[data-cut-track-order="${addedVideo}"] [data-cut-action="track-bring-forward"]`,
    ).first()
    const beforeForward = sameKindIndex(await state(), addedVideo)
    response = null
    await probe(page, {
      surface, name: 'track-bring-forward', actionId: 'track-bring-forward',
      sel: bringForward, group: panel, groupName: 'timeline-track-headers',
      doClick: async () => {
        response = await captureVerbResp(page, 'edit.reorder_track', () => bringForward.click(), 12_000)
      },
      assertResult: async () => {
        const changed = await waitForState(
          (project) => sameKindIndex(project, addedVideo) > beforeForward,
          10_000,
        )
        return {
          ok: !!response?.ok && !!changed,
          detail: `${addedVideo} index ${beforeForward} -> ${sameKindIndex(changed, addedVideo)}`,
        }
      },
    })

    const gain = page.locator(
      `[data-cut-gain-track="${baseAudio}"][data-cut-action="set-gain"]`,
    ).first()
    response = null
    await probe(page, {
      surface, name: 'set-gain', actionId: 'set-gain',
      sel: gain, group: panel, groupName: 'timeline-track-headers',
      doClick: async () => {
        await gain.fill('-3.5')
        response = await captureVerbResp(page, 'edit.gain', () => gain.press('Enter'), 12_000)
      },
      assertResult: async () => {
        const changed = await waitForState((project) => trackOf(project, baseAudio)?.gain_db === -3.5, 10_000)
        return { ok: !!response?.ok && !!changed, detail: `${baseAudio} gain=-3.5 dB` }
      },
    })

    const pan = page.locator(
      `[data-cut-pan-track="${baseAudio}"][data-cut-action="set-pan"]`,
    ).first()
    const panBefore = Number(trackOf(await state(), baseAudio)?.pan ?? 0)
    const panTarget = Math.abs(panBefore - 0.5) < 0.01 ? -0.5 : 0.5
    response = null
    await probe(page, {
      surface, name: 'set-pan', actionId: 'set-pan',
      sel: pan, group: panel, groupName: 'timeline-track-headers',
      doClick: async () => {
        response = await captureVerbResp(page, 'edit.pan', () => pan.selectOption(String(panTarget)), 12_000)
      },
      assertResult: async () => {
        const changed = await waitForState(
          (project) => Math.abs(Number(trackOf(project, baseAudio)?.pan ?? 0) - panTarget) < 0.01,
          10_000,
        )
        return {
          ok: !!response?.ok && !!changed,
          detail: `${baseAudio} pan ${panBefore} -> ${panTarget}; edit.pan ok=${response?.ok}`,
        }
      },
    })

    const mute = page.locator(
      `[data-cut-mute-track="${baseAudio}"][data-cut-action="toggle-mute"]`,
    ).first()
    response = null
    await probe(page, {
      surface, name: 'toggle-mute', actionId: 'toggle-mute',
      sel: mute, group: panel, groupName: 'timeline-track-headers',
      doClick: async () => {
        response = await captureVerbResp(page, 'edit.mute', () => mute.click(), 12_000)
      },
      assertResult: async () => {
        const changed = await waitForState((project) => trackOf(project, baseAudio)?.muted === true, 10_000)
        return { ok: !!response?.ok && !!changed, detail: `${baseAudio} muted=true` }
      },
    })

    const solo = page.locator(
      `[data-cut-solo-track="${baseAudio}"][data-cut-action="toggle-solo"]`,
    ).first()
    response = null
    await probe(page, {
      surface, name: 'toggle-solo', actionId: 'toggle-solo',
      sel: solo, group: panel, groupName: 'timeline-track-headers',
      doClick: async () => {
        response = await captureVerbResp(page, 'edit.solo', () => solo.click(), 12_000)
      },
      assertResult: async () => {
        const changed = await waitForState((project) => trackOf(project, baseAudio)?.solo === true, 10_000)
        return { ok: !!response?.ok && !!changed, detail: `${baseAudio} solo=true` }
      },
    })
    await sleep(120)
  }

  return { run }
}
