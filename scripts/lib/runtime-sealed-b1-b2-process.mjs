import { closeSync, existsSync, openSync, readFileSync } from 'node:fs'
import { spawn } from 'node:child_process'

function sleep(ms) {
  return new Promise((done) => setTimeout(done, ms))
}

export function createBoundedChildController({ child, closeLog = () => {}, waitMs = 8_000, pause = sleep }) {
  let closed = false
  let stopPromise
  let exit = null
  let startupError = null
  let exitResolve
  const exited = new Promise((resolve) => { exitResolve = resolve })
  const close = () => {
    if (!closed) { closed = true; closeLog() }
  }
  child.once('error', (error) => { startupError = error; exitResolve() })
  child.once('exit', (code, signal) => { exit = { code, signal }; exitResolve() })
  const waitForExit = async () => {
    await Promise.race([exited, pause(waitMs)])
    return exit
  }
  const stop = () => {
    if (stopPromise) return stopPromise
    stopPromise = (async () => {
      if (startupError) { close(); return { status: 'spawn-error', error: String(startupError.message || startupError) } }
      if (exit) { close(); return { status: 'already-exited', ...exit } }
      try { child.kill('SIGTERM') } catch (error) { close(); return { status: 'cleanup-failed', error: String(error.message || error) } }
      const graceful = await waitForExit()
      if (graceful) { close(); return { status: 'graceful', ...graceful } }
      try { child.kill('SIGKILL') } catch (error) { close(); return { status: 'cleanup-failed', error: String(error.message || error) } }
      const forced = await waitForExit()
      close()
      return forced ? { status: 'forced', ...forced } : { status: 'cleanup-failed', error: 'process did not exit within bounded cleanup' }
    })()
    return stopPromise
  }
  return { state: () => ({ exit, startupError }), stop }
}

export async function startLoopbackCutd({ cutd, uiDist, env, logPath, spawnFn = spawn, fetchFn = fetch, pause = sleep, readyMs = 20_000 }) {
  const log = openSync(logPath, 'a')
  let child
  try {
    child = spawnFn(cutd, ['serve', '--addr', '127.0.0.1:0', '--ui-dist', uiDist], { cwd: env.SHELLX_CUT_WORKTREE, env, stdio: ['ignore', log, log] })
  } catch (error) {
    closeSync(log)
    throw new Error(`could not start cutd: ${error.message || error}`)
  }
  const controller = createBoundedChildController({ child, closeLog: () => closeSync(log), pause })
  const deadline = Date.now() + readyMs
  try {
    while (true) {
      const state = controller.state()
      if (state.startupError) throw new Error(`cutd startup error: ${state.startupError.message || state.startupError}`)
      if (state.exit) throw new Error(`cutd exited before readiness (${state.exit.code ?? 'null'}${state.exit.signal ? ` ${state.exit.signal}` : ''})`)
      if (Date.now() >= deadline) throw new Error('cutd did not become ready before the bounded timeout')
      const text = existsSync(logPath) ? readFileSync(logPath, 'utf8') : ''
      const addr = text.match(/cutd listening on http:\/\/([^/]+)\//)?.[1]
      if (addr) {
        const response = await fetchFn(`http://${addr}/api/agent`, { signal: AbortSignal.timeout(1_000) }).catch(() => null)
        if (response?.ok) return { url: `http://${addr}`, agent: await response.json(), stop: controller.stop }
      }
      await pause(100)
    }
  } catch (error) {
    const cleanup = await controller.stop()
    throw new Error(`${error.message}; startup cleanup=${cleanup.status}`)
  }
}
