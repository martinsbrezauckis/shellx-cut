import assert from 'node:assert/strict'
import test from 'node:test'

import {
  DEFAULT_WINDOWS_QUALIFICATION_ROOT,
  isWslLocalhostQualificationRoot,
  normalizeWindowsQualificationRoot,
  windowsQualificationLayout,
} from '../lib/windows-qualification-layout.mjs'

test('Windows qualification defaults every disposable surface under one Cut root', () => {
  const layout = windowsQualificationLayout({ runId: 'windows-installed-final-20260810T120000Z' })
  assert.equal(layout.root, DEFAULT_WINDOWS_QUALIFICATION_ROOT)
  for (const path of [layout.stage, layout.evidence, layout.artifacts, layout.localAppData]) {
    assert.match(path, /^C:\\CutQ\\shellx-cut\\/)
  }
  assert.equal(layout.stage, String.raw`C:\CutQ\shellx-cut\runs\windows-installed-final-20260810T120000Z`)
  assert.equal(layout.localAppData, String.raw`C:\CutQ\shellx-cut\runs\windows-installed-final-20260810T120000Z\local-app-data`)
})

test('Windows qualification root rejects relative and broad destructive scopes', () => {
  for (const path of ['', '.', 'C:\\', String.raw`C:\CutQ`, String.raw`\\server\share\CutQ`]) {
    assert.throws(() => normalizeWindowsQualificationRoot(path))
  }
  assert.equal(
    normalizeWindowsQualificationRoot('D:\\Qualification\\shellx-cut\\'),
    String.raw`D:\Qualification\shellx-cut`,
  )
  assert.throws(() => windowsQualificationLayout({ runId: '../escape' }))
})

test('WSL workspace roots accept only the explicit wsl.localhost UNC form', () => {
  const root = String.raw`\\wsl.localhost\Ubuntu-24.04\home\martin-003\shellx-cut\.scratch\windows-native`
  assert.equal(isWslLocalhostQualificationRoot(root), true)
  assert.throws(() => normalizeWindowsQualificationRoot(root), /absolute drive path/)
  assert.equal(
    normalizeWindowsQualificationRoot(root, { allowWslLocalhost: true }),
    root,
  )
  assert.throws(() => normalizeWindowsQualificationRoot(String.raw`\\wsl$\Ubuntu-24.04\home\martin-003`, { allowWslLocalhost: true }))
  assert.throws(() => normalizeWindowsQualificationRoot(String.raw`\\server\share\shellx-cut`, { allowWslLocalhost: true }))
})
