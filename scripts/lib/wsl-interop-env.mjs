export function forwardEnvToWindows(baseEnv, forwardedEnv) {
  const normalized = {}
  for (const [name, value] of Object.entries(forwardedEnv)) {
    if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(name)) {
      throw new Error(`Invalid Windows interop environment name: ${name}`)
    }
    if (value != null) normalized[name] = String(value)
  }

  const entries = String(baseEnv.WSLENV || '').split(':').filter(Boolean)
  const names = new Set(entries.map((entry) => entry.split('/')[0]))
  for (const name of Object.keys(normalized)) {
    if (!names.has(name)) entries.push(name)
  }
  return {
    ...baseEnv,
    ...normalized,
    WSLENV: entries.join(':'),
  }
}
