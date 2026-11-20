import assert from 'node:assert/strict'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'

import {
  hasFetchAction,
  needsManualFfmpegSetup,
  type DoctorCard,
  type FfmpegSetupGuide,
} from '../src/lib/doctor'
import { userActionFailureDetail } from '../src/lib/userActionFeedback'
import type { VerbResult } from '../src/lib/client'
import EnvCardRow from '../src/panels/Environment/EnvCardRow'

// The guide the engine sends for a Mac without ffmpeg (cut_media::tool_setup).
const macGuide: FfmpegSetupGuide = {
  route: 'homebrew',
  fetch_available: false,
  steps: [
    'Open Terminal (Applications > Utilities > Terminal).',
    'Install Homebrew: copy the first command, paste it into Terminal and press Return. It asks for your Mac login password and can take several minutes.',
    'Install video processing: copy the second command, paste it and press Return.',
    'Come back to ShellX Cut and click Check again.',
  ],
  commands: [
    { label: 'Install Homebrew', command: '/bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"' },
    { label: 'Install FFmpeg', command: '/opt/homebrew/bin/brew install ffmpeg-full' },
  ],
  help_url: 'https://brew.sh',
  custom_build: 'Choose an ffmpeg that has ffprobe in the same folder and includes libass (captions), vid.stab (stabilize), zimg (color management), libx264 and libx265 (H.264 and HEVC export).',
}

const macMissing: DoctorCard = {
  id: 'ffmpeg',
  kind: 'tool',
  status: 'missing',
  source: 'missing',
  hint: 'ffmpeg is not installed, so Cut cannot import, preview or export yet.',
  details: { setup: macGuide },
}

// The engine's own word decides where a download exists.
assert.equal(hasFetchAction(macMissing, 'macos', 'aarch64'), false)
assert.equal(
  hasFetchAction({ ...macMissing, details: { setup: { ...macGuide, route: 'builtin_download', fetch_available: true } } }, 'macos', 'aarch64'),
  true,
  'a future built-in macOS download shows Install without a UI change',
)
assert.equal(needsManualFfmpegSetup(macMissing), true)
assert.equal(needsManualFfmpegSetup({ ...macMissing, status: 'unknown' }), false, 'an unverified probe shows no setup steps')
assert.equal(needsManualFfmpegSetup({ ...macMissing, id: 'ffprobe' }), false, 'the steps appear once, on the ffmpeg card')

const render = (card: DoctorCard, os = 'macos', arch = 'aarch64') =>
  renderToStaticMarkup(createElement(EnvCardRow, { card, os, arch, onChanged: () => {} }))

// A Mac without ffmpeg sees complete steps, copyable commands, Check again and
// Choose ffmpeg — and no dead Install button and no restart instruction.
const mac = render(macMissing)
assert.match(mac, /data-cut-ffmpeg-setup="homebrew"/)
for (let step = 1; step <= 4; step++) assert.match(mac, new RegExp(`data-cut-ffmpeg-setup-step="${step}"`))
assert.match(mac, /data-cut-ffmpeg-setup-copy="1"/)
assert.match(mac, /data-cut-ffmpeg-setup-copy="2"/)
assert.match(mac, /\/opt\/homebrew\/bin\/brew install ffmpeg-full/)
assert.match(mac, /Homebrew\/install\/HEAD\/install\.sh/)
assert.match(mac, /data-cut-ffmpeg-setup-check/)
assert.match(mac, />Check again</)
assert.match(mac, /data-cut-ffmpeg-setup-choose/)
assert.match(mac, /ffprobe in the same folder/)
assert.match(mac, /https:\/\/brew\.sh/)
assert.doesNotMatch(mac, /data-cut-env-download/)
assert.doesNotMatch(mac, /restart (ShellX )?Cut|after restart/i)

// Where Cut downloads ffmpeg itself, the card keeps its one Install action.
const windowsMissing: DoctorCard = {
  ...macMissing,
  details: { setup: { ...macGuide, route: 'builtin_download', fetch_available: true, commands: [] } },
}
const windows = render(windowsMissing, 'windows', 'x86_64')
assert.match(windows, /data-cut-env-download="ffmpeg"/)
assert.doesNotMatch(windows, /data-cut-ffmpeg-setup=/)

// ffprobe is essential too; it must not read as optional.
const ffprobe = render({ ...macMissing, id: 'ffprobe' })
assert.match(ffprobe, /Comes with video processing/)
assert.doesNotMatch(ffprobe, /only if you need this feature/)
// The same existing download action repairs a missing ffprobe even while
// ffmpeg itself is still ready; both come from system.fetch_tool ffmpeg.
assert.equal(hasFetchAction({ id: 'ffprobe', kind: 'tool', status: 'missing', details: {} }, 'windows', 'x86_64'), true)
assert.equal(hasFetchAction({ id: 'ffprobe', kind: 'tool', status: 'ok', details: {} }, 'windows', 'x86_64'), false)
assert.equal(hasFetchAction({ id: 'ffprobe', kind: 'tool', status: 'unknown', details: {} }, 'windows', 'x86_64'), false)
assert.match(render({ id: 'ffprobe', kind: 'tool', status: 'missing', details: {} }, 'windows', 'x86_64'), /data-cut-env-download="ffprobe"/)


// With no ffmpeg at all the hardware-encoding card must not say exports work.
const gpu = render({
  id: 'gpu-encode',
  kind: 'tool',
  status: 'degraded',
  hint: 'Exports need video processing first. Set up Video processing above; faster exports are checked after that.',
  details: { hardware_available: false, ffmpeg_missing: true, enable_help: null },
})
assert.match(gpu, /Needs setup/)
assert.match(gpu, /Exports need video processing first/)
assert.doesNotMatch(gpu, /Software export works/)
assert.doesNotMatch(gpu, />Ready</)

// A verb refused for missing tools: plain words for the person (the agent's
// suggested_action is not shown) and the Open setup route, for any verb.
const refused: VerbResult = {
  ok: false,
  error: {
    code: 'tool_missing',
    message: 'ffprobe is not installed',
    cause: 'ShellX Cut needs ffmpeg and ffprobe to import, preview and export',
    suggested_action: 'Cut has no built-in FFmpeg download on macOS. Ask the person to install it … system.set_ffmpeg …',
  },
}
for (const verb of ['media.import', 'render.frame', 'render.final'] as const) {
  const detail = userActionFailureDetail(verb, refused, 'failed')
  assert.ok(detail, verb)
  assert.equal(detail.setupSurface, 'settings-video-performance', verb)
  assert.equal(detail.message, 'ffprobe is not installed. Set up video processing to import, preview and export.')
  assert.doesNotMatch(detail.message, /system\./)
}

console.log('PASS ffmpeg first-run setup')
