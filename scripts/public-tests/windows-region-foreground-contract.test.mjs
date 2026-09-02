import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import test from 'node:test'
import { fileURLToPath } from 'node:url'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..')
const read = (path) => readFileSync(resolve(root, path), 'utf8')

test('Windows Region picker is bound to the exact Tauri main HWND and its owner thread', () => {
  const desktop = read('app/desktop/src-tauri/src/windows_region_picker.rs')
  const native = read('app/server/src/screen_record/windows_region_picker.cpp')

  assert.match(desktop, /\.hwnd\(\)/)
  assert.match(desktop, /sxc_windows_region_picker_present\(main_hwnd\.0, &mut selection\)/)
  assert.match(native, /sxc_windows_region_picker_present\(\s*HWND main_window,/)
  assert.match(native, /GetForegroundWindow\(\) != main_window/)
  assert.match(native, /GetWindowThreadProcessId\(main_window, &process_id\)/)
  assert.match(native, /owner_thread == GetCurrentThreadId\(\)/)
  assert.match(native, /GetWindow\(window, GW_OWNER\) != main_window/)
  assert.match(native, /CreateWindowExW\([\s\S]+?main_window,/)
  assert.match(native, /if \(received == 0\) \{[\s\S]+?PostQuitMessage\(static_cast<int>\(message\.wParam\)\);[\s\S]+?break;/)
  assert.match(native, /if \(received == -1\) \{[\s\S]+?finish\(&state, kPickerFailure\);/)
})

test('the Windows foreground bridge is child-only, correlated, and topology-bound', () => {
  const desktop = read('app/desktop/src-tauri/src/windows_region_bridge.rs')
  const shell = read('app/desktop/src-tauri/src/lib.rs')
  const server = read('app/server/src/screen_record/windows_region_bridge.rs')
  const parent = read('app/server/src/screen_record/windows_region_parent.rs')
  const capture = read('app/recorder/record-capture/src/windows_region_capture.rs')
  const runtime = read('app/recorder/record-capture/src/windows_runtime.rs')
  const topology = read('app/server/src/screen_record/windows_region_topology.cpp')
  const build = read('app/recorder/record-capture/build.rs')
  const permission = read('app/desktop/src-tauri/permissions/windows-region-foreground.toml')

  assert.match(desktop, /app\.run_on_main_thread\(move \|\|/)
  assert.match(desktop, /selection\.require_current\(\)\?/)
  assert.match(desktop, /topology_digest/)
  assert.match(desktop, /bridge\.submit\(selection, start\)/)
  assert.match(shell, /cfg\(any\(target_os = "macos", windows\)\)/)
  assert.match(shell, /allow-windows-region-foreground/)
  assert.match(permission, /commands\.allow = \["start_windows_region_capture"\]/)
  for (const token of [
    'SHELLX_CUT_REGION_BRIDGE_SECRET',
    'SHELLX_CUT_REGION_DESKTOP_EPOCH',
    'SHELLX_CUT_REGION_CUTD_EPOCH',
    'SHELLX_CUT_REGION_DESKTOP_PID',
    'region_request_',
  ]) {
    assert.match(server, new RegExp(token))
  }
  assert.match(server, /constant_time_eq\(value, &self\.secret\)/)
  assert.match(server, /self\.entries\.contains_key\(request_id\)/)
  assert.match(server, /now \+ CORRELATION_TTL/)
  assert.match(server, /clear_child_environment\(\)/)
  assert.match(server, /current_parent_pid\(\) == Some\(self\.desktop_pid\)/)
  assert.match(parent, /CreateToolhelp32Snapshot\(TH32CS_SNAPPROCESS, 0\)/)
  assert.match(parent, /entry\.th32ParentProcessID/)
  assert.match(server, /RegionSelectionValue::with_windows_topology/)
  assert.match(server, /private_windows_region_selection_is_current\(/)
  assert.match(capture, /sxc_windows_topology_snapshot_is_current_ffi/)
  assert.match(capture, /windows_monitor_target::resolve_monitor\(monitor_id\)/)
  assert.match(runtime, /SetThreadDpiAwarenessContext/)
  assert.match(runtime, /DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2/)
  assert.match(topology, /SetThreadDpiAwarenessContext\(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2\)/)
  assert.match(build, /windows_region_topology\.cpp/)
  assert.doesNotMatch(build, /windows_region_picker\.cpp/, 'cutd must never own the foreground picker')
})

test('Windows Region burns its ticket before one ordinary GPU-cropped recorder reservation', () => {
  const start = read('app/server/src/screen_record/windows_region_start.rs')
  const live = read('app/recorder/record-capture/src/windows.rs')
  const handler = read('app/recorder/record-capture/src/windows_wgc_handler.rs')
  const gpuCrop = read('app/recorder/record-capture/src/windows_gpu_crop.rs')
  const schema = JSON.parse(read('schema/verbs.json'))
  const recordStart = schema.verbs.find((verb) => verb.name === 'screen_record.start')
  const sourceControl = read('ui/src/panels/Record/RecordingSourceControl.tsx')
  const capability = read('app/desktop/src-tauri/capabilities/default.json')

  const burnAt = start.indexOf('windows_region_bridge::consume_ticket(&request.ticket)')
  const snapshotAt = start.indexOf('snapshot(state).await?')
  const reserveAt = start.indexOf('create_capture_dir(&dir, &capture_id)')
  const launchAt = start.indexOf('start_capture(')
  assert.ok(burnAt >= 0 && burnAt < snapshotAt, 'ticket must burn before project snapshot')
  assert.ok(snapshotAt < reserveAt && reserveAt < launchAt, 'Region must use ordinary recorder ownership')
  assert.match(start, /Some\(crop\)/)

  assert.match(live, /let flags = EncFlags \{[\s\S]*crop,[\s\S]*\};/)
  assert.match(live, /Handler::start_free_threaded/)
  const gpuCropAt = handler.indexOf('windows_gpu_crop::crop_frame_to_origin(')
  const encoderAt = handler.indexOf('encoder.send_frame(frame)')
  assert.ok(gpuCropAt >= 0 && gpuCropAt < encoderAt)
  assert.match(gpuCrop, /CopySubresourceRegion\(/)
  assert.match(live, /surface\.subsurface_from_native_crop\(crop, parent_w, parent_h\)/)
  assert.match(live, /map_rdevin_input\(surface, w, h,/)
  assert.match(live, /windows_runtime::enter_per_monitor_dpi_v2\(\)/)
  assert.doesNotMatch(gpuCrop, /buffer_crop\(/)
  assert.doesNotMatch(gpuCrop, /send_frame_buffer\(/)
  assert.ok(recordStart)
  assert.equal(Object.hasOwn(recordStart.args.properties, 'region'), false)
  assert.equal(Object.hasOwn(recordStart.args.properties, 'region_ticket'), false)
  assert.doesNotMatch(sourceControl, /data-cut-rec-source-kind-button="region"/)
  assert.doesNotMatch(capability, /windows-region-foreground/)
})
