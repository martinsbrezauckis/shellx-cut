import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import test from 'node:test'
import { fileURLToPath } from 'node:url'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..')
const read = (path) => readFileSync(resolve(root, path), 'utf8')

test('the real Tauri foreground owner compiles and drives the AppKit Region picker', () => {
  const build = read('app/desktop/src-tauri/build.rs')
  const serverBuild = read('app/server/build.rs')
  const bridge = read('app/desktop/src-tauri/src/macos_region_bridge.rs')
  const native = read('app/server/src/screen_record/macos_region_picker.mm')

  assert.match(build, /server\/src\/screen_record\/macos_region_picker\.mm/)
  assert.match(build, /cpp_link_stdlib\(None\)[\s\S]+rustc-link-lib=c\+\+/)
  assert.match(build, /rustc-link-lib=framework=AppKit/)
  assert.doesNotMatch(serverBuild, /macos_region_picker\.mm/, 'cutd must never compile or own the visual picker')
  assert.match(bridge, /app\.run_on_main_thread\(move \|\|/)
  assert.match(bridge, /struct AppKitMainThreadOwner\(std::marker::PhantomData<std::rc::Rc<\(\)>>\)/)
  assert.match(bridge, /sxc_macos_region_picker_is_main_thread\(\)/)
  assert.match(bridge, /window\.is_focused\(\)\.unwrap_or\(false\)/)
  assert.match(bridge, /sxc_macos_region_picker_present\(&mut selection\)/)
  assert.match(native, /- \(void\)resignKeyWindow[\s\S]+\[super resignKeyWindow\][\s\S]+stopModalWithCode:SXCNativePickerRefused/)
  assert.match(native, /keyCode == 53[\s\S]+stopModalWithCode:SXCNativePickerCancelled/)
  assert.match(native, /convertRectToBacking:_selectedScreen\.frame/)
  assert.match(native, /convertRectToBacking:_selection/)
  assert.match(native, /parentHeight - \(selectedMaxY - parentMinY\)/)
  assert.doesNotMatch(native, /NSTextField/, 'the native overlay must not accept typed coordinates')
})

test('the desktop to cutd Region lane is child-only, correlated, and non-portable', () => {
  const desktop = read('app/desktop/src-tauri/src/macos_region_bridge.rs')
  const shell = read('app/desktop/src-tauri/src/lib.rs')
  const server = read('app/server/src/screen_record/macos_region_bridge.rs')
  const http = read('app/server/src/http.rs')
  const permission = read('app/desktop/src-tauri/permissions/macos-region-foreground.toml')

  for (const token of [
    'SHELLX_CUT_REGION_BRIDGE_SECRET',
    'SHELLX_CUT_REGION_DESKTOP_EPOCH',
    'SHELLX_CUT_REGION_CUTD_EPOCH',
    'SHELLX_CUT_REGION_DESKTOP_PID',
    'region_request_',
  ]) {
    assert.match(desktop, new RegExp(token))
    assert.match(server, new RegExp(token))
  }
  assert.match(desktop, /X-ShellX-Cut-Cutd-Pid/)
  assert.match(server, /x-shellx-cut-cutd-pid/)
  assert.match(shell, /ForegroundRegionBridge::for_spawned_child\(addr\.clone\(\)\)/)
  assert.match(shell, /bridge\.bind_spawned_child\(child\.id\(\)\)/)
  assert.match(shell, /let allow_foreground_region = foreground_region_bridge\.is_some\(\)/)
  assert.match(shell, /if allow_foreground_region \{[\s\S]+allow-macos-region-foreground/)
  assert.match(shell, /\.permission\("allow-macos-region-foreground"\)/)
  assert.match(permission, /commands\.allow = \["start_macos_region_capture"\]/)
  assert.match(desktop, /let child_pid = self[\s\S]+child_pid[\s\S]+X-ShellX-Cut-Cutd-Pid/)
  assert.match(server, /current_parent_pid\(\) == self\.desktop_pid/)
  assert.match(server, /clear_child_environment\(\)/, 'cutd must not pass bridge material to its descendants')
  assert.match(server, /std::env::remove_var\(name\)/)
  assert.match(server, /constant_time_eq\(value, &self\.secret\)/)
  assert.match(server, /self\.entries\.contains_key\(request_id\)/)
  assert.match(server, /now \+ CORRELATION_TTL/)
  assert.match(server, /if configuration\(\)\.is_some\(\)[\s\S]+private\/foreground-region-start/)
  assert.match(http, /macos_region_bridge::install_route\(api\)/)
  assert.doesNotMatch(shell, /EngineState::Wired[\s\S]{0,500}region_bridge/, 'bridge material must not enter engine_status')
})

test('headless cutd never receives or initializes the foreground Region bridge', () => {
  const shell = read('app/desktop/src-tauri/src/lib.rs')
  const serverMain = read('app/server/src/main.rs')
  const screenRecord = read('app/server/src/screen_record.rs')
  const server = read('app/server/src/screen_record/macos_region_bridge.rs')

  const uiPresentAt = shell.indexOf('let ui_present = ui_dist.join("index.html").exists()')
  const desktopBridgeAt = shell.indexOf('let mut foreground_region_bridge = if ui_present')
  const childCommandAt = shell.indexOf('let mut cmd = Command::new(&program)')
  assert.ok(uiPresentAt >= 0 && uiPresentAt < desktopBridgeAt && desktopBridgeAt < childCommandAt,
    'desktop bridge material must be created only after UI availability is known and before its own child command')
  assert.match(shell, /let mut foreground_region_bridge = if ui_present \{[\s\S]+?\} else \{\s*None\s*\}/)

  assert.match(serverMain, /initialize_private_foreground_region_bridge\(matches!\(\s*&cli\.command,\s*Command::Serve \{\s*headless: false,\s*\.\.\s*\}\s*\)\)/)
  assert.match(screenRecord, /macos_region_bridge::initialize_from_child_environment\(allow_foreground_bridge\)/)
  const initializeAt = serverMain.indexOf('initialize_private_foreground_region_bridge')
  const runtimeAt = serverMain.indexOf('tokio::runtime::Builder::new_multi_thread()')
  assert.ok(initializeAt >= 0 && initializeAt < runtimeAt,
    'cutd must load and scrub bridge material before constructing Tokio')
  assert.doesNotMatch(serverMain, /#\[tokio::main\]/)

  assert.match(server, /allow_foreground_bridge\s*\.then\(BridgeConfiguration::from_child_environment\)\s*\.flatten\(\)/)
  assert.match(server, /clear_child_environment\(\);\s*let _ = BRIDGE_CONFIGURATION\.set\(configuration\)/)
  assert.doesNotMatch(server, /get_or_init/,
    'the route must never lazily load a headless process environment')
})

test('cutd burns its private ticket before ordinary Region reservation and revalidates scale/crop', () => {
  const bridge = read('app/server/src/screen_record/macos_region_bridge.rs')
  const start = read('app/server/src/screen_record/macos_region_start.rs')
  const capture = read('app/recorder/record-capture/src/macos_region_capture.rs')
  const registry = read('app/server/src/screen_record/region_selection_tests.rs')

  assert.match(bridge, /region_selection::issue\(RegionSelectionValue::new\(monitor, crop\)\)/)
  assert.match(bridge, /region_selection::consume\(ticket\.as_str\(\), \|selection\|/)
  assert.match(bridge, /private_macos_region_selection_is_current\(&monitor_id, crop\)/)
  assert.match(capture, /CaptureConfig, CaptureRegion,/)
  assert.match(capture, /fn selection_is_current\(monitor_id: &str, region: CaptureRegion\) -> bool/)
  assert.match(capture, /prepare_region_capture\(&request, 30, false\)\.is_ok\(\)/)
  assert.match(registry, /fn selection_expires_and_purges_without_becoming_not_found_early/)
  assert.match(registry, /fn consume_is_single_use_and_preserves_the_exact_private_value/)
  assert.match(registry, /fn topology_or_scale_change_burns_the_ticket_before_a_capture_can_reserve/)
  const consumeAt = start.indexOf('macos_region_bridge::consume_ticket(&request.ticket)')
  const validateAt = start.indexOf('validate_capture_settings(request.duration_ms, request.fps)?')
  const reserveAt = start.indexOf('monitor_start_admission::admit(')
  assert.ok(consumeAt >= 0 && consumeAt < validateAt && validateAt < reserveAt, 'ticket must burn before normal preflight and reservation')
  assert.match(start, /start_capture\([\s\S]+target\.exact_id,[\s\S]+Some\(crop\),/)
  assert.doesNotMatch(start, /parse_args|Deserialize|region_ticket/, 'the private start seam must not become a generic request parser')
})

test('Region remains absent from the public verb, capability ID, and recorder controls pending native proof', () => {
  const schema = JSON.parse(read('schema/verbs.json'))
  const start = schema.verbs.find((verb) => verb.name === 'screen_record.start')
  const doctor = read('app/server/src/screen_record/doctor_projection.rs')
  const sourceControl = read('ui/src/panels/Record/RecordingSourceControl.tsx')
  const capability = read('app/desktop/src-tauri/capabilities/default.json')

  assert.ok(start, 'screen_record.start remains an explicit schema verb')
  assert.equal(start.args.additionalProperties, false)
  assert.equal(Object.hasOwn(start.args.properties, 'region'), false)
  assert.equal(Object.hasOwn(start.args.properties, 'region_ticket'), false)
  assert.match(doctor, /"region_selection": \{\s*"supported": false,/)
  assert.match(doctor, /pending compiled\/native qualification/)
  assert.doesNotMatch(sourceControl, /data-cut-rec-source-kind-button="region"/)
  assert.doesNotMatch(capability, /macos-region-foreground/, 'the bootstrap capability must not pregrant Region IPC')
})
