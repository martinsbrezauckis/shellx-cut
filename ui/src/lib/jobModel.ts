// Durable background-job result model, kept outside the broad project model so
// terminal/recovery additions do not grow that compatibility surface.

import type { CutError } from './clientModel'

export interface JobRecord {
  job_id: string
  kind: string
  state: 'queued' | 'running' | 'done' | 'failed'
  /** Terminal quality within state=done. Older persisted records may omit it. */
  completion?: 'success' | 'done_with_warnings'
  /** First-class terminal outcome; absent on active and legacy records. */
  outcome?: 'succeeded' | 'failed' | 'cancelled' | 'interrupted' | 'superseded'
  /** Why a terminal job ended. Never infer cancellation from a failed state. */
  outcome_reason?: 'completed' | 'completed_with_warnings' | 'true_failure' | 'user_cancelled' | 'project_switch_cancelled' | 'restart_interrupted' | 'superseded'
  progress: number
  /** Latest durable human-readable phase, when the worker reported one. */
  message?: string
  /** Present only while a limited job is waiting for shared local capacity. */
  queue?: { resource: string; max_running: number }
  /** Active child job currently awaited by an orchestrator. */
  waiting_on?: { job_id: string; kind: string }
  created_ts: string
  updated_ts: string
  result?: unknown
  error?: CutError
  /** A non-fatal persistence mirror issue for this record, when reported. */
  persistence_error?: string
}
