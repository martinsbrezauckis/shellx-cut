import assert from 'node:assert/strict'
import { mkdtempSync, mkdirSync, readFileSync, rmSync, symlinkSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { execFileSync, spawnSync } from 'node:child_process'
import test from 'node:test'

const ROOT = resolve(import.meta.dirname, '..', '..')
const LAYOUT = 'scripts/lib/macos-build-target-layout.sh'

function layout(cargoTargetDir) {
  const output = execFileSync('bash', ['-c', [
    `source ${LAYOUT}`,
    'configure_macos_build_target_layout',
    'printf "identity=%s\\nengine=%s\\ntauri=%s\\nengine_env=%s\\ntauri_env=%s\\nexternal=%s\\nhelper=%s\\n" "$MACOS_CANDIDATE_TARGET_ROOT" "$CUTD_TARGET_ROOT" "$TAURI_TARGET_ROOT" "${MACOS_ENGINE_CARGO_ENV[*]}" "${MACOS_TAURI_CARGO_ENV[*]}" "$MACOS_EXTERNAL_CARGO_TARGET" "${SHELLX_CUT_TAURI_CARGO_TARGET_DIR:-}"',
  ].join('\n')], {
    cwd: ROOT,
    encoding: 'utf8',
    env: { ...process.env, CARGO_TARGET_DIR: cargoTargetDir },
  })
  return Object.fromEntries(output.trim().split('\n').map((line) => line.split(/=(.*)/s).slice(0, 2)))
}

test('macOS keeps the in-tree developer roots only when no candidate identity is allocated', () => {
  assert.deepEqual(layout(''), {
    identity: '',
    engine: 'app/target',
    tauri: 'app/desktop/src-tauri/target',
    engine_env: '',
    tauri_env: '',
    external: '0',
    helper: '',
  })
})

test('an absolute candidate identity derives separate canonical engine and desktop roots', () => {
  const fixture = mkdtempSync(join(tmpdir(), 'cut-macos-target-layout-'))
  try {
    const actualParent = join(fixture, 'actual-parent')
    const aliasParent = join(fixture, 'alias-parent')
    mkdirSync(actualParent)
    symlinkSync(actualParent, aliasParent)
    const requested = `${aliasParent}/unused/../candidate-target`
    const identity = join(actualParent, 'candidate-target')
    assert.deepEqual(layout(requested), {
      identity,
      engine: `${identity}/engine`,
      tauri: `${identity}/desktop-tauri`,
      engine_env: `CARGO_TARGET_DIR=${identity}/engine`,
      tauri_env: `CARGO_TARGET_DIR=${identity}/desktop-tauri`,
      external: '1',
      helper: `${identity}/desktop-tauri`,
    })
  } finally {
    rmSync(fixture, { recursive: true, force: true })
  }
})

test('the macOS 3.2 shell path never expands an empty nounset array', () => {
  const source = readFileSync(join(ROOT, LAYOUT), 'utf8')
  assert.doesNotMatch(source, /\$\{absent\[@\]\}/)
  assert.match(source, /absent_count=0[\s\S]+absent\[\$absent_count\]=[\s\S]+for \(\(index = absent_count - 1;/)
})

test('relative, root, ambiguous, and file candidate identities fail before either workspace can select output', () => {
  const fixture = mkdtempSync(join(tmpdir(), 'cut-macos-target-invalid-'))
  try {
    const file = join(fixture, 'not-a-directory')
    writeFileSync(file, 'not a directory')
    for (const [value, diagnostic] of [
      ['relative-target', /POSIX-absolute on macOS/],
      ['/', /filesystem root/],
      ['//server/share', /ambiguous \/\/ path/],
      [file, /directories, not a file/],
    ]) {
      const result = spawnSync('bash', ['-c', `source ${LAYOUT}; configure_macos_build_target_layout`], {
        cwd: ROOT,
        encoding: 'utf8',
        env: { ...process.env, CARGO_TARGET_DIR: value },
      })
      assert.equal(result.status, 2)
      assert.match(result.stderr, diagnostic)
    }
  } finally {
    rmSync(fixture, { recursive: true, force: true })
  }
})

test('macOS release signing rejects in-tree fallback and accepts only an allocated candidate identity', () => {
  const fixture = mkdtempSync(join(tmpdir(), 'cut-macos-target-release-'))
  try {
    const noTarget = spawnSync('bash', ['-c', [
      `source ${LAYOUT}`,
      'configure_macos_build_target_layout',
      'require_macos_release_candidate_target_layout',
    ].join('\n')], {
      cwd: ROOT,
      encoding: 'utf8',
      env: { ...process.env, CARGO_TARGET_DIR: '' },
    })
    assert.equal(noTarget.status, 2)
    assert.match(noTarget.stderr, /release signing requires an explicit absolute CARGO_TARGET_DIR candidate identity/)

    const candidate = join(fixture, 'candidate-target')
    const allocated = spawnSync('bash', ['-c', [
      `source ${LAYOUT}`,
      'configure_macos_build_target_layout',
      'require_macos_release_candidate_target_layout',
    ].join('\n')], {
      cwd: ROOT,
      encoding: 'utf8',
      env: { ...process.env, CARGO_TARGET_DIR: candidate },
    })
    assert.equal(allocated.status, 0, allocated.stderr)
  } finally {
    rmSync(fixture, { recursive: true, force: true })
  }
})

test('the maintained macOS pipeline selects and verifies artifacts only from the resolved candidate roots', () => {
  const build = readFileSync(join(ROOT, 'scripts', 'build-macos.sh'), 'utf8')
  const updater = readFileSync(join(ROOT, 'scripts', 'lib', 'tauri-updater-signing.sh'), 'utf8')
  const notary = readFileSync(join(ROOT, 'scripts', 'lib', 'macos-release-notarization.sh'), 'utf8')

  assert.match(build, /source scripts\/lib\/macos-build-target-layout[.]sh[\s\S]+configure_macos_build_target_layout[\s\S]+require_macos_release_candidate_target_layout/)
  const signingFallbackGuard = build.indexOf('require_macos_release_candidate_target_layout')
  const updaterAdmission = build.indexOf('prepare_tauri_updater_signing')
  const outputCleanup = build.indexOf('cleaning previous ShellX Cut DMGs')
  assert.ok(signingFallbackGuard >= 0 && signingFallbackGuard < updaterAdmission, 'release target fallback must fail before updater-key admission')
  assert.ok(signingFallbackGuard < outputCleanup, 'release target fallback must fail before candidate cleanup')
  assert.match(build, /bundle_root="\$TAURI_TARGET_ROOT\/\$TARGET\/\$MODE\/bundle"/)
  assert.match(build, /cutd_bin="\$CUTD_TARGET_ROOT\/\$TARGET\/\$MODE\/cutd"/)
  assert.match(build, /out="\$TAURI_TARGET_ROOT\/\$TARGET\/\$MODE"/)
  assert.match(build, /env "\$\{MACOS_ENGINE_CARGO_ENV\[@\]\}" cargo build/)
  assert.match(build, /export "\$\{MACOS_TAURI_CARGO_ENV\[@\]\}"[\s\S]+cargo tauri build/)
  assert.doesNotMatch(build, /^bundle_root="app\/desktop\/src-tauri\/target/m)
  assert.doesNotMatch(build, /^cutd_bin="app\/target/m)
  assert.doesNotMatch(build, /^out="app\/desktop\/src-tauri\/target/m)

  assert.match(updater, /run_tauri_cargo\(\)[\s\S]+CARGO_TARGET_DIR=\$SHELLX_CUT_TAURI_CARGO_TARGET_DIR/)
  assert.match(updater, /target_env=\("CARGO_TARGET_DIR=\$SHELLX_CUT_TAURI_CARGO_TARGET_DIR"\)/)
  assert.match(updater, /"\$\{target_env\[@\]\}"[\s\S]+cargo run --quiet --manifest-path app\/desktop\/src-tauri\/Cargo.toml/)
  assert.match(updater, /run_tauri_cargo tauri signer sign/)
  assert.match(notary, /macos_release_cargo tauri --version/)
})
