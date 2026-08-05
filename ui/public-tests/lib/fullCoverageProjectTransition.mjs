const delay = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

export async function createProjectWithRetry({
  verb,
  name,
  settings,
  timeoutMs = 180_000,
  retryDelayMs = 1_500,
  sleepFn = delay,
  nowFn = Date.now,
}) {
  if (typeof verb !== 'function') throw new TypeError('createProjectWithRetry requires verb')
  if (!String(name || '').trim()) throw new TypeError('createProjectWithRetry requires name')

  const started = nowFn()
  let attempts = 0
  let response = null

  while (true) {
    attempts += 1
    response = await verb('project.create', { name, settings })
    if (response?.ok || response?.error?.code !== 'job_cancel_pending') {
      return { response, attempts }
    }

    const elapsed = Math.max(0, nowFn() - started)
    if (elapsed >= timeoutMs) break
    await sleepFn(Math.min(retryDelayMs, Math.max(0, timeoutMs - elapsed)))
  }

  throw new Error(
    `project.create(${name}) still returned job_cancel_pending after ${attempts} attempts `
    + `and ${Math.max(0, nowFn() - started)} ms: `
    + JSON.stringify(response?.error || response).slice(0, 500),
  )
}
