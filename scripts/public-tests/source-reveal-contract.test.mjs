import assert from 'node:assert/strict'
import test from 'node:test'
import { readFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..')
const read = (path) => readFileSync(resolve(root, path), 'utf8')

test('desktop source reveal permission stays identity-only and backgrounded', () => {
  const shell = read('app/desktop/src-tauri/src/lib.rs')
  const permission = read('app/desktop/src-tauri/permissions/source-reveal.toml')
  const resolver = read('app/desktop/src-tauri/src/source_reveal.rs')
  assert.match(permission, /identifier = "allow-reveal-registered-source"/)
  assert.match(permission, /commands\.allow = \["reveal_registered_source"\]/)
  assert.doesNotMatch(permission, /commands\.allow.*(?:filesystem|shell|process|path)/i)
  assert.match(shell, /async fn reveal_registered_source[\s\S]+tauri::async_runtime::spawn_blocking[\s\S]+reveal_registered_source_blocking/)
  assert.match(shell, /\.permission\("allow-reveal-registered-source"\)/)
  assert.match(resolver, /#\[serde\(tag = "status", rename_all = "snake_case"\)\]/)
  assert.match(resolver, /windows_reveal_args[\s\S]+"\/select,"/)
  assert.doesNotMatch(resolver, /fn reveal_registered_source\([^)]*path:/)
})
