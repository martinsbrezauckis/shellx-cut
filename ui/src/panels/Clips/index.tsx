// panels/Clips — the social-repurposing drawer for clip.candidates and
// render.bundle.
//
// Role: a right-side drawer (the MusicBed/Grade drawer family) that turns ONE
// edit into shareable short-form. On open it calls clip.candidates and shows
// the ranked windows as cards (thumbnail + excerpt + score + the honest "why");
// one click on a card fires render.bundle for that window across the selected
// platforms, polls the job, and lists the per-platform downloadable files.
//
// RECEIPT PHILOSOPHY: the user sees a CLEAN result — a tidy list of
// "9:16 ✓ · 1:1 ✓" files with open links. The per-platform receipt verdict is a
// single pass/fail badge; the full receipt stays in the Inspect rail (not dumped
// here). Honest labelling: the candidate ranking says scoring:"heuristic".
//
// Callers: App.tsx (mounted when activeDrawer === 'clips'). Deps: lib/client,
// ../drawer.css, ./clips.css.

import { useCallback, useEffect, useRef, useState } from 'react'
import {
  callVerb,
  exportUrl,
  type ClipCandidate,
  type ClipCandidatesResult,
  type BundleResult,
  type BundlePlatform,
} from '../../lib/client'
import { Icon } from '../../icons'
import { useBlockingOverlay } from '../../components/overlay/useBlockingOverlay'
import '../drawer.css'
import './clips.css'

export interface ClipsDrawerProps {
  onClose: () => void
}

/** The platforms a bundle can target (default all on). */
const PLATFORMS = ['9:16', '1:1', '16:9'] as const
const PLATFORM_LABELS: Record<(typeof PLATFORMS)[number], string> = {
  '9:16': 'Vertical',
  '1:1': 'Square',
  '16:9': 'Widescreen',
}

function fmtDur(ms: number): string {
  const s = Math.round(ms / 1000)
  return s >= 60 ? `${Math.floor(s / 60)}m${String(s % 60).padStart(2, '0')}s` : `${s}s`
}

/** A score badge 0..1 → a 0..100 chip with a hot/warm/cool class. */
function scoreClass(score: number): string {
  if (score >= 0.6) return 'clip-score--hot'
  if (score >= 0.4) return 'clip-score--warm'
  return 'clip-score--cool'
}

export default function ClipsDrawer({ onClose }: ClipsDrawerProps) {
  const overlay = useBlockingOverlay<HTMLElement>(onClose)
  const [loading, setLoading] = useState(true)
  const [result, setResult] = useState<ClipCandidatesResult | null>(null)
  const [err, setErr] = useState<string | null>(null)
  // Per-candidate bundle state, keyed by candidate at_ms (stable within a run).
  const [bundling, setBundling] = useState<number | null>(null)
  const [bundleOf, setBundleOf] = useState<Record<number, BundleResult>>({})
  const [platforms, setPlatforms] = useState<Set<string>>(new Set(PLATFORMS))
  const pollTimer = useRef<number | null>(null)

  // Esc closes (drawer family convention).
  // Clear the bundle poll ONLY on unmount (not tied to onClose, whose ref
  // changes every App render — folding it above killed the poll on re-render).
  useEffect(() => () => { if (pollTimer.current) window.clearTimeout(pollTimer.current) }, [])

  // Load candidates on open.
  const load = useCallback(async () => {
    setLoading(true)
    setErr(null)
    const r = await callVerb('clip.candidates', { count: 6 })
    if (r.ok && r.result) setResult(r.result)
    else setErr(`${r.error?.code ?? 'failed'}: ${r.error?.message ?? 'clip.candidates failed'}`)
    setLoading(false)
  }, [])
  useEffect(() => {
    void load()
  }, [load])

  const togglePlatform = (p: string) => {
    if (platforms.has(p) && platforms.size === 1) {
      setErr('Choose at least one platform.')
      return
    }
    setErr(null)
    setPlatforms((prev) => {
      const next = new Set(prev)
      if (next.has(p)) next.delete(p)
      else next.add(p)
      return next
    })
  }

  // Fire render.bundle for a candidate, then poll the job to completion.
  const makeBundle = useCallback(
    async (c: ClipCandidate) => {
      if (bundling !== null) return
      const sel = PLATFORMS.filter((p) => platforms.has(p))
      if (sel.length === 0) {
        setErr('Choose at least one platform.')
        return
      }
      setBundling(c.at_ms)
      setErr(null)
      const r = await callVerb('render.bundle', {
        candidate: { at_ms: c.at_ms, dur_ms: c.dur_ms },
        platforms: sel,
        rationale: 'user: social bundle from Clips',
      })
      if (!r.ok || !r.result) {
        setErr(`${r.error?.code ?? 'failed'}: ${r.error?.message ?? 'render.bundle failed'}`)
        setBundling(null)
        return
      }
      const jobId = (r.result as { job_id: string }).job_id
      // Poll jobs.status until done/failed.
      const poll = async () => {
        const j = await callVerb('jobs.status', { job_id: jobId })
        if (j.ok && j.result) {
          const st = j.result.state
          if (st === 'done') {
            const out = j.result.result as BundleResult | undefined
            if (out) setBundleOf((prev) => ({ ...prev, [c.at_ms]: out }))
            setBundling(null)
            return
          }
          if (st === 'failed') {
            setErr(`bundle failed: ${j.result.error?.message ?? 'render error'}`)
            setBundling(null)
            return
          }
        }
        pollTimer.current = window.setTimeout(() => void poll(), 1000)
      }
      pollTimer.current = window.setTimeout(() => void poll(), 800)
    },
    [bundling, platforms],
  )

  const candidates = result?.candidates ?? []
  const selectedPlatforms = PLATFORMS.filter((p) => platforms.has(p))

  return (
    <div className="cd-scrim" data-cut-clips-scrim onMouseDown={overlay.onScrimMouseDown}>
      <aside
        ref={overlay.dialogRef}
        className="cd-drawer cd-drawer--wide"
        data-cut-clips
        data-cut-clips-open="true"
        role="dialog"
        aria-modal="true"
        aria-label="Repurpose into shorts"
        data-cut-blocking-overlay
        tabIndex={-1}
        onKeyDown={overlay.onDialogKeyDown}
        onMouseDown={(e) => e.stopPropagation()}
      >
        <header className="cd-head">
          <div>
            <h2 className="cd-title">Repurpose into shorts</h2>
            <p className="cd-sub">
              Pick a strong moment, choose the shapes you need, and create a validated set of shareable videos.
            </p>
          </div>
          <button className="cd-btn cd-btn--ghost" data-cut-clips-close onClick={onClose}>
            Close
          </button>
        </header>

        <div className="cd-body">
          {/* Platform selector — applies to every bundle. */}
          <div className="clips-platforms" data-cut-clips-platforms>
            <span className="cd-field-label">Output formats:</span>
            {PLATFORMS.map((p) => (
              <button
                key={p}
                type="button"
                className={`clips-chip ${platforms.has(p) ? 'clips-chip--on' : ''}`}
                data-cut-clips-platform={p}
                data-cut-on={platforms.has(p)}
                aria-pressed={platforms.has(p)}
                aria-label={`${PLATFORM_LABELS[p]} video, ${p}`}
                title={`${PLATFORM_LABELS[p]} video (${p})`}
                onClick={() => togglePlatform(p)}
              >
                {p} {PLATFORM_LABELS[p]}
              </button>
            ))}
          </div>

          {loading && <div className="cd-empty" data-cut-clips-loading role="status" aria-live="polite">Finding the best moments…</div>}
          {err && <div className="cd-err" data-cut-clips-error role="alert">{err}</div>}
          {!loading && !err && candidates.length === 0 && (
            <div className="cd-empty" data-cut-clips-empty>
              No candidates yet — import a video and let it transcribe, then reopen.
              {result?.note && <div className="cd-note">{result.note}</div>}
            </div>
          )}

          {candidates.length > 0 && (
            <p className="cd-note clips-ranking-note">
              Suggestions use opening strength and pacing as guides. Review each reason and choose what fits.
            </p>
          )}

          <div className="clips-list" data-cut-clips-list>
            {candidates.map((c) => {
              const done = bundleOf[c.at_ms]
              const isBundling = bundling === c.at_ms
              return (
                <div className="clip-card" data-cut-clip-card={c.at_ms} key={c.at_ms}>
                  <div className="clip-thumb-wrap" data-cut-clip-thumb-wrap>
                    <span className="clip-thumb-fallback" data-cut-clip-thumb-fallback aria-hidden="true">
                      <Icon name="image" size={20} />
                    </span>
                    <img
                      className="clip-thumb"
                      data-cut-clip-thumb
                      alt={`frame at ${c.at_ms}ms`}
                      src={`/api/frame?at_ms=${c.at_ms}&h=120`}
                      loading="lazy"
                      onError={(e) => {
                        e.currentTarget.hidden = true
                        e.currentTarget.parentElement?.setAttribute('data-cut-thumb-error', 'true')
                      }}
                    />
                  </div>
                  <div className="clip-meta">
                    <div className="clip-row1">
                      <span className={`clip-score ${scoreClass(c.score)}`} data-cut-clip-score>
                        {Math.round(c.score * 100)}
                      </span>
                      <span className="clip-dur" data-cut-clip-dur>{fmtDur(c.dur_ms)}</span>
                      <span className="clip-subscores">
                        hook {Math.round(c.hook_score * 100)} · retention {Math.round(c.retention_score * 100)}
                      </span>
                    </div>
                    <p className="clip-excerpt" data-cut-clip-excerpt>{c.transcript_excerpt}</p>
                    <p className="clip-why" data-cut-clip-why>{c.reason}</p>

                    {!done ? (
                      <button
                        className="cd-btn cd-btn--primary clip-make"
                        data-cut-clip-make={c.at_ms}
                        disabled={platforms.size === 0 || isBundling || bundling !== null}
                        aria-busy={isBundling}
                        onClick={() => void makeBundle(c)}
                      >
                        {isBundling
                          ? 'Rendering pack…'
                          : platforms.size === 0
                            ? 'Choose a format'
                            : `Make bundle → ${selectedPlatforms.join(' · ')}`}
                      </button>
                    ) : (
                      <BundleResultView result={done} />
                    )}
                  </div>
                </div>
              )
            })}
          </div>
        </div>
      </aside>
    </div>
  )
}

/** Map an absolute exports/ file path to its download URL on /api/export/*.
 *  The server fences the path to the project exports/ subtree. */
function bundleFileUrl(path: string | null | undefined): string | null {
  return path ? exportUrl(path) : null
}

/** The clean per-bundle result: one row per platform with a pass badge + the
 *  files. The full receipt lives in the Inspect rail, not here. */
function BundleResultView({ result }: { result: BundleResult }) {
  const status = result.status ?? (result.platforms.every((platform) => platform.pass === true) ? 'ready' : 'needs_review')
  const statusLabel = status === 'ready' ? 'Package ready' : status === 'blocked' ? 'Package blocked' : 'Package needs review'
  return (
    <div className="clip-bundle" data-cut-clip-bundle={result.bundle_id} data-cut-package-status={status} role="status" aria-live="polite">
      <div className={`clip-bundle-head clip-bundle-head--${status}`}>
        <Icon name={status === 'ready' ? 'check' : 'warning'} size={14} tone={status === 'ready' ? 'success' : 'warn'} />
        {statusLabel} · {result.platforms.length} {result.platforms.length === 1 ? 'format' : 'formats'}
      </div>
      {result.platforms.map((p: BundlePlatform) => (
        <div className="clip-bundle-row" data-cut-bundle-platform={p.aspect} key={p.aspect}>
          <span className="clip-bundle-aspect">{p.aspect}</span>
          <span
            className={`clip-bundle-pass ${p.pass ? 'clip-bundle-pass--ok' : 'clip-bundle-pass--fail'}`}
            data-cut-bundle-pass={String(p.pass)}
            title={p.receipt_id ? `receipt ${p.receipt_id} — open Inspect for detail` : 'unverified'}
          >
            {p.pass === null ? 'unverified' : p.pass ? <Icon name="check" size={14} tone="success" label="passed" /> : <Icon name="warning" size={14} tone="warn" label="check failed" />}
          </span>
          <span className="clip-bundle-files">
            {bundleFileUrl(p.path) && (
              <a href={bundleFileUrl(p.path)!} target="_blank" rel="noreferrer" data-cut-bundle-file="mp4">mp4</a>
            )}
            {p.caption_count > 0 && bundleFileUrl(p.caption_path) && (
              <a href={bundleFileUrl(p.caption_path)!} target="_blank" rel="noreferrer" data-cut-bundle-file="srt">srt</a>
            )}
            {p.caption_count > 0 && bundleFileUrl(p.vtt_path) && (
              <a href={bundleFileUrl(p.vtt_path)!} target="_blank" rel="noreferrer" data-cut-bundle-file="vtt">vtt</a>
            )}
          </span>
        </div>
      ))}
      <div className="clip-bundle-manifest">
        {bundleFileUrl(result.manifest_path) ? (
          <a href={bundleFileUrl(result.manifest_path)!} target="_blank" rel="noreferrer" data-cut-bundle-manifest>
            manifest.json
          </a>
        ) : <span>manifest unavailable</span>}
        {result.manifest_hash && <span title={result.manifest_hash}>hashed manifest</span>}
      </div>
      {result.issues?.length > 0 && (
        <ul className="clip-bundle-issues" data-cut-bundle-issues>
          {result.issues.map((issue, index) => (
            <li key={`${issue.code}-${issue.aspect ?? 'package'}-${index}`} data-cut-bundle-issue={issue.severity}>
              {issue.aspect ? `${issue.aspect}: ` : ''}{issue.detail}
            </li>
          ))}
        </ul>
      )}
    </div>
  )
}
