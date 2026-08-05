import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'

const read = (path) => readFileSync(path, 'utf8')

test('native updater reads the persisted opt-out before constructing an updater', () => {
  const shell = read('app/desktop/src-tauri/src/lib.rs')
  const updater = shell.slice(shell.indexOf('async fn check_for_update'))
  assert.match(updater, /if !update_settings::check_on_launch\(&app\)/)
  assert.ok(
    updater.indexOf('update_settings::check_on_launch(&app)') < updater.indexOf('app.updater()'),
    'preference guard must run before updater construction or network access',
  )
  assert.match(shell, /update_settings::get_update_preferences/)
  assert.match(shell, /update_settings::set_update_preferences/)
  assert.match(shell, /permission\("allow-update-preferences"\)/)
})

test('native updater visibly offers an exact-version install and rejects replayed release URLs', () => {
  const shell = read('app/desktop/src-tauri/src/lib.rs')
  const config = JSON.parse(read('app/desktop/src-tauri/tauri.conf.json'))
  assert.deepEqual(config.plugins?.updater?.endpoints, [
    'https://github.com/martinsbrezauckis/shellx-cut/releases/latest/download/latest.json',
  ])
  assert.match(shell, /Ok\(Some\(u\)\) => u,\s*\/\/ an update is available/)
  assert.match(shell, /ShellX Cut \{version\} is available \(you're on \{current\}\)/)
  assert.match(shell, /"Install & restart"\.into\(\)/)
  assert.match(shell, /"Later"\.into\(\)/)
  assert.match(shell, /download_and_install/)
  assert.match(shell, /app\.restart\(\)/)
  assert.match(shell, /default_version_comparator/)
  assert.match(shell, /updater_release_urls_match_version/)
})

test('release builds retain updater signatures and the feed generator verifies them', () => {
  const windows = read('scripts/build-windows.sh')
  const macos = read('scripts/build-macos.sh')
  const generator = read('scripts/release/generate-updater-manifest.mjs')
  const manifest = read('scripts/lib/updater-manifest.mjs')
  assert.match(windows, /signed updater build did not produce \$updater_sig/)
  assert.match(macos, /signed updater build did not produce \$updater_archive/)
  assert.match(macos, /signed updater build did not produce \$updater_sig/)
  assert.match(generator, /'--example',\s*'verify-updater-signature'/)
  assert.match(generator, /verify-updater-signature/)
  assert.match(manifest, /requires cryptographic signature verification/)
  assert.match(manifest, /windows-x86_64/)
  assert.match(manifest, /darwin-aarch64/)
  assert.match(manifest, /Updater tag must be v\$\{version\}/)
})

test('the exact remote capability exposes only the bounded update preference commands', () => {
  const permission = read('app/desktop/src-tauri/permissions/update-preferences.toml')
  assert.match(permission, /commands\.allow = \["get_update_preferences", "set_update_preferences"\]/)
  assert.doesNotMatch(permission, /commands\.allow.*(?:restart|download|execute|filesystem)/i)
})

test('Settings discloses the GitHub launch request and provides installed action coverage', () => {
  const component = read('ui/src/panels/Environment/UpdateNetworkSettings.tsx')
  const coverage = read('ui/public-tests/lib/fullCoverageSettings.mjs')
  assert.match(component, /contacts GitHub once when it opens/)
  assert.match(component, /normal request metadata such as your IP address/)
  assert.match(component, /sends no project,\s*media, edit history, or analytics payload/)
  assert.match(component, /data-cut-action="update-check-on-launch"/)
  assert.match(component, /setLaunchUpdatePreference/)
  assert.match(coverage, /actionId: 'update-check-on-launch'/)
  assert.match(coverage, /originalUpdatePreference/)
  assert.match(coverage, /restored/)
})

test('public docs state the default, data boundary, opt-out location, and next-launch timing', () => {
  const readme = read('README.md')
  assert.match(readme, /contacts GitHub once per launch by default/)
  assert.match(readme, /normal request metadata such as\s*the IP address/)
  assert.match(readme, /adds no project, media, edit-history, or analytics payload/)
  assert.match(readme, /Settings > Storage & privacy > Network\s*activity/)
  assert.match(readme, /applies from the next\s*launch/)
})
