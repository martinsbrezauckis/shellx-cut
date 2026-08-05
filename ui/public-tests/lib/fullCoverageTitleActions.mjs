// Direct native coverage for every configurable action in the Title drawer.
// The three placement modes lower different title.add payloads, so each mode
// creates a real title. Shared pickers are also driven through every option.

export function createTitleActionCoverage({
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
  const surface = 'title-actions'

  function findTitle(project, clipId) {
    for (const track of project?.tracks || []) {
      const clip = (track.clips || []).find((candidate) => candidate.id === clipId)
      if (clip) return clip
    }
    return null
  }

  async function chooseButton(page, panel, { name, actionId, selector, result }) {
    const control = page.locator(selector).first()
    await probe(page, {
      surface, name, actionId,
      sel: control, group: panel, groupName: 'title-placement',
      doClick: async () => { await control.click(); await sleep(70) },
      assertResult: result,
    })
  }

  async function fillControl(page, panel, { name, actionId, selector, value }) {
    const control = page.locator(selector).first()
    await probe(page, {
      surface, name, actionId,
      sel: control, group: panel, groupName: 'title-fields',
      doClick: async () => { await control.fill(String(value)); await sleep(60) },
      assertResult: async () => ({
        ok: (await control.inputValue()) === String(value),
        detail: `${name}=${await control.inputValue()}`,
      }),
    })
  }

  async function selectControl(page, panel, { name, actionId, selector, value }) {
    const control = page.locator(selector).first()
    await probe(page, {
      surface, name, actionId,
      sel: control, group: panel, groupName: 'title-fields',
      doClick: async () => { await control.selectOption(value); await sleep(60) },
      assertResult: async () => ({
        ok: (await control.inputValue()) === value,
        detail: `${name}=${await control.inputValue()}`,
      }),
    })
  }

  async function applyTitle(page, panel, name, expected) {
    const control = page.locator('[data-cut-title-apply]').first()
    let response = null
    let before = 0
    await probe(page, {
      surface,
      name,
      actionId: 'title-apply',
      sel: control,
      group: panel,
      groupName: name,
      doClick: async () => {
        before = await opsLen()
        response = await captureVerbResp(page, 'title.add', () => control.click(), 30_000)
      },
      assertResult: async () => {
        const clipId = response?.result?.clip_id
        const changed = clipId
          ? await waitForState((project) => !!findTitle(project, clipId)?.title_text, 20_000)
          : null
        const landed = await opLanded(
          before,
          'title.add',
          expected,
          { timeoutMs: 20_000 },
        )
        return {
          ok: !!response?.ok && !!changed && landed,
          detail: `title.add ok=${response?.ok}; op=${landed}; clip=${clipId || 'none'}; text=${findTitle(changed, clipId)?.title_text || 'none'}`,
        }
      },
    })
  }

  async function setMode(page, panel, mode) {
    const selector = `[data-cut-title-mode="${mode}"]`
    await chooseButton(page, panel, {
      name: `title-mode-${mode}`,
      actionId: 'title-mode',
      selector,
      result: async () => ({
        ok: (await page.locator(selector).first().getAttribute('aria-selected')) === 'true',
        detail: `title mode ${mode} selected`,
      }),
    })
  }

  async function run(page) {
    await freshProject(page, 'title-actions')
    await closeOverlays(page)
    await page.locator('[data-cut-title-btn]').first().click()
    const panel = page.locator('[data-cut-title]').first()
    await panel.waitFor({ state: 'visible', timeout: 12_000 })
    await sleep(180)

    await setMode(page, panel, 'preset')
    for (const preset of ['lower_third', 'title_card']) {
      await selectControl(page, panel, {
        name: `title-preset-${preset}`,
        actionId: 'title-preset',
        selector: '[data-cut-title-preset]',
        value: preset,
      })
    }
    await fillControl(page, panel, {
      name: 'title-in-preset', actionId: 'title-in',
      selector: '[data-cut-title-in]', value: 0.2,
    })
    await fillControl(page, panel, {
      name: 'title-out-preset', actionId: 'title-out',
      selector: '[data-cut-title-out]', value: 1.3,
    })
    await page.locator('[data-cut-title-text]').fill('FCV preset')
    await applyTitle(page, panel, 'title-apply-preset', (args) => (
      args.text === 'FCV preset'
      && args.preset === 'title_card'
      && args.range_ms?.[0] === 200
      && args.range_ms?.[1] === 1300
      && args.template === undefined
      && args.x === undefined
    ))

    await setMode(page, panel, 'animated')
    for (const template of [
      'typewriter',
      'word_pop',
      'slide_stack',
      'lower_third_reveal',
      'caption_karaoke',
      'kinetic_emphasis',
    ]) {
      await selectControl(page, panel, {
        name: `title-template-${template}`,
        actionId: 'title-template',
        selector: '[data-cut-title-template]',
        value: template,
      })
    }
    await fillControl(page, panel, {
      name: 'title-accent', actionId: 'title-accent',
      selector: '[data-cut-title-accent]', value: '#12AB34',
    })
    await fillControl(page, panel, {
      name: 'title-emphasis', actionId: 'title-emphasis',
      selector: '[data-cut-title-emphasis]', value: 'MOMENT',
    })
    await page.locator('[data-cut-title-text]').fill('FCV animated MOMENT')
    await applyTitle(page, panel, 'title-apply-animated', (args) => (
      args.text === 'FCV animated MOMENT'
      && args.template === 'kinetic_emphasis'
      && args.accent === '#12AB34'
      && args.emphasis === 'MOMENT'
      && args.range_ms?.[0] === 200
      && args.range_ms?.[1] === 1300
      && args.preset === undefined
    ))

    await setMode(page, panel, 'free')
    const pad = page.locator('[data-cut-title-pad]').first()
    const beforePad = await page.locator('[data-cut-title-pos]').textContent()
    await probe(page, {
      surface, name: 'title-pad-pointer', actionId: 'title-pad',
      sel: pad, group: panel, groupName: 'title-free-placement',
      doClick: async () => { await pad.click(); await sleep(80) },
      assertResult: async () => {
        const next = await page.locator('[data-cut-title-pos]').textContent()
        return { ok: next !== beforePad && /x 0\.50 · y 0\.50/.test(next || ''), detail: `position=${next}` }
      },
    })
    for (const anchor of ['tl', 'tc', 'tr', 'ml', 'mc', 'mr', 'bl', 'bc', 'br']) {
      const selector = `[data-cut-title-anchor="${anchor}"]`
      await chooseButton(page, panel, {
        name: `title-anchor-${anchor}`,
        actionId: 'title-anchor',
        selector,
        result: async () => ({
          ok: (await page.locator('[data-cut-title-pos]').textContent() || '').includes(
            anchor === 'tl' || anchor === 'ml' || anchor === 'bl'
              ? 'x 0.10'
              : anchor === 'tc' || anchor === 'mc' || anchor === 'bc'
                ? 'x 0.50'
                : 'x 0.90',
          ),
          detail: `anchor ${anchor} -> ${await page.locator('[data-cut-title-pos]').textContent()}`,
        }),
      })
    }
    for (const align of ['center', 'right', 'left']) {
      await selectControl(page, panel, {
        name: `title-align-${align}`,
        actionId: 'title-align',
        selector: '[data-cut-title-align]',
        value: align,
      })
    }
    await page.locator('[data-cut-title-anchor="ml"]').click()
    await page.locator('[data-cut-title-text]').fill('FCV free')
    await applyTitle(page, panel, 'title-apply-free', (args) => (
      args.text === 'FCV free'
      && args.x === 0.1
      && args.y === 0.5
      && args.align === 'left'
      && args.range_ms?.[0] === 200
      && args.range_ms?.[1] === 1300
      && args.preset === undefined
      && args.template === undefined
    ))

    const project = await state()
    const titleCount = project.tracks
      .flatMap((track) => track.clips || [])
      .filter((clip) => clip.title_text?.startsWith('FCV '))
      .length
    if (titleCount < 3) throw new Error(`Title action fixture persisted only ${titleCount} tested titles`)
  }

  return { run }
}
