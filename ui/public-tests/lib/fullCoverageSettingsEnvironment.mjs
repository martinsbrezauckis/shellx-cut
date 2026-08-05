// Environment-card actions nested inside the installed/native Settings sweep.
// This module keeps global tool preferences reversible: the STT and ffmpeg
// scenarios restore the values that existed before the test.

const DETAIL_CATEGORIES = [
  'video-performance',
  'ai-transcription',
  'services-integrations',
  'storage-privacy',
]

function card(report, id) {
  return (report?.cards || []).find((entry) => entry.id === id)
}

export function createSettingsEnvironmentCoverage({
  probe,
  verb,
  sleep,
  nativePickerClickNa = '',
}) {
  async function category(page, id) {
    await page.locator(`[data-cut-settings-category="${id}"]`).click()
    await page.locator(`[data-cut-settings-body="${id}"]`).waitFor({ state: 'visible', timeout: 12_000 })
  }

  async function rescan(page, id) {
    await page.locator('[data-cut-environment-refresh]').click()
    await sleep(500)
    await category(page, id)
  }

  async function waitDoctorCard(id, predicate, timeoutMs = 30_000) {
    const deadline = Date.now() + timeoutMs
    let last = null
    while (Date.now() < deadline) {
      last = card((await verb('system.doctor', {})).result, id)
      if (predicate(last)) return last
      await sleep(300)
    }
    return last
  }

  async function waitSttConfirmation(page, pattern) {
    const note = page.locator('[data-cut-env-stt-note]')
    await note.waitFor({ state: 'visible', timeout: 30_000 })
    const text = (await note.textContent()) || ''
    if (!pattern.test(text)) {
      throw new Error(`unexpected STT confirmation: ${text || 'empty'}`)
    }
  }

  async function run(page, panel, surface = 'settings') {
    for (const categoryId of DETAIL_CATEGORIES) {
      await category(page, categoryId)
      const toggles = page.locator('[data-cut-env-advanced-toggle]')
      const count = await toggles.count()
      for (let index = 0; index < count; index += 1) {
        const toggle = toggles.nth(index)
        const id = await toggle.getAttribute('data-cut-env-advanced-toggle') || `${categoryId}-${index}`
        await probe(page, {
          surface, name: `settings-env-advanced-${id}`, actionId: `env-advanced-toggle:${id}`,
          sel: toggle, group: panel, groupName: `settings-${categoryId}`,
          doClick: async () => { await toggle.click(); await sleep(60) },
          assertResult: async () => ({
            ok: (await page.locator(`[data-cut-env-advanced="${id}"]`).getAttribute('open')) !== null,
            detail: `${id} advanced details expanded`,
          }),
        })
      }
    }

    await category(page, 'video-performance')
    const gpuHelp = page.locator('[data-cut-env-gpu-help-toggle]')
    if (await gpuHelp.count()) {
      await probe(page, {
        surface, name: 'settings-gpu-help', actionId: 'env-gpu-help-toggle',
        sel: gpuHelp, group: panel, groupName: 'settings-video-performance',
        doClick: async () => { await gpuHelp.click(); await sleep(60) },
        assertResult: async () => ({
          ok: (await page.locator('[data-cut-env-gpu-help]').getAttribute('open')) !== null,
          detail: 'GPU enablement help expanded',
        }),
      })
    }

    const initialDoctor = (await verb('system.doctor', {})).result
    const gpu = card(initialDoctor, 'gpu-encode')
    const initialFfmpeg = gpu?.details?.override_setting ?? null
    const resolvedFfmpeg = gpu?.details?.resolved ?? card(initialDoctor, 'ffmpeg')?.details?.resolved ?? null
    if (resolvedFfmpeg) {
      if (!initialFfmpeg) {
        await verb('system.set_ffmpeg', { path: resolvedFfmpeg })
        await rescan(page, 'video-performance')
      }
      const auto = page.locator('[data-cut-env-ffmpeg-auto]')
      await auto.waitFor({ state: 'visible', timeout: 12_000 })
      await probe(page, {
        surface, name: 'settings-ffmpeg-automatic', actionId: 'env-ffmpeg-auto',
        sel: auto, group: panel, groupName: 'settings-video-performance',
        doClick: async () => { await auto.click(); await sleep(500) },
        assertResult: async () => {
          const updated = await waitDoctorCard(
            'gpu-encode',
            (entry) => (entry?.details?.override_setting ?? null) === null,
          )
          const override = updated?.details?.override_setting ?? null
          return { ok: override === null, detail: `ffmpeg override after Use automatic=${override ?? 'none'}` }
        },
      })
      await rescan(page, 'video-performance')
    }
    const changeFfmpeg = page.locator('[data-cut-env-ffmpeg-change]')
    // "Use automatic" swaps this control into the card on the next React
    // render. Give that render a bounded chance to settle, then always emit the
    // probe so an absent control is a real failed row instead of a silent skip.
    await changeFfmpeg.waitFor({ state: 'visible', timeout: 12_000 }).catch(() => {})
    await probe(page, {
      surface, name: 'settings-ffmpeg-change', actionId: 'env-ffmpeg-change',
      sel: changeFfmpeg, group: panel, groupName: 'settings-video-performance',
      clickNa: nativePickerClickNa,
      nativeAction: { mode: 'cancel' },
      doClick: async () => { await changeFfmpeg.click(); await sleep(100) },
      assertResult: async () => ({
        ok: (await page.locator('[data-cut-env-ffmpeg-change]').count()) === 1,
        detail: 'desktop ffmpeg picker returned without changing the automatic choice',
      }),
    })
    if (initialFfmpeg) await verb('system.set_ffmpeg', { path: initialFfmpeg })

    const rescans = page.locator('[data-cut-env-rescan]')
    for (let index = 0; index < await rescans.count(); index += 1) {
      const rescanButton = rescans.nth(index)
      const id = await rescanButton.getAttribute('data-cut-env-rescan') || String(index)
      await probe(page, {
        surface, name: `settings-card-rescan-${id}`, actionId: `env-rescan:${id}`,
        sel: rescanButton, group: panel, groupName: 'settings-video-performance',
        doClick: async () => { await rescanButton.click(); await sleep(350) },
        assertResult: async () => ({
          ok: (await page.locator('[data-cut-environment-open="true"]').count()) === 1,
          detail: `${id} card re-scan left Settings responsive`,
        }),
      })
    }

    await category(page, 'ai-transcription')
    const initialStt = card((await verb('system.doctor', {})).result, 'perception')?.details?.stt_model ?? null
    if ((await page.locator('[data-cut-env-stt-model]').count()) === 0) {
      await verb('system.set_stt_model', { clear: true })
      await rescan(page, 'ai-transcription')
    }
    const model = page.locator('[data-cut-env-stt-model]')
    await model.waitFor({ state: 'visible', timeout: 12_000 })
    const before = await model.inputValue()
    const next = before === 'nemo-canary-1b-v2' ? 'whisperx-large-v3' : 'nemo-canary-1b-v2'
    await probe(page, {
      surface, name: 'settings-stt-model', actionId: 'env-stt-model',
      sel: model, group: panel, groupName: 'settings-ai-transcription',
      doClick: async () => {
        await model.selectOption(next)
        await waitSttConfirmation(page, /Caption model set to/)
      },
      assertResult: async () => {
        const updated = await waitDoctorCard('perception', (entry) => entry?.details?.stt_model === next)
        const selected = updated?.details?.stt_model
        return { ok: selected === next, detail: `STT model ${before} -> ${selected}` }
      },
    })
    await rescan(page, 'ai-transcription')
    const sttReset = page.locator('[data-cut-env-stt-reset]')
    await sttReset.waitFor({ state: 'visible', timeout: 12_000 })
    await probe(page, {
      surface, name: 'settings-stt-reset', actionId: 'env-stt-reset',
      sel: sttReset, group: panel, groupName: 'settings-ai-transcription',
      doClick: async () => {
        await sttReset.click()
        await waitSttConfirmation(page, /Caption model reset to Parakeet v3/)
      },
      assertResult: async () => {
        const updated = await waitDoctorCard(
          'perception',
          (entry) => entry?.details?.stt_model === 'nemo-parakeet-tdt-0.6b-v3',
        )
        const selected = updated?.details?.stt_model
        return {
          ok: selected === 'nemo-parakeet-tdt-0.6b-v3',
          detail: `STT reset model=${selected ?? 'unknown'}`,
        }
      },
    })
    if (initialStt && initialStt !== 'nemo-parakeet-tdt-0.6b-v3') {
      await verb('system.set_stt_model', { model: initialStt })
    }

    const sttAdvanced = page.locator('[data-cut-env-stt-advanced-toggle]')
    if (await sttAdvanced.count()) {
      await probe(page, {
        surface, name: 'settings-stt-advanced', actionId: 'env-stt-advanced-toggle',
        sel: sttAdvanced, group: panel, groupName: 'settings-ai-transcription',
        doClick: async () => { await sttAdvanced.click(); await sleep(60) },
        assertResult: async () => ({
          ok: (await page.locator('[data-cut-env-stt-advanced]').getAttribute('open')) !== null,
          detail: 'STT model details expanded',
        }),
      })
    }

    await category(page, 'services-integrations')
    const setupToggles = page.locator('[data-cut-env-service-setup-toggle]')
    for (let index = 0; index < await setupToggles.count(); index += 1) {
      const toggle = setupToggles.nth(index)
      const id = await toggle.getAttribute('data-cut-env-service-setup-toggle') || String(index)
      await probe(page, {
        surface, name: `settings-service-steps-${id}`, actionId: `env-service-setup-toggle:${id}`,
        sel: toggle, group: panel, groupName: 'settings-services-integrations',
        doClick: async () => { await toggle.click(); await sleep(60) },
        assertResult: async () => ({
          ok: (await page.locator(`[data-cut-env-service-setup="${id}"]`).getAttribute('open')) !== null,
          detail: `${id} connection steps expanded`,
        }),
      })
    }
  }

  return run
}
