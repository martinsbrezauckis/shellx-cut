import assert from 'node:assert/strict'
import { execFileSync, spawnSync } from 'node:child_process'
import { chmodSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { test } from 'node:test'

const ROOT = resolve(import.meta.dirname, '..', '..')
const LAYOUT = 'scripts/lib/windows-build-target-layout.sh'

function layout(cargoTargetDir, extraEnvironment = {}) {
  const output = execFileSync('bash', ['-c', [
    `source ${LAYOUT}`,
    'configure_windows_build_target_layout',
    'printf "cutd=%s\\ntauri=%s\\nenv=%s\\nexternal=%s\\n" "$CUTD_TARGET_ROOT" "$TAURI_TARGET_ROOT" "${WINDOWS_CARGO_TARGET_ENV[*]}" "$WINDOWS_EXTERNAL_CARGO_TARGET"',
  ].join('\n')], {
    cwd: ROOT,
    encoding: 'utf8',
    env: { ...process.env, CARGO_TARGET_DIR: cargoTargetDir, ...extraEnvironment },
  })
  return Object.fromEntries(output.trim().split('\n').map((line) => line.split(/=(.*)/s).slice(0, 2)))
}

test('Windows build keeps separate developer defaults only when no target is allocated', () => {
  assert.deepEqual(layout(''), {
    cutd: 'app/target',
    tauri: 'app/desktop/src-tauri/target',
    env: '',
    external: '0',
  })
})

test('an absolute CARGO_TARGET_DIR is canonicalized and propagated to both Cargo workspaces', () => {
  const requested = join(tmpdir(), 'cut-build-target-contract', '..', 'cut-build-target-canonical')
  const target = resolve(requested)
  assert.deepEqual(layout(requested), {
    cutd: target,
    tauri: target,
    env: `CARGO_TARGET_DIR=${target}`,
    external: '1',
  })
})

test('a Windows drive-absolute CARGO_TARGET_DIR is converted through cygpath and propagated to both Cargo workspaces', () => {
  const fixture = mkdtempSync(join(tmpdir(), 'cut-build-target-cygpath-'))
  try {
    const bin = join(fixture, 'bin')
    const invocation = join(fixture, 'cygpath-invocation.txt')
    mkdirSync(bin)
    const requested = 'C:\\Users\\Martin\\release-studio\\cargo-targets\\candidate-target'
    const cygpath = join(bin, 'cygpath')
    writeFileSync(cygpath, `#!/usr/bin/env bash
set -euo pipefail
[ "$1" = "-u" ] && [ "$2" = "--" ] && [ "$3" = 'C:\\Users\\Martin\\release-studio\\cargo-targets\\candidate-target' ]
printf '%s\\n' "$*" > "$CYGPATH_INVOCATION"
printf '%s\\n' '/c/Users/Martin/release-studio/cargo-targets/../candidate-target'
`)
    chmodSync(cygpath, 0o755)

    const target = '/c/Users/Martin/release-studio/candidate-target'
    assert.deepEqual(layout(requested, {
      PATH: `${bin}:${process.env.PATH}`,
      CYGPATH_INVOCATION: invocation,
    }), {
      cutd: target,
      tauri: target,
      env: `CARGO_TARGET_DIR=${target}`,
      external: '1',
    })
    assert.equal(readFileSync(invocation, 'utf8').trim(), `-u -- ${requested}`)
  } finally {
    rmSync(fixture, { recursive: true, force: true })
  }
})

test('relative, drive-relative, root, and UNC CARGO_TARGET_DIR values fail before a workspace can select output', () => {
  for (const [value, diagnostic] of [
    ['relative-target', /POSIX-absolute or Windows drive-absolute/],
    ['C:relative-target', /drive-relative/],
    ['/', /filesystem root/],
    ['C:\\', /Windows drive root/],
    ['\\\\server\\share', /ambiguous UNC path/],
    ['//server/share', /ambiguous UNC path/],
  ]) {
    const result = spawnSync('bash', ['-c', `source ${LAYOUT}; configure_windows_build_target_layout`], {
      cwd: ROOT,
      encoding: 'utf8',
      env: { ...process.env, CARGO_TARGET_DIR: value },
    })
    assert.equal(result.status, 2)
    assert.match(result.stderr, diagnostic)
  }
})

test('a drive-absolute target fails clearly when cygpath is unavailable', () => {
  const result = spawnSync('bash', ['-c', `source ${LAYOUT}; PATH=''; configure_windows_build_target_layout`], {
    cwd: ROOT,
    encoding: 'utf8',
    env: { ...process.env, CARGO_TARGET_DIR: 'C:\\Users\\Martin\\candidate-target' },
  })
  assert.equal(result.status, 2)
  assert.match(result.stderr, /cygpath -u is unavailable/)
})

test('the maintained Windows pipeline passes the resolved target explicitly to both Cargo commands and refuses external signing', () => {
  const build = readFileSync(resolve(ROOT, 'scripts/build-windows.sh'), 'utf8')
  assert.match(build, /source scripts\/lib\/windows-build-target-layout[.]sh[\s\S]+configure_windows_build_target_layout/)
  assert.match(build, /out="\$TAURI_TARGET_ROOT\/\$TARGET\/\$MODE"/)
  assert.match(build, /cutd_exe="\$CUTD_TARGET_ROOT\/\$TARGET\/\$MODE\/cutd[.]exe"/)
  assert.match(build, /env "\$\{WINDOWS_CARGO_TARGET_ENV\[@\]\}" RUSTFLAGS="-C target-feature=\+crt-static" cargo xwin build/)
  assert.match(build, /env "\$\{WINDOWS_CARGO_TARGET_ENV\[@\]\}" "\$\{TAURI_SIDECAR_SIGNATURE_SKIP\[@\]\}"[\s\S]+cargo tauri build/)
  assert.match(build, /WINDOWS_EXTERNAL_CARGO_TARGET" = "1"[\s\S]+WINDOWS_AUTHENTICODE_SIGNING" = "1"[\s\S]+supported only for unsigned provenance builds/)
  assert.doesNotMatch(build, /^out="app\/desktop\/src-tauri\/target/m)
  assert.doesNotMatch(build, /^cutd_exe="app\/target/m)
})
