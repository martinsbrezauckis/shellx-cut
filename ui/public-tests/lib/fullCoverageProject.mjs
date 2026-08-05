// fullCoverageProject.mjs - project state/op polling for the exhaustive verifier.
//
// These helpers are the verifier's edit-result proof machinery. They keep
// project.state polls bounded independently from long-running verbs and poll the
// op log long enough to avoid loaded-rig timing flakes without masking no-ops.

export function createProjectWaiters({ verb, sleep, statePollTimeoutMs = 5000 }) {
  if (typeof verb !== 'function') throw new TypeError('createProjectWaiters requires verb')
  if (typeof sleep !== 'function') throw new TypeError('createProjectWaiters requires sleep')

  function flatClips(s) {
    return (s?.tracks || []).flatMap((t) => (t.clips || []).map((c) => ({ ...c, _track: t.id, _kind: t.kind })))
  }

  function findClip(s, id) {
    return flatClips(s).find((c) => c.id === id)
  }

  async function state(opts = {}) {
    return (await verb('project.state', {}, opts)).result
  }

  async function ops() {
    return (await verb('project.ops')).result?.ops || []
  }

  async function opsLen() {
    return (await ops()).length
  }

  async function waitForState(pred, timeoutMs = 18000) {
    const deadline = Date.now() + timeoutMs
    while (Date.now() < deadline) {
      const s = await state({ timeoutMs: statePollTimeoutMs })
      try { if (pred(s)) return s } catch {}
      await sleep(400)
    }
    return null
  }

  async function opLanded(sinceLen, verbName, pred, { timeoutMs = 4500, intervalMs = 150 } = {}) {
    const deadline = Date.now() + timeoutMs
    for (;;) {
      const all = await ops()
      if (all.slice(sinceLen).some((o) => o.verb === verbName && (!pred || pred(o.args || {})))) return true
      if (Date.now() >= deadline) return false
      await sleep(intervalMs)
    }
  }

  return { state, ops, opsLen, waitForState, opLanded, flatClips, findClip }
}
