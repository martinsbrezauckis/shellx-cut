// Direct native coverage for the Mask/privacy drawer. Unlike the legacy mask
// row, this lane draws on the real Preview overlay with native pointer actions.
// It proves rect/ellipse/polygon gestures and both whole/timed verb branches.

export function createMaskActionCoverage({
  probe,
  state,
  waitForState,
  opsLen,
  opLanded,
  captureVerbResp,
  sleep,
  freshProject,
  closeOverlays,
  clipOfKind,
  selectClip,
  primaryMedia,
}) {
  const surface = 'mask-actions'

  function findClip(project, clipId) {
    for (const track of project?.tracks || []) {
      const clip = (track.clips || []).find((candidate) => candidate.id === clipId)
      if (clip) return clip
    }
    return null
  }

  function point(box, x, y) {
    return {
      x: box.x + (box.width * x),
      y: box.y + (box.height * y),
    }
  }

  async function drag(page, locator, from, to) {
    const box = await locator.boundingBox()
    if (!box) throw new Error('native mask target has no bounding box')
    const start = point(box, from[0], from[1])
    const end = point(box, to[0], to[1])
    if (typeof page.mouse.drag === 'function') {
      await page.mouse.drag(start.x, start.y, end.x, end.y)
    } else {
      await page.mouse.move(start.x, start.y)
      await page.mouse.down()
      await page.mouse.move(end.x, end.y, { steps: 8 })
      await page.mouse.up()
    }
    await sleep(100)
  }

  async function webviewPointerDrag(locator, from, to) {
    await locator.evaluate((node, path) => {
      const box = node.getBoundingClientRect()
      const at = (point) => ({
        x: box.x + (box.width * point[0]),
        y: box.y + (box.height * point[1]),
      })
      const dispatch = (type, point, buttons) => {
        const pos = at(point)
        node.dispatchEvent(new PointerEvent(type, {
          bubbles: true,
          cancelable: true,
          button: type === 'pointerup' ? 0 : 0,
          buttons,
          clientX: pos.x,
          clientY: pos.y,
          pointerId: 71,
          pointerType: 'mouse',
          isPrimary: true,
        }))
      }
      dispatch('pointerdown', path.from, 1)
      for (let step = 1; step <= 8; step++) {
        const progress = step / 8
        dispatch('pointermove', [
          path.from[0] + ((path.to[0] - path.from[0]) * progress),
          path.from[1] + ((path.to[1] - path.from[1]) * progress),
        ], 1)
      }
      dispatch('pointerup', path.to, 0)
    }, { from, to })
    await sleep(100)
  }

  async function clickAt(page, locator, x, y) {
    const box = await locator.boundingBox()
    if (!box) throw new Error('native mask capture has no bounding box')
    const target = point(box, x, y)
    await page.mouse.click(target.x, target.y)
    await sleep(80)
  }

  async function webviewPointerClick(locator, x, y) {
    await locator.evaluate((node, point) => {
      const box = node.getBoundingClientRect()
      const init = {
        bubbles: true,
        cancelable: true,
        button: 0,
        clientX: box.x + (box.width * point.x),
        clientY: box.y + (box.height * point.y),
        pointerId: 73,
        pointerType: 'mouse',
        isPrimary: true,
      }
      node.dispatchEvent(new PointerEvent('pointerdown', { ...init, buttons: 1 }))
      node.dispatchEvent(new PointerEvent('pointerup', { ...init, buttons: 0 }))
    }, { x, y })
    await sleep(80)
  }

  async function chooseButton(page, panel, { name, actionId, selector }) {
    const control = page.locator(selector).first()
    await probe(page, {
      surface, name, actionId,
      sel: control, group: panel, groupName: 'mask-options',
      doClick: async () => { await control.click(); await sleep(90) },
      assertResult: async () => ({
        ok: (await control.getAttribute('aria-pressed')) === 'true',
        detail: `${name} pressed=${await control.getAttribute('aria-pressed')}`,
      }),
    })
  }

  async function fillControl(page, panel, { name, actionId, selector, value }) {
    const control = page.locator(selector).first()
    await probe(page, {
      surface, name, actionId,
      sel: control, group: panel, groupName: 'mask-fields',
      doClick: async () => { await control.fill(String(value)); await sleep(70) },
      assertResult: async () => ({
        ok: (await control.inputValue()) === String(value),
        detail: `${name}=${await control.inputValue()}`,
      }),
    })
  }

  async function apply(page, panel, { verb, name, predicate, clipId }) {
    const control = page.locator('[data-cut-mask-apply]').first()
    let response = null
    let before = 0
    await probe(page, {
      surface, name, actionId: 'mask-apply',
      sel: control, group: panel, groupName: name,
      doClick: async () => {
        before = await opsLen()
        response = await captureVerbResp(page, verb, () => control.click(), 25_000)
      },
      assertResult: async () => {
        const changed = await waitForState((project) => !!findClip(project, clipId)?.mask, 15_000)
        const landed = await opLanded(before, verb, predicate, { timeoutMs: 15_000 })
        return {
          ok: !!response?.ok && !!changed && landed,
          detail: `${verb} ok=${response?.ok}; op=${landed}; mask=${!!findClip(changed, clipId)?.mask}`,
        }
      },
    })
  }

  async function run(page) {
    await freshProject(page, 'mask-actions', primaryMedia)
    await closeOverlays(page)
    const clipId = await clipOfKind('video')
    if (!clipId) throw new Error('Mask action fixture has no video clip')
    let selected = await selectClip(page, clipId)
    if (!selected) {
      // WKWebView/WebKitGTK WebDriver may synthesize click without the React
      // mousedown that owns timeline selection. The public context-menu route
      // selects first, then opens the visible menu; close it before Top bar use.
      const clipControl = page.locator(`[data-cut-clip="${clipId}"]`).first()
      await clipControl.dispatchEvent('contextmenu')
      await sleep(140)
      await page.locator('[data-cut-ctx-backdrop]').first().dispatchEvent('mousedown')
      await sleep(120)
      selected = await clipControl.evaluate((element) => element.classList.contains('tl-clip--selected'))
    }
    if (!selected) {
      const rendered = await page.locator('[data-cut-clip]').evaluateAll((nodes) =>
        nodes.map((node) => ({
          id: node.getAttribute('data-cut-clip'),
          className: node.getAttribute('class'),
        })),
      )
      throw new Error(`Mask action fixture could not select ${clipId} through the rendered timeline; rendered=${JSON.stringify(rendered).slice(0, 500)}`)
    }
    await page.locator('[data-cut-mask-btn]').first().click()
    const panel = page.locator('[data-cut-mask]').first()
    await panel.waitFor({ state: 'visible', timeout: 12_000 })
    const clipNote = page.locator(`[data-cut-mask-clip="${clipId}"]`).first()
    if (!(await clipNote.count())) {
      const note = await panel.textContent()
      throw new Error(`Mask drawer lost selected base clip ${clipId}: ${(note || '').replaceAll(/\s+/g, ' ').slice(0, 240)}`)
    }
    const capture = page.locator('[data-cut-mask-capture]').first()
    await capture.waitFor({ state: 'visible', timeout: 12_000 })
    await sleep(180)

    for (const preset of ['face', 'rectangle', 'plate', 'custom']) {
      await chooseButton(page, panel, {
        name: `mask-preset-${preset}`,
        actionId: 'mask-preset',
        selector: `[data-cut-mask-preset="${preset}"]`,
      })
    }
    for (const shape of ['ellipse', 'polygon', 'rect']) {
      await chooseButton(page, panel, {
        name: `mask-shape-${shape}`,
        actionId: 'mask-shape-kind',
        selector: `[data-cut-mask-shape-kind="${shape}"]`,
      })
    }
    for (const effect of ['black', 'blur', 'pixelate']) {
      await chooseButton(page, panel, {
        name: `mask-effect-${effect}`,
        actionId: 'mask-effect',
        selector: `[data-cut-mask-effect="${effect}"]`,
      })
    }
    await fillControl(page, panel, {
      name: 'mask-strength', actionId: 'mask-strength',
      selector: '[data-cut-mask-strength]', value: 22,
    })
    await fillControl(page, panel, {
      name: 'mask-feather', actionId: 'mask-feather',
      selector: '[data-cut-mask-feather]', value: 0.04,
    })
    const invert = page.locator('[data-cut-mask-invert]').first()
    await probe(page, {
      surface, name: 'mask-invert', actionId: 'mask-invert',
      sel: invert, group: panel, groupName: 'mask-fields',
      doClick: async () => { await invert.click(); await sleep(70) },
      assertResult: async () => ({ ok: await invert.isChecked(), detail: `invert=${await invert.isChecked()}` }),
    })

    let rectangleGesture = 'driver-native'
    await probe(page, {
      surface, name: 'mask-capture-rectangle', actionId: 'mask-capture',
      sel: capture, group: panel, groupName: 'mask-preview-gesture',
      doClick: async () => {
        await drag(page, capture, [0.24, 0.26], [0.72, 0.68])
        if ((await panel.getAttribute('data-cut-mask-ready')) !== 'true') {
          rectangleGesture = 'webview-pointer-fallback'
          await webviewPointerDrag(capture, [0.24, 0.26], [0.72, 0.68])
        }
      },
      assertResult: async () => ({
        ok: (await panel.getAttribute('data-cut-mask-ready')) === 'true',
        detail: `ready=${await panel.getAttribute('data-cut-mask-ready')}; gesture=${rectangleGesture}`,
      }),
    })
    const body = page.locator('[data-cut-mask-body].mk-body').first()
    await webviewPointerDrag(body, [0.5, 0.5], [0.56, 0.58])
    const resize = page.locator('[data-cut-mask-handle="br"]').first()
    await webviewPointerDrag(resize, [0.5, 0.5], [4, 4])
    await apply(page, panel, {
      verb: 'edit.add_mask',
      name: 'mask-apply-whole',
      clipId,
      predicate: (args) => (
        args.clip === clipId
        && args.shape === 'rect'
        && args.effect === 'pixelate'
        && args.strength === 22
        && args.feather === 0.04
        && args.invert === true
        && Array.isArray(args.points)
        && args.points.length === 2
      ),
    })

    const remove = page.locator('[data-cut-mask-remove]').first()
    let removeResponse = null
    let removeBefore = 0
    await probe(page, {
      surface, name: 'mask-remove', actionId: 'mask-remove',
      sel: remove, group: panel, groupName: 'mask-actions',
      doClick: async () => {
        removeBefore = await opsLen()
        removeResponse = await captureVerbResp(page, 'edit.add_mask', () => remove.click(), 20_000)
      },
      assertResult: async () => {
        const changed = await waitForState((project) => !findClip(project, clipId)?.mask, 12_000)
        const landed = await opLanded(
          removeBefore,
          'edit.add_mask',
          (args) => args.clip === clipId && args.enabled === false,
          { timeoutMs: 12_000 },
        )
        return {
          ok: !!removeResponse?.ok && !!changed && landed,
          detail: `remove ok=${removeResponse?.ok}; op=${landed}; mask=${!!findClip(changed, clipId)?.mask}`,
        }
      },
    })

    const clear = page.locator('[data-cut-mask-clear-shape]').first()
    await probe(page, {
      surface, name: 'mask-clear-shape', actionId: 'mask-clear-shape',
      sel: clear, group: panel, groupName: 'mask-actions',
      doClick: async () => { await clear.click(); await sleep(240) },
      assertResult: async () => ({
        ok: (await panel.getAttribute('data-cut-mask-ready')) === 'false' && await clear.isDisabled(),
        detail: `ready=${await panel.getAttribute('data-cut-mask-ready')}; disabled=${await clear.isDisabled()}`,
      }),
    })

    await page.locator('[data-cut-mask-shape-kind="ellipse"]').first().click()
    await sleep(100)
    await webviewPointerDrag(page.locator('[data-cut-mask-capture]').first(), [0.3, 0.3], [0.66, 0.64])
    if ((await panel.getAttribute('data-cut-mask-ready')) !== 'true') {
      throw new Error('Native ellipse drag did not produce ready geometry')
    }
    await page.locator('[data-cut-mask-clear-shape]').first().click()
    await page.locator('[data-cut-mask-shape-kind="polygon"]').first().click()
    await sleep(100)
    const polygonCapture = page.locator('[data-cut-mask-capture]').first()
    let polygonGesture = 'driver-native'
    await probe(page, {
      surface, name: 'mask-capture-polygon', actionId: 'mask-capture',
      sel: polygonCapture, group: panel, groupName: 'mask-preview-gesture',
      doClick: async () => {
        await clickAt(page, polygonCapture, 0.28, 0.3)
        await clickAt(page, polygonCapture, 0.7, 0.34)
        await clickAt(page, polygonCapture, 0.52, 0.72)
        if ((await panel.getAttribute('data-cut-mask-ready')) !== 'true') {
          polygonGesture = 'webview-pointer-fallback'
          await page.locator('[data-cut-mask-shape-kind="rect"]').first().click()
          await page.locator('[data-cut-mask-shape-kind="polygon"]').first().click()
          await sleep(100)
          const fallbackCapture = page.locator('[data-cut-mask-capture]').first()
          await webviewPointerClick(fallbackCapture, 0.28, 0.3)
          await webviewPointerClick(fallbackCapture, 0.7, 0.34)
          await webviewPointerClick(fallbackCapture, 0.52, 0.72)
        }
      },
      assertResult: async () => ({
        ok: (await panel.getAttribute('data-cut-mask-ready')) === 'true',
        detail: `ready=${await panel.getAttribute('data-cut-mask-ready')}; gesture=${polygonGesture}`,
      }),
    })
    await webviewPointerDrag(page.locator('[data-cut-mask-handle="v0"]').first(), [0.5, 0.5], [4, 4])

    for (const mode of ['whole', 'timed']) {
      await chooseButton(page, panel, {
        name: `mask-duration-${mode}`,
        actionId: 'mask-duration-mode',
        selector: `[data-cut-mask-duration-mode="${mode}"]`,
      })
    }
    await fillControl(page, panel, {
      name: 'mask-duration-seconds', actionId: 'mask-duration-seconds-input',
      selector: '[data-cut-mask-duration-seconds-input]', value: 1.5,
    })
    await page.locator('[data-cut-mask-effect="black"]').first().click()
    await sleep(80)
    await apply(page, panel, {
      verb: 'edit.redact',
      name: 'mask-apply-timed',
      clipId,
      predicate: (args) => (
        args.clip === clipId
        && args.shape === 'polygon'
        && args.mode === 'box'
        && args.feather === 0.04
        && args.invert === true
        && Array.isArray(args.points)
        && args.points.length === 3
        && Array.isArray(args.range_ms)
        && args.range_ms[1] - args.range_ms[0] === 1500
      ),
    })

    const project = await state()
    if (!findClip(project, clipId)?.mask) throw new Error('Timed mask did not persist on the clip')
  }

  return { run }
}
