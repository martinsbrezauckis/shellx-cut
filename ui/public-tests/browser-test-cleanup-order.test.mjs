// A browser test that registers its Vite server cleanup only after `listen()`
// leaves the server and its file watcher open when `listen()` (or the watch
// setup) rejects, and the test process then hangs instead of failing.
import assert from 'node:assert/strict'
import { readdirSync, readFileSync } from 'node:fs'
import test from 'node:test'
import { fileURLToPath } from 'node:url'

const here = fileURLToPath(new URL('.', import.meta.url))
const files = readdirSync(here).filter((name) => name.endsWith('-browser.test.mjs'))

test('every Vite-backed browser test closes its server even when listen fails', () => {
  assert.ok(files.length > 0, 'browser tests are discovered')
  for (const name of files) {
    const source = readFileSync(here + name, 'utf8')
    // A test may instead close everything it opens in one `finally` of the test
    // body, with the server created and started in a helper called inside it.
    const closesInFinally = /finally\s*\{[\s\S]{0,400}?server\??\.close\(\)/.test(source)
    const listens = [...source.matchAll(/await server\.listen\(\)/g)]
    for (const listen of listens) {
      const before = source.slice(0, listen.index)
      const registration = before.lastIndexOf('server.close()')
      const creation = before.lastIndexOf('createServer(')
      const insideTry = /try\s*\{\s*$/.test(before.slice(-40))
      assert.ok(
        insideTry || registration > creation || closesInFinally,
        `${name}: register the server cleanup (t.after or try/finally) before "await server.listen()"`,
      )
    }
  }
})
