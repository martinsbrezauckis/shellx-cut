import { spawn } from 'node:child_process'
import { resolve } from 'node:path'

function waitForLine(child, predicate, timeoutMs) {
  return new Promise((resolveLine, reject) => {
    let buffer = ''
    let stderr = ''
    const timer = setTimeout(() => {
      cleanup()
      child.kill('SIGTERM')
      reject(new Error(`native action controller timed out after ${timeoutMs}ms${stderr ? `: ${stderr.trim()}` : ''}`))
    }, timeoutMs)
    const cleanup = () => {
      clearTimeout(timer)
      child.stdout?.off('data', onStdout)
      child.stderr?.off('data', onStderr)
      child.off('error', onError)
      child.off('close', onClose)
    }
    const onStderr = (chunk) => { stderr += chunk.toString() }
    const onError = (error) => { cleanup(); reject(error) }
    // `exit` may fire before the child's stdio pipes have drained. The native
    // helper prints a structured `done` proof even when it refuses an unsafe
    // action and exits non-zero, so wait for `close` before declaring the proof
    // missing.
    const onClose = (code, signal) => {
      cleanup()
      reject(new Error(
        `native action controller exited before proof: code=${code} signal=${signal || 'none'}${stderr ? `: ${stderr.trim()}` : ''}`,
      ))
    }
    const onStdout = (chunk) => {
      buffer += chunk.toString()
      for (;;) {
        const end = buffer.indexOf('\n')
        if (end < 0) return
        const line = buffer.slice(0, end).trim()
        buffer = buffer.slice(end + 1)
        if (!line) continue
        let parsed
        try { parsed = JSON.parse(line) } catch { continue }
        if (predicate(parsed)) {
          cleanup()
          resolveLine(parsed)
          return
        }
      }
    }
    child.stdout?.on('data', onStdout)
    child.stderr?.on('data', onStderr)
    child.on('error', onError)
    child.on('close', onClose)
  })
}

export function createNativeOsActionController({
  command = process.env.FCV_NATIVE_ACTION_CONTROLLER || '',
  platform = process.env.FCV_NATIVE_ACTION_PLATFORM || process.platform,
  timeoutMs = Number(process.env.FCV_NATIVE_ACTION_TIMEOUT_MS || 20_000),
} = {}) {
  const executable = command ? resolve(command) : ''

  async function run(action, trigger) {
    if (!executable) return { controlled: false }
    const args = [
      executable,
      '--platform', platform,
      '--action', action.actionId,
      '--mode', action.mode || 'cancel',
    ]
    if (action.path) args.push('--path', action.path)
    const child = spawn(process.execPath, args, {
      env: process.env,
      stdio: ['ignore', 'pipe', 'pipe'],
    })
    // The helper owns the real dialog deadline. Give its structured `done`
    // refusal a small pipe-drain margin; using the exact same deadline here
    // races the parent's timer against `no native dialog appeared`.
    const proofTimeoutMs = timeoutMs + Math.max(
      250,
      Math.min(2_000, Math.ceil(timeoutMs * 0.1)),
    )
    // A safety refusal can happen before the helper emits `ready` (for
    // example, when an unrelated process owns the foreground window). Observe
    // an early `done` message as well so its exact refusal survives.
    const ready = waitForLine(
      child,
      (message) => message.phase === 'ready' || message.phase === 'done',
      proofTimeoutMs,
    )
    const done = waitForLine(child, (message) => message.phase === 'done', proofTimeoutMs)
    // `ready` and `done` intentionally observe the same child. If `ready`
    // rejects first, `run()` exits through the caller's ordinary action-failure
    // path; mark the still-pending `done` rejection handled so it cannot become
    // an unhandled rejection that aborts the entire exhaustive verifier.
    void done.catch(() => {})
    const readyProof = await ready
    if (readyProof.phase === 'done') {
      const proof = await done
      throw new Error(proof.error || 'native action controller refused before ready')
    }
    const readyContext = readyProof.before
      ? `; controller ready state=${JSON.stringify(readyProof.before)}`
      : ''
    let triggerError = null
    const triggerPromise = Promise.resolve()
      .then(trigger)
      .catch((error) => { triggerError = error })
    const [proof] = await Promise.all([done, triggerPromise])
    if (triggerError) {
      const controllerFailure = proof.ok
        ? ''
        : `; native controller failed: ${proof.error || 'returned no proof'}${readyContext}`
      throw new Error(
        `native action trigger failed: ${triggerError.message || triggerError}${controllerFailure}`,
      )
    }
    if (!proof.ok) {
      throw new Error(
        `${proof.error || 'native action controller returned no proof'}${readyContext}`,
      )
    }
    return {
      controlled: true,
      ok: true,
      evidence: proof.evidence || `${platform} dialog ${action.mode || 'cancel'} completed`,
      readyProof,
      proof,
    }
  }

  return {
    enabled: !!executable,
    run,
  }
}
