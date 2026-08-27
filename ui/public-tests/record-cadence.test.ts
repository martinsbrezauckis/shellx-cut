import { strict as assert } from 'node:assert'
import { readFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import {
  probedAverageCadenceLabel,
  requestedCadenceLabel,
  type RecordingCadence,
} from '../src/panels/Record/recordingCadence'

const cadence: RecordingCadence = {
  schema: 'shellx-record/capture-cadence/1',
  requested: { num: 2997, den: 100 },
  backend_requested: { num: 30, den: 1 },
  probed_media: {
    avg_frame_rate: { num: 30000, den: 1001 },
    r_frame_rate: { num: 30, den: 1 },
  },
}

assert.equal(requestedCadenceLabel(cadence, 30), 'Requested 29.97 FPS')
assert.equal(probedAverageCadenceLabel(cadence), 'Measured average: 30000/1001 FPS')
assert.equal(
  probedAverageCadenceLabel({ ...cadence, probed_media: { avg_frame_rate: { num: 0, den: 0 } } }),
  'Measured average: Not measured.',
  'invalid/fabricated probe values remain visibly absent',
)
assert.equal(
  probedAverageCadenceLabel({ ...cadence, schema: 'unknown/cadence/2' }),
  'Measured average: Not measured.',
  'unknown cadence versions cannot be presented as measured evidence',
)

const uiRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const panel = readFileSync(resolve(uiRoot, 'src/panels/Record/index.tsx'), 'utf8')
const frameRateControl = readFileSync(resolve(uiRoot, 'src/panels/Record/RecordFrameRateControl.tsx'), 'utf8')
const verbSchema = JSON.parse(readFileSync(resolve(uiRoot, '../schema/verbs.json'), 'utf8')) as {
  verbs: Array<{ name: string, args?: { properties?: Record<string, { type?: string }> }, result?: string }>
}
const startContract = verbSchema.verbs.find((verb) => verb.name === 'screen_record.start')
const stopContract = verbSchema.verbs.find((verb) => verb.name === 'screen_record.stop')
assert.ok(frameRateControl.includes('Requested ${frameRateLabel(value)} FPS'), 'the selector describes requested, not measured, cadence')
assert.match(panel, /fps,\n\s*audio,/s, 'the UI keeps the legacy fps:number start payload')
assert.match(panel, /setCaptureCadence\(sr\.cadence \?\? null\)/, 'stop response cadence is the only displayed measurement source')
assert.match(panel, /data-cut-rec-cadence/, 'the cadence evidence has a stable visible inspection hook')
assert.equal(startContract?.args?.properties?.fps?.type, 'number', 'the public fps request stays numeric')
assert.match(startContract?.result ?? '', /cadence/, 'start contract returns additive cadence evidence')
assert.match(stopContract?.result ?? '', /cadence/, 'stop contract returns optional cadence evidence')

console.log('PASS recording cadence wording and payload truth')
