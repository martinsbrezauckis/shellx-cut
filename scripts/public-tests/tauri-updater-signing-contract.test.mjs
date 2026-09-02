import assert from 'node:assert/strict'
import { mkdtempSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs'
import { execFileSync } from 'node:child_process'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { test } from 'node:test'

const read = (path) => readFileSync(path, 'utf8')

function validateUpdaterKeyContract(config, contract, shell) {
  const match = contract.match(/UPDATER_PUBLIC_KEY: &str =\s*\n\s*"([A-Za-z0-9+/=]+)"/)
  assert.ok(match, 'the runtime public key must be a literal public build input')

  const runtimeKey = match[1]
  const buildKey = config.plugins?.updater?.pubkey
  assert.equal(typeof buildKey, 'string')
  assert.equal(runtimeKey, buildKey, 'bundle-time and runtime updater keys must match')
  assert.match(Buffer.from(runtimeKey, 'base64').toString('utf8'), /minisign public key/)
  assert.doesNotMatch(Buffer.from(runtimeKey, 'base64').toString('utf8'), /secret key/i)
  assert.match(contract, /Builder::new\(\)\.pubkey\(UPDATER_PUBLIC_KEY\)/)
  assert.match(shell, /updater_key_transition::plugin_builder\(\)[\s\S]+default_version_comparator/)
  assert.doesNotMatch(
    `${contract}\n${shell}`,
    /TAURI_SIGNING_PRIVATE_KEY|minisign secret key|pass\s+(?:show|insert)/i,
    'runtime source must contain neither private-key inputs nor secret-store coordinates',
  )
  return runtimeKey
}

test('both desktop builders use the shared fail-closed updater contract', () => {
  for (const [label, path] of [['Windows', 'scripts/build-windows.sh'], ['macOS', 'scripts/build-macos.sh']]) {
    const source = read(path)
    assert.notEqual(
      statSync(path).mode & 0o111,
      0,
      `${label} release builder must retain an executable Git mode`,
    )
    assert.match(
      source,
      /source scripts\/lib\/tauri-updater-signing\.sh[\s\S]+prepare_tauri_updater_signing[\s\S]+reject_tauri_updater_key_mismatch[\s\S]+verify_tauri_updater_artifact/,
      `${label} build must use the shared fail-closed updater key and signature contract`,
    )
    const cargoInvocation = label === 'Windows'
      ? /\(\s*\n\s*cd app\/desktop\s*\n\s*materialize_tauri_updater_signing_key\s*\n\s*env[\s\S]{0,160}cargo tauri build/
      : /\(\s*\n\s*cd app\/desktop\s*\n\s*materialize_tauri_updater_signing_key\s*\n\s*cargo tauri build/
    assert.match(
      source,
      cargoInvocation,
      `${label} must materialize a key-file input only in the cargo-tauri subprocess`,
    )
  }
})

test('Windows accepts readiness helper path names for Authenticode tools', () => {
  const windowsBuild = read('scripts/build-windows.sh')
  const signWrapper = read('app/desktop/scripts/windows-artifact-sign.sh')
  assert.match(
    windowsBuild,
    /SHELLX_WINDOWS_SIGNTOOL:-\$\{SHELLX_WINDOWS_SIGNTOOL_PATH:-\}[\s\S]+SHELLX_WINDOWS_DLIB:-\$\{SHELLX_WINDOWS_SIGNING_DLIB_PATH:-\}/,
    'the signed Windows build contract must accept readiness helper path names',
  )
  assert.match(
    signWrapper,
    /official signing requires an executable SHELLX_WINDOWS_SIGNING_HELPER/,
    'the public signing hook must fail closed without an explicit external helper',
  )
  assert.match(signWrapper, /refusing a recursive signing helper/)
  assert.match(signWrapper, /"\$helper" "\$artifact"[\s\S]+record_signed_artifact/)
})

test('Windows signs final binaries after Tauri mutation and verifies every shipped exe', () => {
  const windowsBuild = read('scripts/build-windows.sh')
  const preStageSidecar = windowsBuild.indexOf('windows-artifact-sign.sh "$cutd_exe"')
  const sidecarCopy = windowsBuild.indexOf('cp "$cutd_exe" "$sidecar_exe"')
  const tauriBuild = windowsBuild.indexOf('cargo tauri build')
  const finalShell = windowsBuild.indexOf('windows-artifact-sign.sh "$shell_exe"')
  const finalInstaller = windowsBuild.indexOf('verify_windows_authenticode "$installer"')

  assert.ok(preStageSidecar >= 0 && preStageSidecar < sidecarCopy, 'the sidecar must be signed before exact staging')
  assert.match(
    windowsBuild,
    /TAURI_SKIP_SIDECAR_SIGNATURE_CHECK=true[\s\S]+cargo tauri build/,
    'Tauri must preserve the already-signed sidecar payload bytes',
  )
  assert.ok(tauriBuild >= 0 && tauriBuild < finalShell, 'the final standalone shell must be signed after Tauri restores it')
  assert.match(
    windowsBuild,
    /verify_windows_authenticode "\$shell_exe"[\s\S]+verify_windows_authenticode "\$sidecar_exe"/,
    'post-build verification must reject unsigned shell and engine artifacts',
  )
  assert.ok(finalInstaller >= 0, 'post-build verification must reject an unsigned installer')
})

test('key text and key-file paths are distinct and verified', () => {
  const contract = read('scripts/lib/tauri-updater-signing.sh')
  assert.match(
    contract,
    /TAURI_SIGNING_PRIVATE_KEY.*KEY TEXT[\s\S]+TAURI_SIGNING_PRIVATE_KEY_PATH[\s\S]+set exactly one/,
    'the shared updater contract must distinguish key text from a key-file path',
  )
  assert.match(
    contract,
    /write_tauri_updater_artifact_identity[\s\S]+cargo tauri signer sign[\s\S]+verify_tauri_updater_artifact/,
    'the release helper must sign and independently verify every canonical artifact identity',
  )
  assert.match(contract, /version=\$version[\s\S]+platform=\$platform[\s\S]+sha256=\$sha256/)
  assert.match(
    contract,
    /contains a file path; use TAURI_SIGNING_PRIVATE_KEY_PATH/,
    'a filesystem path in the key-text variable must fail before Tauri signs',
  )
  assert.match(
    contract,
    /env -u TAURI_SIGNING_PRIVATE_KEY -u TAURI_SIGNING_PRIVATE_KEY_PATH[\s\S]+verify-updater-signature/,
    'the shared updater contract must independently verify the produced signature without exposing a private key',
  )
  assert.match(
    contract,
    /does not match the public key from[\s\S]+refusing an unusable updater artifact/,
    'a Tauri key-mismatch warning must abort the build',
  )
  assert.match(contract, /SHELLX_TAURI_UPDATER_PUBLIC_KEY_PATH/)
  assert.ok(
    contract.indexOf('assert_tauri_updater_public_key_admission')
      < contract.indexOf('[ ! -s "$TAURI_SIGNING_PRIVATE_KEY_PATH" ]'),
    'the public updater-key admission must run before the protected private-key path is probed',
  )
})

test('both signed desktop updater packages receive a version-and-byte identity', () => {
  const windowsBuild = read('scripts/build-windows.sh')
  const macosBuild = read('scripts/build-macos.sh')
  assert.match(
    windowsBuild,
    /verify_tauri_updater_artifact "\$installer" "\$updater_sig"[\s\S]+write_tauri_updater_artifact_identity "\$installer" "windows-x86_64" "\$version"/,
  )
  assert.match(
    macosBuild,
    /verify_tauri_updater_artifact "\$updater_archive" "\$updater_sig"[\s\S]+write_tauri_updater_artifact_identity "\$updater_archive" "darwin-aarch64" "\$version"/,
  )
})

test('the identity signer verifies raw cargo-tauri output before canonicalizing its public signature field', () => {
  const contract = read('scripts/lib/tauri-updater-signing.sh')
  const verify = contract.indexOf('verify_tauri_updater_artifact "$identity" "$identity_signature"')
  const normalize = contract.indexOf('normalize-tauri-updater-signature.mjs --signature-file "$identity_signature"')
  assert.ok(verify >= 0, 'the raw signer output must be cryptographically verified')
  assert.ok(normalize > verify, 'only verified raw signer output may be normalized for the manifest contract')
})

test('desktop updater uses one public key at bundle time and runtime', () => {
  const config = JSON.parse(read('app/desktop/src-tauri/tauri.conf.json'))
  const contract = read('app/desktop/src-tauri/src/updater_key_transition.rs')
  const shell = read('app/desktop/src-tauri/src/lib.rs')
  validateUpdaterKeyContract(config, contract, shell)
})

test('updater-key contract fails closed on unsafe key shapes', () => {
  const config = JSON.parse(read('app/desktop/src-tauri/tauri.conf.json'))
  const contract = read('app/desktop/src-tauri/src/updater_key_transition.rs')
  const shell = read('app/desktop/src-tauri/src/lib.rs')
  const runtimeKey = validateUpdaterKeyContract(config, contract, shell)

  assert.throws(
    () => validateUpdaterKeyContract(
      { ...config, plugins: { ...config.plugins, updater: { ...config.plugins.updater, pubkey: 'different-public-key' } } },
      contract,
      shell,
    ),
    /must match/,
  )
  assert.throws(
    () => validateUpdaterKeyContract(config, contract.replace(runtimeKey, 'not-base64'), shell),
    /literal public build input/,
  )
  assert.throws(
    () => validateUpdaterKeyContract(config, contract.replace('.pubkey(UPDATER_PUBLIC_KEY)', ''), shell),
    /pubkey/,
  )
  assert.throws(
    () => validateUpdaterKeyContract(config, `${contract}\n// TAURI_SIGNING_PRIVATE_KEY`, shell),
    /private-key inputs/,
  )
})

test('a caller key file is materialized only for the cargo-tauri subprocess contract', () => {
  const temp = mkdtempSync(join(tmpdir(), 'shellx-cut-updater-contract-'))
  const keyPath = join(temp, 'updater.key')
  const publicKeyPath = join(temp, 'updater.key.pub')
  writeFileSync(keyPath, 'test-key-material', { mode: 0o600 })
  writeFileSync(publicKeyPath, JSON.parse(read('app/desktop/src-tauri/tauri.conf.json')).plugins.updater.pubkey)
  try {
    execFileSync('bash', ['-ceu', `
      source scripts/lib/tauri-updater-signing.sh
      unset TAURI_SIGNING_PRIVATE_KEY TAURI_SIGNING_PRIVATE_KEY_PATH
      export TAURI_SIGNING_PRIVATE_KEY_PATH="$1"
      export SHELLX_TAURI_UPDATER_PUBLIC_KEY_PATH="$2"
      prepare_tauri_updater_signing
      [ -z "\${TAURI_SIGNING_PRIVATE_KEY+x}" ]
      materialize_tauri_updater_signing_key
      [ "\$TAURI_SIGNING_PRIVATE_KEY" = 'test-key-material' ]
      [ -z "\${TAURI_SIGNING_PRIVATE_KEY_PATH+x}" ]
    `, '--', keyPath, publicKeyPath], { stdio: 'pipe' })
    assert.throws(
      () => execFileSync('bash', ['-ceu', `
        source scripts/lib/tauri-updater-signing.sh
        export TAURI_SIGNING_PRIVATE_KEY="$1"
        export SHELLX_TAURI_UPDATER_PUBLIC_KEY_PATH="$2"
        prepare_tauri_updater_signing
      `, '--', keyPath, publicKeyPath], { stdio: 'pipe' }),
      { status: 1 },
      'a filesystem path is never accepted as literal key text',
    )
  } finally {
    rmSync(temp, { recursive: true, force: true })
  }
})

test('the updater admission accepts only a supplied encoded public-key record before signing', () => {
  const temp = mkdtempSync(join(tmpdir(), 'shellx-cut-updater-public-admission-'))
  const publicKeyPath = join(temp, 'updater.key.pub')
  const config = JSON.parse(read('app/desktop/src-tauri/tauri.conf.json'))
  try {
    writeFileSync(publicKeyPath, `${config.plugins.updater.pubkey}\r\n`)
    const output = execFileSync('node', [
      'scripts/release/verify-tauri-updater-public-key.mjs', '--public-key-file', publicKeyPath,
    ], { encoding: 'utf8' })
    assert.match(output, /UPDATER_PUBLIC_KEY_ADMISSION_OK sha256=[a-f0-9]{64}/)
    writeFileSync(publicKeyPath, 'not-the-configured-public-key\n')
    assert.throws(
      () => execFileSync('node', [
        'scripts/release/verify-tauri-updater-public-key.mjs', '--public-key-file', publicKeyPath,
      ], { stdio: 'pipe' }),
      { status: 1 },
    )
  } finally {
    rmSync(temp, { recursive: true, force: true })
  }
})

test('Windows signed artifact receipt is emitted after final bundle signing evidence', () => {
  const windowsBuild = read('scripts/build-windows.sh')
  const signWrapper = read('app/desktop/scripts/windows-artifact-sign.sh')
  for (const generatedSuffix of [
    '.exe', '.exe.sig', '.exe.identity', '.exe.identity.sig', '.exe.receipt.json',
    '.msi', '.msi.sig', '.msi.identity', '.msi.identity.sig', '.msi.receipt.json',
  ]) {
    assert.match(
      windowsBuild,
      new RegExp(`-name 'ShellX Cut_[*]${generatedSuffix.replaceAll('.', '[.]')}'`),
      `the Windows build must remove stale ${generatedSuffix} package evidence before rebuilding`,
    )
  }
  assert.match(signWrapper, /SHELLX_WINDOWS_ARTIFACT_SIGNING_EVENT_LOG/)
  assert.match(signWrapper, /createHash\('sha256'\)[.]update\(bytes\)[.]digest\('hex'\)/)
  assert.match(windowsBuild, /windows-artifact-sign-events[.]jsonl/)
  assert.match(windowsBuild, /write-windows-signed-artifact-receipt[.]mjs/)
  assert.ok(
    windowsBuild.indexOf('verify_windows_authenticode "$installer"')
      < windowsBuild.indexOf('write-windows-signed-artifact-receipt.mjs'),
    'the receipt must be produced only after final installer Authenticode verification',
  )
})
