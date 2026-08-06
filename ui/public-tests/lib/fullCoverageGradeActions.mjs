// Exhaustive Color-tab action coverage.
//
// The regular video section proves edit.grade at the engine level, while this
// lane drives every human control in the canonical right-rail surface. Browser
// runs stub only the native .cube picker result; installed sweeps leave that OS
// chooser to the paired OS-action gate so an unpaired dialog cannot stall WebDriver.

export function createGradeActionCoverage({
  probe,
  state,
  waitForState,
  captureVerbResp,
  sleep,
  freshProject,
  closeOverlays,
  selectClip,
  clipOfKind,
  primaryMedia,
  lutPath,
  nativePickerClickNa,
  usePickerFixture = false,
  nativeOsActionsEnabled = false,
}) {
  const surface = 'grade-actions'
  const sameHostPath = (left, right) => {
    const normalize = (value) => String(value || '')
      .replace(/^[/\\]{2}[?][/\\]/, '')
      .replace(/\\/g, '/')
      .replace(/\/+/g, '/')
      .toLowerCase()
    return normalize(left) === normalize(right)
  }

  async function installPickerFixture(page) {
    if (!usePickerFixture) return
    await page.evaluate((selectedPath) => {
      const target = window
      target.__fcvGradeHadTauri = Object.prototype.hasOwnProperty.call(target, '__TAURI__')
      target.__fcvGradeOriginalTauri = target.__TAURI__
      target.__fcvGradeHadInternals = Object.prototype.hasOwnProperty.call(target, '__TAURI_INTERNALS__')
      target.__fcvGradeOriginalInvoke = target.__TAURI_INTERNALS__?.invoke
      target.__fcvGradePickerCalls = []
      const originalInvoke = target.__fcvGradeOriginalInvoke
      const invoke = async (command, args) => {
        if (command === 'plugin:dialog|open') {
          target.__fcvGradePickerCalls.push({ command, args })
          return selectedPath
        }
        if (originalInvoke) return originalInvoke(command, args)
        throw new Error(`unexpected grade fixture command: ${command}`)
      }
      if (target.__TAURI_INTERNALS__) target.__TAURI_INTERNALS__.invoke = invoke
      else target.__TAURI_INTERNALS__ = { invoke }
      if (!target.__TAURI__) {
        target.__TAURI__ = {
          core: { invoke },
          event: { listen: async () => () => {} },
        }
      }
    }, lutPath)
  }

  async function restorePickerFixture(page) {
    if (!usePickerFixture) return
    await page.evaluate(() => {
      const target = window
      if (target.__fcvGradeHadInternals) {
        target.__TAURI_INTERNALS__.invoke = target.__fcvGradeOriginalInvoke
      } else {
        delete target.__TAURI_INTERNALS__
      }
      if (target.__fcvGradeHadTauri) target.__TAURI__ = target.__fcvGradeOriginalTauri
      else delete target.__TAURI__
      delete target.__fcvGradeHadTauri
      delete target.__fcvGradeOriginalTauri
      delete target.__fcvGradeHadInternals
      delete target.__fcvGradeOriginalInvoke
      delete target.__fcvGradePickerCalls
    })
  }

  async function setSlider(page, panel, clipId, attr, value) {
    const slider = page.locator(`[data-cut-grade-input="${attr}"]`).first()
    await probe(page, {
      surface,
      name: `grade-${attr}`,
      actionId: 'grade-input',
      sel: slider,
      group: panel,
      groupName: 'grade-controls',
      doClick: async () => { await slider.fill(String(value)) },
      assertResult: async () => {
        const inputValue = await slider.inputValue().catch(() => '')
        const readout = await page.locator(`[data-cut-grade-val="${attr}"]`).first().textContent().catch(() => '')
        return {
          ok: Number(inputValue) === value && Number.parseFloat(readout || '') === value,
          detail: `${clipId} ${attr} input=${inputValue} readout=${readout}`,
        }
      },
    })
  }

  async function run(page) {
    await freshProject(page, 'grade_actions', primaryMedia)
    await closeOverlays(page)
    const clipId = await clipOfKind('video')
    if (!clipId) throw new Error('grade action coverage requires a video clip')
    await selectClip(page, clipId)
    await installPickerFixture(page)
    try {
      const expand = page.locator('[data-cut-action="expand-rail"]').first()
      if (await expand.count()) await expand.click()
      await page.locator('[data-cut-right-tab="color"]').first().click()
      const panel = page.locator('[data-cut-grade-embed]').first()
      await panel.waitFor({ state: 'visible', timeout: 8000 })

      for (const [attr, value] of [
        ['contrast', 1.35],
        ['brightness', 0.2],
        ['saturation', 0.75],
        ['gamma', 1.4],
      ]) {
        await setSlider(page, panel, clipId, attr, value)
      }

      const temperatureToggle = page.locator('[data-cut-grade-temp-on]').first()
      await probe(page, {
        surface,
        name: 'grade-temperature-toggle',
        actionId: 'grade-temp-on',
        sel: temperatureToggle,
        group: panel,
        groupName: 'grade-controls',
        doClick: async () => { await temperatureToggle.check() },
        assertResult: async () => ({
          ok: await temperatureToggle.isChecked()
            && await page.locator('[data-cut-grade-input="temperature_k"]').first().isVisible(),
          detail: 'white balance enabled and Kelvin slider revealed',
        }),
      })
      await setSlider(page, panel, clipId, 'temperature_k', 4200)

      const advanced = page.locator('[data-cut-grade-lut-advanced]').first()
      await advanced.evaluate((element) => { element.open = false })
      const advancedToggle = page.locator('[data-cut-grade-lut-advanced-toggle]').first()
      await probe(page, {
        surface,
        name: 'grade-lut-advanced',
        actionId: 'grade-lut-advanced-toggle',
        sel: advancedToggle,
        group: panel,
        groupName: 'grade-lut',
        doClick: async () => { await advancedToggle.click() },
        assertResult: async () => ({
          ok: (await advanced.getAttribute('open')) !== null,
          detail: 'advanced LUT path opened',
        }),
      })

      const lutInput = page.locator('[data-cut-grade-lut]').first()
      await probe(page, {
        surface,
        name: 'grade-lut-manual-path',
        actionId: 'grade-lut',
        sel: lutInput,
        group: panel,
        groupName: 'grade-lut',
        doClick: async () => { await lutInput.fill(lutPath) },
        assertResult: async () => {
          const value = await lutInput.inputValue().catch(() => '')
          return { ok: value === lutPath, detail: `manual LUT path=${value}` }
        },
      })

      const picker = page.locator('[data-cut-grade-lut-pick]').first()
      await lutInput.fill('')
      await probe(page, {
        surface,
        name: 'grade-lut-native-picker',
        actionId: 'grade-lut-pick',
        sel: picker,
        group: panel,
        groupName: 'grade-lut',
        clickNa: nativePickerClickNa,
        nativeAction: { mode: 'select', path: lutPath, useDoClick: true, verifyResult: true },
        doClick: async () => {
          await picker.click()
          await page.waitForFunction(
            (expected) => {
              const normalize = (value) => String(value || '')
                .replace(/^[/\\]{2}[?][/\\]/, '')
                .replace(/\\/g, '/')
                .replace(/\/+/g, '/')
                .toLowerCase()
              return normalize(document.querySelector('[data-cut-grade-lut]')?.value) === normalize(expected)
            },
            lutPath,
            { timeout: 8000 },
          )
        },
        assertResult: async () => {
          const result = await page.evaluate((expected) => ({
            value: document.querySelector('[data-cut-grade-lut]')?.value || '',
            chip: document.querySelector('[data-cut-grade-lut-picked]')?.textContent || '',
            calls: window.__fcvGradePickerCalls?.length || 0,
            expected,
          }), lutPath)
          return {
            ok: sameHostPath(result.value, lutPath)
              && (usePickerFixture ? result.calls === 1 : nativeOsActionsEnabled)
              && result.chip.trim().endsWith('.cube'),
            detail: `picker calls=${result.calls}; chip="${result.chip.trim()}"; path selected=${sameHostPath(result.value, lutPath)}; native=${nativeOsActionsEnabled}`,
          }
        },
      })
      if (nativePickerClickNa) await lutInput.fill(lutPath)

      await probe(page, {
        surface,
        name: 'grade-apply-all-controls',
        actionId: 'grade-apply',
        sel: page.locator('[data-cut-grade-apply]').first(),
        group: panel,
        groupName: 'grade-controls',
        doClick: async () => {
          probe._r = await captureVerbResp(page, 'edit.grade', async () => {
            await page.locator('[data-cut-grade-apply]').first().click()
          }, 20_000)
        },
        assertResult: async () => {
          const next = await waitForState((project) => {
            const clip = project.tracks.flatMap((track) => track.clips).find((item) => item.id === clipId)
            return clip?.grade?.contrast === 1.35
              && clip.grade.brightness === 0.2
              && clip.grade.saturation === 0.75
              && clip.grade.gamma === 1.4
              && clip.grade.temperature_k === 4200
              && sameHostPath(clip.grade.lut, lutPath)
          }, 10_000)
          return {
            ok: !!probe._r?.ok && !!next,
            detail: `edit.grade ok=${probe._r?.ok}; all slider, Kelvin, and LUT values landed=${!!next}`,
          }
        },
      })

      await probe(page, {
        surface,
        name: 'grade-reset-to-neutral',
        actionId: 'grade-reset',
        sel: page.locator('[data-cut-grade-reset]').first(),
        group: panel,
        groupName: 'grade-controls',
        doClick: async () => {
          probe._r = await captureVerbResp(page, 'edit.grade', async () => {
            await page.locator('[data-cut-grade-reset]').first().click()
          }, 20_000)
        },
        assertResult: async () => {
          const next = await waitForState((project) => {
            const clip = project.tracks.flatMap((track) => track.clips).find((item) => item.id === clipId)
            return clip && clip.grade == null
          }, 10_000)
          const values = await page.locator('[data-cut-grade-input]').evaluateAll((inputs) =>
            Object.fromEntries(inputs.map((input) => [input.getAttribute('data-cut-grade-input'), input.value])))
          return {
            ok: !!probe._r?.ok && !!next
              && values.contrast === '1'
              && values.brightness === '0'
              && values.saturation === '1'
              && values.gamma === '1',
            detail: `edit.grade neutral ok=${probe._r?.ok}; grade cleared=${!!next}; sliders=${JSON.stringify(values)}`,
          }
        },
      })
      // ---- crash-safe panel restore guard (2026-08-06 Color-panel fix) -----
      // panelPersistGuard blocklists a right tab whose mount never confirmed a
      // paint (software-rendering WebView death). Seed that persisted evidence,
      // re-enter the Color tab, and prove the honest notice gates the mount;
      // the "load anyway" action must mount the real panel and — once the
      // paint confirms — self-heal the blocklist so restore works again.
      await page.evaluate(() => {
        localStorage.setItem('cut.panelBlocked.v1', JSON.stringify({ color: Date.now() }))
      })
      await page.locator('[data-cut-right-tab="properties"]').first().click()
      await page.locator('[data-cut-right-tab="color"]').first().click()
      const blockedNotice = page.locator('[data-cut-panel-render-blocked="color"]').first()
      await blockedNotice.waitFor({ state: 'visible', timeout: 8000 })
      const retryButton = page.locator('[data-cut-panel-render-retry="color"]').first()
      await probe(page, {
        surface,
        name: 'grade-render-guard-load-anyway',
        actionId: 'panel-render-retry',
        sel: retryButton,
        group: blockedNotice,
        groupName: 'panel-render-guard',
        doClick: async () => { await retryButton.click() },
        assertResult: async () => {
          await page.locator('[data-cut-grade-embed]').first().waitFor({ state: 'visible', timeout: 8000 })
          // Confirmed paint (double-rAF + settle ≈ 400ms) must clear the block
          // so the NEXT launch restores the Color tab normally again.
          await page.waitForFunction(() => {
            const raw = localStorage.getItem('cut.panelBlocked.v1')
            if (!raw) return true
            try { return !('color' in JSON.parse(raw)) } catch { return true }
          }, undefined, { timeout: 8000 })
          return { ok: true, detail: 'blocked notice shown; load-anyway mounted the panel; blocklist self-healed after paint' }
        },
      })
    } finally {
      await restorePickerFixture(page)
      await closeOverlays(page)
    }
  }

  return { run }
}
