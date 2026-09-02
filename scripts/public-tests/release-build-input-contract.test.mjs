import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import test from 'node:test'

const root = resolve(import.meta.dirname, '..', '..')
const read = (path) => readFileSync(resolve(root, path), 'utf8')
const required = [
  'SHELLX_RELEASE_STUDIO_ROOT',
  'SHELLX_CUT_PUBLIC_EXPORT_DIR',
  'SHELLX_CUT_CANDIDATE_FREEZE',
  'SHELLX_CUT_CANDIDATE_FREEZE_MANIFEST_SHA256',
]

test('release-only Windows and macOS builders require one explicit Release Studio build-input contract', () => {
  for (const [path, anchor] of [
    ['scripts/build-windows.sh', 'echo "[build-windows] cargo tauri build'],
    ['scripts/build-macos.sh', 'echo "[build-macos] cargo tauri build'],
  ]) {
    const source = read(path)
    const contract = source.indexOf('require_release_build_input_contract()')
    const tauri = source.indexOf(anchor)
    const verifier = source.lastIndexOf('verify_release_build_input', tauri)
    assert.ok(contract >= 0 && tauri >= 0, `${path} must retain the explicit release admission and actual cargo-tauri anchor`)
    assert.ok(verifier >= 0 && verifier < tauri, `${path} must verify the frozen build input before cargo-tauri signing`)
    for (const name of required) assert.match(source, new RegExp(name), `${path} must require ${name}`)
    assert.match(source, /tools\/verify-release-build-input[.]mjs/)
    assert.match(source, /--project shellx-cut[\s\S]+--checkout "\$repo_root"[\s\S]+--public-export "\$SHELLX_CUT_PUBLIC_EXPORT_DIR"[\s\S]+--freeze "\$SHELLX_CUT_CANDIDATE_FREEZE"[\s\S]+--freeze-manifest-sha256 "\$SHELLX_CUT_CANDIDATE_FREEZE_MANIFEST_SHA256"/)
  }
  const windows = read('scripts/build-windows.sh')
  assert.match(windows, /if \[ "\$MODE" = "release" \] && \[ "\$WINDOWS_AUTHENTICODE_SIGNING" = "1" \]; then\n  require_release_build_input_contract/)
  assert.match(windows, /Authenticode signing is release-only; debug builds must remain unsigned/)
  const macos = read('scripts/build-macos.sh')
  assert.match(macos, /if \[ "\$MODE" = "release" \]; then\n  require_release_build_input_contract/)
})

test('Windows rechecks the frozen build input before every Authenticode route', () => {
  const build = read('scripts/build-windows.sh')
  const helper = read('scripts/windows-artifact-sign-command.mjs')
  assert.match(build, /verify_release_build_input\n  app\/desktop\/scripts\/windows-artifact-sign[.]sh "\$cutd_exe"/)
  assert.match(build, /verify_release_build_input\n  app\/desktop\/scripts\/windows-artifact-sign[.]sh "\$shell_exe"/)
  assert.match(helper, /function main\(\) \{[\s\S]*verifyReleaseBuildInputBeforeAuthenticode\(\)\n  const invocation = signingInvocation/)
  assert.match(helper, /SHELLX_CUT_RELEASE_BUILD_INPUT_REQUIRED/)
  assert.match(helper, /Authenticode signing requires SHELLX_CUT_RELEASE_BUILD_INPUT_REQUIRED=1/)
  for (const name of required) assert.match(helper, new RegExp(name), `sign command callback must retain ${name}`)
})
