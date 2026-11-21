import assert from 'node:assert/strict'
import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import test from 'node:test'
import { createServer } from 'vite'
import { chromium } from 'playwright'

const uiRoot = resolve(import.meta.dirname, '..')

test('ui.open confirms mounted and focused Edit panels after full-page workspaces', { timeout: 120_000 }, async () => {
  const cacheDir = await mkdtemp(join(tmpdir(), 'cut-ui-open-'))
  const server = await createServer({ root: uiRoot, cacheDir, server: { host: '127.0.0.1', port: 0 } })
  let browser
  try {
    await server.listen()
    browser = await chromium.launch({ headless: true })
    const page = await browser.newPage({ viewport: { width: 1440, height: 980 } })
    page.on('pageerror', error => console.error(`Page error: ${error.message}`))
    await page.goto(`${server.resolvedUrls.local[0]}?mock=1`)
    await page.evaluate(async () => {
      const { events } = await import('/src/lib/events.ts')
      window.answers = []
      events.answerUiCommand = answer => window.answers.push(answer)
      window.command = panel => {
        const id = (window.nextId = (window.nextId || 0) + 1)
        for (const listener of events.listeners) listener({ type: 'ui_command', verb: 'ui.open', request_id: id, args: { panel } })
        return id
      }
      const focus = HTMLElement.prototype.focus
      HTMLElement.prototype.focus = function (...args) {
        if (!window.refuseFocus) return focus.apply(this, args)
      }
      const query = document.querySelector.bind(document)
      document.querySelector = selector => window.missingTarget === selector ? null : query(selector)
    })
    await page.locator('[data-cut-panel="preview"]').waitFor()
    const command = async panel => {
      const id = await page.evaluate(panel => window.command(panel), panel)
      await page.waitForFunction(id => window.answers.some(a => a.request_id === id), id)
      return page.evaluate(id => window.answers.find(a => a.request_id === id), id)
    }
    const openFrom = async (workspace, panel) => {
      assert.equal((await command(workspace)).applied, true, workspace)
      await page.locator('[data-cut-panel="preview"]').waitFor({ state: 'detached' })
      const answer = await command(panel)
      assert.equal(answer.applied, true, `${workspace} -> ${panel}: ${JSON.stringify(answer.error)}`)
      assert.equal(answer.error, undefined)
      assert.equal(answer.state.active_workspace, 'edit')
      assert.ok(answer.state.open_surface_ids.includes(panel))
      if (panel === 'preview' || panel === 'timeline') {
        await page.locator(`[data-cut-panel="${panel}"]`).evaluate(target => {
          if (target !== document.activeElement && !target.contains(document.activeElement)) throw new Error('panel not focused')
        })
      }
    }
    const openInEdit = async () => {
      for (const panel of ['preview', 'timeline']) {
        for (let round = 0; round < 2; round++) {
          // Even an already-open focus surface must actively regain focus.
          await page.evaluate(() => document.activeElement.blur())
          const answer = await command(panel)
          assert.equal(answer.applied, true, `edit -> ${panel} (${round}): ${JSON.stringify(answer.error)}`)
          assert.equal(answer.error, undefined)
          assert.equal(answer.state.active_workspace, 'edit')
          assert.ok(answer.state.open_surface_ids.includes(panel))
          await page.locator(`[data-cut-panel="${panel}"]`).evaluate(target => {
            if (target !== document.activeElement && !target.contains(document.activeElement)) throw new Error('panel not focused')
          })
        }
      }
    }
    await openInEdit()
    console.log('PASS Edit-origin focus surfaces, repeated while already open')
    // Music is a blocking drawer: the editor is inert, so focus must conflict.
    assert.equal((await command('music')).applied, true)
    for (const panel of ['preview', 'timeline']) {
      const answer = await command(panel)
      assert.equal(answer.applied, false)
      assert.equal(answer.error.code, 'conflict')
      assert.equal(answer.state.overlays.drawer, 'music')
    }
    await page.keyboard.press('Escape')
    await page.locator('[data-cut-musicbed-open="true"]').waitFor({ state: 'detached' })
    console.log('PASS blocking drawer prevents background panel focus')
    // Repeated real unmount/remounts exercise the frame-before-commit race.
    for (const workspace of ['record', 'library']) {
      for (let round = 0; round < 15; round++) {
        for (const panel of ['preview', 'timeline']) await openFrom(workspace, panel)
      }
      for (const panel of ['transcript', 'assets', 'projects', 'generate', 'generate-prompt',
        'generate-storyboard', 'generate-media', 'find-media',
        'properties', 'color', 'audio', 'chat']) {
        console.log(`Checking ${workspace} -> ${panel}`)
        await openFrom(workspace, panel)
      }
      console.log(`PASS ${workspace}: 30 focused transitions and 12 sibling Edit panels`)
    }
    // Neither a committed layout nor a matching state ID is sufficient proof.
    await page.evaluate(() => { window.refuseFocus = true; document.activeElement.blur() })
    const unfocused = await command('preview')
    assert.equal(unfocused.applied, false)
    assert.equal(unfocused.error.code, 'conflict')
    await page.evaluate(() => {
      window.refuseFocus = false
      window.missingTarget = '[data-cut-panel="preview"]'
    })
    const missing = await command('preview')
    assert.equal(missing.applied, false)
    assert.equal(missing.error.code, 'conflict')
    await page.evaluate(() => { window.missingTarget = null })
    assert.equal((await command('preview')).applied, true)
    console.log('PASS genuine focus/target conflicts and subsequent recovery')
  } finally {
    await browser?.close()
    await server.close()
    await rm(cacheDir, { recursive: true, force: true })
  }
})
