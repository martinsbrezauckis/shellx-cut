import { strict as assert } from 'node:assert'
import { renderReceiptFrom } from '../src/lib/events'
import { encodeLine } from '../src/panels/Review/encodeLine'

const base = {
  render_id: 'render_001',
  ts: 'receipt-time',
  output_path: '/p/exports/render_001.mp4',
  output_hash: 'sha256:abc',
  duration_ms: 1000,
  preset: 'standard',
  at_op: 'op_000001',
  checks: [],
  pass: true,
}

// A receipt keeps the encoder record through the parser the UI really uses.
const software = renderReceiptFrom({
  ...base,
  encode: {
    encoder: 'libx264',
    hardware_requested: true,
    hardware_used: false,
    fallback_reason:
      'No usable hardware encoder was found, so this export used software encoding. Update the graphics driver, then restart ShellX Cut.',
    fallback_detail: 'h264_nvenc: Required: 13.1 Found: 13.0',
  },
})
assert.ok(software?.encode, 'parser must keep the encode record')
assert.equal(software.encode.encoder, 'libx264')

const line = encodeLine(software)
assert.ok(line)
assert.match(line.text, /software encoding \(libx264\)/)
assert.match(line.note ?? '', /No usable hardware encoder was found, so this export used software encoding/)
assert.match(line.detail ?? '', /Found: 13\.0/)

// Hardware that ran says so and has no note.
const hardware = renderReceiptFrom({
  ...base,
  encode: { encoder: 'h264_nvenc', hardware_requested: true, hardware_used: true },
})
const hardwareLine = encodeLine(hardware!)
assert.match(hardwareLine?.text ?? '', /graphics card \(h264_nvenc\)/)
assert.equal(hardwareLine?.note, null)

// hardware:"off" is the person's choice, not a fallback: no note.
const off = renderReceiptFrom({
  ...base,
  encode: { encoder: 'libx264', hardware_requested: false, hardware_used: false },
})
assert.equal(encodeLine(off!)?.note, null)

// A receipt from before the record existed says nothing rather than guessing.
assert.equal(encodeLine(renderReceiptFrom(base)!), null)
// A malformed record is dropped, not half shown.
assert.equal(renderReceiptFrom({ ...base, encode: { encoder: 'libx264' } })?.encode, undefined)

console.log('encoder report tests passed')

// The colour record survives the parser and its note reaches the receipt line.
const hdr = renderReceiptFrom({
  ...base,
  encode: {
    encoder: 'libx264',
    hardware_requested: false,
    hardware_used: false,
    color: {
      output: 'SDR BT.709',
      processing_bit_depth: 8,
      file_bit_depth: 8,
      sources: [{ asset: 'a1', file: 'hlg.mp4', transfer: 'arib-std-b67', bit_depth: 10, handling: 'tone_mapped_to_sdr' }],
      note: 'hlg.mp4 is HDR (HLG, 10-bit) and was converted to SDR BT.709 for this export; choose Keep HDR to export 10-bit HDR.',
    },
  },
})
assert.ok(hdr?.encode?.color, 'parser must keep the colour record')
assert.equal(hdr.encode.color.sources[0].handling, 'tone_mapped_to_sdr')
assert.equal(
  encodeLine(hdr)?.color,
  'hlg.mp4 is HDR (HLG, 10-bit) and was converted to SDR BT.709 for this export; choose Keep HDR to export 10-bit HDR.',
)

const keepHdr = renderReceiptFrom({
  ...base,
  encode: {
    encoder: 'libx265',
    hardware_requested: true,
    hardware_used: false,
    fallback_reason: 'Keep HDR uses 10-bit software libx265; hardware encoders in Cut currently target 8-bit SDR.',
    color: {
      output: 'HDR BT.2020 (PQ)',
      transfer: 'smpte2084',
      processing_bit_depth: 10,
      file_bit_depth: 10,
      hdr10_metadata: {
        present_in_sources: true,
        carried: true,
        mastering_display: 'G(13250,34500)B(7500,3000)R(34000,16000)WP(15635,16450)L(10000000,1)',
        max_cll: '1000,400',
      },
      sources: [{ asset: 'a1', file: 'pq_hdr10.mp4', transfer: 'smpte2084', bit_depth: 10, handling: 'kept_hdr' }],
      note: 'HDR10 static metadata carried into the 10-bit PQ stream.',
    },
  },
})
assert.equal(keepHdr?.encode?.color?.transfer, 'smpte2084')
assert.equal(keepHdr?.encode?.color?.hdr10_metadata?.carried, true)
assert.equal(keepHdr?.encode?.color?.hdr10_metadata?.max_cll, '1000,400')
assert.match(encodeLine(keepHdr!)?.color ?? '', /HDR10 static metadata carried/)
// A plain SDR render says nothing extra about colour.
const plain = renderReceiptFrom({
  ...base,
  encode: { encoder: 'libx264', hardware_requested: false, hardware_used: false,
    color: { output: 'SDR BT.709', processing_bit_depth: 8, file_bit_depth: 8, sources: [] } },
})
assert.equal(encodeLine(plain!)?.color, null)
assert.equal(encodeLine(plain!)?.memory, null)

// Item 165: a render that used fewer encoder threads to fit memory says so.
const memoryNote =
  'Software encoding uses 12 of 54 threads so this export fits in the 16.0 GB of memory free now (about 15.6 GB instead of 25.0 GB); it will take longer. Close other programs for full speed, or choose HEVC to encode in hardware.'
const reduced = renderReceiptFrom({
  ...base,
  encode: { encoder: 'libx264', hardware_requested: false, hardware_used: false, memory_note: memoryNote, encoder_threads: 12 },
})
assert.equal(reduced?.encode?.encoder_threads, 12)
assert.equal(encodeLine(reduced!)?.memory, memoryNote)

// The GPU render path was asked for and the standard path ran: the receipt says why.
const pipelineReason =
  'The GPU render path was asked for (SHELLX_CUT_RENDER_GPU), but the timeline has captions, which the GPU render path cannot draw, so this export used the standard render path.'
const standard = renderReceiptFrom({
  ...base,
  encode: { encoder: 'h264_nvenc', hardware_requested: true, hardware_used: true, pipeline: 'standard', pipeline_reason: pipelineReason },
})
assert.equal(standard?.encode?.pipeline, 'standard')
assert.equal(encodeLine(standard!)?.note, pipelineReason)
// The GPU path ran: nothing to explain.
const gpu = renderReceiptFrom({
  ...base,
  encode: { encoder: 'h264_nvenc', hardware_requested: true, hardware_used: true, pipeline: 'gpu' },
})
assert.equal(encodeLine(gpu!)?.note, null)
