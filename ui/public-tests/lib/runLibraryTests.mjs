import { spawnSync } from 'node:child_process'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

import { DEFAULT_PUBLIC_TESTS_ROOT, discoverLibraryTests } from './libraryTestDiscovery.mjs'

const here = dirname(fileURLToPath(import.meta.url))
const uiRoot = resolve(here, '../..')
const tsxCli = resolve(uiRoot, 'node_modules/tsx/dist/cli.mjs')
const noBail = process.argv.slice(2).some((arg) => arg === '--no-bail' || arg === '--collect-all')
const unsupported = process.argv.slice(2).filter((arg) => arg !== '--no-bail' && arg !== '--collect-all')
if (unsupported.length > 0) throw new Error(`unsupported test:lib argument(s): ${unsupported.join(', ')}`)
const { included, excluded } = await discoverLibraryTests()
const failures = []

console.log(`[test:lib] discovered ${included.length} eligible tests; ${excluded.length} explicit exclusions`)
for (const relativePath of included) {
  const testPath = resolve(DEFAULT_PUBLIC_TESTS_ROOT, relativePath)
  const args = relativePath.endsWith('.ts') ? [tsxCli, testPath] : [testPath]
  console.log(`[test:lib] RUN ${relativePath}`)
  const result = spawnSync(process.execPath, args, {
    cwd: uiRoot,
    env: process.env,
    stdio: 'inherit',
  })
  const message = result.error?.message
    ?? (result.signal ? `terminated by ${result.signal}` : null)
    ?? (result.status !== 0 ? `exited with status ${result.status ?? 1}` : null)
  if (message) {
    failures.push({ path: relativePath, message })
    console.error(`[test:lib] FAIL ${relativePath}: ${message}`)
    if (!noBail) break
  }
}

if (failures.length > 0) {
  console.error(`[test:lib] ${failures.length} of ${included.length} tests failed`)
  for (const failure of failures) console.error(`- ${failure.path}: ${failure.message}`)
  process.exitCode = 1
} else {
  console.log(`[test:lib] PASS ${included.length} tests${noBail ? ' (no-bail)' : ''}`)
}
