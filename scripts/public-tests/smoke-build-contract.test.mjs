import assert from 'node:assert/strict'
import { mkdtempSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { resolve } from 'node:path'
import { test } from 'node:test'
import { stageTauriCutd, tauriTargetForHost } from '../release/stage-tauri-cutd.mjs'

const read = (path) => readFileSync(path, 'utf8')

test('hosted smoke targets use the Tauri externalBin names for each shipped OS', () => {
  assert.deepEqual(tauriTargetForHost('linux', 'x64'), {
    target: 'x86_64-unknown-linux-gnu',
    executable: 'cutd',
  })
  assert.deepEqual(tauriTargetForHost('darwin', 'arm64'), {
    target: 'aarch64-apple-darwin',
    executable: 'cutd',
  })
  assert.deepEqual(tauriTargetForHost('darwin', 'x64'), {
    target: 'x86_64-apple-darwin',
    executable: 'cutd',
  })
  assert.deepEqual(tauriTargetForHost('win32', 'x64'), {
    target: 'x86_64-pc-windows-msvc',
    executable: 'cutd.exe',
  })
  assert.throws(() => tauriTargetForHost('linux', 'arm64'), /unsupported hosted Tauri smoke target/)
})

test('staging copies the exact host-built sidecar under Tauri target-qualified name', () => {
  const root = mkdtempSync(resolve(tmpdir(), 'cut-smoke-stage-'))
  const output = resolve(root, 'app', 'target', 'release')
  mkdirSync(output, { recursive: true })
  const source = resolve(output, 'cutd.exe')
  writeFileSync(source, 'exact host-built Cut engine')

  const staged = stageTauriCutd({ root, platform: 'win32', arch: 'x64' })
  assert.equal(staged.target, 'x86_64-pc-windows-msvc')
  assert.equal(
    staged.destination,
    resolve(root, 'app', 'desktop', 'src-tauri', 'binaries', 'cutd-x86_64-pc-windows-msvc.exe'),
  )
  assert.equal(read(staged.destination), read(source))
})

test('the manual smoke workflow builds and stages cutd before unsigned Tauri packaging', () => {
  const workflow = read('.github/workflows/release.yml')
  const tauri = JSON.parse(read('app/desktop/src-tauri/tauri.conf.json'))
  const engine = workflow.indexOf('cargo build --release --manifest-path app/Cargo.toml -p server --bin cutd')
  const stage = workflow.indexOf('node scripts/release/stage-tauri-cutd.mjs')
  const bundle = workflow.indexOf('npx --yes @tauri-apps/cli@2.11.2 build')

  assert.deepEqual(tauri.bundle?.externalBin, ['binaries/cutd'])
  assert.ok(engine >= 0, 'smoke build must build its external Cut engine')
  assert.ok(stage > engine, 'smoke build must stage the engine after it builds')
  assert.ok(bundle > stage, 'Tauri may package only after sidecar staging')
  assert.match(workflow, /createUpdaterArtifacts":false/)
  assert.doesNotMatch(workflow, /TAURI_SIGNING_PRIVATE_KEY|SHELLX_WINDOWS_SIGNING_REQUIRED/)
})
