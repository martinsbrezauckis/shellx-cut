// events.ts — WS events client with auto-reconnect (public verb contract "events").
// Role: single shared connection to GET /api/events; typed event union
// mirroring server::events::Event; exponential-backoff reconnect (0.5s→8s)
// so a restarted cutd picks the UI back up without a reload:
// open the UI at any moment, see live state). Also the UI side of the
// ui-bridge relay: announces this tab as the UI client (ui_hello on every
// open — registration must not depend on a later ui_state push, which is
// dropped while the socket is still CONNECTING), answers screenshot_request
// frames via lib/capture (the ui.screenshot verb, UI contract verification
// primitive), and forwards ui_command frames (ui.open/playhead/select) to
// subscribers. Callers: App.tsx (connect once), panels (subscribe).
// Deps: WebSocket, lib/capture (lazy import — capture lib loads on demand).

import type { FixAction, JudgeEnvelope, OpRecord, RenderReceipt } from './client'
import type { DoctorReport } from './doctor'
import type { UiObservableState } from '../app/uiControlState'

/** Server→client events — mirrors server/src/events.rs exactly, plus the
 * ui_command relay frame (ui_bridge.rs) that agents drive via ui.open /
 * ui.playhead / ui.select. Every current server command carries a request_id;
 * older frames remain parseable but cannot be falsely acknowledged. */
export type CutEvent =
  | { type: 'op_applied'; op: OpRecord }
  | { type: 'job_progress'; job_id: string; kind: string; progress: number; message?: string }
  | { type: 'render_done'; job_id: string; render_id: string; ok: boolean; path?: string }
  | { type: 'receipt_ready'; receipt: RenderReceipt }
  | { type: 'ui_state'; state: unknown }
  | { type: 'project_changed'; open: boolean; name?: string }
  | { type: 'ui_command'; verb: 'ui.open' | 'ui.playhead' | 'ui.select' | 'ui.highlight'; args: Record<string, unknown>; request_id?: number }
  // The environment doctor's capabilities changed (startup scan, refresh,
  // or a completed system.fetch_tool) — the wizard + status-bar chip re-render.
  | { type: 'doctor_updated'; report: DoctorReport }

export type ConnectionState = 'connecting' | 'open' | 'closed'
export type UiCommandVerb = Extract<CutEvent, { type: 'ui_command' }>['verb']

export interface UiCommandResult {
  request_id: number
  verb: UiCommandVerb
  applied: boolean
  requested: Record<string, unknown>
  state: UiObservableState
  surface?: string
  selector?: string
  error?: { code: 'invalid_args' | 'not_found' | 'conflict' | 'no_ui_client'; message: string }
}

type EventListener = (ev: CutEvent) => void
type StatusListener = (s: ConnectionState) => void
type ScreenshotRequest = { type: 'screenshot_request'; request_id: number }

function recordFrom(v: unknown): Record<string, unknown> | null {
  if (v === null || typeof v !== 'object' || Array.isArray(v)) return null
  const out: Record<string, unknown> = {}
  for (const [key, value] of Object.entries(v)) out[key] = value
  return out
}

function stringField(v: Record<string, unknown>, name: string): string | undefined {
  const value = v[name]
  return typeof value === 'string' ? value : undefined
}

function numberField(v: Record<string, unknown>, name: string): number | undefined {
  const value = v[name]
  return typeof value === 'number' ? value : undefined
}

function booleanField(v: Record<string, unknown>, name: string): boolean | undefined {
  const value = v[name]
  return typeof value === 'boolean' ? value : undefined
}

function recordArrayFrom(v: unknown): Record<string, unknown>[] {
  return Array.isArray(v) ? v.map(recordFrom).filter((item) => item !== null) : []
}

/** Preserve the structured judge payload carried by receipt_ready. The former
 * decoder rebuilt only the deterministic receipt fields and silently dropped
 * `judge`, so a completed AI review disappeared until a later full resync. */
function judgeEnvelopeFrom(v: unknown): JudgeEnvelope | undefined {
  const obj = recordFrom(v)
  if (!obj) return undefined
  const out: JudgeEnvelope = {}
  const schema = stringField(obj, 'schema')
  const status = stringField(obj, 'status')
  const notRunReason = stringField(obj, 'not_run_reason')
  const reason = stringField(obj, 'reason')
  if (schema) out.schema = schema
  if (status) out.status = status
  if (notRunReason) out.not_run_reason = notRunReason
  if (reason) out.reason = reason

  const review = recordFrom(obj.review)
  const verdict = review ? stringField(review, 'verdict') : undefined
  if (review && (verdict === 'pass' || verdict === 'fail' || verdict === 'needs_review')) {
    out.review = {
      ...review,
      verdict,
      issues: recordArrayFrom(review.issues).map((issue) => ({ ...issue })),
    }
  } else if (obj.review === null) {
    out.review = null
  }
  const backend = recordFrom(obj.backend)
  if (backend) out.backend = { ...backend }
  const cli = recordFrom(obj.cli)
  if (cli) out.cli = { ...cli }
  return out
}

/** Validate enough of one repair action for status-bar consumers. Invalid
 * entries are dropped instead of turning an untrusted event into a UI crash. */
function fixActionFrom(v: unknown): FixAction | null {
  const obj = recordFrom(v)
  if (!obj) return null
  const check = stringField(obj, 'check')
  const fixVerb = stringField(obj, 'fix_verb')
  const rationale = stringField(obj, 'rationale')
  const fixArgs = recordFrom(obj.fix_args)
  const autoFixable = booleanField(obj, 'auto_fixable')
  if (!check || !fixVerb || !rationale || !fixArgs || autoFixable == null) return null
  return {
    check,
    fix_verb: fixVerb,
    fix_args: fixArgs,
    targets: recordArrayFrom(obj.targets).map((target) => ({ ...target })),
    measured: obj.measured,
    rationale,
    auto_fixable: autoFixable,
  }
}

function screenshotRequestFrom(v: unknown): ScreenshotRequest | null {
  const obj = recordFrom(v)
  if (!obj || obj.type !== 'screenshot_request') return null
  const requestId = numberField(obj, 'request_id')
  return requestId == null ? null : { type: 'screenshot_request', request_id: requestId }
}

function opEffectFrom(v: unknown): { track?: string; [k: string]: unknown } | null {
  const obj = recordFrom(v)
  if (!obj) return null
  const out: { track?: string; [k: string]: unknown } = {}
  for (const [key, value] of Object.entries(obj)) {
    if (key === 'track') {
      if (typeof value === 'string') out.track = value
    } else {
      out[key] = value
    }
  }
  return out
}

function opRecordFrom(v: unknown): OpRecord | null {
  const obj = recordFrom(v)
  const actorObj = obj ? recordFrom(obj.actor) : null
  if (!obj || !actorObj) return null
  const opId = stringField(obj, 'op_id')
  const ts = stringField(obj, 'ts')
  const verb = stringField(obj, 'verb')
  const actorKind = stringField(actorObj, 'kind')
  const actorName = stringField(actorObj, 'name')
  const actorVia = stringField(actorObj, 'via')
  const statusValue = stringField(obj, 'status')
  const status = statusValue === 'applied' || statusValue === 'rejected' ? statusValue : null
  if (!opId || !ts || !verb || !actorName || !actorVia || !status) return null
  if (actorKind !== 'agent' && actorKind !== 'human' && actorKind !== 'system') return null

  const out: OpRecord = {
    op_id: opId,
    ts,
    actor: { kind: actorKind, name: actorName, via: actorVia },
    verb,
    args: obj.args,
    status,
  }
  const rationale = stringField(obj, 'rationale')
  if (rationale) out.rationale = rationale
  const effects = Array.isArray(obj.effects) ? obj.effects.map(opEffectFrom).filter((item) => item !== null) : []
  if (effects.length > 0) out.effects = effects
  const inverseObj = recordFrom(obj.inverse)
  const inverseVerb = inverseObj ? stringField(inverseObj, 'verb') : undefined
  if (inverseObj && inverseVerb) out.inverse = { verb: inverseVerb, args: inverseObj.args }
  return out
}

function renderReceiptFrom(v: unknown): RenderReceipt | null {
  const obj = recordFrom(v)
  if (!obj) return null
  const renderId = stringField(obj, 'render_id')
  const ts = stringField(obj, 'ts')
  const outputPath = stringField(obj, 'output_path')
  const outputHash = stringField(obj, 'output_hash')
  const durationMs = numberField(obj, 'duration_ms')
  const preset = stringField(obj, 'preset')
  const atOp = stringField(obj, 'at_op')
  const pass = booleanField(obj, 'pass')
  if (!renderId || !ts || !outputPath || !outputHash || durationMs == null || !preset || !atOp || pass == null) return null
  const judge = obj.judge === null ? null : judgeEnvelopeFrom(obj.judge)
  const fixActions = recordArrayFrom(obj.fix_actions).map(fixActionFrom).filter((action) => action !== null)
  return {
    render_id: renderId,
    ts,
    output_path: outputPath,
    output_hash: outputHash,
    duration_ms: durationMs,
    preset,
    at_op: atOp,
    checks: recordArrayFrom(obj.checks).map((check) => ({
      name: stringField(check, 'name') ?? 'check',
      pass: booleanField(check, 'pass') ?? false,
      details: check.details,
      evidence: check.evidence,
    })),
    pass,
    ...(judge !== undefined ? { judge } : {}),
    ...(fixActions.length > 0 ? { fix_actions: fixActions } : {}),
  }
}

function doctorReportFrom(v: unknown): DoctorReport | null {
  const obj = recordFrom(v)
  if (!obj) return null
  const schema = stringField(obj, 'schema')
  const scannedAt = stringField(obj, 'scanned_at')
  const os = stringField(obj, 'os')
  const arch = stringField(obj, 'arch')
  const appVersion = stringField(obj, 'app_version')
  const essentialOk = booleanField(obj, 'essential_ok')
  if (!schema || !scannedAt || !os || !arch || !appVersion || essentialOk == null) return null
  return {
    schema,
    scanned_at: scannedAt,
    os,
    arch,
    app_version: appVersion,
    addr: stringField(obj, 'addr'),
    cards: recordArrayFrom(obj.cards).map((card) => ({
      id: stringField(card, 'id') ?? 'unknown',
      kind: stringField(card, 'kind') ?? 'service',
      status: card.status === 'ok' || card.status === 'missing' || card.status === 'degraded' || card.status === 'unknown' ? card.status : 'unknown',
      source: card.source === 'env' || card.source === 'bundled-or-appdata' || card.source === 'path' || card.source === 'missing' ? card.source : undefined,
      version: stringField(card, 'version'),
      hint: stringField(card, 'hint'),
      details: recordFrom(card.details) ?? {},
    })),
    essential_ok: essentialOk,
  }
}

function cutEventFrom(v: unknown): CutEvent | null {
  const obj = recordFrom(v)
  if (!obj) return null
  switch (obj.type) {
    case 'op_applied': {
      const op = opRecordFrom(obj.op)
      return op ? { type: 'op_applied', op } : null
    }
    case 'job_progress': {
      const jobId = stringField(obj, 'job_id')
      const kind = stringField(obj, 'kind')
      const progress = numberField(obj, 'progress')
      if (!jobId || !kind || progress == null) return null
      const message = stringField(obj, 'message')
      return { type: 'job_progress', job_id: jobId, kind, progress, ...(message ? { message } : {}) }
    }
    case 'render_done': {
      const jobId = stringField(obj, 'job_id')
      const renderId = stringField(obj, 'render_id')
      const ok = booleanField(obj, 'ok')
      if (!jobId || !renderId || ok == null) return null
      const path = stringField(obj, 'path')
      return { type: 'render_done', job_id: jobId, render_id: renderId, ok, ...(path ? { path } : {}) }
    }
    case 'receipt_ready': {
      const receipt = renderReceiptFrom(obj.receipt)
      return receipt ? { type: 'receipt_ready', receipt } : null
    }
    case 'ui_state':
      return { type: 'ui_state', state: obj.state }
    case 'project_changed': {
      const open = booleanField(obj, 'open')
      if (open == null) return null
      const name = stringField(obj, 'name')
      return { type: 'project_changed', open, ...(name ? { name } : {}) }
    }
    case 'ui_command': {
      const verb = stringField(obj, 'verb')
      const args = recordFrom(obj.args) ?? {}
      const requestId = numberField(obj, 'request_id')
      switch (verb) {
        case 'ui.open':
        case 'ui.playhead':
        case 'ui.select':
        case 'ui.highlight':
          return { type: 'ui_command', verb, args, ...(requestId == null ? {} : { request_id: requestId }) }
        default:
          return null
      }
    }
    case 'doctor_updated': {
      const report = doctorReportFrom(obj.report)
      return report ? { type: 'doctor_updated', report } : null
    }
    default:
      return null
  }
}

/**
 * One auto-reconnecting WS client for the whole app. Construct once in
 * App.tsx; panels subscribe/unsubscribe on mount/unmount.
 */
export class EventsClient {
  private ws: WebSocket | null = null
  private listeners = new Set<EventListener>()
  private statusListeners = new Set<StatusListener>()
  private backoffMs = 500
  private closedByUser = false
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null

  /** ws(s)://<host>/api/events — same origin (cutd serves us / vite proxies). */
  private url(): string {
    const proto = location.protocol === 'https:' ? 'wss:' : 'ws:'
    return `${proto}//${location.host}/api/events`
  }

  /** Open the connection (idempotent). */
  connect(): void {
    if (this.ws || this.closedByUser) return
    this.emitStatus('connecting')
    const ws = new WebSocket(this.url())
    this.ws = ws
    ws.onopen = () => {
      this.backoffMs = 500 // reset backoff on success
      // Register as THE UI client immediately — ui.screenshot / ui.state /
      // ui_command relay all target sockets that announced themselves.
      // (Piggybacking on the first ui_state push raced the CONNECTING state
      // and could leave the tab unregistered → no_ui_client errors.)
      ws.send(JSON.stringify({ type: 'ui_hello' }))
      // Replay the last known UI state right after announcing — a mount-time push
      // that raced this socket's CONNECTING state was dropped, leaving the server
      // with no ui_state (ui.state → no_ui_client). Sending it here closes the
      // first-state race so ui.state works on the first call.
      if (this.lastUiState) {
        ws.send(JSON.stringify({ type: 'ui_state', state: this.lastUiState }))
      }
      this.emitStatus('open')
    }
    ws.onmessage = (m) => {
      try {
        const ev = JSON.parse(String(m.data))
        const screenshotReq = screenshotRequestFrom(ev)
        if (screenshotReq) {
          // Bridge round-trip, not a broadcast event — answer it here.
          this.answerScreenshot(screenshotReq)
          return
        }
        const event = cutEventFrom(ev)
        if (!event) return
        this.listeners.forEach((l) => l(event))
      } catch {
        // Non-JSON frames are ignored — the stream contract is JSON-only.
      }
    }
    ws.onclose = () => {
      this.ws = null
      this.emitStatus('closed')
      this.scheduleReconnect()
    }
    ws.onerror = () => {
      // onclose follows; nothing to do here.
    }
  }

  /**
   * Answer a relayed ui.screenshot request: capture the app root and send
   * screenshot_result back with the correlation id (ui_bridge.rs contract).
   *
   * Reliability contract (2026-08-06 macOS bug-probe hardening — ui.screenshot
   * is a verification PRIMITIVE agents key on):
   * - ONE bounded retry after a short settle: the observed failure class is a
   *   transient resource-load rejection inside html-to-image (a poster/font
   *   finishing or failing mid-serialize), which a second pass typically
   *   clears. Exactly two attempts — a capture that fails twice must FAIL, not
   *   hang the verb toward its timeout.
   * - On final failure an EXPLICIT, STRUCTURED error frame is sent:
   *   {code, stage, message, attempts} — never the old String(err), which
   *   collapsed html-to-image's Event rejection into "[object Event]".
   */
  private answerScreenshot(req: { request_id: number }): void {
    void (async () => {
      try {
        const capture = await import('./capture')
        let cap: Awaited<ReturnType<typeof capture.captureApp>>
        try {
          cap = await capture.captureApp()
        } catch (firstErr) {
          await new Promise((resolve) => setTimeout(resolve, 300))
          try {
            cap = await capture.captureApp()
          } catch (finalErr) {
            const described = capture.describeCaptureError(finalErr)
            const first = capture.describeCaptureError(firstErr)
            this.ws?.send(
              JSON.stringify({
                type: 'screenshot_result',
                request_id: req.request_id,
                error: {
                  code: 'capture_failed',
                  stage: described.stage,
                  message: described.message,
                  attempts: 2,
                  // Both attempts reported when they failed differently —
                  // a flapping stage is itself diagnostic signal.
                  ...(first.message !== described.message ? { first_attempt: first.message } : {}),
                },
              }),
            )
            return
          }
        }
        this.ws?.send(
          JSON.stringify({
            type: 'screenshot_result',
            request_id: req.request_id,
            png_base64: cap.png_base64,
            notes: cap.notes,
          }),
        )
      } catch (err) {
        // Last-resort guard (send/import failure): still answer, still typed.
        this.ws?.send(
          JSON.stringify({
            type: 'screenshot_result',
            request_id: req.request_id,
            error: { code: 'capture_failed', stage: 'unknown', message: String(err), attempts: 0 },
          }),
        )
      }
    })()
  }

  /** Exponential backoff 0.5s → 8s cap. */
  private scheduleReconnect(): void {
    if (this.closedByUser || this.reconnectTimer) return
    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = null
      this.connect()
    }, this.backoffMs)
    this.backoffMs = Math.min(this.backoffMs * 2, 8000)
  }

  /** Permanently close (page teardown). */
  close(): void {
    this.closedByUser = true
    if (this.reconnectTimer) clearTimeout(this.reconnectTimer)
    this.ws?.close()
    this.ws = null
  }

  /** Subscribe to events; returns the unsubscribe function. */
  subscribe(l: EventListener): () => void {
    this.listeners.add(l)
    return () => this.listeners.delete(l)
  }

  /** Same event listener API, named to avoid confusion with RxJS subscribe(). */
  onEvent(l: EventListener): () => void {
    return this.subscribe(l)
  }

  /** Subscribe to connection-state changes; returns unsubscribe. */
  onStatus(l: StatusListener): () => void {
    this.statusListeners.add(l)
    return () => this.statusListeners.delete(l)
  }

  /** Answer a ui_command only after App has committed and read back the
   * resulting state. This is deliberately not automatic in onmessage. */
  answerUiCommand(result: UiCommandResult): void {
    if (this.ws?.readyState !== WebSocket.OPEN) return
    this.ws.send(JSON.stringify({
      type: 'ui_command_result',
      ...result,
      ts: new Date().toISOString(),
    }))
  }

  private emitStatus(s: ConnectionState): void {
    this.statusListeners.forEach((l) => l(s))
  }

  /**
   * Push this UI client's state to the server (panels open, playhead,
   * selection) — the data behind the ui.state verb. Call on every relevant
   * UI state change; cheap (one small frame).
   */
  pushUiState(state: UiObservableState): void {
    // Cache the latest desired state so a push that races the CONNECTING socket
    // (App's mount effect fires before onopen) is REPLAYED after ui_hello — else
    // the server has no ui_state and the first ui.state verb returns no_ui_client
    // until some later UI change happens to push again.
    this.lastUiState = state
    if (this.ws?.readyState === WebSocket.OPEN) {
      this.ws.send(JSON.stringify({ type: 'ui_state', state }))
    }
  }

  /** The most recent ui_state pushed — replayed on (re)connect after ui_hello so
   *  the server always has this client's current view state. */
  private lastUiState: UiObservableState | null = null
}

/** App-wide singleton (constructed at module load, connected by App.tsx). */
export const events = new EventsClient()
