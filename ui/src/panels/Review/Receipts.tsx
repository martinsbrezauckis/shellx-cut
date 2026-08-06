// panels/Review/Receipts.tsx — the RECEIPTS tab: RenderReceipt cards
// Verdict banner readable across the room (`6/6 PASS` /
// `2 FAILED` / amber `CHECKING…`); check rows with status dot + key metric;
// click row → expandable evidence block with measured values and CLICKABLE
// timecode seek links (evidence is data, never prose-only).
// Profile-aware battery: checks waived by a footage profile render WAIVED
// (amber-muted — never green; measured outcome shown), and the
// `footage_profile` entry renders as a metadata row, not a check row.
// Judge (verify.judge): completed → verdict badge (pass=green / fail=red /
// needs_review=amber) + confidence + backend/cost line + issues with seek
// links; not_run → dashed-amber stub (a stub never looks like a pass, rule 3);
// error → red line with the cause.
// Callers: Review/index.tsx. Deps: ./shared, lib/client types.

import { useState } from 'react'
import { exportUrl as sharedExportUrl } from '../../lib/client'
import type { CheckResult, JudgeEnvelope, JudgeIssue, RenderReceipt } from '../../lib/client'
import { Icon } from '../../icons'
import { fmtDur, fmtTc } from './shared'

/** Map a render's output path to its download URL. Delegates to the shared
 *  cross-platform mapper (lib/clientUrls), like panels/Clips.
 *
 *  It used to own a private copy that returned null unless the path sat under
 *  an `/exports/` segment — so a render delivered to the user's chosen export
 *  folder had NO download link at all, and the copy also missed Windows
 *  backslash paths. The shared mapper emits the exact-file URL for those, which
 *  the engine now serves (fenced to the authorized export roots). */
function exportUrl(absPath: string | null | undefined): string | null {
  return absPath ? sharedExportUrl(absPath) : null
}

export interface ReceiptsProps {
  receipts: RenderReceipt[]
  onSeek: (atMs: number) => void
}

export default function Receipts({ receipts, onSeek }: ReceiptsProps) {
  if (receipts.length === 0) {
    return <div className="rr__empty">No delivery receipts yet — render the timeline to create one.</div>
  }
  // Newest first — prop order is oldest-first (receipt_ready appends).
  const newestFirst = [...receipts].reverse()
  return (
    <div className="rr-receipts" data-cut-receipts="">
      {newestFirst.map((r) => (
        <ReceiptCard key={r.render_id} receipt={r} onSeek={onSeek} />
      ))}
    </div>
  )
}

// ---------------------------------------------------------------------------
// Verdict math: a check with a non-boolean `pass` is PENDING (still running).
// The footage_profile entry documents (never gates) — excluded from counts.
// ---------------------------------------------------------------------------

/** Details payload a profile-waived check carries (perception waive()). */
interface WaiverDetails {
  waived_by_profile?: string
  waiver_reason?: string
  measured_pass?: boolean
}
const isObject = (v: unknown): v is object => v !== null && typeof v === 'object'
const stringProp = (v: object, name: string): string | undefined => {
  const value = Reflect.get(v, name)
  return typeof value === 'string' ? value : undefined
}
const numberProp = (v: object, name: string): number | undefined => {
  const value = Reflect.get(v, name)
  return typeof value === 'number' ? value : undefined
}

function waiverOf(c: CheckResult): WaiverDetails | null {
  const d = c.details
  if (!isObject(d) || !stringProp(d, 'waived_by_profile')) return null
  return {
    waived_by_profile: stringProp(d, 'waived_by_profile'),
    waiver_reason: stringProp(d, 'waiver_reason'),
    measured_pass: Reflect.get(d, 'measured_pass') === true ? true : Reflect.get(d, 'measured_pass') === false ? false : undefined,
  }
}

/** Terminal instrumentation gap. It gates the receipt, but is not a measured
 * content failure and must never feed the repair count. */
function isUnmeasured(c: CheckResult): boolean {
  const d = c.details
  return isObject(d)
    && (stringProp(d, 'status') === 'unmeasured' || Reflect.get(d, 'measured') === false)
}

/** Gating checks = everything except the footage_profile metadata entry. */
function gatingChecks(r: RenderReceipt): CheckResult[] {
  return (r.checks ?? []).filter((c) => c.name !== 'footage_profile')
}

function verdictOf(r: RenderReceipt): { label: string; kind: 'pass' | 'fail' | 'pending' | 'unmeasured' } {
  const checks = gatingChecks(r)
  const pending = checks.filter((c) => typeof c.pass !== 'boolean').length
  const unmeasured = checks.filter(isUnmeasured).length
  const failed = checks.filter((c) => c.pass === false && !isUnmeasured(c)).length
  if (pending > 0 || checks.length === 0) return { label: 'CHECKING…', kind: 'pending' }
  if (failed > 0) {
    return {
      label: `${failed} FAILED${unmeasured > 0 ? ` · ${unmeasured} UNMEASURED` : ''}`,
      kind: 'fail',
    }
  }
  if (unmeasured > 0) return { label: `${unmeasured} UNMEASURED`, kind: 'unmeasured' }
  const waived = checks.filter((c) => waiverOf(c)).length
  const allWaived = waived === checks.length
  // Waivers don't break the aggregate, but the banner must not read as green PASS.
  if (allWaived) return { label: `${waived} WAIVED`, kind: 'pending' }
  if (waived > 0) return { label: `${checks.length - waived}/${checks.length} PASS · ${waived} waived`, kind: 'pending' }
  return {
    label: `${checks.length}/${checks.length} PASS`,
    kind: 'pass',
  }
}

/** Pull one headline metric out of a check's details (`−14.1 LUFS` style). */
function keyMetric(c: CheckResult): string {
  const d = c.details
  if (!d || typeof d !== 'object') return ''
  const integrated = numberProp(d, 'integrated_lufs')
  if (integrated != null) return `${integrated.toFixed(1)} LUFS`
  const peak = numberProp(d, 'true_peak_db')
  if (peak != null) return `${peak.toFixed(1)} dBTP`
  const duration = numberProp(d, 'duration_ms')
  if (duration != null) return fmtDur(duration)
  const count = numberProp(d, 'count')
  if (count != null) return `×${count}`
  // generic fallback: first numeric field
  for (const [k, v] of Object.entries(d)) {
    if (typeof v === 'number') return k.endsWith('_ms') ? fmtTc(v) : String(v)
  }
  return ''
}

/** Recursively collect `*_ms` timestamps from evidence → seek chips (cap 8). */
function collectSeeks(evidence: unknown, out: Array<{ label: string; ms: number }> = []): Array<{ label: string; ms: number }> {
  if (out.length >= 8 || !evidence || typeof evidence !== 'object') return out
  for (const [k, v] of Object.entries(evidence)) {
    if (out.length >= 8) break
    if (k.endsWith('_ms') && typeof v === 'number') out.push({ label: k.replace(/_ms$/, ''), ms: v })
    else if (k.endsWith('_ms') && Array.isArray(v) && typeof v[0] === 'number') out.push({ label: k.replace(/_ms$/, ''), ms: v[0] })
    else if (typeof v === 'object') collectSeeks(v, out)
  }
  return out
}

interface CardProps {
  receipt: RenderReceipt
  onSeek: (atMs: number) => void
}

function ReceiptCard({ receipt, onSeek }: CardProps) {
  const [expanded, setExpanded] = useState<string | null>(null)
  const v = verdictOf(receipt)
  const profileEntry = (receipt.checks ?? []).find((c) => c.name === 'footage_profile')

  return (
    <div className={`rr-rc rr-rc--${v.kind}`} data-cut-receipt={receipt.render_id}>
      {/* banner: the across-the-room verdict */}
      <div className={`rr-rc__banner rr-rc__banner--${v.kind}`}>{v.label}</div>
      <div className="rr-rc__meta">
        {receipt.render_id} · {fmtDur(receipt.duration_ms)} · {receipt.preset} ·{' '}
        {receipt.output_hash?.replace(/^sha256:/, '').slice(0, 10)}
        {/* The rendered VIDEO is the deliverable — give it a one-click download.
            render.final writes exports/<id>.mp4, served (fenced) on /api/export/*. */}
        {exportUrl(receipt.output_path) && (
          <a
            className="rr-rc__download"
            href={exportUrl(receipt.output_path)!}
            download
            data-cut-receipt-download={receipt.render_id}
            title="download the rendered video (.mp4)"
          >
            ⤓ Download video
          </a>
        )}
      </div>
      {profileEntry && <ProfileRow entry={profileEntry} />}
      <div className="rr-rc__checks">
        {gatingChecks(receipt).map((c) => {
          const waiver = waiverOf(c)
          const unmeasured = isUnmeasured(c)
          // waived wins the visual: pass=true under the profile, but it must
          // never read as a green pass (amber-muted, measured outcome shown).
          const state = unmeasured ? 'unmeasured' : waiver ? 'waived' : typeof c.pass !== 'boolean' ? 'pending' : c.pass ? 'pass' : 'fail'
          const metric = keyMetric(c)
          const isOpen = expanded === c.name
          const seeks = isOpen ? collectSeeks(c.evidence) : []
          return (
            <div key={c.name} className={`rr-check rr-check--${state}`} data-cut-check={c.name}>
              <button className="rr-check__row" data-cut-receipt-check-toggle={c.name} onClick={() => setExpanded(isOpen ? null : c.name)}>
                <span className={`rr-dot rr-dot--${state}`} />
                <span className="rr-check__name">{c.name}</span>
                {/* a waived row always shows what was actually measured */}
                {waiver && (
                  <span className={`rr-check__measured rr-check__measured--${waiver.measured_pass ? 'pass' : 'fail'}`}>
                    measured {waiver.measured_pass ? 'PASS' : 'FAIL'}
                  </span>
                )}
                {metric && <span className="rr-check__metric">{metric}</span>}
                <span className={`rr-check__state rr-check__state--${state}`}>{state.toUpperCase()}</span>
              </button>
              {isOpen && (
                <div className="rr-check__evidence" data-cut-evidence={c.name}>
                  {waiver && (
                    <div className="rr-check__waiver-note">
                      waived by profile {waiver.waived_by_profile}
                      {waiver.waiver_reason ? ` — ${waiver.waiver_reason}` : ''}
                    </div>
                  )}
                  {seeks.length > 0 && (
                    <div className="rr-check__seeks">
                      {seeks.map((s, i) => (
                        <button
                          key={`${s.label}-${i}`}
                          className="rr-seek"
                          data-cut-seek={s.ms}
                          onClick={() => onSeek(s.ms)}
                          title={`seek ${s.label}`}
                        >
                          {s.label} @ {fmtTc(s.ms)}
                        </button>
                      ))}
                    </div>
                  )}
                  <pre className="rr-check__json">
                    {JSON.stringify(unmeasured ? { details: c.details, evidence: c.evidence } : c.evidence ?? c.details ?? null, null, 1)}
                  </pre>
                </div>
              )}
            </div>
          )
        })}
      </div>
      <JudgeSection judge={receipt.judge} onSeek={onSeek} />
    </div>
  )
}

// ---------------------------------------------------------------------------
// footage_profile metadata row — documents which profile interpreted the
// battery + the auto-detect proposal. Never gates; rendered as fact (mono),
// distinct from check rows (and excluded from the [data-cut-check] count).
// ---------------------------------------------------------------------------

function ProfileRow({ entry }: { entry: CheckResult }) {
  const d = isObject(entry.details) ? entry.details : null
  const activeProfile = d ? stringProp(d, 'active_profile') : undefined
  const selection = d ? stringProp(d, 'selection') : undefined
  const proposedProfile = d ? stringProp(d, 'proposed_profile') : undefined
  const evidence = isObject(entry.evidence) ? entry.evidence : null
  const reasonsValue = evidence ? Reflect.get(evidence, 'proposal_reasons') : null
  const reasons = Array.isArray(reasonsValue) ? reasonsValue.filter((r): r is string => typeof r === 'string') : []
  const proposes = proposedProfile && proposedProfile !== activeProfile ? proposedProfile : null
  return (
    <div
      className="rr-rc__profile"
      data-cut-profile={activeProfile ?? 'unknown'}
      title={reasons?.length ? `auto-detect:\n${reasons.join('\n')}` : undefined}
    >
      profile {activeProfile ?? '—'} ({selection ?? '—'})
      {proposes && <span className="rr-rc__profile-proposal"> · auto-detect proposes {proposes}</span>}
    </div>
  )
}

// ---------------------------------------------------------------------------
// Judge section (verify.judge / RenderReceipt.judge). Honest statuses only:
// completed renders the real verdict; not_run stays a dashed stub; error is
// red with the cause. The verdict NEVER merges into the deterministic banner —
// instruments own measured verdicts, the judge owns perception (verification contract).
// ---------------------------------------------------------------------------

function JudgeSection({ judge, onSeek }: { judge: JudgeEnvelope | null | undefined; onSeek: (atMs: number) => void }) {
  const [open, setOpen] = useState(false)
  if (!judge || judge.status === 'not_run') {
    return (
      <div className="rr-rc__judge rr-rc__judge--stub" data-cut-judge="not_run">
        JUDGE: NOT RUN
        {judge?.not_run_reason ? ` — ${judge.not_run_reason}` : ''}
      </div>
    )
  }
  if (judge.status === 'error') {
    return (
      <div className="rr-rc__judge rr-rc__judge--error" data-cut-judge="error">
        JUDGE: ERROR — {String(judge.reason ?? judge.not_run_reason ?? 'adapter failed')}
      </div>
    )
  }
  const review = judge.review
  if (!review) {
    // completed without a review payload — defensive; still never pass-like.
    return (
      <div className="rr-rc__judge rr-rc__judge--stub" data-cut-judge="empty">
        JUDGE: NO REVIEW ATTACHED
      </div>
    )
  }
  const verdict = review.verdict
  const vKind = verdict === 'pass' ? 'pass' : verdict === 'fail' ? 'fail' : 'review'
  const issues = review.issues ?? []
  // backend/cost line: model · frames · senses · $cost · wall time
  const b = judge.backend
  const cli = judge.cli
  const costLine = [
    b?.model ?? b?.name,
    typeof b?.frames_sent === 'number' ? `${b.frames_sent} frames` : null,
    b ? (b.listened ? 'watched+listened' : 'vision-only') : null,
    typeof cli?.accounting_cost_usd === 'number' ? `$${cli.accounting_cost_usd.toFixed(2)}` : null,
    typeof cli?.duration_ms === 'number' ? fmtDur(cli.duration_ms) : null,
  ]
    .filter(Boolean)
    .join(' · ')

  return (
    <div className="rr-judge" data-cut-judge={verdict}>
      <button className="rr-judge__head" data-cut-receipt-judge-toggle onClick={() => setOpen((o) => !o)} title="Open the visual quality review">
        <span className={`rr-judge__badge rr-judge__badge--${vKind}`}>
          JUDGE: {verdict === 'needs_review' ? 'NEEDS REVIEW' : verdict.toUpperCase()}
        </span>
        {typeof review.confidence === 'number' && (
          <span className="rr-judge__conf">{review.confidence.toFixed(2)} conf</span>
        )}
        {issues.length > 0 && <span className="rr-judge__count">{issues.length} issue{issues.length > 1 ? 's' : ''}</span>}
        <span className="rr-judge__chev"><Icon name={open ? 'chevronDown' : 'chevronRight'} size={14} /></span>
      </button>
      {costLine && <div className="rr-judge__backend">{costLine}</div>}
      {open && (
        <div className="rr-judge__body">
          {review.summary && <div className="rr-judge__summary">{review.summary}</div>}
          {issues.map((iss: JudgeIssue, i: number) => (
            <div key={i} className="rr-judge__issue" data-cut-judge-issue={i}>
              <div className="rr-judge__issue-head">
                <span className={`rr-judge__sev rr-judge__sev--${iss.severity ?? 'minor'}`}>{iss.severity ?? 'minor'}</span>
                {iss.kind && <span className="rr-judge__kind">{iss.kind}</span>}
                {typeof iss.at_ms === 'number' && (
                  <button className="rr-seek" data-cut-seek={iss.at_ms} onClick={() => onSeek(iss.at_ms!)}>
                    @ {fmtTc(iss.at_ms)}
                  </button>
                )}
              </div>
              {iss.evidence && <div className="rr-judge__evidence">{iss.evidence}</div>}
              {iss.suggested_fix && <div className="rr-judge__fix">fix: {iss.suggested_fix}</div>}
            </div>
          ))}
          {issues.length === 0 && <div className="rr-judge__noissues">no issues raised</div>}
        </div>
      )}
    </div>
  )
}
