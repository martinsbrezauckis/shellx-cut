// Renderer/UI base-video agreement regression. An empty structural video lane
// must not make the first populated program lane look like an overlay.
import { strict as assert } from 'node:assert'
import { readFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { baseVideoTrackId } from '../src/lib/layerStack'

const tracks = [
  { id: 'v-empty-a', kind: 'video', clips: [] },
  { id: 'v-empty-b', kind: 'video', clips: [] },
  { id: 'v-program', kind: 'video', clips: [{ id: 'clip-program' }] },
  { id: 'v-overlay', kind: 'video', clips: [{ id: 'clip-overlay' }] },
]

assert.equal(baseVideoTrackId(tracks), 'v-program', 'first populated video track owns the base canvas')
assert.equal(baseVideoTrackId(tracks.slice(2)), 'v-program', 'populated V1 remains the base canvas')
assert.equal(baseVideoTrackId(tracks.slice(0, 2)), null, 'no populated video track has no base canvas')

const here = dirname(fileURLToPath(import.meta.url))
const timeline = readFileSync(resolve(here, '../src/panels/Timeline/index.tsx'), 'utf8')
const mask = readFileSync(resolve(here, '../src/panels/Mask/index.tsx'), 'utf8')

assert.match(timeline, /baseVideoTrackId\(project\?\.tracks \?\? \[\]\)/,
  'Timeline shares the renderer first-non-empty base-video rule')
assert.match(mask, /baseVideoTrackId\(project\?\.tracks \?\? \[\]\)/,
  'Mask shares the renderer first-non-empty base-video rule')
assert.doesNotMatch(timeline, /project\?\.tracks\.find\(\(t\) => t\.kind === 'video'\)\?\.id/,
  'Timeline cannot restore first-structural-video base semantics')
assert.doesNotMatch(mask, /project\?\.tracks\.find\(\(t\) => t\.kind === 'video'\)/,
  'Mask cannot restore first-structural-video base semantics')

console.log('PASS base-video track agreement across renderer, Timeline, and Mask')
