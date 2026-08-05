// Native action coverage for the Generate Templates / Prompt / Storyboard
// workspace. Catalog filters drive generate.list; manifest controls exercise
// every rendered field type; prompt/storyboard controls persist through their
// shared workspace state. Storyboard question coverage intentionally asks for
// a director brief first so a deterministic adapter can expose the answer UI.

export function createGenerateTemplateActionCoverage({
  probe,
  captureVerbResp,
  sleep,
}) {
  const surface = 'generate'

  async function fill(page, panel, {
    name,
    actionId,
    selector,
    value,
    groupName,
  }) {
    const control = page.locator(selector).first()
    await control.waitFor({ state: 'visible', timeout: 12_000 })
    await probe(page, {
      surface,
      name,
      actionId,
      sel: control,
      group: panel,
      groupName,
      doClick: async () => { await control.fill(String(value)) },
      assertResult: async () => ({
        ok: await control.inputValue() === String(value),
        detail: `value=${await control.inputValue()}`,
      }),
    })
  }

  async function select(page, panel, {
    name,
    actionId,
    selector,
    value,
    groupName,
  }) {
    const control = page.locator(selector).first()
    await control.waitFor({ state: 'visible', timeout: 12_000 })
    await probe(page, {
      surface,
      name,
      actionId,
      sel: control,
      group: panel,
      groupName,
      doClick: async () => { await control.selectOption(value) },
      assertResult: async () => ({
        ok: await control.inputValue() === value,
        detail: `value=${await control.inputValue()}`,
      }),
    })
  }

  async function selectTemplate(page, id, expectedParam) {
    const card = page.locator(`[data-cut-generate-template-id="${id}"]`).first()
    await card.waitFor({ state: 'visible', timeout: 12_000 })
    await card.click()
    await page.locator(`[data-cut-generate-param="${expectedParam}"]`).first()
      .waitFor({ state: 'visible', timeout: 12_000 })
  }

  async function runCatalog(page, panel) {
    const query = page.locator('[data-cut-generate-template-search]').first()
    let response = null
    await probe(page, {
      surface,
      name: 'template-search',
      actionId: 'generate-template-search',
      sel: query,
      group: panel,
      groupName: 'generate-templates-panel',
      doClick: async () => {
        response = await captureVerbResp(
          page,
          'generate.list',
          () => query.fill('kinetic'),
          20_000,
        )
      },
      assertResult: async () => {
        const cards = await page.locator('[data-cut-generate-template-card]').count()
        return {
          ok: response?.ok && await query.inputValue() === 'kinetic' && cards > 0,
          detail: `ok=${response?.ok} query=${await query.inputValue()} cards=${cards}`,
        }
      },
    })
    await captureVerbResp(page, 'generate.list', () => query.fill(''), 20_000)

    for (const kind of ['title', 'caption', 'shape', 'motion', 'social', 'batch', 'all']) {
      const control = page.locator(`[data-cut-generate-kind="${kind}"]`).first()
      response = null
      await probe(page, {
        surface,
        name: `template-kind-${kind}`,
        actionId: 'generate-kind',
        sel: control,
        group: panel,
        groupName: 'generate-templates-panel',
        doClick: async () => {
          response = await captureVerbResp(
            page,
            'generate.list',
            () => control.click(),
            20_000,
          )
        },
        assertResult: async () => {
          const templates = response?.result?.templates || []
          const correctKind = kind === 'all' || templates.every((template) => template.kind === kind)
          return {
            ok: response?.ok
              && templates.length > 0
              && correctKind
              && await control.getAttribute('aria-selected') === 'true',
            detail: `ok=${response?.ok} templates=${templates.length} correctKind=${correctKind} selected=${await control.getAttribute('aria-selected')}`,
          }
        },
      })
    }
    await page.locator('[data-cut-generate-template-id="builtin.lower-third.clean"]').first()
      .waitFor({ state: 'visible', timeout: 12_000 })
  }

  async function runManifestControls(page, panel) {
    await selectTemplate(page, 'builtin.lower-third.clean', 'name')
    await fill(page, panel, {
      name: 'param-string-name',
      actionId: 'generate-param-control',
      selector: '[data-cut-generate-param-control="name"]',
      value: 'FCV Generate',
      groupName: 'generate-templates-panel',
    })
    await fill(page, panel, {
      name: 'param-color-text-accent',
      actionId: 'generate-param-text',
      selector: '[data-cut-generate-param-text="accent"]',
      value: '#33CC99',
      groupName: 'generate-templates-panel',
    })
    await fill(page, panel, {
      name: 'param-color-picker-accent',
      actionId: 'generate-param-control',
      selector: '[data-cut-generate-param-control="accent"]',
      value: '#22aaff',
      groupName: 'generate-templates-panel',
    })
    await fill(page, panel, {
      name: 'param-integer-duration',
      actionId: 'generate-param-control',
      selector: '[data-cut-generate-param-control="duration_ms"]',
      value: 4200,
      groupName: 'generate-templates-panel',
    })
    await fill(page, panel, {
      name: 'template-at-ms',
      actionId: 'generate-at-ms',
      selector: '[data-cut-generate-at-ms]',
      value: 1350,
      groupName: 'generate-templates-panel',
    })

    await selectTemplate(page, 'builtin.caption.kinetic-yellow', 'position')
    await select(page, panel, {
      name: 'param-enum-position',
      actionId: 'generate-param-control',
      selector: '[data-cut-generate-param-control="position"]',
      value: 'top',
      groupName: 'generate-templates-panel',
    })
    await fill(page, panel, {
      name: 'param-integer-font',
      actionId: 'generate-param-control',
      selector: '[data-cut-generate-param-control="font_px"]',
      value: 72,
      groupName: 'generate-templates-panel',
    })
    const checkbox = page.locator('[data-cut-generate-param-control="replace_static"]').first()
    await probe(page, {
      surface,
      name: 'param-boolean-replace-static',
      actionId: 'generate-param-control',
      sel: checkbox,
      group: panel,
      groupName: 'generate-templates-panel',
      doClick: async () => {
        if (await checkbox.isChecked()) await checkbox.click()
      },
      assertResult: async () => ({
        ok: !await checkbox.isChecked(),
        detail: `checked=${await checkbox.isChecked()}`,
      }),
    })

    // Return to the deterministic title fixture used by the caller's
    // preview/insert effect checks.
    await selectTemplate(page, 'builtin.lower-third.clean', 'name')
    await page.locator('[data-cut-generate-param-control="name"]').fill('FCV Generate')
    await page.locator('[data-cut-generate-param-text="accent"]').fill('#33CC99')
  }

  async function runPromptControls(page, panel) {
    const agent = page.locator('[data-cut-generate-prompt-agent]').first()
    for (const value of ['claude', 'codex', 'grok', 'auto']) {
      await select(page, panel, {
        name: `prompt-agent-${value}`,
        actionId: 'generate-prompt-agent',
        selector: '[data-cut-generate-prompt-agent]',
        value,
        groupName: 'generate-prompt-panel',
      })
    }
    if (await agent.inputValue() !== 'auto') await agent.selectOption('auto')
    await fill(page, panel, {
      name: 'prompt-at-ms',
      actionId: 'generate-prompt-at-ms',
      selector: '[data-cut-generate-prompt-at-ms]',
      value: 1750,
      groupName: 'generate-prompt-panel',
    })
  }

  async function runStoryboardControls(page, panel) {
    await fill(page, panel, {
      name: 'storyboard-at-ms',
      actionId: 'generate-storyboard-at-ms',
      selector: '[data-cut-generate-storyboard-at-ms]',
      value: 2250,
      groupName: 'generate-storyboard-panel',
    })
  }

  async function runStoryboardQuestion(page, panel) {
    await page.locator('[data-cut-generate-storyboard-input]').fill('Plan a concise launch video.')
    await page.locator('[data-cut-generate-storyboard-mode]').selectOption('director_brief')
    await page.locator('[data-cut-generate-storyboard-agent]').selectOption('auto')
    const response = await captureVerbResp(
      page,
      'generate.storyboard',
      () => page.locator('[data-cut-generate-storyboard-plan]').click(),
      200_000,
    )
    const answer = page.locator('[data-cut-generate-storyboard-answer]').first()
    if (!response?.ok || await answer.count() === 0) {
      return {
        covered: false,
        detail: `status=${response?.result?.status || 'none'} questions=${response?.result?.questions?.length ?? 0}`,
      }
    }

    const questionId = await answer.getAttribute('data-cut-generate-storyboard-answer')
    const tag = await answer.evaluate((element) => element.tagName.toLowerCase())
    let chosen = ''
    await probe(page, {
      surface,
      name: `storyboard-answer-${tag}`,
      actionId: 'generate-storyboard-answer',
      sel: answer,
      group: panel,
      groupName: 'generate-storyboard-panel',
      doClick: async () => {
        if (tag === 'select') {
          const values = await answer.locator('option').evaluateAll((options) =>
            options.map((option) => option.value).filter(Boolean),
          )
          chosen = values[0] || ''
          await answer.selectOption(chosen)
        } else {
          chosen = 'new customers'
          await answer.fill(chosen)
        }
      },
      assertResult: async () => ({
        ok: chosen.length > 0 && await answer.inputValue() === chosen,
        detail: `question=${questionId || 'unknown'} type=${tag} answer=${await answer.inputValue()}`,
      }),
    })
    await sleep(50)
    return { covered: true, detail: `question=${questionId || 'unknown'} type=${tag}` }
  }

  return {
    runCatalog,
    runManifestControls,
    runPromptControls,
    runStoryboardControls,
    runStoryboardQuestion,
  }
}
