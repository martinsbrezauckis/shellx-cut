import { strict as assert } from 'node:assert'
import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import {
  createSpeedRampDraft,
  insertSpeedRampPoint,
  SPEED_RAMP_MAX_POINTS,
  validateSpeedRampDraft,
} from '../src/panels/Inspector/speedRampEditorModel'

const defaultDraft = createSpeedRampDraft(null, 10_000)
assert.deepEqual(defaultDraft, {
  points: [
    { atSeconds: '0', factor: '1' },
    { atSeconds: '5', factor: '2' },
    { atSeconds: '10', factor: '1' },
  ],
  segments: 24,
}, 'an unramped clip starts with a useful three-point curve')

const storedDraft = createSpeedRampDraft({
  points: [
    { at_ms: 125, factor: 0.75 },
    { at_ms: 4_321, factor: 2.25 },
  ],
  segments: 12,
  preferred_segments: 48,
}, 5_000)
assert.deepEqual(storedDraft, {
  points: [
    { atSeconds: '0.125', factor: '0.75' },
    { atSeconds: '4.321', factor: '2.25' },
  ],
  segments: 48,
}, 'stored millisecond points round-trip and retained detail wins over effective detail')
assert.deepEqual(validateSpeedRampDraft(storedDraft, 5_000).points, [
  { at_ms: 125, factor: 0.75 },
  { at_ms: 4_321, factor: 2.25 },
])

for (const [draft, reason] of [
  [{ ...defaultDraft, points: [{ atSeconds: '', factor: '1' }, ...defaultDraft.points.slice(1)] }, 'Point 1 needs a source time.'],
  [{ ...defaultDraft, points: [{ atSeconds: '0', factor: '0.2' }, ...defaultDraft.points.slice(1)] }, 'Point 1 speed must be 0.25×–4×.'],
  [{ ...defaultDraft, points: [defaultDraft.points[0], { atSeconds: '0.0004', factor: '2' }, defaultDraft.points[2]] }, 'Point 2 must come after point 1.'],
  [{ ...defaultDraft, points: [defaultDraft.points[0], defaultDraft.points[1], { atSeconds: '10.001', factor: '1' }] }, 'Point 3 source time must be between 0 and 10 s.'],
] as const) {
  assert.equal(validateSpeedRampDraft(draft, 10_000).reason, reason)
}
assert.equal(
  validateSpeedRampDraft({
    ...defaultDraft,
    points: [defaultDraft.points[0], { atSeconds: '0', factor: '2' }, defaultDraft.points[2]],
  }, 10_000).invalidPoint,
  1,
  'validation identifies only the row that needs correction',
)

const uneven = {
  points: [
    { atSeconds: '0', factor: '1' },
    { atSeconds: '2', factor: '2' },
    { atSeconds: '10', factor: '1' },
  ],
  segments: 24,
}
assert.deepEqual(insertSpeedRampPoint(uneven, 10_000).points, [
  { atSeconds: '0', factor: '1' },
  { atSeconds: '2', factor: '2' },
  { atSeconds: '6', factor: '1.5' },
  { atSeconds: '10', factor: '1' },
], 'Add point bisects the largest source-time gap and interpolates its speed')

const fullDraft = {
  points: Array.from({ length: SPEED_RAMP_MAX_POINTS }, (_, index) => ({
    atSeconds: String(index),
    factor: '1',
  })),
  segments: 24,
}
assert.equal(insertSpeedRampPoint(fullDraft, 11_000), fullDraft, 'the bounded editor cannot grow beyond twelve points')

const root = resolve(import.meta.dirname, '..')
const editor = readFileSync(resolve(root, 'src/panels/Inspector/SpeedRampCurveEditor.tsx'), 'utf8')
const section = readFileSync(resolve(root, 'src/panels/Inspector/SpeedSection.tsx'), 'utf8')
for (const selector of [
  'data-cut-speed-ramp-editor',
  'data-cut-action="speed-ramp-point"',
  'data-cut-speed-ramp-at=',
  'data-cut-speed-ramp-factor=',
  'data-cut-speed-ramp-add',
  'data-cut-speed-ramp-remove=',
  'data-cut-action="speed-ramp-custom-apply"',
  'data-cut-speed-ramp-validation=',
]) assert.ok(editor.includes(selector), `custom curve editor exposes ${selector}`)
assert.ok(section.includes('data-cut-action="speed-ramp-custom"'), 'Inspector exposes the compact custom-curve disclosure')
assert.ok(section.includes("rationale: 'inspector: custom speed ramp'"), 'custom Apply is attributable in the operation log')
assert.ok(section.includes('segments,'), 'custom Apply preserves the retained render-detail request')
assert.ok(section.includes('!speedRampApplied'), 'the section bypass state includes a stored custom or preset ramp')
assert.ok(section.includes('if (speedRampApplied) clearRamp()'), 'section bypass and reset clear the active ramp')
assert.ok(editor.includes('aria-describedby="cut-speed-ramp-validation"'), 'curve inputs expose the inline validation reason')

console.log('PASS arbitrary speed-ramp editor model and source contract')
