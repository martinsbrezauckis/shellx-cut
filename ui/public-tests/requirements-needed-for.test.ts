// Settings shows under each tool what in the app needs it, from the doctor's
// requirements section; a refusal that names its missing tool opens the
// Settings panel where that tool is set up.

import { strict as assert } from 'node:assert'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'
import NeededFor, { neededForLines, toolsForCard } from '../src/panels/Environment/NeededFor'
import { userActionFailureDetail } from '../src/lib/userActionFeedback'
import type { RequirementTool } from '../src/lib/doctor'

const tool = (over: Partial<RequirementTool>): RequirementTool => ({
  id: 'x', kind: 'tool', name: 'X', what: 'x', state: 'ready', get: {}, needed_for: [], improves: [], ...over,
})

const report = {
  requirements: {
    schema: 'shellx-cut/requirements/1',
    features: [],
    tools: [
      tool({ id: 'ffmpeg', doctor_card: 'ffmpeg', needed_for: ['Import and relink media', 'Export and render', 'Preview frames', 'Transcription', 'Dubbing'] }),
      tool({ id: 'ffmpeg_vidstab', doctor_card: 'ffmpeg', needed_for: ['Stabilize'] }),
      tool({ id: 'gpu_encoder', doctor_card: 'gpu-encode', improves: ['Export and render'] }),
      tool({ id: 'review_judge', doctor_cards_prefix: 'judge.', needed_for: ['Render review'] }),
      tool({ id: 'chat_cli', doctor_cards_prefix: 'judge.', needed_for: ['Agent Chat'], improves: [] }),
    ],
  },
}

// Which registry entries belong to which card.
assert.deepEqual(toolsForCard(report, 'ffmpeg').map((item) => item.id), ['ffmpeg', 'ffmpeg_vidstab'])
assert.deepEqual(toolsForCard(report, 'judge.codex').map((item) => item.id), ['review_judge', 'chat_cli'])
assert.deepEqual(toolsForCard(report, 'disk'), [])
assert.deepEqual(toolsForCard({}, 'ffmpeg'), [], 'an older engine without the section shows nothing')

// The lines: unique names, the long list shortened with the full list in the title.
const ffmpeg = neededForLines(toolsForCard(report, 'ffmpeg'))
assert.equal(ffmpeg.needed.length, 6)
const markup = renderToStaticMarkup(createElement(NeededFor, { cardId: 'ffmpeg', tools: toolsForCard(report, 'ffmpeg') }))
assert.match(markup, /data-cut-env-needed-for="ffmpeg"/)
assert.match(markup, /Needed for: Import and relink media, Export and render, Preview frames, Transcription and 2 more\./)
assert.match(markup, /title="Import and relink media, Export and render, Preview frames, Transcription, Dubbing, Stabilize"/)
const gpu = renderToStaticMarkup(createElement(NeededFor, { cardId: 'gpu-encode', tools: toolsForCard(report, 'gpu-encode') }))
assert.match(gpu, /Makes better: Export and render\./)
assert.doesNotMatch(gpu, /Needed for/)
assert.equal(renderToStaticMarkup(createElement(NeededFor, { cardId: 'disk', tools: [] })), '')

// A refusal naming its requirement opens that Settings panel.
const refused = {
  ok: false as const,
  error: {
    code: 'sidecar',
    message: 'Captions and transcription are not installed',
    cause: 'c',
    suggested_action: 'call system.setup_perception',
    requirement: { id: 'captions_runtime', name: 'Captions and transcription', get: { settings: 'settings-ai-transcription', verb: 'system.setup_perception' } },
  },
}
assert.equal(userActionFailureDetail('media.transcribe', refused, 'failed')?.setupSurface, 'settings-ai-transcription')
const notSettings = { ...refused, error: { ...refused.error, requirement: { id: 'open_project', get: { settings: 'projects' } } } }
assert.equal(userActionFailureDetail('edit.split', notSettings, 'failed')?.setupSurface, undefined, 'only Settings panels get the button')

console.log('requirements needed-for: ok')
