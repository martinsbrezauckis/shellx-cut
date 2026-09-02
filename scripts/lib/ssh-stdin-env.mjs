export const SSH_KEEPALIVE_ARGS = Object.freeze([
  '-o', 'ServerAliveInterval=15',
  '-o', 'ServerAliveCountMax=24',
  '-o', 'TCPKeepAlive=yes',
])

export function readEnvFirstLine(name, env = process.env) {
  if (!/^[A-Z_][A-Z0-9_]*$/.test(name)) throw new Error(`invalid environment variable name: ${name}`)
  return String(env[name] || '').split(/\r?\n/).find((line) => line.trim())?.trim() || ''
}

export function buildSshEnvPayload(value, name, commandPrefix, script) {
  if (!/^[A-Z_][A-Z0-9_]*$/.test(name)) throw new Error(`invalid environment variable name: ${name}`)
  return value
    ? {
        command: `${commandPrefix} bash -c 'IFS= read -r ${name}; export ${name}; exec bash -s'`,
        input: `${value}\n${script}`,
      }
    : {
        command: `${commandPrefix} bash -s`,
        input: script,
      }
}
