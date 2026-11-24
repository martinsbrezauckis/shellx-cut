import { strict as assert } from 'node:assert'
import { inspectTimelineHdr, keepHdrAvailability } from '../src/topbar/hdrExport'
import type { Project } from '../src/lib/client'
import type { DoctorReport } from '../src/lib/doctor'

function makeProject(
  transfer: string | null,
  opts?: { taggedRec709?: boolean; effects?: unknown[]; graded?: boolean; assetName?: string },
): Project {
  const name = opts?.assetName ?? (transfer ? 'hlg.mp4' : 'sdr.mp4')
  return {
    schema: 'shellx-cut/1',
    name: 'hdr-ui-test',
    settings: { width: 1920, height: 1080, fps: 30, audio_rate: 48000 },
    assets: {
      a1: {
        path: `/media/${name}`,
        hash: 'sha256:00',
        probe: {
          raw: {
            streams: [
              transfer
                ? { codec_type: 'video', pix_fmt: 'yuv420p10le', color_transfer: transfer }
                : { codec_type: 'video', pix_fmt: 'yuv420p', color_transfer: 'bt709' },
            ],
          },
        },
      },
    },
    tracks: [
      {
        id: 'v1',
        kind: 'video',
        clips: [
          {
            id: 'c1',
            asset: 'a1',
            src_in_ms: 0,
            src_out_ms: 2000,
            ...(opts?.taggedRec709 ? { input_color_space: 'rec709' } : {}),
            ...(opts?.graded ? { grade: { contrast: 1.2, brightness: 0, saturation: 1, gamma: 1 } } : {}),
            ...(opts?.effects ? { effects: opts.effects } : {}),
          },
        ],
      },
    ],
    markers: [],
    caption_styles: {},
    checkpoints: [],
  }
}

function makeDoctor(details: Record<string, unknown>): DoctorReport {
  return {
    schema: 'shellx-cut/doctor/1',
    scanned_at: '2026-11-24T00:00:00Z',
    os: 'linux',
    arch: 'x86_64',
    app_version: '0.6.122',
    essential_ok: true,
    cards: [{ id: 'ffmpeg', kind: 'tool', status: 'ok', details }],
  }
}

// 1. SDR-only timeline: Keep HDR is greyed with a clear timeline reason.
const sdrOnly = keepHdrAvailability(makeProject(null), 'hevc', null)
assert.equal(sdrOnly.available, false)
assert.equal(sdrOnly.summary.hasHdr, false)
assert.match(sdrOnly.reason ?? '', /HDR/)
assert.equal(sdrOnly.previewNote, null)

// 2. HDR timeline with H.264 selected: Keep HDR is greyed and explains H.265/AV1 requirement.
const hlgH264 = keepHdrAvailability(makeProject('arib-std-b67'), 'h264', null)
assert.equal(hlgH264.available, false)
assert.equal(hlgH264.summary.hasHdr, true)
assert.equal(hlgH264.summary.targetLabel, 'HLG')
assert.equal(hlgH264.reason, 'Keep HDR needs H.265 (HEVC) or AV1 10-bit')
assert.equal(hlgH264.previewNote, 'Preview shows HDR clips tone-mapped')

// 3. HDR timeline with HEVC or AV1 + capable doctor: Keep HDR is enabled.
const docOk = makeDoctor({ zscale: true, can_export_hevc_10bit: true, can_export_av1_10bit: true })
assert.equal(keepHdrAvailability(makeProject('smpte2084'), 'hevc', docOk).available, true)
assert.equal(keepHdrAvailability(makeProject('smpte2084'), 'av1', docOk).available, true)

// 4. Doctor reporting missing zscale or missing 10-bit encoder greys Keep HDR honestly.
assert.match(
  keepHdrAvailability(makeProject('arib-std-b67'), 'hevc', makeDoctor({ zscale: false, can_export_hevc_10bit: true })).reason ?? '',
  /convert HDR \(zimg\)/,
)
assert.match(
  keepHdrAvailability(makeProject('arib-std-b67'), 'hevc', makeDoctor({ zscale: true, can_export_hevc_10bit: false })).reason ?? '',
  /10-bit H\.265/,
)
assert.match(
  keepHdrAvailability(makeProject('arib-std-b67'), 'av1', makeDoctor({ zscale: true, can_export_av1_10bit: false })).reason ?? '',
  /10-bit AV1/,
)

// 5. SDR-only grades and effects emit an honest fallback notice naming the clip and features.
const graded = keepHdrAvailability(
  makeProject('arib-std-b67', { graded: true, effects: [{ type: 'vignette', amount: 0.4 }], assetName: 'sunset.mp4' }),
  'hevc',
  docOk,
)
assert.equal(graded.available, true)
assert.match(graded.fallbackNotice ?? '', /sunset\.mp4 \(color grade, vignette\)/)

// 6. Explicitly tagged rec709 HDR clip is not treated as an active HDR timeline source.
assert.equal(inspectTimelineHdr(makeProject('arib-std-b67', { taggedRec709: true })).hasHdr, false)

console.log('hdr export helper tests passed')
