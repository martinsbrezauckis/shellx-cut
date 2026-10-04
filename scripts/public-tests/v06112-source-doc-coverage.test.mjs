import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import test from 'node:test'

const root = new URL('../../', import.meta.url)

async function read(path) {
  return readFile(new URL(path, root), 'utf8')
}

test('published v0.6.112 capability history remains truthful under the current release identity', async () => {
  const [
    readme,
    appManifest,
    tauriConfig,
    uiPackage,
    features,
    manualIndex,
    manualSource,
    skill,
    reference,
    schema,
  ] = await Promise.all([
    read('README.md'),
    read('app/Cargo.toml'),
    read('app/desktop/src-tauri/tauri.conf.json'),
    read('ui/package.json'),
    read('docs/public/FEATURES.md'),
    read('docs/public/site/manual/cut/index.html'),
    read('docs/public/site/manual/manual.js'),
    read('skill/shellx-cut/SKILL.md'),
    read('skill/shellx-cut/reference.md'),
    read('schema/verbs.json'),
  ])

  const releaseTruth = JSON.parse(schema).release_truth
  const uiVersion = JSON.parse(uiPackage).version
  const versionPattern = releaseTruth.version.replaceAll('.', '[.]')
  const publishedPattern = releaseTruth.published_version.replaceAll('.', '[.]')
  assert.ok(['candidate', 'published'].includes(releaseTruth.status))
  assert.equal(uiVersion, releaseTruth.version)
  assert.equal(JSON.parse(tauriConfig).version, uiVersion)
  assert.match(appManifest, new RegExp(`version = "${versionPattern}"`))
  assert.match(readme, new RegExp(`STATUS — v${versionPattern} ${releaseTruth.status}`))
  assert.match(readme, new RegExp(`v${publishedPattern} is the current published installer/package`))

  for (const phrase of [
    'FFmpeg hardware',
    'native desktop pickers',
    'Source Monitor now has a seekable waveform',
    '24/25/30/50/60 FPS',
    'app-local microphone preference',
    'durable private Windows Pause session',
    'truthful zero-frame cancellation',
    'timeline scrolling',
  ]) assert.match(features, new RegExp(phrase.replaceAll('/', '\\/')))
  assert.match(features, /v0\.6\.112 published-release capabilities/)

  assert.match(manualIndex, new RegExp(`data-app-version="${versionPattern}"`))
  assert.match(manualIndex, new RegExp(`data-release-status="${releaseTruth.status}" data-published-version="${publishedPattern}"`))
  assert.match(manualSource, /Advanced frame rate accepts whole-number custom 1–240 FPS only after Apply or Enter/)
  assert.match(manualSource, /one selected microphone/)
  assert.match(manualSource, /there is no typed filesystem-path field/)
  assert.match(manualSource, /seekable waveform with amber In\/Out marks/)

  assert.match(skill, new RegExp(`Engine v${versionPattern} ${releaseTruth.status}`))
  assert.match(skill, /same cached\s+`media\.waveform` projection used by the timeline/)
  assert.match(skill, /Test microphone/)
  assert.match(reference, new RegExp(`v${versionPattern} ${releaseTruth.status}`))
  assert.match(reference, /shared by timeline and Source Monitor waveform views/)
  assert.match(reference, /`fps` 1\.\.240/)
})
