// lib/encodeReport.ts — the encoder record on a render receipt.
// Mirrors EncodeReport in schema/receipts.schema.json. Absent on receipts
// written before the engine recorded the encoder.

export interface EncodeReport {
  /** ffmpeg's name for the video encoder that ran, e.g. libx264 or h264_nvenc. */
  encoder: string
  /** The request allowed a hardware encoder (hardware:auto); false for off. */
  hardware_requested: boolean
  /** The encoder is a hardware encoder. */
  hardware_used: boolean
  /** Set when hardware was requested and software ran: the plain reason. */
  fallback_reason?: string
  /** ffmpeg's own error line behind the reason. */
  fallback_detail?: string
  /** What the render did with colour (absent on older receipts). */
  color?: ColorReport
  /** Set when fewer libx264 threads ran so the render fit the free memory. */
  memory_note?: string
  /** The reduced libx264 thread count (with memory_note). */
  encoder_threads?: number
  /** Only when the GPU render path was asked for: the path that ran. */
  pipeline?: 'gpu' | 'standard'
  /** The GPU render path was asked for and the standard path ran: why. */
  pipeline_reason?: string
}

/** Mirrors ColorReport in schema/receipts.schema.json. */
export interface ColorReport {
  /** The delivered picture, e.g. "SDR BT.709". */
  output: string
  processing_bit_depth: number
  file_bit_depth: number
  sources: Array<{ asset: string; file: string; transfer?: string; bit_depth?: number; handling: string }>
  /** Plain sentences when a source was converted, reduced in bit depth or not converted. */
  note?: string
}
