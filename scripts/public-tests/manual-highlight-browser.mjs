import assert from 'node:assert/strict'
import { createServer } from 'node:http'
import { readFile } from 'node:fs/promises'
import { extname, relative, resolve } from 'node:path'

import { siteRoot, targetForFeature } from './manual-highlight-atlas.mjs'

export const viewports = [
  { name: 'desktop', width: 1440, height: 1000, minimumHighlightPx: 12 },
  { name: 'mobile', width: 390, height: 844, minimumHighlightPx: 12 },
]

const minimumRenderedHighlightPx = 14
const mimeTypes = {
  '.css': 'text/css; charset=utf-8',
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.png': 'image/png',
  '.svg': 'image/svg+xml',
}

export async function startStaticSite({ transform } = {}) {
  const server = createServer(async (request, response) => {
    try {
      const requestPath = decodeURIComponent(new URL(request.url || '/', 'http://manual.local').pathname)
      const relativePath = requestPath.endsWith('/') ? `${requestPath}index.html` : requestPath
      const target = resolve(siteRoot, `.${relativePath}`)
      if (relative(siteRoot, target).startsWith('..')) {
        response.writeHead(403)
        response.end('forbidden')
        return
      }
      const source = await readFile(target)
      const body = transform ? await transform(target, source) : source
      response.writeHead(200, { 'content-type': mimeTypes[extname(target)] || 'application/octet-stream' })
      response.end(body)
    } catch {
      response.writeHead(404)
      response.end('not found')
    }
  })
  await new Promise((resolveListen, rejectListen) => {
    server.once('error', rejectListen)
    server.listen(0, '127.0.0.1', resolveListen)
  })
  const address = server.address()
  assert.ok(address && typeof address !== 'string', 'manual static server has a numeric port')
  return { server, url: `http://127.0.0.1:${address.port}/manual/cut/` }
}

function errorRow(id, viewport, issue, observed = {}) {
  return { id, viewport, issue, ...observed }
}

function closeEnough(actual, expected, tolerance) {
  return Math.abs(actual - expected) <= tolerance
}

export function expectedRenderedRect(target, imagePixels) {
  const width = Math.min(1, Math.max(target.width, minimumRenderedHighlightPx / imagePixels[0]))
  const height = Math.min(1, Math.max(target.height, minimumRenderedHighlightPx / imagePixels[1]))
  return {
    left: Math.max(0, Math.min(1 - width, target.left + target.width / 2 - width / 2)),
    top: Math.max(0, Math.min(1 - height, target.top + target.height / 2 - height / 2)),
    width,
    height,
  }
}

function expectedGeometryFailures(rect, target, viewport, issuePrefix) {
  const failures = []
  const expected = expectedRenderedRect(target, rect.imagePixels)
  const actualCenter = [rect.left + rect.width / 2, rect.top + rect.height / 2]
  const expectedCenter = [expected.left + expected.width / 2, expected.top + expected.height / 2]
  const centerTolerance = viewport.name === 'mobile' ? 0.025 : 0.012
  const sizeTolerance = viewport.name === 'mobile' ? 0.012 : 0.006
  if (!closeEnough(actualCenter[0], expectedCenter[0], centerTolerance) || !closeEnough(actualCenter[1], expectedCenter[1], centerTolerance)) {
    failures.push(errorRow(target.id, viewport.name, `${issuePrefix} center no longer maps to the independently reviewed UI target`, { actualCenter, expectedCenter, target: target.id }))
  }
  if (!closeEnough(rect.width, expected.width, sizeTolerance) || !closeEnough(rect.height, expected.height, sizeTolerance)) {
    failures.push(errorRow(target.id, viewport.name, `${issuePrefix} dimensions no longer match the independently reviewed UI target`, {
      actualSize: [rect.width, rect.height], expectedSize: [expected.width, expected.height], target: target.id,
    }))
  }
  return failures
}

async function activateFeature(page, id) {
  const locator = page.locator(`[data-feature-id="${id}"]`)
  await locator.scrollIntoViewIfNeeded()
  await locator.click()
  await page.waitForFunction((expectedId) => window.location.hash === `#${expectedId}`, id)
}

async function readRenderedState(page, expectedId) {
  return page.evaluate((id) => {
    const highlight = document.querySelector('[data-manual-highlight]:not([hidden])')
    const surface = highlight?.closest('.manual-surface')
    const image = surface?.querySelector('img')
    const selected = document.querySelector(`[data-feature-id="${id}"]`)
    const detail = document.querySelector('[data-detail-title]')
    if (!highlight || !surface || !image || !selected || !detail) return { missing: true }
    const style = getComputedStyle(highlight)
    const highlightRect = highlight.getBoundingClientRect()
    const surfaceRect = surface.getBoundingClientRect()
    const imageRect = image.getBoundingClientRect()
    return {
      active: selected.classList.contains('active'),
      hash: window.location.hash,
      title: detail.textContent?.trim() || '',
      label: highlight.getAttribute('data-label') || '',
      surface: surface.getAttribute('data-manual-surface') || '',
      visible: style.display !== 'none' && style.visibility !== 'hidden' && Number(style.opacity) > 0,
      contrast: style.borderTopColor,
      borderWidth: Number.parseFloat(style.borderTopWidth),
      background: style.backgroundColor,
      pointerEvents: style.pointerEvents,
      rect: {
        left: (highlightRect.left - imageRect.left) / imageRect.width,
        top: (highlightRect.top - imageRect.top) / imageRect.height,
        width: highlightRect.width / imageRect.width,
        height: highlightRect.height / imageRect.height,
        pixels: [highlightRect.width, highlightRect.height],
        imagePixels: [imageRect.width, imageRect.height],
        viewport: [highlightRect.top, highlightRect.bottom, highlightRect.left, highlightRect.right],
        surface: [surfaceRect.top, surfaceRect.bottom, surfaceRect.left, surfaceRect.right],
      },
      imageReady: image.complete && image.naturalWidth > 0 && image.naturalHeight > 0,
    }
  }, expectedId)
}

function validateFeatureState(state, id, feature, target, viewport) {
  const failures = []
  if (state.missing) return [errorRow(id, viewport.name, 'required rendered manual element is missing')]
  if (!state.imageReady) failures.push(errorRow(id, viewport.name, 'interface capture did not load'))
  if (!state.active) failures.push(errorRow(id, viewport.name, 'selected tree item did not become active'))
  if (state.hash !== `#${id}`) failures.push(errorRow(id, viewport.name, 'feature hash does not match selected id', { hash: state.hash }))
  if (state.title !== feature.title) failures.push(errorRow(id, viewport.name, 'selected detail title does not match its feature definition', { title: state.title, expected: feature.title }))
  if (state.label !== target.label) failures.push(errorRow(id, viewport.name, 'highlight label does not match its independently reviewed target', { label: state.label, expected: target.label }))
  if (state.surface !== target.capture) failures.push(errorRow(id, viewport.name, 'highlight rendered on the wrong manual surface', { surface: state.surface, expected: target.capture }))
  if (!state.visible || state.borderWidth < 1 || state.pointerEvents !== 'none') {
    failures.push(errorRow(id, viewport.name, 'highlight is not visibly rendered as a non-intercepting outline', { visible: state.visible, borderWidth: state.borderWidth, pointerEvents: state.pointerEvents }))
  }
  for (const failure of expectedGeometryFailures(state.rect, target, viewport, 'highlight')) failures.push({ ...failure, id })
  if (state.rect.left < -0.001 || state.rect.top < -0.001 || state.rect.left + state.rect.width > 1.001 || state.rect.top + state.rect.height > 1.001) {
    failures.push(errorRow(id, viewport.name, 'highlight is clipped by the illustrated surface', { rect: state.rect }))
  }
  const visibleMinimum = viewport.minimumHighlightPx - 0.5
  if (state.rect.pixels[0] < visibleMinimum || state.rect.pixels[1] < visibleMinimum) {
    failures.push(errorRow(id, viewport.name, 'highlight is too small to see at this viewport', { pixels: state.rect.pixels, minimum: viewport.minimumHighlightPx }))
  }
  const [top, bottom, left, right] = state.rect.viewport
  if (top < 0 || bottom > viewport.height || left < 0 || right > viewport.width) {
    failures.push(errorRow(id, viewport.name, 'selected highlight is not in the viewport after its manual item is selected', { viewport: state.rect.viewport, viewportSize: [viewport.width, viewport.height] }))
  }
  if (!/^rgb\(/.test(state.contrast) || !/^rgba\(/.test(state.background)) {
    failures.push(errorRow(id, viewport.name, 'highlight lacks an explicit contrasting border and translucent fill', { contrast: state.contrast, background: state.background }))
  }
  return failures
}

async function verifyKeyboardRoute(page, features) {
  const id = 'cut.timeline.track_controls'
  const feature = features[id]
  assert.ok(feature, 'representative keyboard feature is present')
  const locator = page.locator(`[data-feature-id="${id}"]`)
  await locator.focus()
  await page.keyboard.press('Enter')
  const state = await readRenderedState(page, id)
  assert.equal(state.active, true, 'Enter activates the focused manual feature')
  assert.equal(state.hash, `#${id}`, 'Enter updates the feature deep link')
  assert.equal(state.title, feature.title, 'Enter updates the matching detail content')
}

async function verifyLegacyDeepLinks(page, baseUrl, features, aliases, targetsByFeature, viewport) {
  const failures = []
  for (const [legacyId, resolvedId] of Object.entries(aliases)) {
    const feature = features[resolvedId]
    const target = targetForFeature(resolvedId, feature, targetsByFeature)
    const legacyUrl = new URL(baseUrl)
    legacyUrl.searchParams.set('legacy-atlas', legacyId)
    legacyUrl.hash = legacyId
    await page.goto(legacyUrl.toString(), { waitUntil: 'networkidle' })
    await page.addStyleTag({ content: '.manual-highlight { transition: none !important; }' })
    await page.waitForFunction((expectedId) => document.querySelector(`[data-feature-id="${expectedId}"]`)?.classList.contains('active'), resolvedId)
    const state = await readRenderedState(page, resolvedId)
    if (state.missing || state.hash !== `#${legacyId}`) {
      failures.push(errorRow(legacyId, viewport.name, 'legacy feature hash did not remain addressable', { hash: state.hash }))
      continue
    }
    if (!state.active || state.title !== feature.title || state.label !== target.label || state.surface !== target.capture) {
      failures.push(errorRow(legacyId, viewport.name, 'legacy feature hash did not resolve to its current manual target', { active: state.active, title: state.title, label: state.label, surface: state.surface, resolvedId }))
      continue
    }
    for (const failure of expectedGeometryFailures(state.rect, target, viewport, 'legacy feature hash')) failures.push({ ...failure, id: legacyId })
    const [top, bottom, left, right] = state.rect.viewport
    if (top < 0 || bottom > viewport.height || left < 0 || right > viewport.width) {
      failures.push(errorRow(legacyId, viewport.name, 'initial feature hash did not bring its highlight into the viewport', { viewport: state.rect.viewport, viewportSize: [viewport.width, viewport.height] }))
    }
  }
  return failures
}

export async function runMatrix(chromium, baseUrl, sourceFeatures, legacyFeatureAliases, targetsByFeature) {
  const browser = await chromium.launch({ headless: true })
  const failures = []
  let selectableIds = []
  try {
    for (const viewport of viewports) {
      const page = await browser.newPage({ viewport: { width: viewport.width, height: viewport.height }, deviceScaleFactor: 1 })
      const pageErrors = []
      page.on('pageerror', (error) => pageErrors.push(error.message))
      const navigation = await page.goto(baseUrl, { waitUntil: 'networkidle' })
      await page.addStyleTag({ content: '.manual-highlight { transition: none !important; }' })
      if (await page.evaluate(() => window.scrollY) !== 0) failures.push(errorRow('page', viewport.name, 'unlinked manual page load scrolled away from its reading position'))
      if (await page.locator('[data-manual-highlight]').count() !== 2 || await page.locator('[data-manual-highlight]:not([hidden])').count() !== 1) {
        failures.push(errorRow('page', viewport.name, 'manual interface highlight did not initialize', { url: page.url(), status: navigation?.status(), body: (await page.locator('body').textContent())?.slice(0, 160) || '' }))
        await page.close()
        continue
      }
      const ids = await page.locator('[data-feature-id]').evaluateAll((nodes) => nodes.map((node) => node.getAttribute('data-feature-id')).filter(Boolean))
      if (viewport.name === 'desktop') selectableIds = ids
      const expectedIds = Object.keys(sourceFeatures)
      for (const id of expectedIds.filter((id) => !ids.includes(id))) failures.push(errorRow(id, viewport.name, 'feature definition has no selectable manual item'))
      for (const id of ids.filter((id) => !sourceFeatures[id])) failures.push(errorRow(id, viewport.name, 'selectable manual item has no feature definition'))
      for (const id of ids) {
        const feature = sourceFeatures[id]
        if (!feature) continue
        const target = targetForFeature(id, feature, targetsByFeature)
        await activateFeature(page, id)
        failures.push(...validateFeatureState(await readRenderedState(page, id), id, feature, target, viewport))
      }
      if (viewport.name === 'desktop') await verifyKeyboardRoute(page, sourceFeatures)
      failures.push(...await verifyLegacyDeepLinks(page, baseUrl, sourceFeatures, legacyFeatureAliases, targetsByFeature, viewport))
      for (const message of pageErrors) failures.push(errorRow('page', viewport.name, 'browser page error', { message }))
      await page.close()
    }
  } finally {
    await browser.close()
  }
  const failedRows = new Set(failures.filter((failure) => sourceFeatures[failure.id]).map((failure) => `${failure.id}:${failure.viewport}`))
  const failedLegacyRows = new Set(failures.filter((failure) => legacyFeatureAliases[failure.id]).map((failure) => `${failure.id}:${failure.viewport}`))
  return {
    featureDefinitions: Object.keys(sourceFeatures).length,
    selectableFeatures: selectableIds.length,
    targetCount: new Set([...targetsByFeature.values()].map((target) => target.id)).size,
    stateRows: selectableIds.length * viewports.length,
    passedStateRows: selectableIds.length * viewports.length - failedRows.size,
    legacyDeepLinkRows: Object.keys(legacyFeatureAliases).length * viewports.length,
    passedLegacyDeepLinkRows: Object.keys(legacyFeatureAliases).length * viewports.length - failedLegacyRows.size,
    failures,
  }
}
