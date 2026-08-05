import assert from 'node:assert/strict'

describe('ShellX Cut exhaustive native UI action gate', () => {
  it('runs the shared full-coverage scenarios in the native Tauri WebView', async function () {
    this.timeout(Number(process.env.FCV_WDIO_TIMEOUT_MS || 6 * 60 * 60 * 1000))

    const {
      FullCoverageExit,
      runFullCoverageVerify,
    } = await import('../full-coverage-verify.mjs')

    let exitCode = null
    try {
      await runFullCoverageVerify()
    } catch (error) {
      if (!(error instanceof FullCoverageExit)) throw error
      exitCode = error.exitCode
    }

    assert.equal(exitCode, 0, `full native action coverage exited with ${exitCode}`)
  })
})
