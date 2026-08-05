// layout-contract-verify.mjs - focused browser gate for the native window
// contract. Requires a running cutd serving the current ui/dist.

import { mkdirSync } from 'node:fs'
import { join } from 'node:path'
import { chromium } from 'playwright'

const ADDR = process.env.CUTD_ADDR ?? '127.0.0.1:6161'
const APP = process.env.CUT_APP ?? `http://${ADDR}/`
const SCREENSHOT_DIR = process.env.LAYOUT_SCREENSHOT_DIR ?? ''
const VIEWPORTS = [
  { width: 1100, height: 680, label: 'native-minimum' },
  { width: 1280, height: 760, label: 'compact-desktop' },
  { width: 1440, height: 900, label: 'default-desktop' },
  { width: 1920, height: 1080, label: 'wide-desktop' },
]
const LAYOUT = {
  txFrac: 0.4,
  tlH: 280,
  railW: 340,
  railCollapsed: true,
  railPinned: false,
  leftCollapsed: false,
  leftTab: 'assets',
  findSurface: 'find-media',
  workspaceMode: 'edit',
  rightTab: 'properties',
}

let failures = 0
function check(viewport, name, ok, note = '') {
  if (!ok) failures += 1
  console.log(`${ok ? 'PASS' : 'FAIL'} ${viewport} ${name}${note ? ` - ${note}` : ''}`)
}

if (SCREENSHOT_DIR) mkdirSync(SCREENSHOT_DIR, { recursive: true })

const browser = await chromium.launch()
try {
  for (const viewport of VIEWPORTS) {
    const tag = `${viewport.width}x${viewport.height}`
    const page = await browser.newPage({ viewport })
    const pageErrors = []
    page.on('pageerror', (error) => pageErrors.push(String(error)))
    await page.addInitScript((layout) => {
      localStorage.setItem('cut.layout.v1', JSON.stringify(layout))
    }, LAYOUT)
    await page.goto(APP, { waitUntil: 'networkidle' })
    await page.waitForSelector('[data-cut-panel="topbar"]', { timeout: 20000 })
    await page.waitForSelector('[data-cut-transport]', { timeout: 20000 })
    await page.waitForSelector('[data-cut-timeline-toolbar]', { timeout: 20000 })

    const geometry = await page.evaluate(() => {
      const rect = (selector) => {
        const element = document.querySelector(selector)
        if (!element) return null
        const box = element.getBoundingClientRect()
        return {
          left: box.left,
          top: box.top,
          right: box.right,
          bottom: box.bottom,
          width: box.width,
          height: box.height,
        }
      }
      const contains = (outer, inner, tolerance = 1) => !!outer && !!inner
        && inner.left >= outer.left - tolerance
        && inner.right <= outer.right + tolerance
        && inner.top >= outer.top - tolerance
        && inner.bottom <= outer.bottom + tolerance
      const inViewport = (box) => contains(
        { left: 0, top: 0, right: innerWidth, bottom: innerHeight },
        box,
      )
      const intersectionArea = (a, b) => {
        if (!a || !b) return null
        const width = Math.max(0, Math.min(a.right, b.right) - Math.max(a.left, b.left))
        const height = Math.max(0, Math.min(a.bottom, b.bottom) - Math.max(a.top, b.top))
        return width * height
      }

      const root = document.querySelector('#root')
      const topbar = rect('[data-cut-panel="topbar"]')
      const preview = rect('.app__preview')
      const transport = rect('[data-cut-transport]')
      const toolsStrip = rect('[data-cut-action="expand-rail"]')
      const fullscreen = rect('[data-cut-action="fullscreen-toggle"]')
      const timelineToolbar = rect('[data-cut-timeline-toolbar]')
      const timelineTools = rect('[data-cut-tools]')
      const automation = rect('[data-cut-timeline-automation-trigger]')
      const timecode = rect('[data-cut-tc-readout]')
      const zoom = rect('.tl-zoom-chip')

      const primarySelectors = [
        '[data-cut-mode="edit"]',
        '[data-cut-mode="record"]',
        '[data-cut-projects-btn]',
        '[data-cut-library-btn]',
        '[data-cut-settings-btn]',
        '[data-cut-render-btn]',
        '[data-cut-export-btn]',
        '[data-cut-action="fullscreen-toggle"]',
        '[data-cut-action="expand-rail"]',
        '[data-cut-tool="razor"]',
        '[data-cut-tool="trim"]',
        '[data-cut-tool="snap"]',
      ]
      const primary = primarySelectors.map((selector) => ({ selector, box: rect(selector) }))
      const hiddenPrimary = primary.filter(({ box }) => !box || !inViewport(box)).map(({ selector }) => selector)

      const transportButtons = [...document.querySelectorAll('[data-cut-transport] button')]
        .map((element) => {
          const box = element.getBoundingClientRect()
          return {
            selector: element.getAttribute('data-cut-action')
              ?? element.getAttribute('data-cut-transport-btn')
              ?? element.getAttribute('data-cut-audio-toggle')
              ?? element.getAttribute('data-cut-quality-toggle')
              ?? element.getAttribute('aria-label')
              ?? element.tagName,
            box: { left: box.left, top: box.top, right: box.right, bottom: box.bottom },
          }
        })
      const escapedTransport = transportButtons
        .filter(({ box }) => !contains(transport, box))
        .map(({ selector }) => selector)

      const topbarSelectors = [
        '.tb-brand',
        '.tb-modes',
        '[data-cut-projects-btn]',
        '[data-cut-library-btn]',
        '[data-cut-settings-btn]',
        '[data-cut-manual-link]',
        '.tb-jobs',
        '[data-cut-toolbar]',
        '[data-cut-render-btn]',
        '[data-cut-export-btn]',
      ]
      const topbarBoxes = topbarSelectors
        .map((selector) => ({ selector, box: rect(selector) }))
        .filter(({ box }) => box && box.width > 0 && box.height > 0)
      const topbarEscapes = topbarBoxes
        .filter(({ box }) => !contains(topbar, box))
        .map(({ selector }) => selector)
      const topbarOverlaps = []
      for (let i = 0; i < topbarBoxes.length; i += 1) {
        for (let j = i + 1; j < topbarBoxes.length; j += 1) {
          if (intersectionArea(topbarBoxes[i].box, topbarBoxes[j].box) > 1) {
            topbarOverlaps.push(`${topbarBoxes[i].selector}:${topbarBoxes[j].selector}`)
          }
        }
      }

      return {
        rootHorizontalOverflow: root ? root.scrollWidth - root.clientWidth : null,
        rootVerticalOverflow: root ? root.scrollHeight - root.clientHeight : null,
        hiddenPrimary,
        escapedTransport,
        topbarEscapes,
        topbarOverlaps,
        transportContained: contains(preview, transport),
        timelineToolsContained: contains(timelineToolbar, timelineTools),
        automationContained: contains(timelineToolbar, automation),
        timelineGroupsSeparated:
          intersectionArea(timecode, timelineTools) === 0
          && intersectionArea(timelineTools, zoom) === 0,
        automationSeparated:
          intersectionArea(timelineTools, automation) === 0
          && intersectionArea(automation, zoom) === 0,
        stripTransportOverlap: intersectionArea(toolsStrip, transport),
        stripFullscreenOverlap: intersectionArea(toolsStrip, fullscreen),
      }
    })

    check(tag, 'no root horizontal overflow', geometry.rootHorizontalOverflow === 0, `delta=${geometry.rootHorizontalOverflow}`)
    check(tag, 'no root vertical overflow', geometry.rootVerticalOverflow === 0, `delta=${geometry.rootVerticalOverflow}`)
    check(tag, 'primary controls stay in viewport', geometry.hiddenPrimary.length === 0, geometry.hiddenPrimary.join(', '))
    check(tag, 'topbar groups stay contained', geometry.topbarEscapes.length === 0, geometry.topbarEscapes.join(', '))
    check(tag, 'topbar groups do not intersect', geometry.topbarOverlaps.length === 0, geometry.topbarOverlaps.join(', '))
    check(tag, 'transport stays inside preview', geometry.transportContained)
    check(tag, 'transport controls stay contained', geometry.escapedTransport.length === 0, geometry.escapedTransport.join(', '))
    check(tag, 'Tools strip stays outside transport', geometry.stripTransportOverlap === 0, `overlap=${geometry.stripTransportOverlap}`)
    check(tag, 'Tools strip stays outside Full Screen', geometry.stripFullscreenOverlap === 0, `overlap=${geometry.stripFullscreenOverlap}`)
    check(tag, 'timeline tools stay inside toolbar row', geometry.timelineToolsContained)
    check(tag, 'Automate stays inside toolbar row', geometry.automationContained)
    check(tag, 'timeline toolbar groups do not intersect', geometry.timelineGroupsSeparated)
    check(tag, 'Automate does not intersect toolbar groups', geometry.automationSeparated)

    const endReachable = await page.evaluate(() => {
      const scroller = document.querySelector('[data-cut-tools]')
      const target = document.querySelector('[data-cut-action="save-gif"]')
      if (!scroller || !target) return false
      scroller.scrollLeft = scroller.scrollWidth
      const outer = scroller.getBoundingClientRect()
      const inner = target.getBoundingClientRect()
      const reachable = inner.left >= outer.left - 1 && inner.right <= outer.right + 1
        && inner.top >= outer.top - 1 && inner.bottom <= outer.bottom + 1
      scroller.scrollLeft = 0
      return reachable
    })
    check(tag, 'secondary timeline tools remain scroll-reachable', endReachable)

    if (SCREENSHOT_DIR) {
      await page.screenshot({ path: join(SCREENSHOT_DIR, `layout-${tag}.png`) })
    }

    const fullscreenButton = page.locator('[data-cut-action="fullscreen-toggle"]')
    await fullscreenButton.click({ timeout: 5000 })
    const enteredFullscreen = await page.waitForFunction(() => !!document.fullscreenElement, null, { timeout: 3000 })
      .then(() => true)
      .catch(() => false)
    check(tag, 'Full Screen enters with an ordinary click', enteredFullscreen)
    if (enteredFullscreen) {
      await fullscreenButton.click({ timeout: 5000 })
      const exitedFullscreen = await page.waitForFunction(() => !document.fullscreenElement, null, { timeout: 3000 })
        .then(() => true)
        .catch(() => false)
      check(tag, 'Full Screen exits with an ordinary click', exitedFullscreen)
    }

    const railExpand = page.locator('[data-cut-action="expand-rail"]')
    await railExpand.click({ timeout: 5000 })
    await page.waitForSelector('[data-cut-rail-overlay="true"]', { timeout: 3000 })
    const automateHit = await page.locator('[data-cut-timeline-automation-trigger]').evaluate((element) => {
      const box = element.getBoundingClientRect()
      const hit = document.elementFromPoint(box.left + box.width / 2, box.top + box.height / 2)
      return hit === element || element.contains(hit)
    })
    check(tag, 'Automate stays clickable with overlay tools open', automateHit)
    await page.locator('[data-cut-timeline-automation-trigger]').click({ timeout: 5000 })
    const automationOpened = await page.locator('[data-cut-timeline-automation-menu]').isVisible().catch(() => false)
    check(tag, 'Automate opens with an ordinary click', automationOpened)
    await page.keyboard.press('Escape')
    const railPreserved = await page.locator('[data-cut-rail-overlay="true"]').isVisible().catch(() => false)
    check(tag, 'Escape closes Automate before overlay tools', railPreserved)
    await page.keyboard.press('Escape')

    await page.locator('[data-cut-mode="record"]').click({ timeout: 5000 })
    await page.waitForSelector('[data-cut-panel="record"]', { timeout: 5000 })
    const recordGeometry = await page.evaluate(() => {
      const rect = (selector) => {
        const element = document.querySelector(selector)
        if (!element) return null
        const box = element.getBoundingClientRect()
        return { left: box.left, top: box.top, right: box.right, bottom: box.bottom, width: box.width, height: box.height }
      }
      const visible = (box) => !!box
        && box.width > 0
        && box.height > 0
        && box.left >= 0
        && box.right <= innerWidth
        && box.top >= 0
        && box.bottom <= innerHeight
      const intersectionArea = (a, b) => {
        if (!a || !b) return null
        const width = Math.max(0, Math.min(a.right, b.right) - Math.max(a.left, b.left))
        const height = Math.max(0, Math.min(a.bottom, b.bottom) - Math.max(a.top, b.top))
        return width * height
      }
      const source = rect('[data-cut-rec-source]')
      const start = rect('[data-cut-action="record-start"]')
      return {
        readinessVisible: visible(rect('[data-cut-rec-readiness]')),
        sourceVisible: visible(source),
        startVisible: visible(start),
        sourceStartOverlap: intersectionArea(source, start),
        timelineDeferred: document.querySelector('.app__main')?.getAttribute('data-cut-record-timeline-deferred') === 'true',
        timelinePresent: !!document.querySelector('.app__timeline'),
      }
    })
    check(tag, 'record readiness stays in first viewport', recordGeometry.readinessVisible)
    check(tag, 'record Source stays in first viewport', recordGeometry.sourceVisible)
    check(tag, 'record Start stays in first viewport', recordGeometry.startVisible)
    check(tag, 'record Source and Start do not intersect', recordGeometry.sourceStartOverlap === 0, `overlap=${recordGeometry.sourceStartOverlap}`)
    check(tag, 'empty Record defers timeline', recordGeometry.timelineDeferred && !recordGeometry.timelinePresent)

    if (SCREENSHOT_DIR) {
      await page.screenshot({ path: join(SCREENSHOT_DIR, `record-layout-${tag}.png`) })
    }

    check(tag, 'browser page errors', pageErrors.length === 0, pageErrors.join(' | '))
    await page.close()
  }
} finally {
  await browser.close()
}

console.log(`\nlayout-contract-verify: ${failures === 0 ? 'PASS' : `FAIL (${failures})`}`)
process.exitCode = failures === 0 ? 0 : 1
