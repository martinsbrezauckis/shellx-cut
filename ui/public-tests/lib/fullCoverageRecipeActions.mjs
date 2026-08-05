// Deterministic installed-WebView coverage for every conditional Recipes
// control. Catalog/describe/dry-run remain real engine reads; only the expensive
// run job and bundled-sample creation boundaries are replaced with closed
// fixtures so the same action sequence runs on every release host.

export function createRecipeActionCoverage({
  probe,
  sleep,
  freshProject,
  closeOverlays,
  primaryMedia,
}) {
  const surface = 'recipe-actions'

  async function waitFor(check, timeoutMs = 10_000) {
    const deadline = Date.now() + timeoutMs
    let value = null
    while (Date.now() < deadline) {
      try {
        value = await check()
        if (value) return value
      } catch {}
      await sleep(80)
    }
    return value
  }

  async function installRunFixture(page) {
    await page.evaluate(() => {
      const target = window
      target.__fcvRecipeRunOriginalFetch = window.fetch
      target.__fcvRecipeRunFixture = {
        runCalls: [],
        statusCalls: [],
        revertCalls: [],
      }
      const fixture = target.__fcvRecipeRunFixture
      const originalFetch = target.__fcvRecipeRunOriginalFetch
      const envelope = (body) => new Response(JSON.stringify(body), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      })
      const requestArgs = (options) => {
        try { return JSON.parse(options?.body || '{}') } catch { return {} }
      }
      window.fetch = async (...args) => {
        const input = args[0]
        const options = args[1]
        const url = typeof input === 'string' ? input : input?.url || ''
        let pathname = ''
        try { pathname = new URL(String(url), window.location.href).pathname } catch {}
        const body = requestArgs(options)
        if (pathname === '/api/verb/recipe.run' && body.policy === 'run') {
          fixture.runCalls.push(body)
          return envelope({ ok: true, result: {
            job_id: 'job_fcv_recipe_run',
            checkpoint: 'checkpoint_fcv_recipe',
            recipe: body.name,
            stages: [
              { id: 'retakes', verb: 'transcript.remove_retakes' },
              { id: 'tighten', verb: 'transcript.remove_silences' },
            ],
          } })
        }
        if (pathname === '/api/verb/jobs.status' && body.job_id === 'job_fcv_recipe_run') {
          fixture.statusCalls.push(body)
          return envelope({ ok: true, result: {
            job_id: body.job_id,
            state: 'done',
            progress: 1,
            result: {
              summary_line: 'Edit for clarity completed with two reviewed timeline changes.',
              recipe: 'edit-for-clarity',
              status: 'completed',
              policy: 'run',
              stages_run: 2,
              stage_results: [
                {
                  id: 'retakes',
                  verb: 'transcript.remove_retakes',
                  ok: true,
                  op_ids: ['op_recipe_1'],
                  gate: { pass: true, state: [] },
                },
                {
                  id: 'tighten',
                  verb: 'transcript.remove_silences',
                  ok: true,
                  op_ids: ['op_recipe_2'],
                  gate: { pass: true, state: [] },
                },
              ],
              changed: { ops: 2, clips_added: 0, clips_removed: 2, duration_delta_ms: -2_400, tracks_touched: ['v1'] },
              checkpoint: 'checkpoint_fcv_recipe',
              receipt_ids: ['receipt_fcv_recipe'],
              restore_hint: 'project.revert to checkpoint_fcv_recipe',
            },
          } })
        }
        if (pathname === '/api/verb/project.revert' && body.to === 'checkpoint_fcv_recipe') {
          fixture.revertCalls.push(body)
          return envelope({ ok: true, result: { reverted_to: body.to, op_ids: ['op_recipe_restore'] } })
        }
        return originalFetch(...args)
      }
    })
  }

  async function runFixtureState(page) {
    return page.evaluate(() => JSON.parse(JSON.stringify(window.__fcvRecipeRunFixture)))
  }

  async function restoreRunFixture(page) {
    await page.evaluate(() => {
      const target = window
      if (target.__fcvRecipeRunOriginalFetch) window.fetch = target.__fcvRecipeRunOriginalFetch
      delete target.__fcvRecipeRunOriginalFetch
      delete target.__fcvRecipeRunFixture
    })
  }

  async function installSampleFixture(page) {
    await page.evaluate(() => {
      const target = window
      target.__fcvRecipeSampleOriginalFetch = window.fetch
      target.__fcvRecipeSampleFixture = {
        created: false,
        imported: false,
        listCalls: [],
        createCalls: [],
        importCalls: [],
        statusCalls: [],
      }
      const fixture = target.__fcvRecipeSampleFixture
      const originalFetch = target.__fcvRecipeSampleOriginalFetch
      const envelope = (body) => new Response(JSON.stringify(body), {
        status: 200,
        headers: { 'content-type': 'application/json' },
      })
      const requestArgs = (options) => {
        try { return JSON.parse(options?.body || '{}') } catch { return {} }
      }
      const sampleProject = () => ({
        schema: 'shellx-cut/1',
        name: 'First edit sample',
        settings: { width: 640, height: 360, fps: 24, audio_rate: 48_000 },
        assets: fixture.imported
          ? { a_sample: { path: '/fixture/first-edit-sample.mp4', hash: 'sha256:sample' } }
          : {},
        tracks: fixture.imported
          ? [{ id: 'v1', kind: 'video', clips: [{ id: 'c_sample', asset: 'a_sample', src_in_ms: 0, src_out_ms: 4_000 }] }]
          : [],
        markers: [],
        caption_styles: {},
        checkpoints: [],
      })
      window.fetch = async (...args) => {
        const input = args[0]
        const options = args[1]
        const url = typeof input === 'string' ? input : input?.url || ''
        let pathname = ''
        try { pathname = new URL(String(url), window.location.href).pathname } catch {}
        const body = requestArgs(options)
        if (pathname === '/api/verb/project.list') {
          fixture.listCalls.push(body)
          return envelope({ ok: true, result: { projects: [] } })
        }
        if (pathname === '/api/verb/project.create') {
          fixture.createCalls.push(body)
          fixture.created = true
          return envelope({ ok: true, result: {
            path: '/fixture/First edit sample.cutproj',
            starter_asset_path: '/fixture/first-edit-sample.mp4',
          } })
        }
        if (pathname === '/api/verb/project.state' && fixture.created) {
          return envelope({ ok: true, result: sampleProject() })
        }
        if (pathname === '/api/verb/project.ops' && fixture.created) {
          return envelope({ ok: true, result: { ops: [] } })
        }
        if (pathname === '/api/verb/media.import' && fixture.created) {
          fixture.importCalls.push(body)
          fixture.imported = true
          return envelope({ ok: true, result: {
            asset_id: 'a_sample',
            job_id: 'job_fcv_recipe_sample_import',
          } })
        }
        if (pathname === '/api/verb/jobs.status' && body.job_id === 'job_fcv_recipe_sample_import') {
          fixture.statusCalls.push(body)
          return envelope({ ok: true, result: {
            job_id: body.job_id,
            state: 'done',
            progress: 1,
            result: { asset_id: 'a_sample' },
          } })
        }
        return originalFetch(...args)
      }
    })
  }

  async function sampleFixtureState(page) {
    return page.evaluate(() => JSON.parse(JSON.stringify(window.__fcvRecipeSampleFixture)))
  }

  async function restoreSampleFixture(page) {
    await page.evaluate(() => {
      const target = window
      if (target.__fcvRecipeSampleOriginalFetch) window.fetch = target.__fcvRecipeSampleOriginalFetch
      delete target.__fcvRecipeSampleOriginalFetch
      delete target.__fcvRecipeSampleFixture
    })
  }

  async function openRecipe(page, name) {
    await page.locator('[data-cut-recipes-btn]').click()
    await page.locator(`[data-cut-recipe="${name}"]`).waitFor({ state: 'visible', timeout: 10_000 })
    await page.locator(`[data-cut-recipe="${name}"]`).click()
    await page.locator(`[data-cut-recipe-detail="${name}"]`).waitFor({ state: 'visible', timeout: 10_000 })
  }

  async function closeCurrentProject(page) {
    const result = await page.evaluate(async () => {
      const response = await fetch('/api/verb/project.close', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: '{}',
      })
      return response.json()
    })
    if (!result?.ok) throw new Error(`project.close before sample fixture=${JSON.stringify(result)}`)
    await page.reload({ waitUntil: 'domcontentloaded' })
    await page.locator('[data-cut-panel="topbar"]').waitFor({ state: 'visible', timeout: 20_000 })
    await sleep(500)
  }

  async function run(page) {
    await freshProject(page, 'recipe_actions', primaryMedia)
    await closeOverlays(page)
    await installRunFixture(page)
    try {
      await openRecipe(page, 'edit-for-clarity')
      const drawer = page.locator('[data-cut-recipes]')
      const intensity = drawer.locator('[data-cut-recipe-param-input="intensity"]')

      await probe(page, {
        surface,
        name: 'change-recipe-parameter',
        actionId: 'recipe-param-input',
        sel: intensity,
        group: drawer.locator('[data-cut-recipe-params]'),
        groupName: 'recipe-parameters',
        doClick: async () => { await intensity.selectOption('jumpy') },
        assertResult: async () => ({
          ok: await intensity.inputValue() === 'jumpy'
            && await drawer.locator('[data-cut-recipe-preview-required]').isVisible(),
          detail: `intensity=${await intensity.inputValue()}; preview required=${await drawer.locator('[data-cut-recipe-preview-required]').isVisible()}`,
        }),
      })

      await probe(page, {
        surface,
        name: 'open-recipe-technical-stages',
        actionId: 'recipe-technical-toggle',
        sel: drawer.locator('[data-cut-recipe-technical-toggle]'),
        group: drawer.locator('[data-cut-recipe-technical]'),
        groupName: 'recipe-technical-stages',
        doClick: async () => {
          await drawer.locator('[data-cut-recipe-technical-toggle]').click()
          await drawer.locator('[data-cut-recipe-technical][open]').waitFor({ state: 'visible', timeout: 5_000 })
        },
        assertResult: async () => ({
          ok: (await drawer.locator('[data-cut-recipe-technical-stage]').count()) >= 2
            && (await drawer.locator('[data-cut-recipe-technical]').textContent())?.includes('transcript.remove_silences'),
          detail: `technical stages=${await drawer.locator('[data-cut-recipe-technical-stage]').count()}`,
        }),
      })

      await drawer.locator('[data-cut-recipe-preview]').click()
      await drawer.locator('[data-cut-recipe-plan-status="planned"]').waitFor({ state: 'visible', timeout: 15_000 })
      const planArgs = drawer.locator('[data-cut-recipe-plan-technical="tighten"]')
      await planArgs.scrollIntoViewIfNeeded()
      await probe(page, {
        surface,
        name: 'open-recipe-plan-technical-args',
        actionId: 'recipe-plan-technical-toggle',
        sel: planArgs.locator('[data-cut-recipe-plan-technical-toggle="tighten"]'),
        group: planArgs,
        groupName: 'recipe-plan-technical-args',
        doClick: async () => {
          await planArgs.locator('[data-cut-recipe-plan-technical-toggle="tighten"]').click()
          await drawer.locator('[data-cut-recipe-plan-technical="tighten"][open]').waitFor({ state: 'visible', timeout: 5_000 })
        },
        assertResult: async () => ({
          ok: (await planArgs.textContent())?.includes('aggressiveness: jumpy'),
          detail: `resolved args="${(await planArgs.textContent())?.replace(/\s+/g, ' ').trim()}"`,
        }),
      })

      await probe(page, {
        surface,
        name: 'run-recipe',
        actionId: 'recipe-run',
        sel: drawer.locator('[data-cut-recipe-run]'),
        group: drawer.locator('.rc-actions'),
        groupName: 'recipe-run-action',
        doClick: async () => {
          await drawer.locator('[data-cut-recipe-run]').click()
          await drawer.locator('[data-cut-recipe-report]').waitFor({ state: 'visible', timeout: 8_000 })
        },
        assertResult: async () => {
          const fixture = await runFixtureState(page)
          const call = fixture.runCalls[0]
          const exact = call?.name === 'edit-for-clarity'
            && call?.policy === 'run'
            && call?.args?.intensity === 'jumpy'
            && call?.rationale === 'human: run recipe edit-for-clarity'
          return {
            ok: exact
              && await drawer.locator('[data-cut-recipe-status="completed"]').isVisible()
              && await drawer.locator('[data-cut-recipe-result]').count() === 2,
            detail: `exact run=${exact}; results=${await drawer.locator('[data-cut-recipe-result]').count()}; args=${JSON.stringify(call)}`,
          }
        },
      })

      const recipeReport = drawer.locator('[data-cut-recipe-report]')
      await recipeReport.scrollIntoViewIfNeeded()
      await probe(page, {
        surface,
        name: 'restore-recipe-run',
        actionId: 'recipe-restore',
        sel: drawer.locator('[data-cut-recipe-restore]'),
        group: recipeReport,
        groupName: 'recipe-report-restore',
        doClick: async () => {
          await drawer.locator('[data-cut-recipe-restore]').click()
          await drawer.locator('[data-cut-recipe-restored]').waitFor({ state: 'visible', timeout: 5_000 })
        },
        assertResult: async () => {
          const fixture = await runFixtureState(page)
          const call = fixture.revertCalls[0]
          const exact = call?.to === 'checkpoint_fcv_recipe' && call?.rationale === 'undo recipe run'
          return { ok: exact, detail: `exact restore=${exact}; args=${JSON.stringify(call)}` }
        },
      })

      await probe(page, {
        surface,
        name: 'inspect-recipe-run',
        actionId: 'recipe-inspect',
        sel: drawer.locator('[data-cut-recipe-inspect]'),
        group: drawer.locator('[data-cut-recipe-report]'),
        groupName: 'recipe-report-inspect',
        doClick: async () => {
          await drawer.locator('[data-cut-recipe-inspect]').click()
          await page.locator('[data-cut-review-tab="receipts"][aria-selected="true"]')
            .waitFor({ state: 'visible', timeout: 8_000 })
        },
        assertResult: async () => ({
          ok: await page.locator('[data-cut-panel="review"]').count() === 1
            && await page.locator('[data-cut-review-tab="receipts"]').getAttribute('aria-selected') === 'true',
          detail: `Review mounted=${await page.locator('[data-cut-panel="review"]').count()}; Receipts selected=${await page.locator('[data-cut-review-tab="receipts"]').getAttribute('aria-selected')}`,
        }),
      })

      await probe(page, {
        surface,
        name: 'back-to-recipe-list',
        actionId: 'recipes-back',
        sel: drawer.locator('[data-cut-recipes-back]'),
        group: drawer,
        groupName: 'recipe-detail-back',
        doClick: async () => {
          await drawer.locator('[data-cut-recipes-back]').click()
          await drawer.locator('[data-cut-recipes-list]').waitFor({ state: 'visible', timeout: 5_000 })
        },
        assertResult: async () => ({
          ok: await drawer.getAttribute('data-cut-recipes-view') === 'list'
            && await drawer.locator('[data-cut-recipe="first-project"]').isVisible(),
          detail: `view=${await drawer.getAttribute('data-cut-recipes-view')}; first-project visible=${await drawer.locator('[data-cut-recipe="first-project"]').isVisible()}`,
        }),
      })
      await drawer.locator('[data-cut-recipes-close]').click()
    } finally {
      await restoreRunFixture(page)
    }

    await closeCurrentProject(page)
    await installSampleFixture(page)
    try {
      await openRecipe(page, 'first-project')
      const drawer = page.locator('[data-cut-recipes]')
      const noProject = drawer.locator('[data-cut-recipes-noproject]')
      await noProject.waitFor({ state: 'visible', timeout: 5_000 })
      await probe(page, {
        surface,
        name: 'open-first-edit-sample',
        actionId: 'recipe-sample',
        sel: drawer.locator('[data-cut-recipe-sample]'),
        group: noProject,
        groupName: 'recipe-first-edit-sample',
        doClick: async () => {
          await drawer.locator('[data-cut-recipe-sample]').click()
          await noProject.waitFor({ state: 'detached', timeout: 8_000 })
          await waitFor(async () =>
            (await drawer.locator('[data-cut-recipe-param-input="asset"]').inputValue()) === 'a_sample')
        },
        assertResult: async () => {
          const fixture = await sampleFixtureState(page)
          const create = fixture.createCalls[0]
          const imported = fixture.importCalls[0]
          const exactCreate = create?.name === 'First edit sample'
            && create?.starter === 'first-edit'
            && create?.settings?.width === 640
            && create?.settings?.height === 360
            && create?.settings?.fps === 24
            && Object.keys(create.settings).length === 3
          const exactImport = imported?.path === '/fixture/first-edit-sample.mp4'
            && imported?.proxy === false
            && imported?.rationale === 'guided First edit sample'
          return {
            ok: exactCreate && exactImport
              && fixture.statusCalls.length === 1
              && await drawer.locator('[data-cut-recipe-param-input="asset"]').inputValue() === 'a_sample',
            detail: `exact create=${exactCreate}; exact import=${exactImport}; status calls=${fixture.statusCalls.length}; selected asset=${await drawer.locator('[data-cut-recipe-param-input="asset"]').inputValue()}`,
          }
        },
      })
      await drawer.locator('[data-cut-recipes-close]').click()
    } finally {
      await restoreSampleFixture(page)
      await freshProject(page, 'recipe_actions_after_sample', primaryMedia)
      await closeOverlays(page)
    }
  }

  return { run }
}
