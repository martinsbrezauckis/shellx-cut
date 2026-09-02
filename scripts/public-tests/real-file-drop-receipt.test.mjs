import test, { afterEach } from 'node:test'
import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { tmpdir } from 'node:os'

import {
  createRealFileDropReceipt,
  parseRealFileDropArgs,
} from '../lib/real-file-drop-receipt.mjs'

const roots = []

afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true })
})

const CASES = [
  {
    schemaVersion: 'shellx-cut/windows-installed-real-file-drop@1',
    surface: 'windows-installed', platform: 'win32', manager: 'explorer', mode: 'authenticode',
  },
  {
    schemaVersion: 'shellx-cut/macos-installed-real-file-drop@1',
    surface: 'macos-installed', platform: 'darwin', manager: 'finder', mode: 'codesign-notarization',
  },
  {
    schemaVersion: 'shellx-cut/linux-installed-real-file-drop@1',
    surface: 'linux-control', platform: 'linux', manager: 'nautilus', mode: 'package-integrity',
  },
]

function sha256(value) {
  return createHash('sha256').update(value).digest('hex')
}

function write(root, name, value) {
  const path = join(root, name)
  writeFileSync(path, value)
  return path
}

function fixture(dropCase = CASES[0]) {
  const root = mkdtempSync(join(tmpdir(), 'cut-real-file-drop-receipt-'))
  roots.push(root)
  const shellPath = write(root, 'shell', 'signed shell bytes')
  const cutdPath = write(root, 'cutd', 'signed engine bytes')
  const videoPath = write(root, 'video.mp4', 'video bytes')
  const imagePath = write(root, 'image.png', 'image bytes')
  const videoNative = write(root, 'video-native.json', '{"gesture":"recorded"}\n')
  const imageNative = write(root, 'image-native.json', '{"gesture":"recorded"}\n')
  const shellSha256 = sha256('signed shell bytes')
  const cutdSha256 = sha256('signed engine bytes')
  const videoSha256 = sha256('video bytes')
  const imageSha256 = sha256('image bytes')
  const gesture = dropCase.schemaVersion.includes('windows')
    ? 'real-explorer-ole-file-drag'
    : dropCase.schemaVersion.includes('macos')
      ? 'real-finder-file-window-drag'
      : 'real-nautilus-x11-file-drag'
  const project = (kind, mediaSha256, evidencePath) => {
    const name = `dropped-${kind}`
    return {
      kind,
      name,
      native: { gesture, protocol: `real ${dropCase.manager} drag`, evidencePath },
      state: {
        name,
        settings: { width: 1920, height: 1080, fps: 30 },
        assets: {
          a1: {
            hash: `sha256:${mediaSha256}`,
            probe: kind === 'video'
              ? { kind: 'video', width: 1920, height: 1080, fps: 30 }
              : { kind: 'image', width: 800, height: 600 },
          },
        },
        tracks: [{
          kind: 'video',
          clips: [{ asset: 'a1', src_in_ms: 0, src_out_ms: kind === 'image' ? 5_000 : 10_000 }],
        }],
      },
    }
  }
  return {
    schema: 'shellx-cut/real-file-drop-input@1',
    schemaVersion: dropCase.schemaVersion,
    surface: dropCase.surface,
    platform: dropCase.platform,
    manager: dropCase.manager,
    installedApp: true,
    source: { head: 'a'.repeat(40) },
    runtime: {
      shellPath,
      cutdPath,
      integrity: {
        schema: 'shellx-cut/real-file-drop-runtime-integrity@1',
        status: 'pass',
        surface: dropCase.surface,
        sourceHead: 'a'.repeat(40),
        pre: { shellSha256, cutdSha256 },
        post: { shellSha256, cutdSha256 },
        verification: { mode: dropCase.mode, shell: true, cutd: true },
      },
    },
    media: { videoPath, imagePath },
    projectsFirst: { left: { active_tab: 'projects', collapsed: false }, project: { open: false } },
    projects: [project('video', videoSha256, videoNative), project('image', imageSha256, imageNative)],
  }
}

test('generic producer emits one canonical receipt for each real file-manager surface', () => {
  for (const dropCase of CASES) {
    const receipt = createRealFileDropReceipt(fixture(dropCase), { generatedAt: '2026-08-12T00:00:00.000Z' })
    assert.equal(receipt.schema, dropCase.schemaVersion)
    assert.equal(receipt.source.head, 'a'.repeat(40))
    assert.equal(receipt.runtime.integrity.verification.mode, dropCase.mode)
    assert.equal(receipt.runtime.integrity.pre.cutdSha256, receipt.runtime.cutd.sha256)
    assert.equal(receipt.media.video.sha256.replace(/^sha256:/, ''), sha256('video bytes'))
    assert.deepEqual(receipt.checks.map((check) => check.id), [
      'projects-first',
      `video-real-${dropCase.manager}-drop-create`,
      `image-real-${dropCase.manager}-drop-create`,
    ])
    assert.equal(receipt.projects[1].state.assets.a1.probe.kind, 'image')
    assert.equal(receipt.projects[1].native.evidence.sha256, sha256('{"gesture":"recorded"}\n'))
  }
})

test('generic producer rejects unsealed runtime, synthetic gesture, and unbound media', () => {
  const mutations = [
    (input) => { input.runtime.integrity.post.cutdSha256 = 'f'.repeat(64) },
    (input) => { input.runtime.integrity.verification.shell = false },
    (input) => { input.projects[0].native.gesture = 'webdriver-event' },
    (input) => { input.projects[0].state.assets.a1.hash = `sha256:${'f'.repeat(64)}` },
    (input) => { input.projects[1].state.tracks[0].clips[0].src_out_ms = 4_000 },
  ]
  for (const mutate of mutations) {
    const input = fixture()
    mutate(input)
    assert.throws(() => createRealFileDropReceipt(input), /real file-drop receipt:/)
  }
})

test('CLI argument parser requires an explicit private input and output', () => {
  assert.deepEqual(parseRealFileDropArgs(['--input', 'draft.json', '--out', 'private/receipt.json']), {
    inputPath: 'draft.json', outPath: 'private/receipt.json', help: false,
  })
  assert.throws(() => parseRealFileDropArgs(['--unexpected']), /unknown argument/)
})
