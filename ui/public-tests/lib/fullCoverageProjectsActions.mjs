// Native action coverage for Projects-list navigation and registry hygiene.
// Every destructive fixture is created in the harness-managed projects root,
// verified by name/path, and removed before the UI forgets its registry entry.

import { rmSync } from 'node:fs'
import { basename } from 'node:path'

export function createProjectsActionCoverage({
  probe,
  verb,
  captureVerbResp,
  resolveDriverPath,
  sleep,
}) {
  async function createFixtures() {
    const fixtures = []
    for (const suffix of ['searchA', 'missingB']) {
      const name = `fcv_${suffix}_${Math.random().toString(36).slice(2, 6)}`
      const created = await verb('project.create', {
        name,
        settings: { width: 1280, height: 720, fps: 30 },
      })
      fixtures.push({ name, path: created.result?.path || '', ok: created.ok })
    }
    return fixtures
  }

  async function refresh(page) {
    await page.locator('[data-cut-left-tab="transcript"]').click().catch(() => {})
    await sleep(180)
    await page.locator('[data-cut-left-tab="projects"]').click()
    await page.locator('[data-cut-panel="projects"]').waitFor({ state: 'visible', timeout: 12_000 })
    await sleep(300)
  }

  function removeFixture(fixture) {
    const path = resolveDriverPath(fixture.path)
    const leaf = basename(path)
    if (!fixture.name || !path || !leaf.startsWith(fixture.name) || !leaf.endsWith('.cutproj')) {
      throw new Error(`refusing to remove unverified project fixture: ${path || '(empty)'}`)
    }
    rmSync(path, { recursive: true })
  }

  async function run(page, { fixtures, projectRows }) {
    const surface = 'projects'
    await refresh(page)
    let panel = page.locator('[data-cut-panel="projects"]').first()
    const [forgetFixture, missingFixture] = fixtures
    const forgetRow = projectRows.find((row) => row.name === forgetFixture?.name)
    const missingRow = projectRows.find((row) => row.name === missingFixture?.name)

    const search = page.locator('[data-cut-projects-search]')
    await probe(page, {
      surface, name: 'projects-search', actionId: 'projects-search',
      sel: search, group: panel, groupName: 'projects-panel',
      doClick: async () => {
        await search.fill(forgetFixture.name)
        await sleep(180)
      },
      assertResult: async () => {
        const visible = await page.locator('[data-cut-project-card]').count()
        const found = forgetRow
          ? (await page.locator(`[data-cut-project-card="${forgetRow.id}"]`).count()) === 1
          : false
        return { ok: found && visible === 1, detail: `search result cards=${visible} exact=${found}` }
      },
    })
    await search.fill('')
    await sleep(180)

    if (forgetRow && forgetFixture.path) removeFixture(forgetFixture)
    const forget = page.locator(`[data-cut-project-forget="${forgetRow?.id || 'missing-fixture'}"]`)
    let forgetResponse = null
    await probe(page, {
      surface, name: 'project-forget', actionId: 'project-forget',
      sel: forget, group: panel, groupName: 'projects-panel',
      doClick: async () => {
        forgetResponse = await captureVerbResp(page, 'project.forget', () => forget.click(), 12_000)
        await sleep(350)
      },
      assertResult: async () => ({
        ok: !!forgetResponse?.ok
          && forgetResponse?.result?.forgotten === true
          && (await page.locator(`[data-cut-project-card="${forgetRow?.id}"]`).count()) === 0,
        detail: `forgotten=${forgetResponse?.result?.forgotten === true}; files fixture already removed`,
      }),
    })

    if (missingRow && missingFixture.path) removeFixture(missingFixture)
    await refresh(page)
    panel = page.locator('[data-cut-panel="projects"]').first()
    const clearMissing = page.locator('[data-cut-projects-clear-missing]')
    let clearResponse = null
    await probe(page, {
      surface, name: 'projects-clear-missing', actionId: 'projects-clear-missing',
      sel: clearMissing, group: panel, groupName: 'projects-panel',
      doClick: async () => {
        clearResponse = await captureVerbResp(page, 'project.forget', () => clearMissing.click(), 12_000)
        await sleep(350)
      },
      assertResult: async () => ({
        ok: !!clearResponse?.ok
          && (await page.locator(`[data-cut-project-card="${missingRow?.id}"]`).count()) === 0
          && (await page.locator('[data-cut-projects-clear-missing]').count()) === 0,
        detail: `bulk missing cleanup ok=${clearResponse?.ok}`,
      }),
    })
  }

  return { createFixtures, run }
}
