// Native action coverage for every Assemble parameter. The caller seeds a real
// engine-readable transcript before opening the drawer, so these form controls
// and their result jumps remain testable even when optional live STT is absent.

export function createAssembleActionCoverage({
  probe,
  verb,
  sleep,
}) {
  const surface = 'assemble'

  async function mode(page, value) {
    await page.locator(`[data-cut-assemble-mode-opt="${value}"]`).click()
    await page.locator(`[data-cut-assemble][data-cut-assemble-mode="${value}"]`).waitFor()
  }

  async function fill(page, drawer, {
    name,
    actionId,
    selector,
    value,
  }) {
    const control = page.locator(selector).first()
    await probe(page, {
      surface,
      name,
      actionId,
      sel: control,
      group: drawer,
      groupName: 'assemble-drawer',
      doClick: async () => { await control.fill(String(value)) },
      assertResult: async () => ({
        ok: await control.inputValue() === String(value),
        detail: `value=${await control.inputValue()}`,
      }),
    })
  }

  async function select(page, drawer, {
    name,
    actionId,
    selector,
    value,
  }) {
    const control = page.locator(selector).first()
    await probe(page, {
      surface,
      name,
      actionId,
      sel: control,
      group: drawer,
      groupName: 'assemble-drawer',
      doClick: async () => { await control.selectOption(value) },
      assertResult: async () => ({
        ok: await control.inputValue() === value,
        detail: `value=${await control.inputValue()}`,
      }),
    })
  }

  async function chooseAsset(page, drawer, modeName, primaryAsset, secondaryAsset) {
    await mode(page, modeName)
    const control = page.locator('[data-cut-assemble-asset]').first()
    let selectedSecondary = false
    await probe(page, {
      surface,
      name: `asset-${modeName}`,
      actionId: 'assemble-asset',
      sel: control,
      group: drawer,
      groupName: 'assemble-drawer',
      doClick: async () => {
        await control.selectOption(secondaryAsset)
        selectedSecondary = await control.inputValue() === secondaryAsset
        await control.selectOption(primaryAsset)
      },
      assertResult: async () => ({
        ok: selectedSecondary && await control.inputValue() === primaryAsset,
        detail: `secondarySelected=${selectedSecondary} restoredPrimary=${await control.inputValue() === primaryAsset}`,
      }),
    })
  }

  async function runInputs(page, drawer, {
    primaryAsset,
    secondaryAsset,
  }) {
    for (const modeName of ['shorts', 'repurpose', 'from_script']) {
      await chooseAsset(page, drawer, modeName, primaryAsset, secondaryAsset)
    }

    await mode(page, 'shorts')
    await fill(page, drawer, {
      name: 'count-shorts',
      actionId: 'assemble-count',
      selector: '[data-cut-assemble-count]',
      value: 3,
    })
    await fill(page, drawer, {
      name: 'target-shorts',
      actionId: 'assemble-target',
      selector: '[data-cut-assemble-target]',
      value: 12,
    })
    for (const value of ['1:1', '4:5', '16:9', '9:16']) {
      await select(page, drawer, {
        name: `aspect-${value.replace(':', 'x')}`,
        actionId: 'assemble-aspect',
        selector: '[data-cut-assemble-aspect]',
        value,
      })
    }

    await mode(page, 'repurpose')
    await fill(page, drawer, {
      name: 'count-repurpose',
      actionId: 'assemble-count',
      selector: '[data-cut-assemble-count]',
      value: 2,
    })
    await fill(page, drawer, {
      name: 'target-repurpose',
      actionId: 'assemble-target',
      selector: '[data-cut-assemble-target]',
      value: 9,
    })
    await fill(page, drawer, {
      name: 'prompt-repurpose',
      actionId: 'assemble-prompt',
      selector: '[data-cut-assemble-prompt]',
      value: 'human speech captions',
    })

    await mode(page, 'from_script')
    await fill(page, drawer, {
      name: 'min-score-from-script',
      actionId: 'assemble-minscore',
      selector: '[data-cut-assemble-minscore]',
      value: 0.4,
    })

    await mode(page, 'broll')
    await fill(page, drawer, {
      name: 'place-at-broll',
      actionId: 'assemble-at',
      selector: '[data-cut-assemble-at]',
      value: 2,
    })
    await fill(page, drawer, {
      name: 'duration-broll',
      actionId: 'assemble-dur',
      selector: '[data-cut-assemble-dur]',
      value: 4,
    })

    // Restore the deterministic analysis starting mode for the caller's runs.
    await mode(page, 'shorts')
  }

  async function proveJump(page, drawer, {
    modeName,
    expectedAtMs,
  }) {
    const control = page.locator('[data-cut-assemble-jump]').first()
    const expected = Math.round(expectedAtMs)
    const sentinel = expected + 777
    let prepared = null
    let finalState = null
    let selfRelayRequests = 0
    const noteRequest = (request) => {
      if (request.url().includes('/api/verb/ui.playhead')) selfRelayRequests += 1
    }
    await probe(page, {
      surface,
      name: `jump-${modeName}`,
      actionId: 'assemble-jump',
      sel: control,
      group: drawer,
      groupName: 'assemble-drawer',
      doClick: async () => {
        prepared = await verb('ui.playhead', { at_ms: sentinel })
        for (let attempt = 0; attempt < 40; attempt++) {
          const observed = await verb('ui.state', {})
          if (observed.ok && observed.result?.playhead_ms === sentinel) break
          await sleep(25)
        }
        page.on('request', noteRequest)
        await control.click()
        page.off('request', noteRequest)
        for (let attempt = 0; attempt < 80; attempt++) {
          const observed = await verb('ui.state', {})
          if (observed.ok && observed.result?.playhead_ms === expected) {
            finalState = observed
            break
          }
          await sleep(25)
        }
      },
      assertResult: async () => ({
        ok: prepared?.ok
          && prepared.result?.applied === true
          && finalState?.result?.playhead_ms === expected
          && selfRelayRequests === 0,
        detail: `prepared=${prepared?.ok}/${prepared?.result?.applied} playhead=${finalState?.result?.playhead_ms} expected=${expected} selfRelayRequests=${selfRelayRequests}`,
      }),
    })
  }

  return { runInputs, proveJump }
}
