// Resolve the application root used by deterministic conditional fixtures.
//
// Installed Tauri/WebView runs do not use the Vite development URL. When the
// configured URL is intentionally empty, retain the active installed origin
// and strip any fixture query/hash before constructing the next scenario.

export async function resolveCoverageAppUrl(page, configuredApp = '') {
  const current = String(configuredApp || '').trim()
    || String(await page.evaluate(() => window.location.href)).trim()
  if (!current) throw new Error('Conditional action coverage requires the app URL')

  const url = new URL(current)
  url.search = ''
  url.hash = ''
  return url.toString()
}
