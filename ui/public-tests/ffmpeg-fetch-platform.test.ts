import assert from 'node:assert/strict'

import { hasFetchAction, type DoctorCard } from '../src/lib/doctor'

const missingFfmpeg: DoctorCard = {
  id: 'ffmpeg',
  kind: 'tool',
  status: 'missing',
  details: {},
}

assert.equal(hasFetchAction(missingFfmpeg, 'windows', 'x86_64'), true, 'Windows x86_64 offers the matching built-in FFmpeg install')
assert.equal(hasFetchAction(missingFfmpeg, 'linux', 'x86_64'), true, 'Linux x86_64 offers the matching built-in FFmpeg install')

for (const [os, arch] of [
  ['macos', 'aarch64'],
  ['windows', 'aarch64'],
  ['linux', 'aarch64'],
  ['freebsd', 'x86_64'],
] as const) {
  assert.equal(hasFetchAction(missingFfmpeg, os, arch), false, `${os}/${arch} has no matching built-in FFmpeg payload and shows no Install action`)
}

assert.equal(hasFetchAction({ ...missingFfmpeg, status: 'unknown' }, 'windows', 'x86_64'), false, 'an unverified Doctor result never advertises Install')
// A missing ffprobe is repaired by the same download (the archive carries both
// programs); a degraded ffprobe is not, and no other card starts the installer.
assert.equal(hasFetchAction({ ...missingFfmpeg, id: 'ffprobe' }, 'windows', 'x86_64'), true, 'a missing ffprobe offers the same FFmpeg download')
assert.equal(hasFetchAction({ ...missingFfmpeg, id: 'ffprobe', status: 'degraded' }, 'windows', 'x86_64'), false, 'a degraded ffprobe is not repaired by the download')
assert.equal(hasFetchAction({ ...missingFfmpeg, id: 'ffprobe' }, 'macos', 'x86_64'), false, 'ffprobe follows the same platform rule as ffmpeg')
assert.equal(hasFetchAction({ ...missingFfmpeg, id: 'python' }, 'windows', 'x86_64'), false, 'only the ffmpeg and ffprobe cards can start the FFmpeg installer')

console.log('PASS FFmpeg fetch platform availability')
