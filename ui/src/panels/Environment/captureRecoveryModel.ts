// Recorder-recovery pagination for Settings > Health & Recovery.
//
// The API is intentionally not revision-bound. Treat one completed traversal as
// evidence reported by this check, validate every path-free page strictly, and
// discard the whole inventory when any continuation is inconsistent.

import type { VerbArgs, VerbResult } from '../../lib/client'
import type {
  CaptureRecoveryItem,
  CaptureRecoveryReceipt,
  CaptureRecoveryState,
  ScreenRecordRecoveryStatusResult,
} from '../../lib/clientResults'

const PAGE_LIMIT = 100
const MAX_CAPTURE_ROWS = 4_096
const MAX_PAGES = Math.ceil(MAX_CAPTURE_ROWS / PAGE_LIMIT)
const CAPTURE_ID = /^[A-Za-z0-9_-]{1,128}$/
const SIMPLE_BASENAME = /^[A-Za-z0-9][A-Za-z0-9._-]*$/
const CAPTURE_STATES = new Set<CaptureRecoveryState>([
  'complete', 'recovered', 'quarantined', 'interrupted',
  'owner_ambiguous', 'torn_journal', 'corrupt',
])
const RECEIPT_STATES = new Set<CaptureRecoveryReceipt['state']>([
  'complete', 'recovered', 'quarantined', 'interrupted',
])
const TERMINAL_RECEIPT_STATES = new Set<CaptureRecoveryReceipt['state']>([
  'complete', 'recovered', 'quarantined', 'interrupted',
])

export interface CaptureRecoveryInventory {
  captures: CaptureRecoveryItem[]
  complete: true
}

export type CaptureRecoveryCall = (
  args: VerbArgs['screen_record.recovery_status'],
) => Promise<VerbResult<ScreenRecordRecoveryStatusResult>>

function invalid(message: string): never {
  throw new Error(`screen_record.recovery_status ${message}`)
}

function record(value: unknown, label: string): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value)) invalid(`${label} is malformed`)
  return value as Record<string, unknown>
}

function safeCount(value: unknown, label: string): number {
  if (!Number.isSafeInteger(value) || (value as number) < 0) invalid(`${label} is malformed`)
  return value as number
}

function safeOptionalCount(value: unknown, label: string): number | null {
  return value === null ? null : safeCount(value, label)
}

function safeBasename(value: unknown): string | null {
  if (value === null) return null
  if (typeof value !== 'string' || !SIMPLE_BASENAME.test(value) || value === '.' || value === '..') {
    invalid('returned an unsafe source basename')
  }
  return value
}

function decodeReceipt(value: unknown): CaptureRecoveryReceipt | null {
  if (value === undefined || value === null) return null
  const receipt = record(value, 'receipt')
  const state = receipt.state
  if (typeof state !== 'string' || !RECEIPT_STATES.has(state as CaptureRecoveryReceipt['state'])) {
    invalid('returned an invalid receipt state')
  }
  const lostTailMs = safeOptionalCount(receipt.lost_tail_ms, 'receipt.lost_tail_ms')
  const lostTailLowerBoundMs = safeCount(receipt.lost_tail_lower_bound_ms, 'receipt.lost_tail_lower_bound_ms')
  const lostTailUpperBoundMs = safeOptionalCount(receipt.lost_tail_upper_bound_ms, 'receipt.lost_tail_upper_bound_ms')
  if (lostTailUpperBoundMs !== null && lostTailLowerBoundMs > lostTailUpperBoundMs) {
    invalid('returned inconsistent receipt loss bounds')
  }
  if (lostTailMs !== null && (lostTailMs < lostTailLowerBoundMs || (lostTailUpperBoundMs !== null && lostTailMs > lostTailUpperBoundMs))) {
    invalid('returned a receipt loss value outside its declared bounds')
  }
  return {
    state: state as CaptureRecoveryReceipt['state'],
    recovered_segments: safeCount(receipt.recovered_segments, 'receipt.recovered_segments'),
    lost_tail_ms: lostTailMs,
    lost_tail_lower_bound_ms: lostTailLowerBoundMs,
    lost_tail_upper_bound_ms: lostTailUpperBoundMs,
    audio_first_packet_offset_ms: safeOptionalCount(receipt.audio_first_packet_offset_ms, 'receipt.audio_first_packet_offset_ms'),
    source: safeBasename(receipt.source),
  }
}

function validateReceiptFacts(
  rowState: CaptureRecoveryState,
  checkpoints: number,
  receipt: CaptureRecoveryReceipt,
): void {
  if (receipt.recovered_segments > checkpoints) {
    invalid('returned a receipt with more recovered segments than checkpoints')
  }
  // A torn journal can preserve a previously sealed terminal receipt. Its row
  // state is torn_journal, so validate the retained receipt by its own state.
  const state = rowState === 'torn_journal' ? receipt.state : rowState
  switch (state) {
    case 'complete':
      if (receipt.recovered_segments !== checkpoints || receipt.lost_tail_ms !== 0 ||
        receipt.lost_tail_lower_bound_ms !== 0 || receipt.lost_tail_upper_bound_ms !== 0 ||
        receipt.source !== 'source.mp4') {
        invalid('returned an inconsistent complete receipt')
      }
      return
    case 'recovered':
      if (receipt.recovered_segments === 0 || receipt.source !== 'recovered.mp4') {
        invalid('returned an inconsistent recovered receipt')
      }
      return
    case 'quarantined':
      if (receipt.recovered_segments >= checkpoints ||
        (receipt.recovered_segments === 0 ? receipt.source !== null : receipt.source !== 'recovered.mp4')) {
        invalid('returned an inconsistent quarantined receipt')
      }
      return
    case 'interrupted':
      if (receipt.recovered_segments !== 0 || receipt.source !== null) {
        invalid('returned an inconsistent interrupted receipt')
      }
      return
    default:
      invalid('returned a terminal receipt for a nonterminal capture state')
  }
}

function decodeCapture(value: unknown): CaptureRecoveryItem {
  const capture = record(value, 'capture row')
  const captureId = capture.capture_id
  const state = capture.state
  if (typeof captureId !== 'string' || !CAPTURE_ID.test(captureId)) invalid('returned a malformed capture id')
  if (typeof state !== 'string' || !CAPTURE_STATES.has(state as CaptureRecoveryState)) {
    invalid('returned an invalid capture state')
  }
  if (typeof capture.has_open_segment !== 'boolean') invalid('returned a malformed has_open_segment flag')
  const checkpoints = safeCount(capture.checkpoints, 'checkpoints')
  const receipt = decodeReceipt(capture.receipt)
  const hasOpenSegment = capture.has_open_segment
  if (TERMINAL_RECEIPT_STATES.has(state as CaptureRecoveryReceipt['state']) &&
    (!receipt || receipt.state !== state || hasOpenSegment)) {
    invalid('returned terminal capture evidence without its matching sealed receipt')
  }
  if ((state === 'owner_ambiguous' || state === 'corrupt') && receipt !== null) {
    invalid('returned an owner-ambiguous or corrupt capture with a terminal receipt')
  }
  if (receipt) validateReceiptFacts(state as CaptureRecoveryState, checkpoints, receipt)
  return {
    capture_id: captureId,
    state: state as CaptureRecoveryState,
    checkpoints,
    has_open_segment: hasOpenSegment,
    receipt,
  }
}

function decodePage(value: unknown): { captures: CaptureRecoveryItem[]; nextCursor: string | null } {
  const page = record(value, 'result')
  if (!Array.isArray(page.captures)) invalid('returned malformed captures')
  if (page.captures.length > PAGE_LIMIT) invalid(`returned more than the ${PAGE_LIMIT}-row page limit`)
  if (!Object.prototype.hasOwnProperty.call(page, 'next_cursor')) invalid('returned a missing next_cursor')
  const nextCursor = page.next_cursor
  if (nextCursor !== null && (typeof nextCursor !== 'string' || !CAPTURE_ID.test(nextCursor))) {
    invalid('returned a malformed next_cursor')
  }
  return { captures: page.captures.map(decodeCapture), nextCursor }
}

/** Read every lexical page or throw. Callers only receive a completed inventory,
 * so a failure can never leave partial capture evidence in the Health UI. */
export async function loadCaptureRecovery(call: CaptureRecoveryCall): Promise<CaptureRecoveryInventory> {
  const captures: CaptureRecoveryItem[] = []
  const seenIds = new Set<string>()
  const seenCursors = new Set<string>()
  let after: string | undefined
  let previousId: string | undefined

  for (let pageIndex = 0; pageIndex < MAX_PAGES; pageIndex += 1) {
    const response = await call(after ? { after, limit: PAGE_LIMIT } : { limit: PAGE_LIMIT })
    if (!response.ok || !response.result) invalid('request failed')
    const page = decodePage(response.result)
    if (page.nextCursor !== null && (seenCursors.has(page.nextCursor) || (after !== undefined && page.nextCursor <= after))) {
      invalid('returned a repeated or nonprogress cursor')
    }
    for (const capture of page.captures) {
      if (seenIds.has(capture.capture_id)) invalid('returned a duplicate capture id')
      if (previousId !== undefined && capture.capture_id <= previousId) {
        invalid('returned non-increasing capture ids')
      }
      seenIds.add(capture.capture_id)
      if (captures.length >= MAX_CAPTURE_ROWS) invalid(`exceeded the ${MAX_CAPTURE_ROWS}-capture traversal limit`)
      captures.push(capture)
      previousId = capture.capture_id
    }
    if (page.nextCursor === null) return { captures, complete: true }
    if (page.captures.length === 0 || page.nextCursor !== previousId) {
      invalid('returned a next_cursor that does not match the final capture id')
    }
    seenCursors.add(page.nextCursor)
    after = page.nextCursor
  }
  invalid(`exceeded the ${MAX_CAPTURE_ROWS}-capture traversal limit`)
}
