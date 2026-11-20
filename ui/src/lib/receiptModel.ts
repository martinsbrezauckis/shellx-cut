// A render receipt as the engine writes it (receipts/render_*.json), and the
// checks' one-line verdict the engine adds to it. Re-exported by clientModel.

import type { CheckResult, FixAction, JudgeEnvelope } from './clientModel'

export interface RenderReceipt {
  render_id: string
  ts: string
  output_path: string
  output_hash: string
  duration_ms: number
  preset: string
  at_op: string
  checks: CheckResult[]
  pass: boolean
  judge?: JudgeEnvelope | null
  /** One entry per recognised FAILING check (empty when all pass). Drives the
   *  status-bar "what changed" summary + the autopilot self-fix loop. */
  fix_actions?: FixAction[]
  encode?: import('./encodeReport').EncodeReport
  /** The checks' verdict in one line (engine-authored); absent on older receipts. */
  outcome?: ReceiptCheckOutcome
}

/** problems = the written file is not the timeline; warnings = quality checks only. */
export interface ReceiptCheckOutcome {
  verdict: 'passed' | 'problems' | 'warnings' | 'unmeasured'
  summary: string
  action?: string
  /** Checks with nothing to check on this export (never failures or warnings). */
  not_applicable?: string[]
}
