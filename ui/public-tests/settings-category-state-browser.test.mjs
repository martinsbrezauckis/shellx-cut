import assert from 'node:assert/strict'
import { mkdtemp, readFile, rm } from 'node:fs/promises'
import { createRequire } from 'node:module'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import test from 'node:test'

const uiRoot = resolve(import.meta.dirname, '..')
const requireFromUi = createRequire(join(uiRoot, 'package.json'))
const entryPath = '/__settings_category_regression__.tsx'
// A component-level source check: no engine, project, Doctor or native state is
// supplied. The real observable-state builder reports the parent's category.
const entry = `
import React, { useState } from 'react'
import { createRoot } from 'react-dom/client'
import { EnvironmentPanel } from '/src/panels/Environment/index.tsx'
import { createUiObservableState, readUiDomState } from '/src/app/uiControlState.ts'
import { LAYOUT_DEFAULTS } from '/src/layout/useLayout.ts'
function Fixture() {
  const [category, setCategory] = useState('overview')
  window.navigateSettingsFromParent = setCategory
  const state = createUiObservableState({ revision: 1, layout: LAYOUT_DEFAULTS,
    generateTab: 'agent', wizardOpen: false, envOpen: true, envCategory: category,
    commentsOpen: false, activeDrawer: null, highlight: null, playheadMs: 0,
    selectedClipIds: [], exportRange: null, project: null, dom: readUiDomState() })
  window.reportedSettingsCategory = state.overlays.settings
  window.categoryChangeCount ||= 0
  return <EnvironmentPanel report={null} onRefresh={async () => null} onClose={() => {}}
    initialCategory={category} onCategoryChange={next => { window.categoryChangeCount++; setCategory(next) }} />
}
createRoot(document.getElementById('root')).render(<Fixture />)
`

test('ordinary Settings navigation reports the rendered category to its parent without an effect loop', {
  skip: process.env.SHELLX_CUT_RUN_SETTINGS_CATEGORY_BROWSER_TEST !== '1'
    ? 'set SHELLX_CUT_RUN_SETTINGS_CATEGORY_BROWSER_TEST=1 for the local React browser regression'
    : false,
  timeout: 120_000,
}, async () => {
  // This ties the exercised component callback to the actual App publisher.
  const app = await readFile(join(uiRoot, 'src/App.tsx'), 'utf8')
  assert.match(app, /<EnvironmentPanel[\s\S]*?initialCategory=\{envCategory\}[\s\S]*?onCategoryChange=\{setEnvCategory\}/)
  const { createServer } = requireFromUi('vite')
  const { chromium } = requireFromUi('playwright')
  const cacheDir = await mkdtemp(join(tmpdir(), 'cut-settings-category-'))
  const server = await createServer({ root: uiRoot, cacheDir, server: { host: '127.0.0.1', port: 0 }, plugins: [{
    name: 'settings-category-regression-entry',
    resolveId(id) { if (id === entryPath) return join(uiRoot, entryPath) },
    load(id) { if (id === join(uiRoot, entryPath)) return entry },
    configureServer(dev) {
      dev.middlewares.use(async (request, response, next) => {
        if (request.url !== '/__settings_category_regression__') return next()
        response.setHeader('content-type', 'text/html')
        response.end(await dev.transformIndexHtml(request.url,
          `<div id="root"></div><script type="module" src="${entryPath}"></script>`))
      })
    },
  }] })
  let browser
  try {
    await server.listen()
    browser = await chromium.launch({ headless: true })
    const page = await browser.newPage({ viewport: { width: 1440, height: 980 } })
    const errors = []
    page.on('pageerror', error => errors.push(error.message))
    await page.goto(`${server.resolvedUrls.local[0]}__settings_category_regression__`)
    const agrees = async category => {
      await page.locator(`[data-cut-settings-body="${category}"]`).waitFor({ state: 'visible' })
      await page.waitForFunction(expected => window.reportedSettingsCategory === expected, category)
    }
    await agrees('overview')
    assert.equal(await page.evaluate(() => window.categoryChangeCount), 0)
    await page.locator('[data-cut-settings-category="ai-transcription"]').click()
    await agrees('ai-transcription')
    await page.setViewportSize({ width: 1100, height: 980 })
    await page.locator('[data-cut-settings-category-select]').selectOption('overview')
    await agrees('overview')
    await page.locator('[data-cut-settings-search]').fill('keyboard')
    await page.locator('[data-cut-settings-search-result="editing"]').click()
    await agrees('editing')
    await page.locator('[data-cut-settings-category-select]').selectOption('overview')
    await agrees('overview')
    await page.locator('[data-cut-settings-overview-row="ai"] button').click()
    await agrees('ai-transcription')
    const count = await page.evaluate(() => window.categoryChangeCount)
    assert.equal(count, 5, 'each real navigation reports once')
    await page.evaluate(() => window.navigateSettingsFromParent('editing'))
    await agrees('editing')
    assert.equal(await page.evaluate(() => window.categoryChangeCount), count,
      'parent/ui.open category synchronization does not echo a navigation callback')
    assert.deepEqual(errors, [])
  } finally {
    await browser?.close()
    await server.close()
    await rm(cacheDir, { recursive: true, force: true })
  }
})
