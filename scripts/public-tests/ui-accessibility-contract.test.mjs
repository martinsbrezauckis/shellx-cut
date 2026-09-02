import assert from 'node:assert/strict'
import { readdirSync, readFileSync, statSync } from 'node:fs'
import { dirname, join, relative, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import test from 'node:test'

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '../..')
const UI_SRC = join(ROOT, 'ui/src')

function source(path) {
  return readFileSync(join(ROOT, path), 'utf8')
}

function sourceFiles(dir) {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name)
    return statSync(path).isDirectory() ? sourceFiles(path) : path.endsWith('.tsx') ? [path] : []
  })
}

function luminance(hex) {
  const channels = [1, 3, 5]
    .map((offset) => Number.parseInt(hex.slice(offset, offset + 2), 16) / 255)
    .map((value) => value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4)
  return 0.2126 * channels[0] + 0.7152 * channels[1] + 0.0722 * channels[2]
}

function contrast(a, b) {
  const first = luminance(a)
  const second = luminance(b)
  return (Math.max(first, second) + 0.05) / (Math.min(first, second) + 0.05)
}

test('every blocking dialog uses the shared focus and Escape contract', () => {
  const nonBlockingDialogs = new Set([
    'ui/src/panels/Mask/index.tsx',
    'ui/src/topbar/PreflightWarning.tsx',
  ])
  const dialogs = sourceFiles(UI_SRC)
    .filter((path) => readFileSync(path, 'utf8').includes('role="dialog"'))
    .map((path) => relative(ROOT, path).replaceAll('\\', '/'))

  assert.ok(dialogs.length >= 20, `expected the full dialog inventory, got ${dialogs.length}`)
  for (const path of dialogs) {
    const text = source(path)
    if (nonBlockingDialogs.has(path)) {
      assert.doesNotMatch(text, /data-cut-blocking-overlay/, `${path} must stay explicitly non-modal`)
      continue
    }
    assert.match(text, /useBlockingOverlay/, `${path} imports the shared blocking contract`)
    assert.match(text, /aria-modal=/, `${path} declares modal semantics`)
    assert.match(text, /data-cut-blocking-overlay/, `${path} exposes the blocking-overlay identity`)
    assert.match(text, /onKeyDown=.*onDialogKeyDown/, `${path} installs focus containment and Escape`)
  }
})

test('blocking contract isolates nested and portalled dialog backgrounds', () => {
  const hook = source('ui/src/components/overlay/useBlockingOverlay.ts')
  for (const expected of [
    'function backgroundRegions',
    "document.querySelector<HTMLElement>('[data-cut-app-root]')",
    "!sibling.hasAttribute('data-cut-overlay-part')",
    '!dialog.isConnected',
  ]) {
    assert.ok(hook.includes(expected), `blocking contract owns ${expected}`)
  }
})

test('dark muted text tokens clear WCAG AA on every app surface', () => {
  const theme = source('ui/src/theme.css')
  const root = theme.match(/:root\s*\{([\s\S]*?)\n\}/)?.[1] || ''
  const token = (name) => root.match(new RegExp(`--${name}:\\s*(#[0-9a-fA-F]{6})`))?.[1]
  const background = token('surface-3')
  assert.ok(background, 'dark highest-elevation surface token exists')
  for (const name of ['ink-3', 'ink-4']) {
    const color = token(name)
    assert.ok(color, `${name} token exists`)
    const ratio = contrast(color, background)
    assert.ok(ratio >= 4.5, `${name} contrast ${ratio.toFixed(2)}:1 on surface-3 is below WCAG AA`)
  }
})

test('Settings capability cards expose one canonical status vocabulary', () => {
  const row = source('ui/src/panels/Environment/EnvCardRow.tsx')
  const css = source('ui/src/panels/Environment/environment.css')
  for (const label of ['Ready', 'Needs attention', 'Needs setup', 'Optional', 'Check again']) {
    assert.ok(row.includes(`label: '${label}'`), `canonical status ${label} exists`)
  }
  for (const stale of ["label: 'OK'", "label: 'DEGRADED'", "label: 'MISSING'", "CAN'T VERIFY"]) {
    assert.ok(!row.includes(stale), `stale status ${stale} is absent`)
  }
  assert.ok(!row.includes('serviceRuntimeRequirement'), 'service rows do not render a second connection status')
  assert.doesNotMatch(css.match(/[.]env-st\s*\{([\s\S]*?)\}/)?.[1] || '', /text-transform:\s*uppercase/)
})
