import { unlinkSync } from 'node:fs'

export async function runOfflineMediaRelinkCoverage({
  page,
  panel,
  asset,
  relinkPair,
  pickerProbe,
  nativeOsActionsEnabled,
  openAssets,
}) {
  if (!nativeOsActionsEnabled) return panel

  panel = await openAssets(page)
  unlinkSync(relinkPair.replacementDriver)
  await page.locator('[data-cut-media-health-refresh]').first().click()
  await page.locator(`[data-cut-asset-offline="${asset}"]`).waitFor({
    state: 'visible',
    timeout: 12_000,
  })
  await page.locator(`[data-cut-preview-relink="${asset}"]`).waitFor({
    state: 'visible',
    timeout: 12_000,
  })
  await pickerProbe(page, {
    name: 'preview-relink-offline',
    actionId: 'preview-relink-offline',
    selector: `[data-cut-action="preview-relink-offline"][data-cut-preview-relink="${asset}"]`,
    panel: page.locator('body'),
    surface: 'preview',
    groupName: 'preview-offline',
    selectPath: relinkPair.secondReplacementEngine,
    selectVerb: 'media.relink',
    selectAsset: asset,
    browserEvidence: async () => false,
  })

  unlinkSync(relinkPair.secondReplacementDriver)
  panel = await openAssets(page)
  await page.locator('[data-cut-media-health-refresh]').first().click()
  const timelineRelink = page.locator(`[data-cut-timeline-relink="${asset}"]`).first()
  await timelineRelink.waitFor({
    state: 'visible',
    timeout: 12_000,
  })
  await page.evaluate((assetId) => {
    document.querySelector(`[data-cut-timeline-relink="${assetId}"]`)
      ?.scrollIntoView({ block: 'center', inline: 'center' })
  }, asset)
  try {
    await page.waitForFunction((assetId) => {
      const control = document.querySelector(`[data-cut-timeline-relink="${assetId}"]`)
      if (!(control instanceof HTMLButtonElement) || control.disabled) return false
      const rect = control.getBoundingClientRect()
      const x = rect.left + rect.width / 2
      const y = rect.top + rect.height / 2
      const hit = document.elementFromPoint(x, y)
      return rect.width > 0 && rect.height > 0
        && x >= 0 && y >= 0 && x < window.innerWidth && y < window.innerHeight
        && (hit === control || control.contains(hit))
    }, asset, { timeout: 12_000 })
  } catch {
    const geometry = await page.evaluate((assetId) => {
      const control = document.querySelector(`[data-cut-timeline-relink="${assetId}"]`)
      if (!(control instanceof HTMLButtonElement)) return { control: 'missing' }
      const rect = control.getBoundingClientRect()
      const x = rect.left + rect.width / 2
      const y = rect.top + rect.height / 2
      const hit = document.elementFromPoint(x, y)
      const clip = control.closest('.tl-clip')
      const clipRect = clip?.getBoundingClientRect()
      const coveringSeams = [...document.querySelectorAll('.tl-seam')]
        .map((seam) => ({ seam, rect: seam.getBoundingClientRect() }))
        .filter(({ rect: seamRect }) => (
          x >= seamRect.left && x <= seamRect.right
          && y >= seamRect.top && y <= seamRect.bottom
        ))
      return {
        disabled: control.disabled,
        rect: { x: rect.x, y: rect.y, width: rect.width, height: rect.height },
        clip: clipRect ? {
          x: clipRect.x,
          y: clipRect.y,
          width: clipRect.width,
          height: clipRect.height,
          overflow: getComputedStyle(clip).overflow,
          zIndex: getComputedStyle(clip).zIndex,
        } : null,
        viewport: { width: window.innerWidth, height: window.innerHeight },
        hit: hit instanceof HTMLElement
          ? { tag: hit.tagName, className: hit.className, action: hit.dataset.cutAction || '' }
          : null,
        coveringSeams: coveringSeams.map(({ seam, rect: seamRect }) => ({
          id: seam.getAttribute('data-cut-seam'),
          x: seamRect.x,
          y: seamRect.y,
          width: seamRect.width,
          height: seamRect.height,
          zIndex: getComputedStyle(seam).zIndex,
        })),
      }
    }, asset)
    throw new Error(`timeline Relink is not pointer-actionable: ${JSON.stringify(geometry)}`)
  }
  await pickerProbe(page, {
    name: 'timeline-relink-offline',
    actionId: 'timeline-relink-offline',
    selector: `[data-cut-action="timeline-relink-offline"][data-cut-timeline-relink="${asset}"]`,
    panel: page.locator('body'),
    surface: 'timeline',
    groupName: 'timeline-offline',
    selectPath: relinkPair.thirdReplacementEngine,
    selectVerb: 'media.relink',
    selectAsset: asset,
    browserEvidence: async () => false,
  })

  unlinkSync(relinkPair.thirdReplacementDriver)
  panel = await openAssets(page)
  await page.locator('[data-cut-media-health-refresh]').first().click()
  await page.locator(`[data-cut-asset-offline="${asset}"]`).waitFor({
    state: 'visible',
    timeout: 12_000,
  })
  return panel
}
