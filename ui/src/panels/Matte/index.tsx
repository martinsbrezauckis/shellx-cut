// panels/Matte — the AI background-removal drawer for edit.matte.
// Role: a right-side drawer (the Grade/Title drawer family) that drives ONE verb
// — edit.matte — to cut a subject out of the SELECTED media clip WITHOUT a green
// screen, plus the SEAMLESS requirements path (system.setup_matte) when the local
// matte runtime is absent.
//
// UX LOCKED: an in-app sidebar/tab — NEVER a second window. When the
// runtime is missing the drawer shows a REQUIREMENTS CARD (install Background
// Removal ~14 MB, or the Premium MatAnyone2 tier with a non-commercial consent),
// gated off the doctor `matte` / `matte_premium` cards. When ready it shows the
// manual controls (model · mode · quality · replace-bg · premium subject seed) —
// usable WITHOUT an agent. The preview composite already works through the render
// pipeline, so applying a matte flips the Preview to COMPOSED and the cutout
// shows at the playhead.
//
// TRUST STORY: no live preview in the drawer. Fires the verb; the Preview poster
// shows the actually-matted frame. Receipt = the verb result. Relay-drivable:
// ui.open{panel:"matte"} opens it; the drawer is visible in ui.state.
//
// Callers: App.tsx (mounted when open, with the selected clip id + playhead).
// Deps: lib/client (verbs), ../drawer.css.

import { useCallback, useEffect, useRef, useState } from 'react'
import { callVerb, type Project } from '../../lib/client'
import { useBlockingOverlay } from '../../components/overlay/useBlockingOverlay'
import { matteSubjectSeed, matteResultFromReceipt, type MatteResult } from './contract'
import { useMatteDoctor } from './useMatteDoctor'
import { premiumMatteAvailability } from '../../lib/doctor'
import { PremiumTierPanel } from './PremiumTierPanel'
import '../drawer.css'

export interface MatteDrawerProps {
  project: Project | null
  /** The clip to matte (App passes selectedClipIds[0]). */
  clipId: string | null
  /** Current playhead (ms) — the default frame for the premium subject seed. */
  playheadMs: number
  onClose: () => void
}

/** Is `clipId` a media clip in the project? (matte applies to media only.) */
function isMediaClip(project: Project | null, clipId: string | null): boolean {
  if (!project || !clipId) return false
  for (const t of project.tracks) {
    for (const c of t.clips) {
      if ('asset' in c && c.id === clipId) return true
    }
  }
  return false
}

export default function MatteDrawer({ project, clipId, playheadMs, onClose }: MatteDrawerProps) {
  const overlay = useBlockingOverlay<HTMLElement>(onClose)
  const projectOrigin = project?.project_identity?.origin_path_sha256 ?? null
  const { probeState, premiumReady, premiumCard, docHint, probe, currentScope } = useMatteDoctor(projectOrigin)
  const premiumAvailability = premiumMatteAvailability(premiumCard)
  // The controls-view Premium tab reveals the current Doctor state. Confirmed
  // missing Premium offers consent; hardware and probe problems offer guidance.
  const [showPremiumConsent, setShowPremiumConsent] = useState(false)

  // The runtime is usable when either tier is installed. Premium is a complete
  // matte path, not merely an add-on to the base RVM card.
  const ready = probeState === 'ready' || (probeState === 'absent' && premiumReady)

  // Controls.
  const [model, setModel] = useState<'rvm' | 'matanyone'>('rvm')
  const [mode, setMode] = useState<'remove' | 'replace'>('remove')
  const [quality, setQuality] = useState<'fast' | 'good'>('good')
  const [bgColor, setBgColor] = useState('#00B140') // a green for replace
  const [usePick, setUsePick] = useState(false) // premium: SAM2 subject seed
  const [pickX, setPickX] = useState(0.5)
  const [pickY, setPickY] = useState(0.5)

  const [busy, setBusy] = useState(false)
  const [installing, setInstalling] = useState<string | null>(null)
  const [result, setResult] = useState<MatteResult | null>(null)
  const [err, setErr] = useState<string | null>(null)
  const installScopeEpoch = useRef(0)
  useEffect(() => {
    installScopeEpoch.current += 1
    setInstalling(null)
    setErr(null)
    return () => { installScopeEpoch.current += 1 }
  }, [projectOrigin])

  const hasClip = isMediaClip(project, clipId)

  useEffect(() => {
    if (premiumReady && probeState !== 'ready') setModel('matanyone')
    else if (!premiumReady && probeState === 'ready' && model === 'matanyone') {
      setModel('rvm')
      setShowPremiumConsent(true)
    }
  }, [premiumReady, probeState, model])

  // A new Doctor response can make a previously selected tier unavailable before
  // the state-sync effect runs. Never submit that stale Premium choice.
  const effectiveModel = model === 'matanyone' && !premiumReady ? 'rvm' : model

  const pollSetupJob = useCallback(async (jobId: string, current: () => boolean) => {
    for (let i = 0; i < 720; i += 1) {
      if (!current()) return false
      const r = await callVerb('jobs.status', { job_id: jobId })
      if (!current()) return false
      if (!r.ok) {
        const msg = r.error?.message ?? 'could not read setup job status'
        setErr(`setup failed: ${msg}`)
        return false
      }
      const job = r.result as { state?: string; error?: { message?: string }; message?: string } | undefined
      if (job?.state === 'done') return true
      if (job?.state === 'failed') {
        const msg = job.error?.message ?? job.message ?? 'installer job failed'
        setErr(`setup failed: ${msg}`)
        return false
      }
      await new Promise((resolve) => window.setTimeout(resolve, 1000))
    }
    if (current()) setErr('setup failed: timed out waiting for installer job')
    return false
  }, [])

  // Esc closes (drawer family convention).

  // Install the base (RVM) or premium (MatAnyone2) runtime via system.setup_matte.
  const install = async (tier: 'rvm' | 'matanyone') => {
    const installOrigin = projectOrigin
    const installEpoch = installScopeEpoch.current
    const current = () => installScopeEpoch.current === installEpoch && currentScope(installOrigin)
    setInstalling(tier)
    setErr(null)
    try {
      const args = tier === 'matanyone'
        ? { model: 'matanyone' as const, accept_noncommercial: true }
        : { model: 'rvm' as const }
      const r = await callVerb('system.setup_matte', args)
      if (!current()) return
      if (!r.ok) {
        setErr(`${r.error?.code ?? 'failed'}: ${r.error?.message ?? 'setup failed'}`)
        return
      }
      const jobId = (r.result as { job_id?: string } | undefined)?.job_id
      if (jobId) {
        const ok = await pollSetupJob(jobId, current)
        if (!ok) return
      }
      // Re-probe so the card flips ready when the model lands.
      if (current()) await probe(true)
    } catch {
      if (current()) setErr('server unreachable')
    } finally {
      if (current()) setInstalling(null)
    }
  }

  // Fire edit.matte for the selected clip.
  const apply = async (enabled: boolean) => {
    if (!hasClip || !clipId) return
    setBusy(true)
    setErr(null)
    setResult(null)
    try {
      const args: Record<string, unknown> = { clip: clipId, model: effectiveModel, quality, enabled }
      if (enabled) {
        args.mode = mode
        if (mode === 'replace') args.bg = { type: 'color', color: bgColor }
        if (effectiveModel === 'matanyone' && usePick) {
          args.seed = matteSubjectSeed(project, clipId, playheadMs, pickX, pickY)
        }
      }
      const r = await callVerb('edit.matte', args as never)
      if (r.ok) {
        setResult(matteResultFromReceipt(r.result, clipId, enabled))
        document.dispatchEvent(new CustomEvent('cut:show-composed'))
      } else {
        setErr(`${r.error?.code ?? 'failed'}: ${r.error?.message ?? 'edit.matte failed'}`)
      }
    } catch (error) {
      setErr(error instanceof Error ? error.message : 'Background removal could not be applied.')
    } finally {
      setBusy(false)
    }
  }

  return (
    <div className="cd-scrim" data-cut-matte-scrim onMouseDown={overlay.onScrimMouseDown}>
      <aside
        ref={overlay.dialogRef}
        className="cd-drawer"
        data-cut-matte
        data-cut-matte-open="true"
        data-cut-matte-ready={ready ? 'true' : 'false'}
        role="dialog"
        aria-modal="true"
        aria-label="Background removal"
        data-cut-blocking-overlay
        tabIndex={-1}
        onKeyDown={overlay.onDialogKeyDown}
        onMouseDown={(e) => e.stopPropagation()}
      >
        <header className="cd-head">
          <div>
            <h2 className="cd-title">Background removal</h2>
            <p className="cd-sub">Cut the subject out of the selected clip without a green screen.</p>
          </div>
          <button className="cd-btn cd-btn--ghost" data-cut-matte-close onClick={onClose}>Close</button>
        </header>

        <div className="cd-body">
          {probeState === 'probing' ? (
            <div className="cd-empty" data-cut-matte-probing>Checking the background-removal runtime…</div>
          ) : probeState === 'error' ? (
            /* PROBE FAILED — we could NOT determine install state (doctor !ok, threw,
               or no matte card). Do NOT claim "isn't set up"; offer a Re-check. The
               runtime may well be installed and the read merely failed transiently
               (e.g. cutd still starting or the perception env not inherited). */
            <div className="cd-result" data-cut-matte-probe-error>
              <div className="cd-result-head">Couldn’t check background removal</div>
              <p className="cd-note">
                The runtime check didn’t complete — cutd may still be starting, or its
                perception environment wasn’t available. This does <strong>not</strong> mean
                background removal isn’t installed.
              </p>
              <button
                className="cd-btn cd-btn--primary"
                data-cut-matte-recheck
                onClick={() => { void probe(true) }}
              >
                Re-check
              </button>
              {err && <div className="cd-err" data-cut-matte-error role="alert">{err}</div>}
            </div>
          ) : !ready && probeState === 'absent' ? (
            /* REQUIREMENTS CARD — neither tier is ready. Each tier keeps its own Doctor status. */
            <div className="cd-result" data-cut-matte-requirements>
              <div className="cd-result-head">Background removal isn’t set up yet</div>
              <p className="cd-note">{docHint ?? 'AI background removal needs a small on-device model.'}</p>
              <button
                className="cd-btn cd-btn--primary"
                data-cut-matte-install-rvm
                disabled={installing !== null}
                onClick={() => void install('rvm')}
              >
                {installing === 'rvm' ? 'Installing…' : 'Install Background Removal (~14 MB)'}
              </button>
              <div style={{ height: 10 }} />
              <PremiumTierPanel availability={premiumAvailability} hint={premiumCard?.hint ?? null}
                installing={installing} error={err} context="requirements"
                onInstall={() => void install('matanyone')} onRecheck={() => void probe(true)} />
              <div style={{ height: 10 }} />
              <button className="cd-btn cd-btn--ghost" data-cut-matte-recheck onClick={() => void probe(true)}>
                Re-check
              </button>
              {err && <div className="cd-err" data-cut-matte-error role="alert">{err}</div>}
            </div>
          ) : !hasClip ? (
            <div className="cd-empty" data-cut-matte-noclip>
              Select a <strong>media clip</strong> on the timeline to remove its background.
            </div>
          ) : (
            /* CONTROLS — the runtime is ready and a media clip is selected. */
            <>
              {/* model */}
              <div className="cd-field">
                <span className="cd-field-label">Model</span>
                <div className="cd-seg" role="tablist" data-cut-matte-model>
                  <button
                    role="tab" aria-selected={effectiveModel === 'rvm'}
                    className={`cd-seg-btn ${effectiveModel === 'rvm' ? 'cd-seg-btn--on' : ''}`}
                    data-cut-matte-model-rvm onClick={() => setModel('rvm')}
                  >Standard (RVM)</button>
                  <button
                    role="tab" aria-selected={effectiveModel === 'matanyone'}
                    className={`cd-seg-btn ${effectiveModel === 'matanyone' ? 'cd-seg-btn--on' : ''}`}
                    data-cut-matte-model-premium
                    // Keep unavailable Premium clickable for status and Re-check.
                    disabled={installing !== null}
                    title={premiumReady ? 'MatAnyone2 premium' : 'Check Premium availability'}
                    onClick={() => { if (premiumReady) setModel('matanyone'); else setShowPremiumConsent(true) }}
                  >Premium{premiumReady ? '' : premiumAvailability === 'missing' ? ' (install)' : premiumAvailability === 'hardware-unavailable' ? ' (no GPU)' : ' (check)'}</button>
                </div>
              </div>

              {/* The same status panel shows consent only for confirmed missing Premium. */}
              {!premiumReady && (showPremiumConsent || model === 'matanyone') && (
                <PremiumTierPanel availability={premiumAvailability} hint={premiumCard?.hint ?? null}
                  installing={installing} error={err} context="controls"
                  onInstall={() => void install('matanyone')} onRecheck={() => void probe(true)} />
              )}

              {/* mode */}
              <div className="cd-field">
                <span className="cd-field-label">Mode</span>
                <div className="cd-seg" role="tablist" data-cut-matte-mode>
                  <button
                    role="tab" aria-selected={mode === 'remove'}
                    className={`cd-seg-btn ${mode === 'remove' ? 'cd-seg-btn--on' : ''}`}
                    data-cut-matte-mode-remove onClick={() => setMode('remove')}
                  >Remove (reveal track below)</button>
                  <button
                    role="tab" aria-selected={mode === 'replace'}
                    className={`cd-seg-btn ${mode === 'replace' ? 'cd-seg-btn--on' : ''}`}
                    data-cut-matte-mode-replace onClick={() => setMode('replace')}
                  >Replace background</button>
                </div>
              </div>

              {mode === 'replace' && (
                <label className="cd-field cd-field--inline">
                  <span className="cd-field-label">Background color</span>
                  <input
                    className="cd-input cd-input--mono" data-cut-matte-bg type="text"
                    spellCheck={false} placeholder="#00B140" value={bgColor}
                    onChange={(e) => setBgColor(e.target.value)} style={{ maxWidth: 120 }}
                  />
                </label>
              )}

              {/* quality */}
              <div className="cd-field">
                <span className="cd-field-label">Quality</span>
                <div className="cd-seg" role="tablist" data-cut-matte-quality>
                  <button
                    role="tab" aria-selected={quality === 'fast'}
                    className={`cd-seg-btn ${quality === 'fast' ? 'cd-seg-btn--on' : ''}`}
                    data-cut-matte-quality-fast onClick={() => setQuality('fast')}
                  >Fast</button>
                  <button
                    role="tab" aria-selected={quality === 'good'}
                    className={`cd-seg-btn ${quality === 'good' ? 'cd-seg-btn--on' : ''}`}
                    data-cut-matte-quality-good onClick={() => setQuality('good')}
                  >Good</button>
                </div>
              </div>

              {/* premium subject seed (SAM2 click-to-pick, manual point) */}
              {effectiveModel === 'matanyone' && (
                <div className="cd-field">
                  <label className="cd-check">
                    <input
                      type="checkbox" data-cut-matte-pick checked={usePick}
                      onChange={(e) => setUsePick(e.target.checked)}
                    />
                    <span>Pick the subject (SAM2) at a point</span>
                  </label>
                  {usePick && (
                    <div className="cd-row" data-cut-matte-pick-xy>
                      <label className="cd-field"><span className="cd-field-label">x (0–1)</span>
                        <input className="cd-input cd-input--mono" data-cut-matte-pick-x type="number" min={0} max={1} step={0.01}
                          value={pickX} onChange={(e) => setPickX(Math.min(1, Math.max(0, Number(e.target.value) || 0)))} />
                      </label>
                      <label className="cd-field"><span className="cd-field-label">y (0–1)</span>
                        <input className="cd-input cd-input--mono" data-cut-matte-pick-y type="number" min={0} max={1} step={0.01}
                          value={pickY} onChange={(e) => setPickY(Math.min(1, Math.max(0, Number(e.target.value) || 0)))} />
                      </label>
                    </div>
                  )}
                  <p className="cd-note">The point picks WHICH subject to keep (at the current playhead).</p>
                </div>
              )}

              <button
                className="cd-btn cd-btn--primary" data-cut-matte-apply
                disabled={busy} onClick={() => void apply(true)}
              >{busy ? 'Baking…' : 'Apply background removal'}</button>
              <div style={{ height: 8 }} />
              <button
                className="cd-btn cd-btn--ghost" data-cut-matte-remove
                disabled={busy} onClick={() => void apply(false)}
              >Clear matte (restore the clip)</button>

              {err && <div className="cd-err" data-cut-matte-error role="alert">{err}</div>}
              {result && (
                <div className="cd-result" data-cut-matte-result>
                  <div className="cd-result-head" data-cut-matte-result-state>
                    {result.enabled === false ? 'matte cleared' : 'matte applied'} · {result.clip}
                  </div>
                  <div className="cd-result-foot">Scrub to the clip — the cutout shows in the composed preview.</div>
                </div>
              )}
            </>
          )}
        </div>
      </aside>
    </div>
  )
}
