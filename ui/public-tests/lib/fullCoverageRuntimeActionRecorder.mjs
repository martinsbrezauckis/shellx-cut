// Records which stable source-action identities were actually actuated in the
// current installed page, independently of probe labels and static ownership.

function browserInstaller() {
  if (window.__shellxCutRuntimeActionRecorderInstalled) return
  const candidates = (event) => {
    const origin = event.target instanceof Element
      ? event.target.closest('button, input, select, textarea, summary')
      : null
    if (!origin) return []
    const out = []
    const direct = origin.getAttribute('data-cut-action')
    if (direct) out.push(direct)
    for (const attribute of origin.attributes) {
      if (!attribute.name.startsWith('data-cut-') || attribute.name === 'data-cut-action') continue
      out.push(attribute.name.slice('data-cut-'.length))
    }
    return [...new Set(out.filter(Boolean))]
  }
  const record = (event) => {
    const actions = candidates(event)
    if (actions.length) void window.__shellxCutRecordRuntimeAction(actions)
  }
  const recordMouseDownAction = (event) => {
    const origin = event.target instanceof Element
      ? event.target.closest('[data-cut-action]')
      : null
    const action = origin?.getAttribute('data-cut-action')
    if (action) void window.__shellxCutRecordRuntimeAction([action])
  }
  // Blocking-overlay scrims close on mousedown and can unmount before the
  // browser emits click. Observe the explicit action at the event that drives
  // the product behavior, matching the native WebDriver bridge.
  document.addEventListener('mousedown', recordMouseDownAction, true)
  document.addEventListener('click', record, true)
  document.addEventListener('input', record, true)
  document.addEventListener('change', record, true)
  window.__shellxCutRuntimeActionRecorderInstalled = true
}

export async function createRuntimeActionRecorder(page, expectedActionIds) {
  const expected = new Set((expectedActionIds || []).map(String))
  const observed = new Set()
  const unexpected = new Set()
  const accept = (value) => {
    const candidates = Array.isArray(value?.actions)
      ? value.actions
      : Array.isArray(value)
        ? value
        : []
    const normalized = [...new Set(candidates.map(String).filter(Boolean))]
    const matched = normalized.filter((candidate) => expected.has(candidate))
    if (matched.length) {
      for (const candidate of matched) observed.add(candidate)
      return
    }
    if (normalized[0]) unexpected.add(normalized[0])
  }

  if (typeof page.exposeFunction === 'function' && typeof page.addInitScript === 'function') {
    await page.exposeFunction('__shellxCutRecordRuntimeAction', accept)
    await page.addInitScript(browserInstaller)
  } else {
    page.on('action', accept)
  }

  return {
    ids: () => [...observed, ...unexpected].sort(),
    observed: () => [...observed].sort(),
    unexpected: () => [...unexpected].sort(),
  }
}
