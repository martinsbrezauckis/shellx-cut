import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import test from 'node:test'

const root = new URL('../../', import.meta.url)

async function read(path) {
  return readFile(new URL(path, root), 'utf8')
}

test('v0.6.112 source identity and user documentation stay coherent', async () => {
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
  ])

  const uiVersion = JSON.parse(uiPackage).version
  assert.equal(uiVersion, '0.6.112')
  assert.equal(JSON.parse(tauriConfig).version, uiVersion)
  assert.match(appManifest, /version = "0\.6\.112"/)
  assert.match(readme, /STATUS — 0\.6\.112 release/)
  assert.match(readme, /Installing 0\.6\.112/)

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
  assert.match(features, /current v0\.6\.112 release capabilities/)

  assert.match(manualIndex, /data-app-version="0\.6\.112">0\.6\.112</)
  assert.match(manualSource, /custom 1–240 FPS validation/)
  assert.match(manualSource, /one selected microphone/)
  assert.match(manualSource, /there is no typed filesystem-path field/)
  assert.match(manualSource, /seekable waveform with amber In\/Out marks/)

  assert.match(skill, /Engine v0\.6\.112/)
  assert.match(skill, /same cached\s+`media\.waveform` projection used by the timeline/)
  assert.match(skill, /Test microphone/)
  assert.match(reference, /v0\.6\.112 current/)
  assert.match(reference, /shared by timeline and Source Monitor waveform views/)
  assert.match(reference, /`fps` 1\.\.240/)
})
