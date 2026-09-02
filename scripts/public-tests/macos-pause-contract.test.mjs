import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import test from 'node:test'
import { fileURLToPath } from 'node:url'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..')
const read = (path) => readFileSync(resolve(root, path), 'utf8')

test('macOS Pause publishes explicit opt-in start and durable non-toggle controls', () => {
  const schema = JSON.parse(read('schema/verbs.json'))
  const start = schema.verbs.find((verb) => verb.name === 'screen_record.start')
  const pause = schema.verbs.find((verb) => verb.name === 'screen_record.pause')
  const resume = schema.verbs.find((verb) => verb.name === 'screen_record.resume')

  assert.deepEqual(start.args.properties.pause.required, ['mode'])
  assert.equal(start.args.properties.pause.properties.mode.const, 'enabled')
  assert.equal(start.args.properties.pause.additionalProperties, false)
  assert.match(start.result, /pause:\{enabled\}/)
  assert.deepEqual(pause.args.required, ['capture_id'])
  assert.deepEqual(resume.args.required, ['capture_id'])
  assert.match(pause.result, /action:\"pause\"/)
  assert.match(resume.result, /action:\"resume\"/)
  assert.match(pause.result, /saved:true/)
  assert.match(resume.result, /state:\"recording\"/)
  assert.equal(schema.verbs.some((verb) => verb.name === 'screen_record.pause_toggle'), false)
})

test('Pause admission is macOS-only, pre-reservation, exact-display and Scenes-safe', () => {
  const controls = read('app/server/src/screen_record/recording_controls.rs')
  const start = read('app/server/src/screen_record/start_handler.rs')
  const capture = read('app/server/src/screen_record/macos_pause_capture.rs')
  const doctor = read('app/server/src/screen_record/doctor_projection.rs')

  for (const token of ['cfg!(target_os = "macos")', 'exact_display', 'integer_fps', '"scenes"', '"camera"', '"keys"', '"window"', '"quality"']) {
    assert.ok(controls.includes(token), `pause capability includes ${token}`)
  }
  const admissionAt = start.indexOf('recording_controls::admit_public_start(')
  const captureIdAt = start.indexOf('let capture_id = new_capture_id()')
  const reservationAt = start.indexOf('create_capture_dir(&dir, &capture_id)')
  assert.ok(admissionAt >= 0 && admissionAt < captureIdAt && captureIdAt < reservationAt,
    'Pause incompatibilities must reject before capture reservation')
  assert.match(start, /\(!pause_enabled\)[\s\S]*recording_scenes::admit_start_config/)
  assert.match(capture, /SelectedCaptureStreams::new\(microphone, system_audio, false, false\)/)
  assert.match(capture, /controls\.reject_queued/)
  assert.match(doctor, /"pause": recording_controls::capability\(\)/)
})

test('Pause UI selects one capability-gated action only after an acknowledgement', () => {
  const record = read('ui/src/panels/Record/index.tsx')
  const pause = read('ui/src/panels/Record/useRecordingPause.ts')
  const control = read('ui/src/panels/Record/RecordingPauseControl.tsx')
  const source = read('ui/src/panels/Record/RecordingSourceControl.tsx')
  const sourceSetup = read('ui/src/panels/Record/RecordingSourceSetup.tsx')

  assert.match(record, /const \{ setDoctorCapability: setPauseDoctorCapability \} = recordingPause/)
  assert.match(record, /setPauseDoctorCapability\(res\.pause\)/)
  const probe = record.slice(record.indexOf('const probe = useCallback'), record.indexOf('useEffect(() => { void probe() }, [probe])'))
  assert.match(probe, /}, \[setPauseDoctorCapability, setDoctorSceneCapability, setQualityCapability\]\)/,
    'Doctor keeps one stable probe across Pause capability state updates')
  assert.doesNotMatch(probe, /\[recordingPause[\],]/,
    'the fresh hook result object must not retrigger the mount-time Doctor effect')
  assert.match(record, /startArgs\.pause = \{ mode: 'enabled' \}/)
  assert.match(record, /pauseEnabled=\{recordingPause\.enabled\}/)
  assert.match(sourceSetup, /allowWindow=\{!pauseEnabled\}/)
  assert.match(record, /Scenes are unavailable while Pause & resume is enabled/)
  assert.match(record, /Quality is unavailable while Pause & resume is enabled/)
  assert.match(pause, /recordingPauseAcknowledged\(response\.result, action\)/)
  assert.match(pause, /screen_record\.\$\{action\}/)
  assert.match(control, /data-cut-action="record-pause-resume"/)
  assert.equal((control.match(/data-cut-action="record-pause-resume"/g) ?? []).length, 1)
  assert.match(control, /data-cut-action="record-pause-enable"/)
  assert.match(source, /allowWindow = true/)
})
