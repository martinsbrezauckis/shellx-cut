import assert from 'node:assert/strict'
import { mkdtemp, rm } from 'node:fs/promises'
import { createRequire } from 'node:module'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import test from 'node:test'

const uiRoot = resolve(import.meta.dirname, '..')
const requireFromUi = createRequire(join(uiRoot, 'package.json'))
const entryPath = '/__right_body_regression__.tsx'
const entry = `
import React, { useCallback, useRef, useState } from 'react'
import { createRoot } from 'react-dom/client'
import AppRightRail from '/src/app/AppRightRail.tsx'
import { LAYOUT_DEFAULTS } from '/src/layout/useLayout.ts'
import { useUiStatePublisher } from '/src/app/useUiStatePublisher.ts'
import { useUiCommandController } from '/src/app/useUiCommandController.ts'
import { events } from '/src/lib/events.ts'

window.answers = []
events.answerUiCommand = result => window.answers.push(result)
window.command = panel => {
  const id = (window.nextCommandId = (window.nextCommandId || 0) + 1)
  for (const listener of events.listeners) listener({ type: 'ui_command', verb: 'ui.open', request_id: id, args: { panel } })
  return id
}

function Fixture() {
  const [layout, setLayout] = useState({ ...LAYOUT_DEFAULTS, railCollapsed: false })
  const [mode, setMode] = useState('real')
  window.setBodyMode = setMode
  window.selectRightTab = tab => setLayout(current => ({ ...current, rightTab: tab }))
  const [playheadMs, setPlayheadMs] = useState(0)
  const [selectedClipIds, setSelectedClipIds] = useState([])
  const [highlight, setHighlight] = useState(null)
  const highlightNonce = useRef(0)
  const stateRef = useUiStatePublisher({ layout, generateTab: 'templates', wizardOpen: false,
    envOpen: false, envCategory: 'overview', commentsOpen: false, activeDrawer: null,
    highlight, playheadMs, selectedClipIds, exportRange: null, project: null })
  const openSurface = useCallback(id => {
    if (!['properties', 'color', 'audio', 'chat'].includes(id)) return false
    setLayout(current => ({ ...current, workspaceMode: 'edit', railCollapsed: false, rightTab: id }))
    return true
  }, [])
  useUiCommandController({ stateRef, project: null, setPlayheadMs, setSelectedClipIds,
    setHighlight, highlightNonce, openSurface })
  window.currentState = stateRef
  if (mode !== 'real') return <div className="app__rtabs" data-cut-right-tabs={layout.rightTab}>
    {['properties', 'color', 'audio', 'chat'].map(tab => <button key={tab}
      data-cut-right-tab={tab} aria-selected={layout.rightTab === tab}>{tab}</button>)}
    {mode === 'failed'
      ? <section data-cut-panel-render-failed={layout.rightTab}>
          <button data-cut-panel-render-retry={layout.rightTab}>Try again</button>
        </section>
      : <div data-cut-loading>Loading</div>}
  </div>
  return <AppRightRail layout={layout} setLayout={setLayout} dragRail={() => {}}
    project={null} projectSession={0} doctor={null} ops={[]} receipts={[]}
    selectedClipId={null} playheadMs={playheadMs} onSeek={setPlayheadMs}
    agentChatPrefill={null} onUndo={() => {}} onRedo={() => {}} />
}
createRoot(document.getElementById('root')).render(<Fixture />)
`

test('mounted right rail reports a painted body and refuses blocked ui.open until recovery', {
  skip: process.env.SHELLX_CUT_RUN_RIGHT_BODY_BROWSER_TEST !== '1'
    ? 'set SHELLX_CUT_RUN_RIGHT_BODY_BROWSER_TEST=1 for the mounted browser regression'
    : false,
  timeout: 120_000,
}, async () => {
  const { createServer } = requireFromUi('vite')
  const { chromium } = requireFromUi('playwright')
  const cacheDir = await mkdtemp(join(tmpdir(), 'cut-right-body-'))
  const server = await createServer({ root: uiRoot, cacheDir, server: { host: '127.0.0.1', port: 0 }, plugins: [{
    name: 'right-body-regression-entry',
    resolveId(id) { if (id === entryPath) return join(uiRoot, entryPath) },
    load(id) { if (id === join(uiRoot, entryPath)) return entry },
    configureServer(dev) {
      dev.middlewares.use(async (request, response, next) => {
        if (request.url !== '/__right_body_regression__') return next()
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
    await page.route('**/api/verb/grade.list', route => route.fulfill({
      status: 200, contentType: 'application/json', body: JSON.stringify({ ok: true, result: { looks: [] } }),
    }))
    await page.addInitScript(() => {
      localStorage.setItem('cut.panelBlocked.v1', JSON.stringify({ color: Date.now() }))
    })
    await page.goto(`${server.resolvedUrls.local[0]}__right_body_regression__`)
    await page.locator('[data-cut-right-body-loaded="properties"]').waitFor({ state: 'attached' })
    assert.equal(await page.evaluate(() => window.currentState.current.right.body_status), 'loaded')

    const blockedId = await page.evaluate(() => window.command('color'))
    await page.waitForFunction(id => window.answers.some(answer => answer.request_id === id), blockedId)
    const blocked = await page.evaluate(id => window.answers.find(answer => answer.request_id === id), blockedId)
    assert.equal(blocked.applied, false)
    assert.equal(blocked.state.right.active_tab, 'color')
    assert.equal(blocked.state.right.body_status, 'blocked')
    assert.equal(blocked.selector, '[data-cut-panel-render-retry="color"]')
    assert.ok(!blocked.state.open_surface_ids.includes('color'))
    await page.locator(blocked.selector).click()
    await page.locator('[data-cut-right-body-loaded="color"]').waitFor({ state: 'attached' })
    await page.waitForFunction(() => window.currentState.current.right.body_status === 'loaded')
    assert.ok(await page.evaluate(() => window.currentState.current.open_surface_ids.includes('color')))

    await page.locator('[data-cut-right-tab="properties"]').click()
    await page.locator('[data-cut-right-body-loaded="properties"]').waitFor({ state: 'attached' })
    const loadedId = await page.evaluate(() => window.command('color'))
    await page.waitForFunction(id => window.answers.some(answer => answer.request_id === id), loadedId)
    const loaded = await page.evaluate(id => window.answers.find(answer => answer.request_id === id), loadedId)
    assert.equal(loaded.applied, true)
    assert.equal(loaded.state.right.body_status, 'loaded')

    await page.evaluate(() => { window.selectRightTab('properties'); window.setBodyMode('failed') })
    await page.waitForFunction(() => window.currentState.current.right.body_status === 'failed')
    const failedId = await page.evaluate(() => window.command('color'))
    await page.waitForFunction(id => window.answers.some(answer => answer.request_id === id), failedId)
    const failed = await page.evaluate(id => window.answers.find(answer => answer.request_id === id), failedId)
    assert.equal(failed.applied, false)
    assert.equal(failed.state.right.body_status, 'failed')
    assert.equal(failed.selector, '[data-cut-panel-render-retry="color"]')
    assert.ok(!failed.state.open_surface_ids.includes('color'))

    await page.evaluate(() => { window.selectRightTab('properties'); window.setBodyMode('loading') })
    await page.waitForFunction(() => window.currentState.current.right.body_status === 'loading')
    const loadingId = await page.evaluate(() => window.command('audio'))
    await page.waitForFunction(id => window.answers.some(answer => answer.request_id === id), loadingId)
    const loading = await page.evaluate(id => window.answers.find(answer => answer.request_id === id), loadingId)
    assert.equal(loading.applied, false)
    assert.equal(loading.state.right.body_status, 'loading')
    assert.ok(!loading.state.open_surface_ids.includes('audio'))
    assert.deepEqual(errors, [])
  } finally {
    await browser?.close()
    await server.close()
    await rm(cacheDir, { recursive: true, force: true })
  }
})
