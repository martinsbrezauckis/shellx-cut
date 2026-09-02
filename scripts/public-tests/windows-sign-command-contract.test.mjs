import assert from 'node:assert/strict'
import { execFileSync, spawnSync } from 'node:child_process'
import { accessSync, chmodSync, constants, existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { createHash } from 'node:crypto'
import { dirname, isAbsolute, join, resolve } from 'node:path'
import { tmpdir } from 'node:os'
import { fileURLToPath } from 'node:url'
import { test } from 'node:test'

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..')

function read(path) {
  return readFileSync(resolve(ROOT, path), 'utf8')
}

function tauriBundler211SignArguments({ args }, artifact, cwd) {
  return args.map((arg) => {
    if (arg === '%1') return artifact

    // Tauri 2.11 / tauri-bundler 2.9 resolves only existing relative command
    // arguments against its own current working directory. This is required for
    // the later NSIS uninstaller invocation, which has a different cwd.
    const candidate = resolve(cwd, arg)
    return !isAbsolute(arg) && existsSync(candidate) ? candidate : arg
  })
}

function withoutSigningEnvironment() {
  return Object.fromEntries(
    Object.entries(process.env).filter(
      ([key]) => !/^SHELLX_WINDOWS_(?:SIGNING|ARTIFACT_SIGNING)_/.test(key),
    ),
  )
}

test('Windows Tauri sign command resolves the maintained hook from tauri-bundler cwd', () => {
  const config = JSON.parse(read('app/desktop/src-tauri/tauri.conf.json'))
  const signCommand = config.bundle?.windows?.signCommand
  assert.deepEqual(signCommand, {
    cmd: 'bash', args: ['../scripts/windows-artifact-sign.sh', '%1'],
  })

  const tauriBundlerCwd = resolve(ROOT, 'app/desktop/src-tauri')
  const temporaryDir = mkdtempSync(join(tmpdir(), 'shellx-cut-windows-sign-command-'))
  const inertArtifact = join(temporaryDir, 'ShellX Cut inert artifact.exe')

  try {
    writeFileSync(inertArtifact, 'inert Windows signing contract artifact\n')
    const bashArgs = tauriBundler211SignArguments(signCommand, inertArtifact, tauriBundlerCwd)

    assert.deepEqual(bashArgs, [
      resolve(ROOT, 'app/desktop/scripts/windows-artifact-sign.sh'),
      inertArtifact,
    ])
    assert.deepEqual(
      tauriBundler211SignArguments(
        { args: ['./not-an-existing-sign-hook'] },
        inertArtifact,
        tauriBundlerCwd,
      ),
      ['./not-an-existing-sign-hook'],
      'tauri-bundler must retain a relative argument when it does not exist from its cwd',
    )
    accessSync(bashArgs[0], constants.X_OK)

    const stdout = execFileSync(signCommand.cmd, bashArgs, {
      cwd: tauriBundlerCwd,
      encoding: 'utf8',
      env: withoutSigningEnvironment(),
    })
    assert.match(stdout, /windows-artifact-sign: unsigned public-source build/)
  } finally {
    rmSync(temporaryDir, { recursive: true, force: true })
  }
})

test('Windows signing event log remains absolute when Tauri runs the hook from src-tauri', () => {
  const config = JSON.parse(read('app/desktop/src-tauri/tauri.conf.json'))
  const tauriBundlerCwd = resolve(ROOT, 'app/desktop/src-tauri')
  const temporaryDir = mkdtempSync(join(tmpdir(), 'shellx-cut-windows-sign-event-'))
  const inertArtifact = join(temporaryDir, 'ShellX Cut inert artifact.exe')
  const fakeHelper = join(temporaryDir, 'inert-signing-helper.sh')
  const eventLog = join(temporaryDir, 'windows-artifact-sign-events.jsonl')

  try {
    writeFileSync(inertArtifact, 'inert Windows signing event artifact\n')
    writeFileSync(fakeHelper, '#!/usr/bin/env bash\nset -euo pipefail\nexit 0\n')
    chmodSync(fakeHelper, 0o755)
    writeFileSync(eventLog, '')

    const bashArgs = tauriBundler211SignArguments(
      config.bundle.windows.signCommand,
      inertArtifact,
      tauriBundlerCwd,
    )
    const signingEnvironment = {
      ...withoutSigningEnvironment(),
      SHELLX_WINDOWS_SIGNING_REQUIRED: '1',
      SHELLX_WINDOWS_SIGNING_HELPER: fakeHelper,
      SHELLX_WINDOWS_ARTIFACT_SIGNING_EVENT_LOG: eventLog,
    }

    execFileSync(config.bundle.windows.signCommand.cmd, bashArgs, {
      cwd: tauriBundlerCwd,
      encoding: 'utf8',
      env: signingEnvironment,
    })
    const rows = readFileSync(eventLog, 'utf8').trim().split('\n').map((line) => JSON.parse(line))
    assert.deepEqual(rows, [{
      artifactPath: resolve(inertArtifact),
      name: 'ShellX Cut inert artifact.exe',
      sha256: createHash('sha256').update(readFileSync(inertArtifact)).digest('hex'),
      signatureStatus: 'Valid',
    }])

    const relativeEventLog = spawnSync(config.bundle.windows.signCommand.cmd, bashArgs, {
      cwd: tauriBundlerCwd,
      encoding: 'utf8',
      env: {
        ...signingEnvironment,
        SHELLX_WINDOWS_ARTIFACT_SIGNING_EVENT_LOG: 'relative-signing-events.jsonl',
      },
    })
    assert.equal(relativeEventLog.status, 1, 'a relative event-log path must fail after the bundler changes cwd')
    assert.match(
      relativeEventLog.stderr,
      /signing event log does not exist: relative-signing-events[.]jsonl/,
    )

    assert.match(
      read('scripts/build-windows.sh'),
      /signing_event_log="\$\(realpath -e "\$out"\)\/windows-artifact-sign-events[.]jsonl"/,
      'the signed build must export an absolute event log before cargo-tauri changes cwd',
    )
  } finally {
    rmSync(temporaryDir, { recursive: true, force: true })
  }
})
