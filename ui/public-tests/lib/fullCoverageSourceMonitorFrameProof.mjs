// Exact, rendered Source Monitor proof shared by the timeline Match Frame rows.
// Keeping this separate prevents source-frame behavior from being reduced to a
// menu-presence check when either context owner evolves.
export function formatSourceMonitorTime(value) {
  const ms = Math.max(0, Math.round(Number(value) || 0))
  const minutes = Math.floor(ms / 60_000)
  const seconds = Math.floor((ms % 60_000) / 1_000)
  return `${minutes}:${String(seconds).padStart(2, '0')}.${String(ms % 1_000).padStart(3, '0')}`
}

export async function proveSourceMonitorFrame(page, { assetId, expectedSourceMs }) {
  const source = page.locator(`[data-cut-source-monitor="${assetId}"]`).first()
  const expectedText = formatSourceMonitorTime(expectedSourceMs)
  await source.waitFor({ state: 'visible', timeout: 8_000 })
  const settled = await page.waitForFunction(({ asset, expected }) => (
    document.querySelector(`[data-cut-source-monitor="${asset}"] [data-cut-source-current]`)?.textContent === expected
  ), { asset: assetId, expected: expectedText }, { timeout: 8_000 }).then(() => true).catch(() => false)
  const rendered = await source.locator('[data-cut-source-current]').first().textContent().catch(() => '')
  const mediaMs = await source.locator('video, audio').first().evaluate((node) => (
    node instanceof HTMLMediaElement && Number.isFinite(node.currentTime)
      ? Math.round(node.currentTime * 1_000)
      : null
  )).catch(() => null)
  const exact = settled && rendered === expectedText && mediaMs === Math.round(expectedSourceMs)
  await source.locator('[data-cut-source-monitor-close]').first().click().catch(() => {})
  await source.waitFor({ state: 'detached', timeout: 4_000 }).catch(() => {})
  return {
    ok: exact,
    detail: `asset=${assetId}; expected=${expectedText}; rendered=${rendered || 'missing'}; mediaMs=${mediaMs ?? 'missing'}`,
  }
}
