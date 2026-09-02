import assert from 'node:assert/strict'
import { mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import test from 'node:test'

import { resolveWindowsInstalledMediaInputs } from '../lib/windows-installed-media-inputs.mjs'

test('Windows media inputs keep release roles explicit and reject missing local files', async () => {
  const root = await mkdtemp(join(tmpdir(), 'shellx-cut-windows-media-inputs-'))
  const media = Object.fromEntries(['scene', 'speech', 'face', 'speakers', 'second'].map((role) => [role, join(root, `${role}.mp4`)]))
  const arg = (name, fallback = '') => media[name.slice(2)] || fallback
  try {
    await Promise.all(Object.values(media).map((path) => writeFile(path, 'media fixture')))
    assert.deepEqual(resolveWindowsInstalledMediaInputs({
      root, signedFinal: false, requestedFace: media.face, arg, argv: [],
    }), media)
    assert.throws(
      () => resolveWindowsInstalledMediaInputs({ root, signedFinal: false, requestedFace: '', arg, argv: [] }),
      /requires --face with a detector-proven local video/,
    )
    assert.throws(
      () => resolveWindowsInstalledMediaInputs({
        root, signedFinal: true, requestedFace: media.face, arg,
        argv: ['--scene', '--speech', '--face', '--speakers'],
      }),
      /--signed-final requires --second with real release media/,
    )
    assert.deepEqual(resolveWindowsInstalledMediaInputs({
      root, signedFinal: true, requestedFace: media.face, arg,
      argv: ['--scene', '--speech', '--face', '--speakers', '--second'],
    }), media)
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})
