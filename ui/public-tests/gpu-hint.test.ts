import { strict as assert } from 'node:assert'
import { encoderSizeRows, softwareExportHint } from '../src/panels/Environment/gpuHint'

// The faster-exports card names an old driver and what to do; the generic line
// stays only for a build without encoders.
assert.match(
  softwareExportHint({
    hardware_reason: 'driver_too_old',
    hardware_reason_text: 'The graphics driver is too old for the hardware encoder in this ffmpeg.',
    enable_help: 'Update the graphics driver, then restart ShellX Cut.',
  }),
  /graphics driver is too old.*Update the graphics driver/,
)
assert.equal(
  softwareExportHint({ hardware_reason: 'no_encoder_in_build', enable_help: 'Install an ffmpeg build' }),
  'Software export works. Add GPU support only if you need faster renders.',
)
assert.equal(
  softwareExportHint({}),
  'Software export works. Add GPU support only if you need faster renders.',
)

// Item 97: the card names the largest size each hardware encoder took.
assert.deepEqual(
  encoderSizeRows({
    size_limits: [
      { encoder: 'h264_nvenc', largest_accepted: '4096x2160', smallest_refused: '7680x4320' },
      { encoder: 'hevc_nvenc', largest_accepted: '8192x4320', smallest_refused: null },
      { encoder: 'av1_qsv', largest_accepted: null, smallest_refused: '3840x2160' },
    ],
  }),
  [
    ['Largest size, h264_nvenc', '4096x2160 (not 7680x4320)'],
    ['Largest size, hevc_nvenc', '8192x4320'],
    ['Largest size, av1_qsv', 'below 3840x2160 (not 3840x2160)'],
  ],
)
assert.deepEqual(encoderSizeRows({}), [])

console.log('gpu hint tests passed')
