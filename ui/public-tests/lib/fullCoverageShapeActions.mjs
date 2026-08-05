// Direct native coverage for every configurable action in the Shape drawer.
// Repeated preset buttons are each exercised because their shared handlers carry
// different geometry. Two real shapes prove the chosen box/line values persist.

export function createShapeActionCoverage({
  probe,
  state,
  waitForState,
  opsLen,
  opLanded,
  captureVerbResp,
  sleep,
  freshProject,
  closeOverlays,
}) {
  const surface = 'shape-actions'

  async function chooseButton(page, panel, {
    name,
    actionId,
    selector,
    result,
  }) {
    const control = page.locator(selector).first()
    await probe(page, {
      surface, name, actionId,
      sel: control, group: panel, groupName: 'shape-presets',
      doClick: async () => { await control.click(); await sleep(60) },
      assertResult: result,
    })
  }

  async function fillControl(page, panel, {
    name,
    actionId,
    selector,
    value,
  }) {
    const control = page.locator(selector).first()
    await probe(page, {
      surface, name, actionId,
      sel: control, group: panel, groupName: 'shape-fields',
      doClick: async () => { await control.fill(String(value)); await sleep(60) },
      assertResult: async () => ({
        ok: (await control.inputValue()) === String(value),
        detail: `${name}=${await control.inputValue()}`,
      }),
    })
  }

  function findShape(project, clipId) {
    for (const track of project?.tracks || []) {
      const clip = (track.clips || []).find((candidate) => candidate.id === clipId)
      if (clip) return clip
    }
    return null
  }

  async function applyShape(page, panel, expectedKind, expected) {
    const control = page.locator('[data-cut-shape-apply]').first()
    let response = null
    let before = 0
    await probe(page, {
      surface,
      name: `shape-apply-${expectedKind}`,
      actionId: 'shape-apply',
      sel: control,
      group: panel,
      groupName: `shape-apply-${expectedKind}`,
      doClick: async () => {
        before = await opsLen()
        response = await captureVerbResp(page, 'edit.add_shape', () => control.click(), 20_000)
      },
      assertResult: async () => {
        const clipId = response?.result?.clip_id
        const changed = clipId
          ? await waitForState((project) => findShape(project, clipId)?.shape_kind === expectedKind, 15_000)
          : null
        const clip = findShape(changed, clipId)
        const landed = await opLanded(
          before,
          'edit.add_shape',
          (args) => args.shape === expectedKind && expected(args),
          { timeoutMs: 15_000 },
        )
        return {
          ok: !!response?.ok && !!changed && landed,
          detail: `edit.add_shape ok=${response?.ok}; op=${landed}; clip=${clipId || 'none'}; kind=${clip?.shape_kind || 'none'}`,
        }
      },
    })
  }

  async function run(page) {
    await freshProject(page, 'shape-actions')
    await closeOverlays(page)
    await page.locator('[data-cut-shape-btn]').first().click()
    const panel = page.locator('[data-cut-shape]').first()
    await panel.waitFor({ state: 'visible', timeout: 12_000 })
    await sleep(180)

    for (const preset of ['tl', 'tc', 'tr', 'ml', 'mc', 'mr', 'bl', 'bc', 'br']) {
      const selector = `[data-cut-shape-box-preset="${preset}"]`
      await chooseButton(page, panel, {
        name: `shape-box-preset-${preset}`,
        actionId: 'shape-box-preset',
        selector,
        result: async () => ({
          ok: (await page.locator(selector).first().getAttribute('class') || '').includes('cd-grid3-cell--on'),
          detail: `box preset ${preset} selected`,
        }),
      })
    }

    for (const kind of ['ellipse', 'line', 'arrow', 'rect']) {
      const selector = `[data-cut-shape-kind-opt="${kind}"]`
      await chooseButton(page, panel, {
        name: `shape-kind-opt-${kind}`,
        actionId: 'shape-kind-opt',
        selector,
        result: async () => ({
          ok: (await page.locator(selector).first().getAttribute('aria-selected')) === 'true',
          detail: `shape kind ${kind} selected`,
        }),
      })
    }

    const fillToggle = page.locator('[data-cut-shape-fill-on]').first()
    const fillBefore = await fillToggle.isChecked()
    await probe(page, {
      surface, name: 'shape-fill-on', actionId: 'shape-fill-on',
      sel: fillToggle, group: panel, groupName: 'shape-fields',
      doClick: async () => { await fillToggle.click(); await sleep(60) },
      assertResult: async () => ({
        ok: (await fillToggle.isChecked()) !== fillBefore,
        detail: `fill ${fillBefore} -> ${await fillToggle.isChecked()}`,
      }),
    })
    if (!(await fillToggle.isChecked())) {
      await fillToggle.click()
      await sleep(60)
    }

    await fillControl(page, panel, {
      name: 'shape-fill', actionId: 'shape-fill',
      selector: '[data-cut-shape-fill]', value: '#FF00AA',
    })
    await fillControl(page, panel, {
      name: 'shape-stroke', actionId: 'shape-stroke',
      selector: '[data-cut-shape-stroke]', value: '#00FFFF',
    })
    await fillControl(page, panel, {
      name: 'shape-strokepx', actionId: 'shape-strokepx',
      selector: '[data-cut-shape-strokepx]', value: 9,
    })

    const animation = page.locator('[data-cut-shape-anim]').first()
    await probe(page, {
      surface, name: 'shape-anim', actionId: 'shape-anim',
      sel: animation, group: panel, groupName: 'shape-fields',
      doClick: async () => { await animation.selectOption('pop'); await sleep(60) },
      assertResult: async () => ({ ok: (await animation.inputValue()) === 'pop', detail: 'animation=pop' }),
    })
    await fillControl(page, panel, {
      name: 'shape-in', actionId: 'shape-in',
      selector: '[data-cut-shape-in]', value: 1.2,
    })
    await fillControl(page, panel, {
      name: 'shape-out', actionId: 'shape-out',
      selector: '[data-cut-shape-out]', value: 4.4,
    })
    await page.locator('[data-cut-shape-text]').fill('FCV box')
    await page.locator('[data-cut-shape-kind-opt="ellipse"]').click()
    await page.locator('[data-cut-shape-box-preset="br"]').click()
    await applyShape(page, panel, 'ellipse', (args) => (
      args.text === 'FCV box'
      && args.fill === '#FF00AA'
      && args.stroke === '#00FFFF'
      && args.stroke_px === 9
      && args.animation === 'pop'
      && args.x === 0.6
      && args.y === 0.7
      && args.w === 0.34
      && args.h === 0.22
    ))

    await page.locator('[data-cut-shape-kind-opt="arrow"]').click()
    await sleep(80)
    for (const preset of ['lr', 'diag', 'up']) {
      const selector = `[data-cut-shape-line-preset="${preset}"]`
      await chooseButton(page, panel, {
        name: `shape-line-preset-${preset}`,
        actionId: 'shape-line-preset',
        selector,
        result: async () => ({
          ok: (await page.locator(selector).first().getAttribute('class') || '').includes('cd-seg-btn--on'),
          detail: `line preset ${preset} selected`,
        }),
      })
    }
    await applyShape(page, panel, 'arrow', (args) => (
      args.stroke === '#00FFFF'
      && args.stroke_px === 9
      && args.animation === 'pop'
      && args.x === 0.2
      && args.y === 0.8
      && args.x2 === 0.8
      && args.y2 === 0.3
    ))

    const project = await state()
    const shapeCount = project.tracks
      .flatMap((track) => track.clips || [])
      .filter((clip) => clip.shape_kind === 'ellipse' || clip.shape_kind === 'arrow')
      .length
    if (shapeCount < 2) throw new Error(`Shape action fixture persisted only ${shapeCount} tested shapes`)
  }

  return { run }
}
