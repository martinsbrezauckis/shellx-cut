import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import test from 'node:test'
import { fileURLToPath } from 'node:url'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..')
const read = (path) => readFileSync(resolve(root, path), 'utf8')

test('Recorder camera discovery stays passive and Start owns explicit native use', () => {
  const schema = JSON.parse(read('schema/verbs.json'))
  const doctor = schema.verbs.find((verb) => verb.name === 'screen_record.doctor')
  const start = schema.verbs.find((verb) => verb.name === 'screen_record.start')
  const capability = read('app/server/src/screen_record/camera_public.rs')
  const sidecar = read('app/server/src/screen_record/camera_capture_sidecar.rs')
  const native = read('app/recorder/record-capture/src/macos_camera_native.mm')
  const nativeRust = read('app/recorder/record-capture/src/macos_camera_native.rs')
  const macAdapter = read('app/recorder/record-capture/src/macos_camera.rs')
  const recorderBuild = read('app/recorder/record-capture/build.rs')
  const macInfo = read('app/desktop/src-tauri/Info.plist')
  const macEntitlements = read('app/desktop/src-tauri/macos-entitlements.plist')
  const tauriConfig = read('app/desktop/src-tauri/tauri.conf.json')

  assert.match(doctor.result, /camera:\{supported,devices:/)
  assert.ok(start.args.properties.camera_id)
  assert.match(start.args.properties.camera_id.description, /explicit Use camera action/)
  assert.match(capability, /private_camera_owner::devices\(\)/)
  assert.match(capability, /private_camera_owner::readiness\(device_id\)/)
  assert.match(capability, /cfg\(any\(windows, target_os = "macos", target_os = "linux"\)\)/)
  assert.doesNotMatch(capability, /use_camera\(/)
  assert.match(sidecar, /owner\.use_camera\(&device_id, &clock, &stop\)/)
  assert.match(sidecar, /cfg\(any\(windows, target_os = "macos", target_os = "linux"\)\)/)
  const linuxAdapter = read('app/recorder/record-capture/src/linux_camera.rs')
  assert.match(linuxAdapter, /fn private_readiness\(device_id: &str\)[\s\S]*selected\(device_id\)/)
  assert.match(linuxAdapter, /fn selected\(id: &str\)[\s\S]*linux_camera_devices::devices\(\)/)
  assert.match(sidecar, /finish_for_screen/)
  assert.match(native, /requestAccessForMediaType/)
  assert.ok(
    native.indexOf('requestAccessForMediaType') > native.indexOf('static bool sxc_authorize'),
    'permission request must remain inside the explicit start helper',
  )
  assert.match(native, /AVErrorRecordingSuccessfullyFinishedKey/)
  assert.match(native, /_movieStopRequested/)
  assert.match(native, /\(AVCaptureMovieFileOutput \*\)output stopRecording/)
  assert.match(native, /@available\(macOS 12\.3, \*\)/)
  assert.match(native, /synchronizationClock[\s\S]*CFRetain\(movieClock\)/)
  const stopBody = native.slice(native.indexOf('extern "C" int32_t sxc_macos_camera_stop'))
  assert.match(stopBody, /_movieStopRequestClockNs[\s\S]*CMClockGetTime[\s\S]*_movieStopRequested\.store\(true\)/)
  const finishBody = native.slice(native.indexOf('didFinishRecordingToOutputFileAtURL'), native.indexOf('- (void)captureDeviceDisconnected'))
  assert.match(finishBody, /_movieFinishClockNs[\s\S]*CMClockGetTime[\s\S]*dispatch_semaphore_signal\(_finished\)/)
  assert.match(nativeRust, /if status == 0[\s\S]*if timing\.finish_clock_ns == 0/)
  for (const field of ['stop_request_clock', 'finish_clock']) {
    assert.match(native, new RegExp(`uint64_t \\*movie_${field}_ns`))
    assert.match(nativeRust, new RegExp(`movie_${field}_ns: \\*mut u64`))
    assert.match(nativeRust, new RegExp(`&mut timing\\.${field}_ns`))
  }
  assert.match(native, /sxc_wait_camera_event\(handle\.delegate->_finished, 15\)/)
  assert.match(native, /\[\[NSRunLoop currentRunLoop\] runMode:NSDefaultRunLoopMode/)
  assert.doesNotMatch(native, /dispatch_semaphore_wait\(handle\.delegate->_finished/)
  assert.match(native, /AVCaptureDeviceWasDisconnectedNotification/)
  assert.match(native, /AVCaptureSessionRuntimeErrorNotification/)
  assert.match(nativeRust, /device_lost: bool/)
  assert.match(macAdapter, /fn terminal_state[\s\S]*CameraTerminalState::DeviceLost/)
  assert.match(native, /renameatx_np\(.+RENAME_EXCL/)
  assert.match(recorderBuild, /\.file\("src\/macos_camera_native\.mm"\)/)
  assert.match(recorderBuild, /rustc-link-lib=framework=AVFoundation/)
  assert.match(macInfo, /<key>NSCameraUsageDescription<\/key>/)
  assert.match(macEntitlements, /<key>com\.apple\.security\.device\.camera<\/key>/)
  assert.match(tauriConfig, /"entitlements": "macos-entitlements\.plist"/)
})

test('Recorder presents a selectable camera and preserves a separate editable take', () => {
  const panel = read('ui/src/panels/Record/index.tsx')
  const session = read('ui/src/app/useRecordingSession.ts')
  const control = read('ui/src/panels/Record/CameraControl.tsx')
  const preview = read('ui/src/panels/Record/StudioPreview.tsx')
  const placement = read('app/server/src/dispatch/screen_record_handlers/camera_foundation.rs')

  for (const selector of [
    'data-cut-rec-camera-toggle',
    'data-cut-rec-camera-device',
    'data-cut-rec-camera-state',
  ]) assert.match(control, new RegExp(selector))
  assert.match(control, /separate, editable camera take/)
  assert.doesNotMatch(control, /type="text"/)
  assert.match(panel, /studio\.camera\.enabled && cameraDeviceId && !rawCapture && !recordingPause\.enabled \? \{ cameraId: cameraDeviceId \} : \{\}/)
  assert.match(panel, /session\.requestStart\(preset\)/)
  assert.match(session, /if \(preset\.cameraId && !preset\.raw && !preset\.pause\) args\.camera_id = preset\.cameraId/)
  assert.match(panel, /screen_record\.studio_event/)
  assert.match(preview, /data-cut-rec-camera-layout-preview/)
  assert.match(placement, /ensure_camera_track/)
  assert.match(placement, /edit\.insert/)
  assert.match(placement, /camera_track_id/)
})
