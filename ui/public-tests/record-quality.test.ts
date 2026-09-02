import { strict as assert } from 'node:assert'
import { readFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import {
  finalQualityLabel,
  qualityCapability,
  qualityResolution,
} from '../src/panels/Record/recordingQuality'

const capability = qualityCapability({
  supported: true,
  output_sizes: ['source', '1080p', '720p'],
  profiles: ['standard', 'high'],
})
assert.deepEqual(capability?.output_sizes, ['source', '1080p', '720p'])
assert.deepEqual(capability?.profiles, ['standard', 'high'])
assert.equal(
  qualityCapability({ supported: true, output_sizes: ['720p'], profiles: ['high'] }),
  null,
  'a stale Doctor response cannot grow a picker without novice-safe defaults',
)
assert.equal(
  qualityCapability({ supported: false, output_sizes: ['source'], profiles: ['standard'] }),
  null,
  'unsupported backends must omit the picker',
)

const resolution = qualityResolution({
  schema: 'shellx-record/capture-quality/1',
  requested: { output_size: '720p', profile: 'high' },
  width: 1280,
  height: 720,
  encoder: 'libx264',
})
assert.equal(finalQualityLabel(resolution!, null), 'Resolved 1280 × 720')
assert.equal(
  qualityResolution({
    schema: 'shellx-record/capture-quality/1',
    requested: { output_size: '720p', profile: 'high' },
    width: 0,
    height: 720,
    encoder: 'libx264',
  }),
  null,
  'unmeasured dimensions cannot become a completed quality result',
)

const uiRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const panel = readFileSync(resolve(uiRoot, 'src/panels/Record/index.tsx'), 'utf8')
const control = readFileSync(resolve(uiRoot, 'src/panels/Record/RecordingQualityControl.tsx'), 'utf8')
const css = readFileSync(resolve(uiRoot, 'src/panels/Record/record.css'), 'utf8')
assert.match(
  panel,
  /if \(!recordingPause\.enabled && qualityRequest\) startArgs\.quality = qualityRequest/,
  'only an advertised request reaches Start, and bounded Pause cannot carry Quality',
)
assert.match(panel, /setQualityResolution\(sr\.quality\)/, 'Stop is the sole source of final output facts')
assert.match(control, /data-cut-rec-quality-size/, 'the size picker has a stable selector')
assert.match(control, /data-cut-rec-quality-profile/, 'the profile picker has a stable selector')
assert.match(control, /role="group"/, 'pressed segmented buttons use matching group semantics')
assert.doesNotMatch(control, /radiogroup/, 'toggle buttons must not claim radio semantics')
assert.match(control, /Advanced facts/, 'implementation facts are disclosed instead of editable codec controls')
assert.match(control, /data-cut-action="rec-quality-advanced"/, 'advanced quality facts have a stable debug action')
assert.doesNotMatch(control, /crf|bitrate|preset/, 'numeric codec internals remain outside the novice UI')
assert.match(css, /min-height: 44px/, 'narrow quality choices retain touch-sized targets')

console.log('PASS recording quality capability, verified-facts, selectors, and novice-control contract')
