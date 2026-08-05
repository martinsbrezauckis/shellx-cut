import { resolve } from 'node:path'

const appBinaryPath = process.env.SHELLX_CUT_WDIO_APP
const driverProvider = process.env.SHELLX_CUT_WDIO_PROVIDER || 'embedded'
const captureBackendLogs = process.env.WDIO_CAPTURE_BACKEND_LOGS === '1'
const captureFrontendLogs = process.env.WDIO_CAPTURE_FRONTEND_LOGS === '1'

if (!appBinaryPath) {
  throw new Error('SHELLX_CUT_WDIO_APP must point to the test-built ShellX Cut binary')
}
if (!['embedded', 'external', 'crabnebula'].includes(driverProvider)) {
  throw new Error(`unsupported SHELLX_CUT_WDIO_PROVIDER=${driverProvider}`)
}

const serviceOptions = {
  appBinaryPath: resolve(appBinaryPath),
  driverProvider,
  startTimeout: 120000,
  // Keep exhaustive release runs quiet by default, but allow a failing native
  // host gate to preserve the Rust/WebView evidence needed for diagnosis.
  captureBackendLogs,
  captureFrontendLogs,
  ...(driverProvider === 'embedded'
    ? {
        embeddedPort: Number(process.env.SHELLX_CUT_WDIO_PORT || 4445),
        statusPollTimeout: 10000,
      }
    : {}),
  ...(driverProvider === 'external' && process.env.SHELLX_CUT_TAURI_DRIVER
    ? { tauriDriverPath: resolve(process.env.SHELLX_CUT_TAURI_DRIVER) }
    : {}),
}

export const config = {
  runner: 'local',
  specs: ['./public-tests/wdio/**/*.e2e.mjs'],
  maxInstances: 1,
  logLevel: process.env.WDIO_LOG_LEVEL || 'silent',
  bail: 0,
  waitforTimeout: 20000,
  connectionRetryTimeout: 120000,
  connectionRetryCount: 0,
  framework: 'mocha',
  reporters: ['spec'],
  mochaOpts: {
    ui: 'bdd',
    // The exhaustive native action matrix intentionally runs for much longer
    // than the focused WDIO specs. Keep one env-controlled budget shared with
    // the spec so Mocha cannot terminate a healthy sweep at the old four-minute
    // suite default.
    timeout: Number(process.env.FCV_WDIO_TIMEOUT_MS || 6 * 60 * 60 * 1000),
  },
  services: [
    ['tauri', serviceOptions],
  ],
  capabilities: [{
    browserName: 'tauri',
    'tauri:options': {
      application: resolve(appBinaryPath),
    },
  }],
}
