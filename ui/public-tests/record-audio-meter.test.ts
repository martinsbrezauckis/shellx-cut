import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { recordingAudioMeterPresentation } from '../src/panels/Record/recordingAudioMeterPresentation'
import {
  RECORDING_AUDIO_METER_POLL_MS,
  RecordingAudioMeterPollGuard,
  recordingAudioMeterCaptureEndDetail,
  recordingAudioMeterNotFoundEndDetail,
  recordingAudioMeterStatusError,
  recordingAudioMeterStatusIsNotFound,
  recordingAudioMeterStatusIsTerminal,
} from '../src/panels/Record/recordingAudioMeterPolling'
import type { ScreenRecordAudioMeter } from '../src/lib/clientResults'

function meter(overrides: Partial<ScreenRecordAudioMeter> = {}): ScreenRecordAudioMeter {
  return {
    state: 'live', peak_dbfs: -3, rms_dbfs: -12, decayed_peak_dbfs: -2,
    sample_age_ms: 20, stale: false, clipping: false,
    ...overrides,
  }
}

const live = recordingAudioMeterPresentation('Microphone', meter({ clipping: true }))
assert.equal(live.live, true, 'only a current native snapshot may become live')
assert.equal(live.fillPercent, 80, 'RMS maps a -60..0 dBFS range to the compact bar')
assert.equal(live.clipping, true, 'clipping stays visible only while the reading is current')

const stale = recordingAudioMeterPresentation('Microphone', meter({ stale: true, clipping: true }))
assert.equal(stale.state, 'stale', 'a contradictory live/stale payload fails closed')
assert.equal(stale.fillPercent, 0, 'retained decay may not fill a live meter')
assert.equal(stale.clipping, false, 'a stale packet cannot continue to claim clipping')

const lost = recordingAudioMeterPresentation('System audio', meter({ state: 'device_lost', stale: true }))
assert.equal(lost.live, false)
assert.match(lost.detail, /stopped delivering samples/i)

const unavailable = recordingAudioMeterPresentation('System audio', meter({
  state: 'unavailable', stale: true, detail: 'Core Audio transfers PCM only at Stop.',
}))
assert.equal(unavailable.fillPercent, 0)
assert.match(unavailable.detail, /only at Stop/)

const component = readFileSync(
  fileURLToPath(new URL('../src/panels/Record/RecordingAudioMeters.tsx', import.meta.url)),
  'utf8',
)
assert.match(component, /data-cut-rec-audio-meters/, 'the meter component keeps a stable inspection selector')
assert.match(component, /role="meter"/, 'the compact status is accessible when mounted by the Record owner')

const guard = new RecordingAudioMeterPollGuard()
const first = guard.begin('capture-one')
assert.equal(guard.isCurrent(first), true, 'the current capture owns status responses')
const second = guard.begin('capture-two')
assert.equal(guard.isCurrent(first), false, 'a prior capture response cannot overwrite the replacement capture')
assert.equal(guard.isCurrent(second), true)
guard.cancel(second)
assert.equal(guard.isCurrent(second), false, 'unmount invalidates an outstanding response before it can reach React')
assert.equal(RECORDING_AUDIO_METER_POLL_MS, 250, 'meter reads use one bounded sub-second cadence')
assert.equal(recordingAudioMeterStatusIsTerminal({ terminal: true } as never), true, 'terminal status stops the poller')
assert.equal(recordingAudioMeterStatusIsNotFound({ code: 'not_found' }), true, 'missing active capture stops the poller')
assert.match(
  recordingAudioMeterCaptureEndDetail({ terminal: true, source_lifecycle: { state: 'source_lost' } } as never) ?? '',
  /selected capture source closed/i,
  'an observed source loss carries a factual note into recovery-aware finalization',
)
assert.match(
  recordingAudioMeterCaptureEndDetail({ terminal: true } as never) ?? '',
  /native capture ended/i,
  'a bounded native terminal is evidence to finalize, not an immediate recording error',
)
assert.equal(recordingAudioMeterCaptureEndDetail({ terminal: false } as never), null, 'live status cannot end the HUD')
assert.match(recordingAudioMeterNotFoundEndDetail({ code: 'not_found' }) ?? '', /no longer active/i)
assert.match(recordingAudioMeterStatusError({ code: 'not_found' }), /no longer active/i)
assert.match(recordingAudioMeterStatusError(new TypeError('offline')), /recorder unreachable/i)

const hook = readFileSync(
  fileURLToPath(new URL('../src/panels/Record/useRecordingAudioMeters.ts', import.meta.url)),
  'utf8',
)
const css = readFileSync(
  fileURLToPath(new URL('../src/panels/Record/recordingAudioMeters.css', import.meta.url)),
  'utf8',
)
const record = readFileSync(
  fileURLToPath(new URL('../src/panels/Record/index.tsx', import.meta.url)),
  'utf8',
)
assert.match(hook, /callVerb\('screen_record\.status', \{ capture_id: captureId \}\)/,
  'the hook reads only the existing status projection for its exact capture')
assert.match(hook, /RecordingAudioMeterPollGuard/, 'late capture generations and unmount responses are guarded')
assert.match(hook, /recordingAudioMeterNotFoundEndDetail\(response\.error\)[\s\S]*if \(captureEndDetail\) stopPolling\(\)/,
  'not_found stops meter polling rather than retrying a dead capture')
assert.match(hook, /recordingAudioMeterStatusIsTerminal\(status\)\) stopPolling\(\)/,
  'terminal status stops meter polling')
assert.match(hook, /recordingAudioMeterCaptureEndDetail\(status\)/,
  'a terminal status carries a capture-end conclusion to the Record owner')
assert.match(hook, /recordingAudioMeterNotFoundEndDetail\(response\.error\)/,
  'not_found carries the same capture-end conclusion rather than merely hiding meters')
assert.match(hook, /sourceLifecycle: status\.source_lifecycle/,
  'the existing bounded status poll retains the real source lifecycle')
assert.match(hook, /controllerPlacement: status\.controller_placement/,
  'the existing bounded status poll retains observed controller placement')
assert.doesNotMatch(hook, /screen_record\.doctor|system_audio_probe|screen_record\.start/,
  'meter status errors never trigger a probe or reopen a capture input')
assert.match(component, /import '\.\/recordingAudioMeters\.css'/,
  'the meter owns its small styling surface instead of growing Record CSS')
assert.match(component, /meter\.state === 'not_requested' \|\| meter\.state === 'unavailable'/,
  'not-requested and unavailable sources render compactly without a fake level track')
assert.match(component, /data-cut-rec-audio-meter-status-error/, 'status-read errors are inspectable when mounted')
assert.match(css, /\.rec-audio-meter--compact/, 'compact unavailable styles are isolated with the component')
assert.match(record, /useRecordingAudioMeters\(sceneCaptureId, phase === 'recording'\)/,
  'the meter polls only for the exact active recording capture')
assert.match(record, /recordingAudioMeters\.captureEndDetail[\s\S]*void finalize\(sceneCaptureId, null\)[\s\S]*setNote\(captureEndedDetail\)/,
  'terminal, source_lost, and not_found status use normal recovery-aware finalization once, with an informational note')
assert.match(record, /if \(left <= 0\) void finalize\(res\.capture_id, null\)/,
  'a bounded natural terminal uses the same finalizer as an observed terminal status')
assert.match(record, /if \(captureRef\.current !== captureId\) return[\s\S]*captureRef\.current = null[\s\S]*callVerb\('screen_record\.stop'/,
  'the capture ref makes a duration/status race invoke the finalizer only once before Stop')
assert.doesNotMatch(record, /if \(captureEndedDetail\)[\s\S]*setErr\(/,
  'terminal evidence never becomes an immediate error before recovery-aware Stop has run')
assert.match(record, /<RecordingLiveControls[\s\S]*audioMeters=\{<RecordingAudioMeters \{\.\.\.recordingAudioMeters\} \/>\}/,
  'capture-owned meters mount inside the compact live HUD')
