import assert from 'node:assert/strict'
import { createElement } from 'react'
import { renderToStaticMarkup } from 'react-dom/server'

import {
  ffmpegDownloadOffer,
  hasFetchAction,
  needsManualFfmpegSetup,
  type DoctorCard,
  type FfmpegDownloadOffer,
  type FfmpegSetupGuide,
} from '../src/lib/doctor'
import { userActionFailureDetail } from '../src/lib/userActionFeedback'
import type { VerbResult } from '../src/lib/client'
import EnvCardRow from '../src/panels/Environment/EnvCardRow'

// What the engine sends on Apple silicon (cut_media::tool_setup): the pinned
// download first, Homebrew as the other way.
const riedl: FfmpegDownloadOffer = {
  publisher: 'Martin Riedl',
  publisher_url: 'https://ffmpeg.martin-riedl.de',
  version: '9.0.2',
  size_bytes: 56_713_400,
  size_is_exact: true,
  license: 'GNU GPL version 3',
  license_url: 'https://www.gnu.org/licenses/gpl-3.0.html',
  source_url: 'https://ffmpeg.org/releases/ffmpeg-9.0.2.tar.xz',
  limitation: 'This build cannot stabilize video. For Stabilize, install the complete Homebrew build instead (steps below).',
  consent: 'Download FFmpeg 9.0.2 for Apple silicon (57 MB) from its publisher Martin Riedl (ffmpeg.martin-riedl.de). FFmpeg is free software under the GNU GPL version 3, separate from ShellX Cut. Source code: https://ffmpeg.org/releases/ffmpeg-9.0.2.tar.xz',
}

const siliconGuide: FfmpegSetupGuide = {
  route: 'builtin_download',
  fetch_available: true,
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
  download: riedl,
}

const card = (status: DoctorCard['status'], setup: FfmpegSetupGuide): DoctorCard => ({
  id: 'ffmpeg',
  kind: 'tool',
  status,
  details: { setup },
})
const render = (c: DoctorCard, os: string, arch: string) =>
  renderToStaticMarkup(createElement(EnvCardRow, { card: c, os, arch, onChanged: () => {} }))

// A Mac without ffmpeg: Download first, with the full consent beside it, the
// one-sentence limitation, then the other ways (Homebrew, Choose ffmpeg).
const missing = card('missing', siliconGuide)
assert.equal(hasFetchAction(missing, 'macos', 'aarch64'), true)
assert.equal(ffmpegDownloadOffer(missing)?.publisher, 'Martin Riedl')
assert.equal(needsManualFfmpegSetup(missing), true, 'Homebrew stays available beside the download')
const mac = render(missing, 'macos', 'aarch64')
assert.match(mac, /data-cut-env-download="ffmpeg"[^>]*>Download</)
assert.match(mac, /data-cut-ffmpeg-consent-text/)
for (const needle of ['Martin Riedl', 'ffmpeg.martin-riedl.de', '9.0.2', '57 MB', 'GNU GPL version 3', 'https://ffmpeg.org/releases/ffmpeg-9.0.2.tar.xz']) {
  assert.ok(mac.includes(needle), `consent shows ${needle}`)
}
assert.match(mac, /data-cut-ffmpeg-consent-limitation[^>]*>This build cannot stabilize video/)
assert.match(mac, /data-cut-ffmpeg-setup-other[^>]*>Other ways</)
assert.match(mac, /\/opt\/homebrew\/bin\/brew install ffmpeg-full/)
assert.match(mac, /data-cut-ffmpeg-setup-choose/)
assert.ok(mac.indexOf('data-cut-env-download') < mac.indexOf('data-cut-ffmpeg-setup-other'), 'Download comes first')

// After the download the build works but cannot stabilize: the card is
// degraded, and downloading the same build again would not help.
const degraded = card('degraded', siliconGuide)
assert.equal(hasFetchAction(degraded, 'macos', 'aarch64'), false)
assert.doesNotMatch(render(degraded, 'macos', 'aarch64'), /data-cut-env-download/)

// Windows and Linux: the same standard — publisher and licence beside Download.
for (const [os, system, size] of [['windows', 'Windows', '205'], ['linux', 'Linux', '160']] as const) {
  const offer: FfmpegDownloadOffer = {
    ...riedl,
    publisher: 'FFmpeg-Builds',
    publisher_url: 'https://github.com/FFmpeg-Builds/FFmpeg-Builds',
    version: 'current build',
    size_is_exact: false,
    source_url: 'https://github.com/FFmpeg-Builds/FFmpeg-Builds',
    limitation: null,
    consent: `Download the current FFmpeg build for ${system} (about ${size} MB) from its publisher FFmpeg-Builds (github.com/FFmpeg-Builds). FFmpeg is free software under the GNU GPL version 3, separate from ShellX Cut. Source code and build scripts: https://github.com/FFmpeg-Builds/FFmpeg-Builds`,
  }
  const guide: FfmpegSetupGuide = { ...siliconGuide, steps: [], commands: [], download: offer }
  const html = render(card('missing', guide), os, 'x86_64')
  assert.match(html, /data-cut-env-download="ffmpeg"[^>]*>Download</, os)
  assert.ok(html.includes('FFmpeg-Builds'), `${os} names the publisher`)
  assert.ok(html.includes('GNU GPL version 3'), `${os} names the licence`)
  assert.doesNotMatch(html, /data-cut-ffmpeg-setup=/, `${os} has no other-ways block`)
  // A complete build may be downloaded again over a degraded one.
  assert.equal(hasFetchAction(card('degraded', guide), os, 'x86_64'), true)
}

// Stabilize refused for a missing filter: the person sees plain words and the
// Open setup route (the engine sends tool_missing with the reason).
const refused: VerbResult = {
  ok: false,
  error: {
    code: 'tool_missing',
    message: 'Stabilize needs FFmpeg with vid.stab, and the installed FFmpeg has none',
    cause: 'Without it, turn Stabilize off on those clips.',
    suggested_action: 'Install Homebrew\'s complete FFmpeg, which has vid.stab: … system.set_ffmpeg …',
  },
}
const detail = userActionFailureDetail('edit.stabilize', refused, 'Could not stabilize')
assert.equal(detail?.setupSurface, 'settings-video-performance')
assert.ok(detail?.message.startsWith('Stabilize needs FFmpeg with vid.stab'), detail?.message)

console.log('PASS ffmpeg download consent')

// After the download: the card names what this build cannot do and keeps the
// steps to the complete build, without a second Download.
{
  const after: DoctorCard = {
    id: 'ffmpeg',
    kind: 'tool',
    status: 'degraded',
    details: { setup: siliconGuide, can_stabilize: false, can_burn_captions: true, can_color_manage: true },
  }
  const html = render(after, 'macos', 'aarch64')
  assert.match(html, /Not available with this build: Stabilize\./)
  assert.match(html, /\/opt\/homebrew\/bin\/brew install ffmpeg-full/)
  assert.doesNotMatch(html, /data-cut-env-download/)
  assert.doesNotMatch(html, /data-cut-ffmpeg-setup-other/, 'no "Other ways" without a Download above')
  console.log('PASS degraded download build explains itself')
}

// Which ffmpeg is in use and why: the engine's details.selection sentence is
// shown under Advanced details of the ffmpeg card.
{
  const text = 'Picked automatically: of the 2 FFmpeg builds found, this one can encode with the graphics hardware.'
  const inUse: DoctorCard = {
    id: 'ffmpeg',
    kind: 'tool',
    status: 'ok',
    version: '9.0.2',
    details: {
      setup: siliconGuide,
      resolved: '/opt/homebrew/opt/ffmpeg-full/bin/ffmpeg',
      selection: { reason: 'automatic_hardware', automatic: 'on', found: 2, text },
    },
  }
  const html = render(inUse, 'macos', 'aarch64')
  assert.ok(html.includes('Why this ffmpeg'), 'the reason row is shown')
  assert.ok(html.includes(text), 'with the engine sentence')
  const without = render({ ...inUse, details: { setup: siliconGuide } }, 'macos', 'aarch64')
  assert.ok(!without.includes('Why this ffmpeg'), 'no row when the engine sent no reason')
  console.log('PASS ffmpeg card says why this build is in use')
}
