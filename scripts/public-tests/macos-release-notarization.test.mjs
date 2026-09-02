import assert from 'node:assert/strict'
import { chmodSync, mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { spawnSync } from 'node:child_process'
import test from 'node:test'

const root = resolve(import.meta.dirname, '..', '..')
const helper = join(root, 'scripts', 'lib', 'macos-release-notarization.sh')
const buildScript = join(root, 'scripts', 'build-macos.sh')
const buildingGuide = join(root, 'docs', 'public', 'BUILDING.md')

function writeExecutable(path, source) {
  writeFileSync(path, source, { mode: 0o755 })
  chmodSync(path, 0o755)
}

function fixture() {
  const dir = mkdtempSync(join(tmpdir(), 'cut-macos-notary-fixture-'))
  const bin = join(dir, 'bin')
  const log = join(dir, 'commands.log')
  const dmg = join(dir, 'ShellX Cut_0.6.110_aarch64.dmg')
  const apiKey = join(dir, 'AuthKey_fixture.p8')
  mkdirSync(bin)
  writeFileSync(dmg, 'fixture DMG bytes')
  writeFileSync(apiKey, 'fixture API key bytes')

  writeExecutable(join(bin, 'xcrun'), `#!/usr/bin/env bash
set -euo pipefail
{ printf 'xcrun'; printf ' <%s>' "$@"; printf '\\n'; } >> "$FAKE_NOTARY_LOG"
case "\${1:-}:\${2:-}" in
  --find:notarytool|--find:stapler) printf '/fixture/%s\\n' "$2" ;;
  notarytool:submit)
    printf '%b\\n' "\${FAKE_NOTARY_OUTPUT:-id: fixture\\nstatus: Accepted}"
    exit "\${FAKE_SUBMIT_STATUS:-0}"
    ;;
  stapler:staple) exit "\${FAKE_STAPLE_STATUS:-0}" ;;
  stapler:validate) exit "\${FAKE_STAPLE_VALIDATE_STATUS:-0}" ;;
  *) echo "unexpected xcrun command" >&2; exit 99 ;;
esac
`)
  for (const tool of ['node', 'npm', 'cargo', 'codesign', 'hdiutil', 'shasum', 'stat']) {
    writeExecutable(join(bin, tool), '#!/usr/bin/env bash\nexit 0\n')
  }
  writeExecutable(join(bin, 'spctl'), `#!/usr/bin/env bash
set -euo pipefail
{ printf 'spctl'; printf ' <%s>' "$@"; printf '\\n'; } >> "$FAKE_NOTARY_LOG"
exit "\${FAKE_SPCTL_STATUS:-0}"
`)

  const env = {
    PATH: `${bin}:${process.env.PATH}`,
    FAKE_NOTARY_LOG: log,
    APPLE_SIGNING_IDENTITY: 'Developer ID Application: Martins Brezauckis (4M329JW6R4)',
    APPLE_API_KEY: 'fixture-key-id',
    APPLE_API_ISSUER: 'fixture-issuer-id',
    APPLE_API_KEY_PATH: apiKey,
    TAURI_UPDATER_ARTIFACTS_SIGNED: '1',
  }
  return { dir, dmg, log, env }
}

function runHelper({ env, body }) {
  return spawnSync('bash', ['-c', `set -euo pipefail\nsource "${helper}"\n${body}`], {
    cwd: root,
    env,
    encoding: 'utf8',
  })
}

function withFixture(callback) {
  const value = fixture()
  try {
    return callback(value)
  } finally {
    rmSync(value.dir, { recursive: true, force: true })
  }
}

function appVerificationFixture() {
  const dir = mkdtempSync(join(tmpdir(), 'cut-macos-app-verify-fixture-'))
  const bin = join(dir, 'bin')
  const log = join(dir, 'commands.log')
  const app = join(dir, 'ShellX Cut.app')
  mkdirSync(bin)
  mkdirSync(app)
  writeExecutable(join(bin, 'codesign'), `#!/usr/bin/env bash
set -euo pipefail
{ printf 'codesign'; printf ' <%s>' "$@"; printf '\\n'; } >> "$FAKE_NOTARY_LOG"
if [ "\${1:-}" = '-d' ]; then
  printf '%s\\n' "\${FAKE_CODESIGN_DETAILS:-Identifier=lv.shellx.cut
TeamIdentifier=4M329JW6R4
Authority=Developer ID Application: Martins Brezauckis (4M329JW6R4)}" >&2
fi
exit 0
`)
  writeExecutable(join(bin, 'spctl'), `#!/usr/bin/env bash
set -euo pipefail
{ printf 'spctl'; printf ' <%s>' "$@"; printf '\\n'; } >> "$FAKE_NOTARY_LOG"
exit "\${FAKE_SPCTL_STATUS:-0}"
`)
  writeExecutable(join(bin, 'xcrun'), `#!/usr/bin/env bash
set -euo pipefail
{ printf 'xcrun'; printf ' <%s>' "$@"; printf '\\n'; } >> "$FAKE_NOTARY_LOG"
case "\${1:-}:\${2:-}" in
  stapler:validate) exit "\${FAKE_STAPLE_VALIDATE_STATUS:-0}" ;;
  *) echo "unexpected xcrun command" >&2; exit 99 ;;
esac
`)
  return {
    dir,
    log,
    app,
    env: { PATH: `${bin}:${process.env.PATH}`, FAKE_NOTARY_LOG: log },
  }
}

function withAppVerificationFixture(callback) {
  const value = appVerificationFixture()
  try {
    return callback(value)
  } finally {
    rmSync(value.dir, { recursive: true, force: true })
  }
}

test('release build wires fail-closed notarization after the signed updater identity', () => {
  const source = readFileSync(buildScript, 'utf8')
  const helperSource = readFileSync(helper, 'utf8')
  const guide = readFileSync(buildingGuide, 'utf8')
  const releaseAdmission = source.indexOf('require_macos_release_admission')
  const outputCleanup = source.indexOf('cleaning previous ShellX Cut DMGs')
  const appVerification = source.indexOf('verify_macos_release_app_bundle "$app_bundle"')
  const updaterIdentity = source.indexOf('write_tauri_updater_artifact_identity')
  const notarize = source.indexOf('notarize_release_dmg "$dmg"')
  const dmgAppIdentity = source.indexOf('verify_macos_release_dmg_app_identity "$dmg" "$app_bundle"')
  const finalDmgHash = source.indexOf('sha "$dmg"', notarize)

  assert.match(source, /source scripts\/lib\/macos-release-notarization[.]sh/)
  assert.match(source, /if \[ "\$MODE" = "release" \]; then\n  require_macos_release_admission/)
  assert.match(source, /NOT RELEASE-QUALIFIED/)
  assert.ok(releaseAdmission >= 0 && releaseAdmission < outputCleanup, 'release admission must precede output cleanup')
  assert.ok(updaterIdentity >= 0, 'signed updater identity remains required')
  assert.ok(appVerification > updaterIdentity, '.app verification follows the completed signed updater bundle')
  assert.ok(notarize > appVerification, 'DMG notarization follows independently qualified app evidence')
  assert.ok(dmgAppIdentity > notarize, 'final DMG must be byte-bound to the verified .app after stapling')
  assert.ok(finalDmgHash > dmgAppIdentity, 'DMG hash is recorded only after stapling and app identity binding')
  const appVerificationBody = helperSource.slice(
    helperSource.indexOf('verify_macos_release_app_bundle()'),
    helperSource.indexOf('verify_macos_release_dmg_app_identity()'),
  )
  assert.match(appVerificationBody, /spctl --assess --type execute/, 'generated app must independently pass Gatekeeper before DMG acceptance')
  assert.match(appVerificationBody, /xcrun stapler validate/, 'generated app must carry a validated notarization ticket before DMG acceptance')
  assert.match(guide, /The generated `[.]app` must pass[\s\S]+Gatekeeper, and stapled[\s\S]+before the final DMG is accepted/)
  assert.match(guide, /Both artifacts are required[\s\S]+regardless of the cargo-tauri implementation/)
})

test('release credentials are required before DMG notarization', () => {
  const result = runHelper({
    env: {
      PATH: process.env.PATH,
      APPLE_SIGNING_IDENTITY: 'Developer ID Application: Martins Brezauckis (4M329JW6R4)',
      TAURI_UPDATER_ARTIFACTS_SIGNED: '1',
    },
    body: 'require_macos_release_credentials',
  })

  assert.equal(result.status, 1)
  assert.match(result.stderr, /APPLE_API_KEY APPLE_API_ISSUER APPLE_API_KEY_PATH/)
})

test('release admission validates the expected Developer ID and read-only local toolchain before build work', () => withFixture(({ env, log }) => {
  const result = runHelper({ env, body: 'require_macos_release_admission' })

  assert.equal(result.status, 0, result.stderr)
  assert.match(readFileSync(log, 'utf8'), /xcrun <--find> <notarytool>/)
  assert.match(readFileSync(log, 'utf8'), /xcrun <--find> <stapler>/)
}))

test('release admission rejects an unexpected signing identity before build work', () => withFixture(({ env }) => {
  const result = runHelper({
    env: { ...env, APPLE_SIGNING_IDENTITY: 'Developer ID Application: Other (OTHERTEAM)' },
    body: 'require_macos_release_admission',
  })

  assert.equal(result.status, 1)
  assert.match(result.stderr, /requires expected Developer ID identity/)
}))

test('post-build app verification requires strict signature, expected identity, Gatekeeper, and a staple before DMG acceptance', () => withAppVerificationFixture(({ app, env, log }) => {
  const result = runHelper({ env, body: `verify_macos_release_app_bundle "${app}"` })

  assert.equal(result.status, 0, result.stderr)
  assert.match(result.stdout, /codesign, identity, Gatekeeper, and staple validation passed/)
  assert.deepEqual(readFileSync(log, 'utf8').trim().split('\n'), [
    `codesign <--verify> <--deep> <--strict> <--verbose=2> <${app}>`,
    `codesign <-d> <--verbose=4> <${app}>`,
    `spctl <--assess> <--type> <execute> <--verbose=4> <${app}>`,
    `xcrun <stapler> <validate> <${app}>`,
  ])
}))

test('post-build app verification rejects a mismatched TeamIdentifier before Gatekeeper', () => withAppVerificationFixture(({ app, env, log }) => {
  const result = runHelper({
    env: { ...env, FAKE_CODESIGN_DETAILS: 'Identifier=lv.shellx.cut\nTeamIdentifier=WRONGTEAM\nAuthority=Developer ID Application: Martins Brezauckis (4M329JW6R4)' },
    body: `verify_macos_release_app_bundle "${app}"`,
  })

  assert.equal(result.status, 1)
  assert.match(result.stderr, /TeamIdentifier did not match/)
  assert.doesNotMatch(readFileSync(log, 'utf8'), /^spctl|^xcrun/m)
}))

test('notary submission error fails closed without attempting a staple', () => withFixture(({ dmg, env, log }) => {
  const result = runHelper({
    env: { ...env, FAKE_SUBMIT_STATUS: '1', FAKE_NOTARY_OUTPUT: 'network unavailable' },
    body: `notarize_release_dmg "${dmg}"`,
  })

  assert.equal(result.status, 1)
  assert.match(result.stderr, /DMG notarization submission failed/)
  assert.equal(readFileSync(log, 'utf8').trim().split('\n').length, 1)
  assert.doesNotMatch(readFileSync(log, 'utf8'), /stapler/)
}))

test('a non-Accepted notary result fails closed without attempting a staple', () => withFixture(({ dmg, env, log }) => {
  const result = runHelper({
    env: { ...env, FAKE_NOTARY_OUTPUT: 'id: fixture\nstatus: Invalid' },
    body: `notarize_release_dmg "${dmg}"`,
  })

  assert.equal(result.status, 1)
  assert.match(result.stderr, /result was not Accepted/)
  assert.doesNotMatch(readFileSync(log, 'utf8'), /stapler/)
}))

test('a staple failure fails closed after an Accepted result', () => withFixture(({ dmg, env, log }) => {
  const result = runHelper({
    env: { ...env, FAKE_STAPLE_STATUS: '1' },
    body: `notarize_release_dmg "${dmg}"`,
  })

  assert.equal(result.status, 1)
  assert.match(result.stderr, /DMG staple failed after Accepted notarization/)
  const commands = readFileSync(log, 'utf8')
  assert.match(commands, /stapler> <staple>/)
  assert.doesNotMatch(commands, /stapler> <validate>/)
}))

test('staple validation failure fails closed before Gatekeeper assessment', () => withFixture(({ dmg, env, log }) => {
  const result = runHelper({
    env: { ...env, FAKE_STAPLE_VALIDATE_STATUS: '1' },
    body: `notarize_release_dmg "${dmg}"`,
  })

  assert.equal(result.status, 1)
  assert.match(result.stderr, /DMG staple validation failed/)
  assert.doesNotMatch(readFileSync(log, 'utf8'), /^spctl/m)
}))

test('independent Gatekeeper validation failure fails closed', () => withFixture(({ dmg, env }) => {
  const result = runHelper({
    env: { ...env, FAKE_SPCTL_STATUS: '1' },
    body: `notarize_release_dmg "${dmg}"`,
  })

  assert.equal(result.status, 1)
  assert.match(result.stderr, /DMG independent Gatekeeper validation failed/)
}))

test('Accepted, stapled, and independently assessed fixture DMG is qualified', () => withFixture(({ dmg, env, log }) => {
  const result = runHelper({
    env,
    body: `notarize_release_dmg "${dmg}"`,
  })

  assert.equal(result.status, 0, result.stderr)
  assert.match(result.stdout, /DMG notarization, staple, and independent validation passed/)
  assert.deepEqual(readFileSync(log, 'utf8').trim().split('\n'), [
    `xcrun <notarytool> <submit> <${dmg}> <--key> <${env.APPLE_API_KEY_PATH}> <--key-id> <fixture-key-id> <--issuer> <fixture-issuer-id> <--wait>`,
    `xcrun <stapler> <staple> <${dmg}>`,
    `xcrun <stapler> <validate> <${dmg}>`,
    `spctl <--assess> <--type> <open> <--context> <context:primary-signature> <-vv> <${dmg}>`,
  ])
}))

test('debug/no-notary status explicitly rejects release qualification', () => {
  const result = runHelper({
    env: { PATH: process.env.PATH },
    body: 'report_macos_dev_notary_status debug',
  })

  assert.equal(result.status, 0)
  assert.match(result.stdout, /NOT RELEASE-QUALIFIED/)
  assert.match(result.stdout, /skips DMG notarization and validation/)
})
