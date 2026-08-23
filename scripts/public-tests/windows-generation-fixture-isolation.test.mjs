import assert from 'node:assert/strict'
import { mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import test from 'node:test'

import { prepareWindowsQualificationEnvironment } from '../lib/windows-installed-qualification.mjs'

test('Windows signed-final generation mode stages no provider executable so every CLI resolves canonically', async () => {
  const root = await mkdtemp(join(tmpdir(), 'shellx-cut-windows-live-generation-env-'))
  const fixtureDir = join(root, 'stage')
  await mkdir(join(root, 'scripts/release/fixtures'), { recursive: true })
  await mkdir(join(root, 'ui/public-tests/fixtures'), { recursive: true })
  await mkdir(fixtureDir)
  const providerArtifacts = [
    'claude', 'claude.cmd', 'codex', 'codex.cmd', 'grok', 'grok.cmd',
    'agy', 'agy.cmd', 'agy-windows-launcher.rs', 'agy.exe',
    'agent-chat-provider-fixture.mjs', 'agent-edit-fixture.mjs',
  ]
  for (const name of providerArtifacts) await writeFile(join(root, 'scripts/release/fixtures', name), name)
  for (const name of ['comment-draft-adapter.py', 'judge-adapter.py']) {
    await writeFile(join(root, 'scripts/release/fixtures', name), name)
  }
  await writeFile(join(root, 'ui/public-tests/fixtures', 'generate-prompt-adapter.py'), 'prompt')
  await writeFile(join(root, 'ui/public-tests/fixtures', 'generate-storyboard-adapter.py'), 'storyboard')
  try {
    const env = prepareWindowsQualificationEnvironment({
      root,
      fixtureDir,
      fixtureWin: 'C:\\stage',
      fixtureProviders: [],
      harnessFfmpegWin: 'C:\\tools\\ffmpeg\\bin\\ffmpeg.exe',
      windowsBasePath: 'C:\\canonical-provider-bin;C:\\Windows\\System32',
      adapterPythonWin: 'C:\\perception\\python.exe',
      stageWin: 'C:\\candidate',
    })
    for (const name of ['comment-draft-adapter.py', 'judge-adapter.py']) {
      assert.equal(await readFile(join(fixtureDir, name), 'utf8'), name)
    }
    for (const name of [
      'claude', 'claude.cmd', 'codex', 'codex.cmd', 'grok', 'grok.cmd',
      'agy', 'agy.cmd', 'agy-windows-launcher.rs', 'agy.exe',
      'agent-chat-provider-fixture.mjs', 'agent-edit-fixture.mjs',
    ]) {
      await assert.rejects(readFile(join(fixtureDir, name), 'utf8'), `${name} must not shadow the canonical generation CLI`)
    }
    assert.equal(
      env.PATH,
      'C:\\stage;C:\\tools\\ffmpeg\\bin;C:\\canonical-provider-bin;C:\\Windows\\System32',
      'the staged adapter directory contains no provider executable; every CLI must resolve from the canonical base PATH or its registered install rung',
    )
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})
