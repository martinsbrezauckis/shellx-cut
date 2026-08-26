import { spawnSync } from 'node:child_process'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

import { DEFAULT_PUBLIC_TESTS_ROOT, discoverLibraryTests } from './libraryTestDiscovery.mjs'

const here = dirname(fileURLToPath(import.meta.url))
const uiRoot = resolve(here, '../..')
const tsxCli = resolve(uiRoot, 'node_modules/tsx/dist/cli.mjs')
const { included, excluded } = await discoverLibraryTests()

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
  if (result.error) throw result.error
  if (result.signal) throw new Error(`${relativePath} terminated by ${result.signal}`)
  if (result.status !== 0) process.exit(result.status ?? 1)
}
