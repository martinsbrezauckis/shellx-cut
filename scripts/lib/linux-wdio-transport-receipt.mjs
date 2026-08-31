import { createHash } from 'node:crypto'
import { closeSync, openSync, readFileSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'

const args = process.argv.slice(2)
const has = (name) => args.includes(name)
const value = (name) => {
  const index = args.indexOf(name)
  const found = index >= 0 ? args[index + 1] : ''
  if (!found) throw new Error(`${name} is required`)
  return found
}
const fail = (message) => {
  console.error(`linux-wdio-transport-receipt: ${message}`)
  process.exit(2)
}

try {
  if (has('--create-log')) {
    if (args.length !== 2) fail('usage: --create-log <path>')
    const log = value('--create-log')
    closeSync(openSync(log, 'wx', 0o600))
  } else if (has('--summarize')) {
    if (args.length !== 5) fail('usage: --summarize --log <path> --exit <code>')
    const log = value('--log')
    const exitCode = Number(value('--exit'))
    if (!Number.isInteger(exitCode) || exitCode < 0) fail('--exit must be a non-negative integer')
    const bytes = readFileSync(log)
    const receipt = {
      schema: 'shellx-cut/linux-native-wdio-transport@1',
      generatedAt: new Date().toISOString(),
      exitCode,
      log: { path: log, sha256: createHash('sha256').update(bytes).digest('hex'), bytes: bytes.length },
    }
    const summary = join(dirname(log), 'wdio-transport-summary.json')
    writeFileSync(summary, `${JSON.stringify(receipt, null, 2)}\n`, { flag: 'wx', mode: 0o600 })
    console.log(`WDIO native terminal: exit=${exitCode} log=${log} sha256=${receipt.log.sha256} bytes=${bytes.length}`)
  } else {
    fail('choose --create-log or --summarize')
  }
} catch (error) {
  fail(error?.message || String(error))
}
