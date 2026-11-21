// panels/Review/encodeLine.ts — what the Render history & checks tab says about the encoder.
// Pure: turns a receipt's `encode` record into the person-facing line and, when
// hardware was asked for and software ran, the plain reason.
// Callers: Receipts.tsx. Deps: lib/client types only.

import type { RenderReceipt } from '../../lib/client'

export interface EncodeLine {
  /** One short fact: which kind of encoder made the file, with its name. */
  text: string
  /** Set when hardware was requested and software ran, or the GPU render
   *  path was asked for and the standard path ran: why, and what to do. */
  note: string | null
  /** ffmpeg's own error line behind the note, for the details. */
  detail: string | null
  /** What happened to colour when it was not plain SDR in and out: an HDR
   *  source converted to SDR, a 10-bit source exported in 8 bits. */
  color: string | null
  /** Set when the export used fewer encoder threads to fit the free memory
   *  (it was slower): the engine's sentence with the numbers. */
  memory: string | null
}

/** Null for a receipt written before the engine recorded the encoder: say
 *  nothing rather than guess. */
export function encodeLine(receipt: Pick<RenderReceipt, 'encode'>): EncodeLine | null {
  const encode = receipt.encode
  if (!encode) return null
  const text = encode.hardware_used
    ? `Made with the graphics card (${encode.encoder})`
    : `Made with software encoding (${encode.encoder})`
  const hardwareNote = encode.hardware_requested && !encode.hardware_used
    ? encode.fallback_reason ?? 'No usable hardware encoder was found, so this export used software encoding.'
    : null
  // The GPU render path was asked for and the standard path ran: say why too.
  const pipelineNote = encode.pipeline === 'standard' ? encode.pipeline_reason?.trim() || null : null
  const note = [hardwareNote, pipelineNote].filter(Boolean).join(' ') || null
  const color = encode.color?.note?.trim() || null
  const memory = encode.memory_note?.trim() || null
  return { text, note, detail: hardwareNote ? encode.fallback_detail ?? null : null, color, memory }
}
