// Native action coverage for the AI-media Generate drawer and generated-media
// history. Safe form/navigation actions run everywhere. The multi-take compare
// and cancellable-job path runs only with the deterministic release fixture so
// exhaustive coverage never spends provider credits.

export function createGeneratedMediaActionCoverage({
  probe,
  rec,
  state,
  waitForState,
  captureVerbResp,
  awaitJob,
  sleep,
  fixtureActive,
}) {
  const surface = 'assets'

  async function ensureGenerateOpen(page) {
    const generate = page.locator('[data-cut-generate]').first()
    if (await generate.isVisible().catch(() => false)) return
    const assetsTab = page.locator('[data-cut-left-tab="assets"]').first()
    await assetsTab.waitFor({ state: 'visible', timeout: 12_000 })
    await assetsTab.click()
    const open = page.locator('[data-cut-action="generate-asset"]').first()
    await open.waitFor({ state: 'visible', timeout: 12_000 })
    await open.click()
    await generate.waitFor({ state: 'visible', timeout: 12_000 })
  }

  async function clickPlacement(page, mode) {
    // Selecting a timeline clip for generated-media replacement can move the
    // active left workspace away from Assets. Re-enter the drawer before the
    // next lifecycle step instead of clicking a hidden, still-mounted control.
    await ensureGenerateOpen(page)
    const control = page.locator(`[data-cut-generate-placement-mode="${mode}"]`).first()
    await control.waitFor({ state: 'visible', timeout: 12_000 })
    await control.click()
    await page.locator(`[data-cut-generate-placement-mode="${mode}"][aria-selected="true"]`)
      .waitFor({ state: 'visible', timeout: 8_000 })
  }

  async function runStaticControls(page, gen) {
    const references = page.locator('[data-cut-generate-references]').first()
    const referencesToggle = page.locator('[data-cut-generate-references-toggle]').first()
    await probe(page, {
      surface,
      name: 'generate-references-toggle',
      actionId: 'generate-references-toggle',
      sel: referencesToggle,
      group: gen,
      groupName: 'generate-surface',
      doClick: async () => { await referencesToggle.click() },
      assertResult: async () => ({
        ok: await references.getAttribute('open') !== null,
        detail: `open=${await references.getAttribute('open') !== null}`,
      }),
    })

    const reference = page.locator('[data-cut-generate-reference-toggle]').first()
    let selected = false
    await probe(page, {
      surface,
      name: 'generate-reference-toggle',
      actionId: 'generate-reference-toggle',
      sel: reference,
      group: gen,
      groupName: 'generate-surface',
      doClick: async () => {
        await reference.click()
        selected = await reference.isChecked()
        await reference.click()
      },
      assertResult: async () => ({
        ok: selected
          && !await reference.isChecked()
          && await page.locator('[data-cut-generate-reference-count]').textContent() === '0/4',
        detail: `selectedOnce=${selected} finalChecked=${await reference.isChecked()} count=${await page.locator('[data-cut-generate-reference-count]').textContent()}`,
      }),
    })

    const advanced = page.locator('[data-cut-generate-advanced]').first()
    const advancedToggle = page.locator('[data-cut-generate-advanced-toggle]').first()
    await probe(page, {
      surface,
      name: 'generate-advanced-toggle',
      actionId: 'generate-advanced-toggle',
      sel: advancedToggle,
      group: gen,
      groupName: 'generate-surface',
      doClick: async () => { await advancedToggle.click() },
      assertResult: async () => ({
        ok: await advanced.getAttribute('open') !== null,
        detail: `open=${await advanced.getAttribute('open') !== null}`,
      }),
    })

    const model = page.locator('[data-cut-generate-model]').first()
    await probe(page, {
      surface,
      name: 'generate-model',
      actionId: 'generate-model',
      sel: model,
      group: gen,
      groupName: 'generate-surface',
      doClick: async () => { await model.fill('fixture/model-override') },
      assertResult: async () => ({
        ok: await model.inputValue() === 'fixture/model-override',
        detail: `value=${await model.inputValue()}`,
      }),
    })
    await model.fill('')

    await page.locator('[data-cut-generate-provider]').selectOption('grok')
    for (const kind of ['video', 'image']) {
      const control = page.locator(`[data-cut-generate-kind-opt="${kind}"]`).first()
      await probe(page, {
        surface,
        name: `generate-kind-${kind}`,
        actionId: 'generate-kind-opt',
        sel: control,
        group: gen,
        groupName: 'generate-surface',
        doClick: async () => { await control.click() },
        assertResult: async () => ({
          ok: await control.getAttribute('aria-selected') === 'true',
          detail: `selected=${await control.getAttribute('aria-selected')}`,
        }),
      })
    }
    await page.locator('[data-cut-generate-provider]').selectOption('codex')

    await clickPlacement(page, 'insert')
    const track = page.locator('[data-cut-generate-placement-track]').first()
    const trackValues = await track.locator('option:not([disabled])').evaluateAll((options) =>
      options.map((option) => option.value),
    )
    const targetTrack = trackValues.at(-1) || ''
    await probe(page, {
      surface,
      name: 'generate-placement-track',
      actionId: 'generate-placement-track',
      sel: track,
      group: gen,
      groupName: 'generate-surface',
      doClick: async () => { await track.selectOption(targetTrack) },
      assertResult: async () => ({
        ok: targetTrack.length > 0 && await track.inputValue() === targetTrack,
        detail: `value=${await track.inputValue()} choices=${trackValues.join(',')}`,
      }),
    })

    const duration = page.locator('[data-cut-generate-placement-duration]').first()
    await probe(page, {
      surface,
      name: 'generate-placement-duration',
      actionId: 'generate-placement-duration',
      sel: duration,
      group: gen,
      groupName: 'generate-surface',
      doClick: async () => { await duration.fill('4.2') },
      assertResult: async () => ({
        ok: await duration.inputValue() === '4.2',
        detail: `seconds=${await duration.inputValue()}`,
      }),
    })
    await clickPlacement(page, 'asset')

    await page.locator('[data-cut-generate-prompt]').fill('safe arm cancellation check')
    await page.locator('[data-cut-generate-run]').click()
    await page.locator('[data-cut-generate-run][data-cut-generate-armed]').waitFor()
    const armCancel = page.locator('[data-cut-generate-cancel]').first()
    await probe(page, {
      surface,
      name: 'generate-cancel-armed',
      actionId: 'generate-cancel',
      sel: armCancel,
      group: gen,
      groupName: 'generate-surface',
      doClick: async () => { await armCancel.click() },
      assertResult: async () => ({
        ok: await page.locator('[data-cut-generate-run][data-cut-generate-armed]').count() === 0,
        detail: `armed=${await page.locator('[data-cut-generate-run][data-cut-generate-armed]').count()}`,
      }),
    })

  }

  async function generateCurrent(page) {
    const run = page.locator('[data-cut-generate-run]').first()
    await run.click()
    await page.locator('[data-cut-generate-run][data-cut-generate-armed]').waitFor()
    const queued = await captureVerbResp(page, 'assets.generate', () => run.click(), 90_000)
    const jobId = queued?.result?.job_id
    const terminal = queued?.ok && jobId ? await awaitJob(jobId, 90_000) : null
    return { queued, jobId, terminal }
  }

  async function waitForHistoryItem(page, assetId) {
    const item = page.locator(`[data-cut-generated-asset="${assetId}"]`).first()
    try {
      await item.waitFor({ state: 'visible', timeout: 8_000 })
    } catch {
      // The engine can finish a fast fixture job between the drawer's project
      // refresh and its next jobs.status poll. Reopening the drawer is the same
      // durable-history path a user exercises after returning later and forces
      // a fresh assets.generated_list request instead of relying on timing.
      const close = page.locator('[data-cut-generate-close]').first()
      if (await close.count()) await close.click()
      await page.locator('[data-cut-left-tab="assets"]').first().click().catch(() => {})
      await page.locator('[data-cut-action="generate-asset"]').first().click()
      await page.locator('[data-cut-generate]').first().waitFor({ state: 'visible', timeout: 8_000 })
      await item.waitFor({ state: 'visible', timeout: 15_000 })
    }
    return item
  }

  async function runHistoryLifecycle(page, gen, baseAssetId) {
    const baseItem = await waitForHistoryItem(page, baseAssetId)
    const selectTake = baseItem.locator('[data-cut-generated-select]').first()
    await probe(page, {
      surface,
      name: 'generated-select-base-take',
      actionId: 'generated-select',
      sel: selectTake,
      group: gen,
      groupName: 'generate-surface',
      doClick: async () => { await selectTake.click() },
      assertResult: async () => ({
        ok: await baseItem.getAttribute('data-cut-generated-chosen') === 'true',
        detail: `chosen=${await baseItem.getAttribute('data-cut-generated-chosen')}`,
      }),
    })

    const useReference = baseItem.locator('[data-cut-generated-use-reference]').first()
    await probe(page, {
      surface,
      name: 'generated-use-reference',
      actionId: 'generated-use-reference',
      sel: useReference,
      group: gen,
      groupName: 'generate-surface',
      doClick: async () => { await useReference.click() },
      assertResult: async () => ({
        ok: await page.locator('[data-cut-generate-reference-count]').textContent() === '1/4',
        detail: `count=${await page.locator('[data-cut-generate-reference-count]').textContent()}`,
      }),
    })

    const prepareVariation = baseItem.locator('[data-cut-generated-variation]').first()
    await prepareVariation.click()
    const variation = page.locator('[data-cut-generate-variation]').first()
    await variation.waitFor({ state: 'visible', timeout: 8_000 })
    const clearVariation = page.locator('[data-cut-generate-variation-clear]').first()
    await probe(page, {
      surface,
      name: 'generate-variation-clear',
      actionId: 'generate-variation-clear',
      sel: clearVariation,
      group: gen,
      groupName: 'generate-surface',
      doClick: async () => { await clearVariation.click() },
      assertResult: async () => ({
        ok: await page.locator('[data-cut-generate-variation]').count() === 0,
        detail: `variationMounted=${await page.locator('[data-cut-generate-variation]').count()}`,
      }),
    })

    if (!fixtureActive) {
      return {
        extended: false,
        detail: 'same-family compare and cancellation require FCV_AGENT_FIXTURES=1 to avoid provider spend',
      }
    }

    await prepareVariation.click()
    await page.locator('[data-cut-generate-variation]').waitFor({ state: 'visible', timeout: 8_000 })
    const generated = await generateCurrent(page)
    const variationAssetId = generated.terminal?.result?.asset_id
    if (generated.terminal?.state !== 'done' || !variationAssetId) {
      throw new Error(`deterministic variation generation failed: ${JSON.stringify(generated.terminal || generated.queued).slice(0, 500)}`)
    }
    const variationItem = await waitForHistoryItem(page, variationAssetId)

    for (const [label, item] of [['base', baseItem], ['variation', variationItem]]) {
      const control = item.locator('[data-cut-generated-compare-select]').first()
      await probe(page, {
        surface,
        name: `generated-compare-select-${label}`,
        actionId: 'generated-compare-select',
        sel: control,
        group: gen,
        groupName: 'generate-surface',
        doClick: async () => { await control.click() },
        assertResult: async () => ({
          ok: await control.isChecked(),
          detail: `checked=${await control.isChecked()}`,
        }),
      })
    }

    const openCompare = async () => {
      await page.locator('[data-cut-generated-compare]').click()
      await page.locator('[data-cut-generated-compare-dialog]').waitFor({ state: 'visible', timeout: 8_000 })
    }
    await openCompare()
    let dialog = page.locator('[data-cut-generated-compare-dialog]').first()
    let close = page.locator('[data-cut-generated-compare-close]').first()
    await probe(page, {
      surface,
      name: 'generated-compare-close',
      actionId: 'generated-compare-close',
      sel: close,
      group: dialog,
      groupName: 'generated-compare-dialog',
      doClick: async () => { await close.click() },
      assertResult: async () => ({
        ok: await page.locator('[data-cut-generated-compare-dialog]').count() === 0,
        detail: `dialog=${await page.locator('[data-cut-generated-compare-dialog]').count()}`,
      }),
    })

    await openCompare()
    dialog = page.locator('[data-cut-generated-compare-dialog]').first()
    const backdrop = page.locator('[data-cut-generated-compare-backdrop]').first()
    await probe(page, {
      surface,
      name: 'generated-compare-backdrop',
      actionId: 'generated-compare-backdrop',
      sel: backdrop,
      group: dialog,
      groupName: 'generated-compare-dialog',
      doClick: async () => { await backdrop.click({ position: { x: 2, y: 2 } }) },
      assertResult: async () => ({
        ok: await page.locator('[data-cut-generated-compare-dialog]').count() === 0,
        detail: `dialog=${await page.locator('[data-cut-generated-compare-dialog]').count()}`,
      }),
    })

    await openCompare()
    dialog = page.locator('[data-cut-generated-compare-dialog]').first()
    const choose = page.locator(`[data-cut-generated-choose="${variationAssetId}"]`).first()
    await probe(page, {
      surface,
      name: 'generated-choose-compared-take',
      actionId: 'generated-choose',
      sel: choose,
      group: dialog,
      groupName: 'generated-compare-dialog',
      doClick: async () => { await choose.click() },
      assertResult: async () => ({
        ok: await page.locator('[data-cut-generated-compare-dialog]').count() === 0
          && await variationItem.getAttribute('data-cut-generated-chosen') === 'true',
        detail: `dialog=${await page.locator('[data-cut-generated-compare-dialog]').count()} chosen=${await variationItem.getAttribute('data-cut-generated-chosen')}`,
      }),
    })

    const insert = variationItem.locator('[data-cut-generated-insert]').first()
    let inserted = null
    let insertedClip = ''
    await probe(page, {
      surface,
      name: 'generated-insert-variation-take',
      actionId: 'generated-insert',
      sel: insert,
      group: gen,
      groupName: 'generate-surface',
      doClick: async () => {
        inserted = await captureVerbResp(page, 'edit.insert', () => insert.click(), 30_000)
        insertedClip = inserted?.result?.clip_id || ''
        if (insertedClip) {
          await waitForState(
            (snapshot) => snapshot.tracks.some((track) =>
              track.clips?.some((clip) =>
                clip.id === insertedClip && clip.asset === variationAssetId)),
            8_000,
          )
        }
      },
      assertResult: async () => {
        const snapshot = await state()
        const landed = snapshot.tracks.some((track) =>
          track.clips?.some((clip) =>
            clip.id === insertedClip && clip.asset === variationAssetId))
        return {
          ok: inserted?.ok && insertedClip.length > 0 && landed,
          detail: `ok=${inserted?.ok} clip=${insertedClip || 'none'} asset=${variationAssetId} landed=${landed}`,
        }
      },
    })

    if (!insertedClip) {
      throw new Error(`generated-media history insert returned no clip: ${JSON.stringify(inserted).slice(0, 500)}`)
    }
    const insertedTimelineClip = page.locator(`[data-cut-clip="${insertedClip}"]`).first()
    await insertedTimelineClip.click()
    await page.locator(`[data-cut-clip="${insertedClip}"].tl-clip--selected`)
      .waitFor({ state: 'visible', timeout: 8_000 })
    await ensureGenerateOpen(page)
    await page.waitForFunction((assetId) => {
      const button = document.querySelector(
        `[data-cut-generated-asset="${assetId}"] [data-cut-generated-replace]`,
      )
      return button instanceof HTMLButtonElement && !button.disabled
    }, baseAssetId)
    const replace = baseItem.locator('[data-cut-generated-replace]').first()
    await replace.waitFor({ state: 'visible', timeout: 12_000 })
    await replace.scrollIntoViewIfNeeded()
    let replaced = null
    await probe(page, {
      surface,
      name: 'generated-replace-selected-clip',
      actionId: 'generated-replace',
      sel: replace,
      group: gen,
      groupName: 'generate-surface',
      doClick: async () => {
        replaced = await captureVerbResp(page, 'edit.replace', () => replace.click(), 30_000)
        await waitForState(
          (snapshot) => snapshot.tracks.some((track) =>
            track.clips?.some((clip) =>
              clip.id === insertedClip && clip.asset === baseAssetId)),
          8_000,
        )
      },
      assertResult: async () => {
        const snapshot = await state()
        const preserved = snapshot.tracks.some((track) =>
          track.clips?.some((clip) =>
            clip.id === insertedClip && clip.asset === baseAssetId))
        return {
          ok: replaced?.ok
            && replaced?.result?.target_clip === insertedClip
            && preserved,
          detail: `ok=${replaced?.ok} target=${replaced?.result?.target_clip || 'none'} clip=${insertedClip} asset=${baseAssetId} preserved=${preserved}`,
        }
      },
    })

    const resetVariation = page.locator('[data-cut-generate-variation-clear]').first()
    if (await resetVariation.isVisible().catch(() => false)) await resetVariation.click()
    await clickPlacement(page, 'insert')
    await page.locator('[data-cut-generate-prompt]').fill('deterministic cancelled generated slot')
    const run = page.locator('[data-cut-generate-run]').first()
    await run.click()
    await page.locator('[data-cut-generate-run][data-cut-generate-armed]').waitFor()
    const queued = await captureVerbResp(page, 'assets.generate', () => run.click(), 90_000)
    const cancelJobId = queued?.result?.job_id
    const retryTarget = queued?.result?.placement?.target_clip
    if (!queued?.ok || !cancelJobId || !retryTarget) {
      throw new Error(`deterministic cancellable generation was not queued: ${JSON.stringify(queued).slice(0, 500)}`)
    }

    const cancel = page.locator(`[data-cut-generate-job-cancel="${cancelJobId}"]`).first()
    // The fixture deliberately leaves this job cancellable, but keep a missed
    // render as an ordinary failing action row instead of crashing the entire
    // Assets section and hiding every later control.
    await cancel.waitFor({ state: 'visible', timeout: 8_000 }).catch(() => {})
    let cancelled = null
    let terminal = null
    await probe(page, {
      surface,
      name: 'generate-job-cancel',
      actionId: 'generate-job-cancel',
      sel: cancel,
      group: gen,
      groupName: 'generate-surface',
      doClick: async () => {
        cancelled = await captureVerbResp(page, 'jobs.cancel', () => cancel.click(), 30_000)
        terminal = await awaitJob(cancelJobId, 30_000)
        await page.locator(`[data-cut-generate-retry="${retryTarget}"]`).waitFor({ state: 'visible', timeout: 8_000 })
      },
      assertResult: async () => ({
        ok: cancelled?.ok
          && terminal?.state === 'failed'
          && terminal?.error?.code === 'job_cancelled'
          && await page.locator(`[data-cut-generate-retry="${retryTarget}"]`).count() === 1,
        detail: `cancelOk=${cancelled?.ok} state=${terminal?.state} code=${terminal?.error?.code} retry=${await page.locator(`[data-cut-generate-retry="${retryTarget}"]`).count()}`,
      }),
    })

    const retry = page.locator('[data-cut-generate-retry-prepare]').first()
    await probe(page, {
      surface,
      name: 'generate-retry-prepare',
      actionId: 'generate-retry-prepare',
      sel: retry,
      group: gen,
      groupName: 'generate-surface',
      doClick: async () => { await retry.click() },
      assertResult: async () => ({
        ok: await page.locator('[data-cut-generate-run][data-cut-generate-armed]').count() === 1
          && (await page.locator('[data-cut-generate-note]').textContent())?.includes(retryTarget),
        detail: `armed=${await page.locator('[data-cut-generate-run][data-cut-generate-armed]').count()} note=${(await page.locator('[data-cut-generate-note]').textContent())?.slice(0, 90)}`,
      }),
    })
    const disarmRetry = page.locator('[data-cut-generate-cancel]').first()
    if (await disarmRetry.count() > 0) await disarmRetry.click()

    const pending = await state()
    const pendingExists = pending.tracks.some((track) => track.clips.some((clip) => 'id' in clip && clip.id === retryTarget))
    return {
      extended: true,
      detail: `variation=${variationAssetId} familyCompare=true cancelled=${cancelJobId} retryTarget=${retryTarget} pending=${pendingExists}`,
    }
  }

  function recordExtendedSkip(detail) {
    for (const name of [
      'generated compare close/backdrop/choose',
      'generation job cancel and retry',
    ]) {
      rec(surface, name, {
        present: 'na',
        render: 'na',
        click: 'na',
        result: 'na',
      }, detail)
    }
  }

  return {
    runStaticControls,
    runHistoryLifecycle,
    recordExtendedSkip,
  }
}
