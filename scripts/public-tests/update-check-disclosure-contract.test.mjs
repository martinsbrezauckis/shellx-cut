// update-check-disclosure-contract.test.mjs — keeps the DOCUMENTED update
// behavior and the SHIPPED update behavior identical, on every surface.
//
// The update flow spans four layers that can silently drift apart: the shell
// service (update_state.rs — launch + 6-hourly checks, install on request),
// the narrow bridge grants (permissions + capability), the UI surfaces
// (topbar button, Settings > About, Storage & privacy toggle), and the public
// disclosure (README / SECURITY). These content-coupled assertions fail the
// moment any layer changes without the others — the disclosure must stay
// EXACTLY true.

import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { test } from 'node:test'

const read = (path) => readFileSync(path, 'utf8')

test('automatic checks are preference-gated at launch AND on the 6-hour periodic timer', () => {
  const service = read('app/desktop/src-tauri/src/update_state.rs')
  // Cadence: 6 hours, in one named constant the loop sleeps on.
  assert.match(service, /AUTO_CHECK_INTERVAL: Duration = Duration::from_secs\(6 \* 60 \* 60\)/)
  const driver = service.slice(service.indexOf('pub(crate) async fn run_automatic_checks(app: tauri::AppHandle)'))
  // Launch: the preference guard runs before any check.
  assert.ok(
    driver.indexOf('should_auto_check(crate::update_settings::check_on_launch(&app))')
      < driver.indexOf('perform_check(&app).await'),
    'the launch check must consult the persisted preference before any network access',
  )
  // Periodic: the loop sleeps the interval and RE-READS the preference each
  // tick, so turning the toggle off stops automatic checks immediately.
  const loop = driver.slice(driver.indexOf('loop {'))
  assert.match(loop, /tokio::time::sleep\(AUTO_CHECK_INTERVAL\)\.await/)
  assert.match(loop, /should_auto_check\(crate::update_settings::check_on_launch\(&app\)\)/)
  assert.match(loop, /periodic update check disabled in Settings/)
})

test('the manual check ignores the automatic preference — Settings > About works with auto off', () => {
  const service = read('app/desktop/src-tauri/src/update_state.rs')
  const start = service.indexOf('pub(crate) async fn update_check_now')
  const end = service.indexOf('pub(crate) async fn update_install_now')
  assert.ok(start > 0 && end > start, 'update_check_now and update_install_now must exist')
  const manual = service.slice(start, end)
  assert.match(manual, /perform_check\(&app\)\.await/)
  assert.doesNotMatch(
    manual,
    /check_on_launch/,
    'the manual command must not consult the automatic-check preference — an explicit click is its own consent',
  )
})

test('no startup modal: checks only update quiet state; the confirm dialog lives in the install flow', () => {
  const service = read('app/desktop/src-tauri/src/update_state.rs')
  // The check path broadcasts state and never blocks on a dialog.
  const check = service.slice(
    service.indexOf('async fn perform_check'),
    service.indexOf('async fn install_pending'),
  )
  assert.match(check, /update_and_broadcast/)
  assert.doesNotMatch(check, /blocking_show|dialog\(\)/, 'a release-feed check must never pop a dialog')
  // The install flow (explicit user request) confirms, installs, restarts —
  // in that order.
  const install = service.slice(service.indexOf('async fn install_pending'))
  assert.match(install, /ShellX Cut \{version\} is available \(you're on \{current\}\)/)
  assert.match(install, /"Install & restart"\.into\(\)/)
  assert.match(install, /"Later"\.into\(\)/)
  assert.ok(
    install.indexOf('blocking_show') < install.indexOf('download_and_install'),
    'the native confirm must precede download_and_install',
  )
  assert.match(install, /download_and_install/)
  assert.match(install, /app\.restart\(\)/)
  // Every transition reaches the webview on one event name.
  assert.match(service, /EVENT_NAME: &str = "cut:update-state"/)
})

test('native updater rejects replayed release URLs and registers the state service', () => {
  const shell = read('app/desktop/src-tauri/src/lib.rs')
  const config = JSON.parse(read('app/desktop/src-tauri/tauri.conf.json'))
  assert.deepEqual(config.plugins?.updater?.endpoints, [
    'https://github.com/martinsbrezauckis/shellx-cut/releases/latest/download/latest.json',
  ])
  assert.match(shell, /default_version_comparator/)
  assert.match(shell, /updater_release_urls_match_version/)
  assert.match(shell, /update_state::run_automatic_checks/)
  // The bridge commands are registered and granted to the validated origin.
  assert.match(shell, /update_state::get_update_state/)
  assert.match(shell, /update_state::update_check_now/)
  assert.match(shell, /update_state::update_install_now/)
  assert.match(shell, /permission\("allow-update-state"\)/)
  assert.match(shell, /update_settings::get_update_preferences/)
  assert.match(shell, /update_settings::set_update_preferences/)
  assert.match(shell, /permission\("allow-update-preferences"\)/)
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

test('the exact remote capability exposes only the bounded update commands', () => {
  const preferences = read('app/desktop/src-tauri/permissions/update-preferences.toml')
  assert.match(preferences, /commands\.allow = \["get_update_preferences", "set_update_preferences"\]/)
  assert.doesNotMatch(preferences, /commands\.allow.*(?:restart|download|execute|filesystem)/i)
  const state = read('app/desktop/src-tauri/permissions/update-state.toml')
  assert.match(state, /commands\.allow = \["get_update_state", "update_check_now", "update_install_now"\]/)
  assert.doesNotMatch(state, /commands\.allow.*(?:restart|download|execute|filesystem)/i)
})

test('the topbar shows ONE quiet button only while an update is offered', () => {
  const button = read('ui/src/topbar/UpdateButton.tsx')
  assert.match(button, /shouldShowUpdateButton\(state\)/)
  assert.match(button, /data-cut-update-btn/)
  assert.match(button, /installUpdateNow/)
  assert.doesNotMatch(button, /window\.confirm|alert\(/, 'the shell owns the confirm — no webview dialogs')
  assert.match(read('ui/src/topbar/index.tsx'), /<UpdateButton \/>/)
  // The pure model hides the button for every non-available state.
  const model = read('ui/src/lib/updateState.ts')
  assert.match(model, /state\.status === 'available'/)
  assert.match(model, /'idle', 'none', 'available', 'error', 'unsupported'/)
})

test('Settings > About carries the full honest update surface', () => {
  const about = read('ui/src/panels/Environment/About.tsx')
  for (const hook of [
    'data-cut-about-update-status',
    'data-cut-about-update-checked',
    'data-cut-about-check-updates',
    'data-cut-about-install-update',
    'data-cut-about-release-notes',
  ]) {
    assert.match(about, new RegExp(hook), `About must expose ${hook}`)
  }
  assert.match(about, /checkForUpdatesNow/)
  assert.match(about, /installUpdateNow/)
  // Linux honesty: controls render only when the snapshot says supported.
  assert.match(about, /snapshot\?\.supported && \(/)
  // Disclosure prose matches the real cadence.
  assert.match(about, /at launch and every 6 hours while it stays open/)
  // Failure states name what failed (honest-degradation contract).
  const model = read('ui/src/lib/updateState.ts')
  assert.match(model, /Update check failed: \$\{state\.error \?\? 'unknown error'\}/)
  assert.match(model, /Linux builds update through deb\/rpm package downloads/)
})

test('the bridge is the narrow window.__TAURI__ pattern with validated payloads', () => {
  const bridge = read('ui/src/lib/tauri.ts')
  assert.match(bridge, /invoke<unknown>\('get_update_state'\)/)
  assert.match(bridge, /invoke<unknown>\('update_check_now'\)/)
  assert.match(bridge, /invoke<unknown>\('update_install_now'\)/)
  assert.match(bridge, /listen\('cut:update-state'/)
  assert.match(bridge, /validShellUpdateState/)
})

test('Settings discloses the launch + periodic checks and provides installed action coverage', () => {
  const component = read('ui/src/panels/Environment/UpdateNetworkSettings.tsx')
  const coverage = read('ui/public-tests/lib/fullCoverageSettings.mjs')
  assert.match(component, /contacts GitHub when it opens, and then every 6 hours while\s+it stays open/)
  assert.match(component, /normal request metadata such as\s+your IP address/)
  assert.match(component, /sends no project, media, edit history, or analytics payload/)
  assert.match(component, /data-cut-action="update-check-on-launch"/)
  assert.match(component, /setLaunchUpdatePreference/)
  // The one toggle governs BOTH automatic checks; the manual path survives.
  assert.match(component, /Covers the launch check and the 6-hour re-check/)
  assert.match(component, /at launch and every 6 hours while the app stays open/)
  assert.match(component, /manual Check for updates button in About still works/)
  assert.match(coverage, /actionId: 'update-check-on-launch'/)
  assert.match(coverage, /originalUpdatePreference/)
  assert.match(coverage, /restored/)
  // The new update controls are inventoried for the native sweep (their
  // staged-bridge scenarios live in the dedicated settings-update module).
  const updateCoverage = read('ui/public-tests/lib/fullCoverageSettingsUpdate.mjs')
  assert.match(updateCoverage, /data-cut-about-check-updates/)
  assert.match(updateCoverage, /data-cut-about-install-update/)
  assert.match(updateCoverage, /data-cut-update-btn/)
  assert.match(coverage, /runUpdateSurfaceCoverage/)
  // Native shells (Tauri 2.11+) FREEZE window.__TAURI__, so the fixture must
  // wrap __TAURI_INTERNALS__.invoke — assigning into the frozen namespace
  // silently no-ops and the rows test nothing (2026-08-06 macOS strict run,
  // 3 false-fail rows). Contract: internals patched FIRST, and every patch
  // is read back with a loud throw when the assignment did not take.
  assert.match(updateCoverage, /__TAURI_INTERNALS__\.invoke/)
  assert.match(updateCoverage, /originalInternalsInvoke/)
  assert.match(updateCoverage, /assignment did not take/)
  const internalsIdx = updateCoverage.indexOf('const internals = target.__TAURI_INTERNALS__')
  const legacyIdx = updateCoverage.indexOf('fx.originalInvoke.call(fx.originalCore')
  assert.ok(internalsIdx > 0 && legacyIdx > internalsIdx, 'internals patch path must come before the legacy core.invoke path')
})

test('public docs state the cadence, data boundary, opt-out scope, and manual-path survival', () => {
  const readme = read('README.md')
  assert.match(readme, /contacts GitHub by default\s*to read the signed release feed: once at launch, then once every 6 hours while\s*the app stays open/)
  assert.match(readme, /normal request metadata such as the IP\s*address/)
  assert.match(readme, /adds no project, media, edit-history, or analytics payload/)
  assert.match(readme, /Settings > Storage & privacy > Network\s*activity/)
  assert.match(readme, /applies immediately to both the launch and periodic checks/)
  assert.match(readme, /manual "Check for updates" button in Settings > About keeps working/)
  assert.match(readme, /never interrupts the session/)
  const security = read('SECURITY.md')
  assert.match(security, /update checks \(automatic at launch and every 6 hours unless turned off,\s*or manual from Settings > About\)/)
})

test('Linux packages skip every automatic check honestly (deb/rpm update flow, no feed noise)', () => {
  // The release feed carries only windows-x86_64 + darwin-aarch64, so a Linux
  // check could only ever fail. The Linux build must skip BEFORE any network
  // access, say so once in the log, report `unsupported` state to the UI
  // (which explains the packaging instead of showing dead buttons), and the
  // README must disclose that Linux makes no request.
  const service = read('app/desktop/src-tauri/src/update_state.rs')
  assert.match(service, /#\[cfg\(all\(desktop, target_os = "linux"\)\)\]\s*\npub\(crate\) async fn run_automatic_checks/)
  assert.match(service, /Linux builds update through deb\/rpm packages — launch update check skipped/)
  assert.match(service, /#\[cfg\(all\(desktop, not\(target_os = "linux"\)\)\)\]\s*\npub\(crate\) async fn run_automatic_checks/)
  assert.match(service, /const PLATFORM_SUPPORTED: bool = cfg!\(all\(desktop, not\(target_os = "linux"\)\)\)/)
  const readme = read('README.md')
  assert.match(readme, /Linux packages \(deb\/rpm\) skip the launch and periodic update checks entirely/)
  assert.match(readme, /a Linux launch makes no GitHub\s+request/)
})

test('the QA feed override stays inside the trust chain', () => {
  const service = read('app/desktop/src-tauri/src/update_state.rs')
  // Staging exists (SHELLX_CUT_UPDATE_FEED_URL) so an update-available run is
  // provable on a rig, but it moves only the FEED — signature verification and
  // the version-bound-URL comparator still apply to whatever it serves.
  assert.match(service, /ENV_UPDATE_FEED_URL: &str = "SHELLX_CUT_UPDATE_FEED_URL"/)
  const build = service.slice(service.indexOf('fn build_updater'), service.indexOf('async fn perform_check'))
  assert.match(build, /updater_builder\(\)/)
  assert.match(build, /endpoints\(vec!\[url\]\)/)
})
