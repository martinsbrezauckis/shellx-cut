import assert from 'node:assert/strict'
import { mkdir } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import { createWebdriverIoPage } from '../lib/webdriverIoPage.mjs'

const OUT_DIR = process.env.SHELLX_CUT_WDIO_OUT ||
  join(tmpdir(), `shellx-cut-webdriver-adapter-${Date.now()}`)

async function step(label, operation, timeout = 30000) {
  process.stdout.write(`[native-adapter] ${label}\n`)
  let timer
  try {
    const result = await Promise.race([
      operation(),
      new Promise((_, reject) => {
        timer = setTimeout(
          () => reject(new Error(`${label} timed out after ${timeout}ms`)),
          timeout,
        )
      }),
    ])
    process.stdout.write(`[native-adapter] ${label}: ok\n`)
    return result
  } finally {
    clearTimeout(timer)
  }
}

describe('ShellX Cut native page adapter', () => {
  it('drives locators, native clicks, form input, events, and screenshots', async function () {
    this.timeout(120000)
    await mkdir(OUT_DIR, { recursive: true })

    await step('raw DOM execute', () => browser.execute(() => document.location.href))
    const native = await step(
      'create page adapter',
      () => createWebdriverIoPage(browser),
    )
    const { page } = native
    try {
      await step('wait for topbar', () => page.waitForSelector(
        '[data-cut-panel="topbar"]',
        { timeout: 30000 },
      ), 35000)
      const settings = page.locator('[data-cut-settings-btn]').first()
      assert.equal(await step('count settings button', () => settings.count()), 1)
      assert.equal(await step('check settings visibility', () => settings.isVisible()), true)
      const box = await step('measure settings button', () => settings.boundingBox())
      assert.ok(box && box.width > 1 && box.height > 1)

      await step('open settings', () => settings.click())
      await step('wait for settings overview', () => page.waitForSelector(
        '[data-cut-settings-body="overview"]',
        { timeout: 10000 },
      ))
      await step('open General settings', () => page.locator(
        '[data-cut-settings-category="general"]',
      ).click())
      await step('wait for General settings', () => page.waitForSelector(
        '[data-cut-settings-body="general"]',
        { timeout: 10000 },
      ))

      const originalViewport = page.viewportSize()
      await step('resize to narrow native viewport', () => page.setViewportSize({
        width: 1100,
        height: Math.max(680, originalViewport.height),
      }))
      await step('wait for narrow category selector', () => page.waitForSelector(
        '[data-cut-settings-category-select]',
        { timeout: 10000 },
      ))
      await step('choose Agent control at narrow width', () => page.locator(
        '[data-cut-settings-category-select]',
      ).selectOption('agent-control'))
      await step('wait for Agent control settings', () => page.waitForSelector(
        '[data-cut-settings-body="agent-control"]',
        { timeout: 10000 },
      ))
      await step('restore native viewport', () => page.setViewportSize(originalViewport))
      await step('reopen General settings', () => page.locator(
        '[data-cut-settings-category="general"]',
      ).click())

      const search = page.locator('[data-cut-settings-search]')
      await step('fill settings search', () => search.fill('agent'))
      assert.equal(await step('read settings search', () => search.inputValue()), 'agent')
      await step('wait for agent-control result', () => page.waitForSelector(
        '[data-cut-settings-search-result="agent-control"]',
        { timeout: 10000 },
      ))

      let observed = null
      const onResponse = async (response) => {
        if (!response.url().includes('/api/verb/project.list')) return
        observed = await response.json()
      }
      page.on('response', onResponse)
      await step('run instrumented API request', () => page.evaluate(() => fetch('/api/verb/project.list', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: '{}',
      }).then((response) => response.text())))
      for (let i = 0; i < 50 && observed === null; i++) {
        await new Promise((resolve) => setTimeout(resolve, 100))
      }
      page.off('response', onResponse)
      assert.equal(observed?.ok, true)

      await step('capture screenshot', () => page.screenshot({
        path: join(OUT_DIR, 'native-adapter-settings.png'),
      }))
      await step('press Escape', () => page.keyboard.press('Escape'))
      await step('wait for settings close', () => page.waitForSelector(
        '[data-cut-settings-body="general"]',
        {
        state: 'hidden',
        timeout: 10000,
        },
      ))
    } finally {
      await native.close()
    }
  })
})
