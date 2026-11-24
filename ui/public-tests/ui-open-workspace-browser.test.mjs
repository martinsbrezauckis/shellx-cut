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
    // Music is a blocking drawer: background panels, workspaces, and Settings are
    // inert behind it, so ui.open rejects quickly naming Music and how to close it,
    // without mutating background tabs, while drawer->drawer replacement keeps working.
    assert.equal((await command('music')).applied, true)
    for (const panel of ['preview', 'timeline', 'assets', 'color', 'record', 'environment']) {
      const startedAt = Date.now()
      const answer = await command(panel)
      const elapsedMs = Date.now() - startedAt
      assert.equal(answer.applied, false, `${panel} must be blocked while music is open`)
      assert.equal(answer.error.code, 'conflict')
      assert.equal(answer.state.overlays.drawer, 'music')
      assert.ok(elapsedMs < 500, `blocked ui.open (${panel}) must reject quickly instead of timing out (took ${elapsedMs} ms)`)
      assert.match(answer.error.message, /music/i, `blocked ui.open (${panel}) must name the blocking drawer: ${answer.error.message}`)
      assert.match(answer.error.message, /close|escape/i, `blocked ui.open (${panel}) must say how to close the drawer: ${answer.error.message}`)
      assert.notEqual(answer.state.right.active_tab, 'color', 'blocked panel open must not silently mutate background tabs')
    }
    const alreadyMusic = await command('music')
    assert.equal(alreadyMusic.applied, false)
    assert.equal(alreadyMusic.error.code, 'conflict')
    assert.match(alreadyMusic.error.message, /already open/)
    for (const bad of ['', 'nonexistent-surface']) {
      const invalid = await command(bad)
      assert.equal(invalid.applied, false)
      assert.equal(invalid.error.code, 'not_found')
    }
    assert.equal((await command('title')).applied, true, 'opening another drawer still replaces music')
    const blockedByTitle = await command('transcript')
    assert.equal(blockedByTitle.applied, false)
    assert.match(blockedByTitle.error.message, /title/i)
    await page.locator('[data-cut-title-mode="free"]').click()
    assert.equal((await command('transcript')).applied, true, 'Title free placement mode does not block background panels')
    await page.locator('[data-cut-title-close]').click()
    await page.locator('[data-cut-title-open="true"]').waitFor({ state: 'detached' })
    assert.equal((await command('mask')).applied, true)
    assert.equal((await command('environment')).applied, true, 'Mask drawer does not block Settings and yields on navigation')
    await page.locator('[data-cut-mask-open="true"]').waitFor({ state: 'detached' })
    await page.locator('[data-cut-environment-close]').click()
    await page.locator('[data-cut-environment-open="true"]').waitFor({ state: 'detached' })
    await page.evaluate(() => document.activeElement?.blur?.())
    await page.keyboard.press('Control+k')
    await page.locator('[data-cut-command-palette]').waitFor()
    for (const panel of ['preview', 'music']) {
      const blockedByPalette = await command(panel)
      assert.equal(blockedByPalette.applied, false)
      assert.equal(blockedByPalette.error.code, 'conflict')
      assert.match(blockedByPalette.error.message, /command palette/i)
      assert.match(blockedByPalette.error.message, /close|escape/i)
    }
    await page.keyboard.press('Escape')
    await page.locator('[data-cut-command-palette]').waitFor({ state: 'detached' })
    console.log('PASS blocking drawers, Title free vs blocking, Mask->Settings, CommandPalette, and invalid surface checks')
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
    assert.match(missing.error.message, /not confirmed in time/)
    await page.evaluate(() => { window.missingTarget = null })
    assert.equal((await command('preview')).applied, true)
    // Delayed PanelPaintConfirm readiness (>1.5 s old deadline, inside 6 s bounded deadline)
    // confirms cleanly when the real body-loaded sentinel mounts at ~1.8 s.
    assert.equal((await command('properties')).applied, true)
    await page.evaluate(() => {
      const origSetTimeout = window.setTimeout.bind(window)
      let delayedOnce = false
      window.setTimeout = (fn, ms, ...rest) => {
        if (!delayedOnce && ms === 350) {
          delayedOnce = true
          return origSetTimeout(fn, 1_800, ...rest)
        }
        return origSetTimeout(fn, ms, ...rest)
      }
    })
    const delayed = await command('color')
    assert.equal(delayed.applied, true, `delayed confirmation (>1.5 s) should succeed: ${JSON.stringify(delayed.error)}`)
    assert.equal(delayed.state.right.active_tab, 'color')
    console.log('PASS genuine focus/target conflicts, delayed (>1.5 s) confirmation, and subsequent recovery')
  } finally {
    await browser?.close()
    await server.close()
    await rm(cacheDir, { recursive: true, force: true })
  }
})
