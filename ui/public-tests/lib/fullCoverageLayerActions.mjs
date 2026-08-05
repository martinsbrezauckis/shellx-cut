// Direct native coverage for every interactive control in the Layer / PiP
// drawer. Local fields are proven individually; durable actions additionally
// require the exact REST response and matching project-state mutation.

export function createLayerActionCoverage({
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
  const surface = 'layer-actions'

  function findClip(project, clipId) {
    for (const track of project?.tracks || []) {
      const clip = (track.clips || []).find((candidate) => candidate.id === clipId)
      if (clip) return clip
    }
    return null
  }

  async function openLayer(page, tag) {
    await freshProject(page, tag)
    await closeOverlays(page)
    const project = await state()
    const clip = project.tracks
      .find((track) => track.kind === 'video')
      ?.clips?.find((candidate) => candidate.asset)
    if (!clip?.id) throw new Error(`Layer fixture ${tag} has no video clip`)
    const clipControl = page.locator(`[data-cut-clip="${clip.id}"]`).first()
    await clipControl.waitFor({
      state: 'visible',
      timeout: 12_000,
    })
    // macOS WebDriver can synthesize click without the React mousedown that
    // owns timeline selection. Right-click is itself a public selection route:
    // the delegated contextmenu handler selects the clip before opening its
    // menu. Its visible Transform command is the canonical Layer entry point.
    await clipControl.dispatchEvent('contextmenu').catch(() => {})
    await sleep(180)
    const transformMenuItem = page.locator('[data-cut-ctx="transform"]').first()
    if (await transformMenuItem.count()) {
      // WebDriver's pointer click can be intercepted while the fixed context
      // menu settles against a narrow/native viewport. Dispatch the element's
      // real click handler directly, matching the proven fit-to-fill route.
      await transformMenuItem.evaluate((element) => element.click())
    } else {
      if (!(await selectClip(page, clip.id))) {
        const selected = await verb('ui.select', { clip_ids: [clip.id] })
        await sleep(180)
        const ui = await verb('ui.state', {})
        if (!selected.ok || !ui.result?.selected_clip_ids?.includes(clip.id)) {
          throw new Error(
            `Layer fixture ${tag} could not select ${clip.id}: ` +
            `${selected.error?.message || selected.error?.code || 'selection not confirmed'}`,
          )
        }
      }
      await page.locator('[data-cut-action="open-layer"]').first().click()
    }
    const panel = page.locator('[data-cut-layer]').first()
    await panel.waitFor({ state: 'visible', timeout: 12_000 })
    await sleep(250)
    return { panel, clipId: clip.id }
  }

  async function probeInput(page, panel, attr, value) {
    const control = page.locator(`[data-cut-layer-input="${attr}"]`).first()
    await probe(page, {
      surface,
      name: `layer-input-${attr}`,
      actionId: 'layer-input',
      sel: control,
      group: panel,
      groupName: 'layer-fields',
      doClick: async () => {
        await control.fill(String(value))
        await sleep(80)
      },
      assertResult: async () => {
        const actual = Number(await control.inputValue())
        return {
          ok: Number.isFinite(actual) && Math.abs(actual - Number(value)) < 0.001,
          detail: `${attr} local value=${actual}`,
        }
      },
    })
  }

  async function runTransformCropMotion(page) {
    const { panel, clipId } = await openLayer(page, 'layer-core')
    const cropW = page.locator('[data-cut-layer-input="crop_w"]').first()
    const cropH = page.locator('[data-cut-layer-input="crop_h"]').first()
    const maxCropW = Number(await cropW.getAttribute('max'))
    const maxCropH = Number(await cropH.getAttribute('max'))
    if (!Number.isFinite(maxCropW) || !Number.isFinite(maxCropH)) {
      throw new Error('Layer crop controls have no probed source dimensions')
    }

    const fields = [
      ['scale', 0.8],
      ['x', 0.2],
      ['y', 0.3],
      ['opacity', 0.7],
      ['crop_x', Math.min(10, Math.max(0, maxCropW - 2))],
      ['crop_y', Math.min(10, Math.max(0, maxCropH - 2))],
      ['crop_w', Math.max(1, maxCropW - 12)],
      ['crop_h', Math.max(1, maxCropH - 12)],
      ['freeze_at', 400],
      ['kenburns_amount', 0.5],
      ['slide_ms', 650],
      ['kf_time', 500],
      ['kf_value', 0.75],
    ]
    for (const [attr, value] of fields) await probeInput(page, panel, attr, value)

    const reset = page.locator('[data-cut-layer-reset]').first()
    await probe(page, {
      surface, name: 'layer-reset', actionId: 'layer-reset',
      sel: reset, group: panel, groupName: 'layer-transform',
      doClick: async () => { await reset.click(); await sleep(80) },
      assertResult: async () => {
        const values = {}
        for (const attr of ['scale', 'x', 'y', 'opacity']) {
          values[attr] = Number(await page.locator(`[data-cut-layer-input="${attr}"]`).inputValue())
        }
        return {
          ok: values.scale === 1 && values.x === 0 && values.y === 0 && values.opacity === 1,
          detail: `identity=${JSON.stringify(values)}`,
        }
      },
    })

    await page.locator('[data-cut-layer-input="scale"]').fill('0.75')
    await page.locator('[data-cut-layer-input="x"]').fill('0.25')
    await page.locator('[data-cut-layer-input="y"]').fill('0.15')
    await page.locator('[data-cut-layer-input="opacity"]').fill('0.65')
    const apply = page.locator('[data-cut-layer-apply]').first()
    let transformResponse = null
    await probe(page, {
      surface, name: 'layer-apply', actionId: 'layer-apply',
      sel: apply, group: panel, groupName: 'layer-transform',
      doClick: async () => {
        transformResponse = await captureVerbResp(page, 'edit.transform', () => apply.click(), 12_000)
      },
      assertResult: async () => {
        const changed = await waitForState((project) => {
          const transform = findClip(project, clipId)?.transform
          return transform?.scale === 0.75
            && transform?.x === 0.25
            && transform?.y === 0.15
            && transform?.opacity === 0.65
        }, 10_000)
        return {
          ok: !!transformResponse?.ok && !!changed,
          detail: `edit.transform ok=${transformResponse?.ok}; transform=${JSON.stringify(findClip(changed, clipId)?.transform || null)}`,
        }
      },
    })

    await page.locator('[data-cut-layer-input="crop_x"]').fill('10')
    await page.locator('[data-cut-layer-input="crop_y"]').fill('8')
    await cropW.fill(String(Math.max(1, maxCropW - 20)))
    await cropH.fill(String(Math.max(1, maxCropH - 18)))
    const cropReset = page.locator('[data-cut-layer-crop-reset]').first()
    await probe(page, {
      surface, name: 'layer-crop-reset', actionId: 'layer-crop-reset',
      sel: cropReset, group: panel, groupName: 'layer-crop',
      doClick: async () => { await cropReset.click(); await sleep(80) },
      assertResult: async () => {
        const values = {
          x: Number(await page.locator('[data-cut-layer-input="crop_x"]').inputValue()),
          y: Number(await page.locator('[data-cut-layer-input="crop_y"]').inputValue()),
          w: Number(await cropW.inputValue()),
          h: Number(await cropH.inputValue()),
        }
        return {
          ok: values.x === 0 && values.y === 0 && values.w === maxCropW && values.h === maxCropH,
          detail: `whole-frame crop=${JSON.stringify(values)}`,
        }
      },
    })

    const cropX = Math.min(10, Math.max(0, maxCropW - 2))
    const cropY = Math.min(8, Math.max(0, maxCropH - 2))
    const croppedW = Math.max(1, maxCropW - cropX - 8)
    const croppedH = Math.max(1, maxCropH - cropY - 8)
    await page.locator('[data-cut-layer-input="crop_x"]').fill(String(cropX))
    await page.locator('[data-cut-layer-input="crop_y"]').fill(String(cropY))
    await cropW.fill(String(croppedW))
    await cropH.fill(String(croppedH))
    const cropApply = page.locator('[data-cut-layer-crop-apply]').first()
    let cropResponse = null
    await probe(page, {
      surface, name: 'layer-crop-apply', actionId: 'layer-crop-apply',
      sel: cropApply, group: panel, groupName: 'layer-crop',
      doClick: async () => {
        cropResponse = await captureVerbResp(page, 'edit.crop', () => cropApply.click(), 12_000)
      },
      assertResult: async () => {
        const changed = await waitForState((project) => {
          const crop = findClip(project, clipId)?.crop
          return crop?.x === cropX && crop?.y === cropY && crop?.w === croppedW && crop?.h === croppedH
        }, 10_000)
        return {
          ok: !!cropResponse?.ok && !!changed,
          detail: `edit.crop ok=${cropResponse?.ok}; crop=${JSON.stringify(findClip(changed, clipId)?.crop || null)}`,
        }
      },
    })

    const reverse = page.locator('[data-cut-layer-reverse]').first()
    let reverseResponse = null
    await probe(page, {
      surface, name: 'layer-reverse', actionId: 'layer-reverse',
      sel: reverse, group: panel, groupName: 'layer-motion',
      doClick: async () => {
        reverseResponse = await captureVerbResp(page, 'edit.reverse', () => reverse.click(), 12_000)
      },
      assertResult: async () => {
        const changed = await waitForState((project) => findClip(project, clipId)?.reverse === true, 10_000)
        return { ok: !!reverseResponse?.ok && !!changed, detail: `edit.reverse ok=${reverseResponse?.ok}; reverse=true` }
      },
    })

    const freeze = page.locator('[data-cut-layer-freeze]').first()
    let freezeResponse = null
    await probe(page, {
      surface, name: 'layer-freeze', actionId: 'layer-freeze',
      sel: freeze, group: panel, groupName: 'layer-motion',
      doClick: async () => {
        freezeResponse = await captureVerbResp(page, 'edit.freeze', () => freeze.click(), 12_000)
      },
      assertResult: async () => {
        const changed = await waitForState((project) => !!findClip(project, clipId)?.freeze, 10_000)
        return {
          ok: !!freezeResponse?.ok && !!changed,
          detail: `edit.freeze ok=${freezeResponse?.ok}; freeze=${JSON.stringify(findClip(changed, clipId)?.freeze || null)}`,
        }
      },
    })

    const beforeTracks = (await state()).tracks.filter((track) => track.kind === 'video').length
    const add = page.locator('[data-cut-layer-add]').first()
    let addResponse = null
    await probe(page, {
      surface, name: 'layer-add', actionId: 'layer-add',
      sel: add, group: panel, groupName: 'layer-stacking',
      doClick: async () => {
        addResponse = await captureVerbResp(page, 'edit.add_track', () => add.click(), 12_000)
      },
      assertResult: async () => {
        const changed = await waitForState(
          (project) => project.tracks.filter((track) => track.kind === 'video').length === beforeTracks + 1,
          10_000,
        )
        return {
          ok: !!addResponse?.ok && !!changed,
          detail: `video tracks ${beforeTracks} -> ${changed?.tracks?.filter((track) => track.kind === 'video').length ?? '?'}`,
        }
      },
    })
  }

  async function runKenBurnsClear(page) {
    const { panel, clipId } = await openLayer(page, 'layer-kenburns')
    const apply = page.locator('[data-cut-layer-kenburns-apply]').first()
    const seeded = await captureVerbResp(page, 'edit.animate', () => apply.click(), 12_000)
    const animated = await waitForState((project) => !!findClip(project, clipId)?.animation, 10_000)
    if (!seeded?.ok || !animated) throw new Error('Layer Ken Burns setup did not persist an animation')
    const clear = page.locator('[data-cut-layer-kenburns-clear]').first()
    let clearResponse = null
    await probe(page, {
      surface, name: 'layer-kenburns-clear', actionId: 'layer-kenburns-clear',
      sel: clear, group: panel, groupName: 'layer-kenburns',
      doClick: async () => {
        clearResponse = await captureVerbResp(page, 'edit.animate', () => clear.click(), 12_000)
      },
      assertResult: async () => {
        const changed = await waitForState((project) => !findClip(project, clipId)?.animation, 10_000)
        return { ok: !!clearResponse?.ok && !!changed, detail: `edit.animate enabled:false ok=${clearResponse?.ok}` }
      },
    })
  }

  async function runSlide(page) {
    const { panel, clipId } = await openLayer(page, 'layer-slide-actions')
    const edge = page.locator('[data-cut-layer-slide-edge]').first()
    await probe(page, {
      surface, name: 'layer-slide-edge', actionId: 'layer-slide-edge',
      sel: edge, group: panel, groupName: 'layer-slide',
      doClick: async () => { await edge.selectOption('right'); await sleep(80) },
      assertResult: async () => ({ ok: (await edge.inputValue()) === 'right', detail: 'edge=right' }),
    })
    const mode = page.locator('[data-cut-layer-slide-mode]').first()
    await probe(page, {
      surface, name: 'layer-slide-mode', actionId: 'layer-slide-mode',
      sel: mode, group: panel, groupName: 'layer-slide',
      doClick: async () => { await mode.selectOption('out'); await sleep(80) },
      assertResult: async () => ({ ok: (await mode.inputValue()) === 'out', detail: 'mode=out' }),
    })
    const apply = page.locator('[data-cut-action="edit-slide"]').first()
    let slideResponse = null
    await probe(page, {
      surface, name: 'edit-slide', actionId: 'edit-slide',
      sel: apply, group: panel, groupName: 'layer-slide',
      doClick: async () => {
        slideResponse = await captureVerbResp(page, 'edit.slide', () => apply.click(), 12_000)
      },
      assertResult: async () => {
        const changed = await waitForState((project) => (
          (findClip(project, clipId)?.keyframes || []).some((track) => (
            (track.param === 'pos_x' || track.param === 'pos_y') && (track.points || []).length >= 2
          ))
        ), 10_000)
        return { ok: !!slideResponse?.ok && !!changed, detail: `edit.slide ok=${slideResponse?.ok}; position keyframes persisted` }
      },
    })
  }

  async function seedKeyframe(page, clipId) {
    const add = page.locator('[data-cut-layer-kf-add]').first()
    const response = await captureVerbResp(page, 'edit.keyframe', () => add.click(), 12_000)
    const changed = await waitForState((project) => (
      (findClip(project, clipId)?.keyframes || []).some((track) => track.param === 'pos_x' && (track.points || []).length > 0)
    ), 10_000)
    if (!response?.ok || !changed) throw new Error('Layer keyframe setup did not persist a pos_x point')
  }

  async function runKeyframes(page) {
    const { panel, clipId } = await openLayer(page, 'layer-keyframe-actions')
    const param = page.locator('[data-cut-layer-kf-param]').first()
    await probe(page, {
      surface, name: 'layer-kf-param', actionId: 'layer-kf-param',
      sel: param, group: panel, groupName: 'layer-keyframes',
      doClick: async () => { await param.selectOption('pos_x'); await sleep(100) },
      assertResult: async () => ({ ok: (await param.inputValue()) === 'pos_x', detail: 'keyframe parameter=pos_x' }),
    })
    await seedKeyframe(page, clipId)

    const interp = page.locator('[data-cut-layer-kf-interp]').first()
    let interpResponse = null
    await probe(page, {
      surface, name: 'layer-kf-interp', actionId: 'layer-kf-interp',
      sel: interp, group: panel, groupName: 'layer-keyframes',
      doClick: async () => {
        interpResponse = await captureVerbResp(
          page,
          'edit.keyframe',
          () => interp.selectOption('ease_in_out_cubic'),
          12_000,
        )
      },
      assertResult: async () => {
        const changed = await waitForState((project) => (
          findClip(project, clipId)?.keyframes?.find((track) => track.param === 'pos_x')?.interp === 'ease_in_out_cubic'
        ), 10_000)
        return { ok: !!interpResponse?.ok && !!changed, detail: `interp=${findClip(changed, clipId)?.keyframes?.find((track) => track.param === 'pos_x')?.interp}` }
      },
    })

    const remove = page.locator('[data-cut-layer-kf-remove]').first()
    let removeResponse = null
    await probe(page, {
      surface, name: 'layer-kf-remove', actionId: 'layer-kf-remove',
      sel: remove, group: panel, groupName: 'layer-keyframes',
      doClick: async () => {
        removeResponse = await captureVerbResp(page, 'edit.keyframe', () => remove.click(), 12_000)
      },
      assertResult: async () => {
        const changed = await waitForState((project) => (
          !(findClip(project, clipId)?.keyframes || []).some((track) => track.param === 'pos_x' && (track.points || []).length)
        ), 10_000)
        return { ok: !!removeResponse?.ok && !!changed, detail: `remove point ok=${removeResponse?.ok}` }
      },
    })

    await seedKeyframe(page, clipId)
    const clear = page.locator('[data-cut-layer-kf-clear]').first()
    let clearResponse = null
    await probe(page, {
      surface, name: 'layer-kf-clear', actionId: 'layer-kf-clear',
      sel: clear, group: panel, groupName: 'layer-keyframes',
      doClick: async () => {
        clearResponse = await captureVerbResp(page, 'edit.keyframe', () => clear.click(), 12_000)
      },
      assertResult: async () => {
        const changed = await waitForState((project) => (
          !(findClip(project, clipId)?.keyframes || []).some((track) => track.param === 'pos_x' && (track.points || []).length)
        ), 10_000)
        return { ok: !!clearResponse?.ok && !!changed, detail: `clear keyframes ok=${clearResponse?.ok}` }
      },
    })
  }

  async function run(page) {
    await runTransformCropMotion(page)
    await runKenBurnsClear(page)
    await runSlide(page)
    await runKeyframes(page)
  }

  return { run }
}
