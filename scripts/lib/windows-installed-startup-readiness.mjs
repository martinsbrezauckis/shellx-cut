import { join, resolve } from 'node:path'

function budgetArgs(arg) {
  return [
    ['--startup-listener-budget-ms', arg('--startup-listener-budget-ms', '').trim()],
    ['--startup-api-budget-ms', arg('--startup-api-budget-ms', '').trim()],
    ['--startup-ui-client-budget-ms', arg('--startup-ui-client-budget-ms', '').trim()],
    ['--startup-dom-budget-ms', arg('--startup-dom-budget-ms', '').trim()],
  ].flatMap(([name, value]) => value ? [name, value] : [])
}

/**
 * Builds only the test-side launch arguments. The timestamp is captured by the
 * installed runner immediately before it starts the native shell.
 */
export function prepareWindowsInstalledStartupReadiness({
  arg,
  out,
  windowsPath,
  expandHome,
  t0EpochMs = Date.now(),
} = {}) {
  const receiptPath = join(out, 'installed-startup-readiness.json')
  const slowMarker = arg('--startup-slow-ffmpeg-marker', '').trim()
  const slowMarkerArgs = slowMarker
    ? ['--startup-slow-ffmpeg-marker', windowsPath(resolve(expandHome(slowMarker)))]
    : []
  return {
    receiptPath,
    probeArgs: [
      '--startup-out', windowsPath(receiptPath),
      '--startup-t0-epoch-ms', String(t0EpochMs),
      ...budgetArgs(arg),
      ...slowMarkerArgs,
    ],
  }
}
