import { strict as assert } from 'node:assert'
import { readFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import {
  STUDIO_BACKGROUND_PRESETS,
  backgroundLabel,
  defaultStudioState,
} from '../src/panels/Record/studioTypes'

const here = dirname(fileURLToPath(import.meta.url))
const read = (path: string) => readFileSync(resolve(here, path), 'utf8')
const record = read('../src/panels/Record/index.tsx')
const controls = read('../src/panels/Record/StudioControls.tsx')
const preview = read('../src/panels/Record/StudioPreview.tsx')
const studioCss = read('../src/panels/Record/record.css')
const exportHook = read('../src/panels/Record/useRecordingExport.ts')
const serverStudio = read('../../app/server/src/screen_record_studio.rs')

assert.deepEqual(
  STUDIO_BACKGROUND_PRESETS.map((preset) => preset.id),
  ['gradient', 'solid', 'blur_screen', 'none'],
  'the one preset catalog owns Studio background options in their editor order',
)
assert.equal(defaultStudioState().background, 'gradient', 'the default comes from the centralized preset catalog')
assert.equal(backgroundLabel('none'), 'None', 'None retains its distinct user-facing label')
assert.notEqual(
  STUDIO_BACKGROUND_PRESETS.find((preset) => preset.id === 'none')?.description,
  STUDIO_BACKGROUND_PRESETS.find((preset) => preset.id === 'solid')?.description,
  'None and Solid state different outcomes rather than duplicate labels',
)
assert.match(controls, /STUDIO_BACKGROUND_PRESETS\.map/, 'Studio controls render the centralized preset catalog')
assert.match(controls, /data-cut-studio-background-description/, 'Studio controls expose a stable preset-description selector')
assert.match(preview, /data-cut-studio-preset=\{preset\.id\}/, 'Studio preview exposes its selected preset for browser inspection')
assert.match(studioCss, /\.rec-studio-preview--none \.rec-studio-preview__screen\s*\{[\s\S]*?inset: 0;/,
  'None visibly removes the framed backdrop while Solid keeps the framed preview')
assert.match(serverStudio, /"none" => Ok\(record_core::Background::Transparent\)/,
  'None persists a transparent output plan instead of the Solid color')
assert.match(serverStudio, /if background == "none" \{[\s\S]*?plan\.frame\.enabled = false;/,
  'None disables the decorative frame so its transparent plan is full-bleed')

assert.doesNotMatch(record, /data-cut-rec-output-path=/, 'ordinary output DOM attributes do not expose the chosen absolute path')
assert.doesNotMatch(record, /data-cut-rec-raw-path=/, 'raw completion DOM attributes do not expose the saved absolute path')
assert.doesNotMatch(record, /Raw recording saved → \{lastRaw\.path\}/, 'raw completion copy is path-light')
assert.doesNotMatch(record, /title=\{recordOutputPath/, 'ordinary output labels do not put the path in a browser tooltip')
assert.match(record, /data-cut-rec-output-kind=\{recordOutputPath \? 'custom' : 'default'\}/,
  'the public DOM exposes output state without exposing an output path')
assert.doesNotMatch(exportHook, /Saved \$\{job\.format\.toUpperCase\(\)\} →/, 'export completion copy does not echo the saved path')
assert.doesNotMatch(exportHook, /\$\{path\}\)/, 'export failure copy does not echo the chosen path')

console.log('PASS Studio preset semantics and path-light Recorder output contract')
