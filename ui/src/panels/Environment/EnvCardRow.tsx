import { useState } from 'react'
import { callVerb } from '../../lib/client'
import {
  cardLabel,
  ffmpegDownloadOffer,
  ffmpegSetupGuide,
  ffmpegVersionAdvice,
  fetchTool,
  hasFetchAction,
  hasMatteSetupAction,
  hasSetupAction,
  needsManualFfmpegSetup,
  premiumMatteAvailability,
  environmentCardStatus,
  setupMatte,
  setupPerception,
  unavailableFeatures,
  type DoctorCard,
} from '../../lib/doctor'
import { Availability, availabilitySentence, useAvailability } from '../../lib/availability'
import { pickFfmpeg } from '../../lib/tauri'
import { encoderSizeRows, softwareExportHint } from './gpuHint'
import FfmpegSetupGuide, { FfmpegDownloadConsent } from './FfmpegSetupGuide'
import NeededFor from './NeededFor'
import type { RequirementTool } from '../../lib/doctor'
import { ServiceRuntimeActions, ServiceRuntimeDetail } from './ServiceRuntime'
import { SttModelControl } from './SttModelControl'
import { useEnvironmentSetupJob } from './useEnvironmentSetupJob'

/** Status -> display token (label + color class). Fixed family semantics. */
function statusChip(card: DoctorCard): { label: string; cls: string } {
  const status = environmentCardStatus(card)
  if (ffmpegVersionAdvice(card)) {
    return { label: 'Works, older than tested', cls: `env-st--${status}` }
  }
  if (card.id === 'gpu-encode' && card.details?.ffmpeg_missing === true) {
    return { label: 'Needs setup', cls: 'env-st--missing' }
  }
  if (card.id === 'gpu-encode') {
    return status === 'ok' && card.details?.hardware_available === true
      ? { label: 'Ready', cls: 'env-st--ok' }
      : card.details?.hardware_available === true
        ? { label: 'Needs attention', cls: 'env-st--degraded' }
        : { label: 'Ready', cls: 'env-st--ok' }
  }
  if (status === 'unknown') {
    return card.kind === 'service'
      ? { label: 'Optional', cls: 'env-st--unknown' }
      : { label: 'Check again', cls: 'env-st--unknown' }
  }
  switch (status) {
    case 'ok':
      return { label: 'Ready', cls: 'env-st--ok' }
    case 'degraded':
      return { label: 'Needs attention', cls: 'env-st--degraded' }
    case 'missing':
    default:
      return card.kind === 'service'
        ? { label: 'Optional', cls: 'env-st--unknown' }
        : { label: 'Needs setup', cls: 'env-st--missing' }
  }
}

function compactFact(card: DoctorCard): string | null {
  if (card.id === 'gpu-encode' && card.details?.ffmpeg_missing === true) return null
  if (card.id === 'gpu-encode') {
    return card.details?.hardware_available === true ? 'GPU acceleration' : 'Software export'
  }
  if (card.kind === 'disk' && card.details?.free_human != null) {
    return `${String(card.details.free_human)} free`
  }
  return null
}

function compactHint(card: DoctorCard): string | null {
  const status = environmentCardStatus(card)
  const advice = ffmpegVersionAdvice(card)
  if (advice) {
    const missing = unavailableFeatures(card)
    return `${advice.message} ${advice.upgrade}${missing.length > 0 ? ` Not available with this build: ${missing.join(', ')}.` : ''}`
  }
  if (card.id === 'matte_premium') {
    const availability = premiumMatteAvailability(card)
    if (availability === 'ready') return null
    if (availability === 'missing') return 'Install Premium only for cleaner edges and subject picking on supported NVIDIA machines.'
    if (availability === 'hardware-unavailable') return `${card.hint || 'Premium is installed, but no NVIDIA CUDA GPU is available.'} Use Standard (RVM) or a supported NVIDIA machine.`
    if (availability === 'unverified') return card.hint || 'Premium hardware could not be verified. Re-scan before using this tier.'
    return card.hint || 'Premium could not be prepared. Review the diagnostic hint and re-scan.'
  }
  if (card.kind === 'judge') {
    if (card.details?.judge_ready === false) {
      const reason = typeof card.details?.availability_reason === 'string'
        ? card.details.availability_reason
        : 'This agent cannot review exports right now.'
      return `${reason} Re-scan to check again. Agent Chat is checked separately.`
    }
    if (card.details?.judge_ready !== true && status !== 'missing') {
      return 'Could not check whether this agent can review exports. Re-scan to try again; Agent Chat is checked separately.'
    }
  }
  if (card.id === 'gpu-encode' && card.details?.ffmpeg_missing === true) {
    return card.hint || 'Exports need video processing first.'
  }
  if (card.id === 'gpu-encode') {
    if (status === 'ok' && card.details?.hardware_available === true) return null
    if (card.details?.hardware_available === true) return 'Re-scan or choose a working video tool if faster exports are not available.'
    return softwareExportHint(card.details)
  }
  if (status === 'ok' || status === 'unknown') return null
  if (card.id === 'ffmpeg' && status === 'degraded' && unavailableFeatures(card).length > 0) {
    return `Works for editing and export. Not available with this build: ${unavailableFeatures(card).join(', ')}.`
  }
  if (card.id === 'ffmpeg' && needsManualFfmpegSetup(card)) {
    return ffmpegDownloadOffer(card)
      ? 'Needed to import, preview, and export. Download it, or use another way below.'
      : 'Needed to import, preview, and export. Follow these steps, then click Check again.'
  }
  if (card.id === 'ffmpeg') return 'Install video processing so imports, previews, and exports work.'
  if (card.id === 'ffprobe') return 'Comes with video processing. Set that up and this follows.'
  if (card.id === 'perception') return 'Install captions when you need transcripts, word edits, silence cleanup, or search.'
  if (card.id === 'vc-runtime') return 'Captions on Windows need it first. Install gets it from Microsoft; Windows asks for administrator permission.'
  if (card.id === 'matte') return 'Install the standard cutout model for on-device background removal.'
  if (status === 'degraded') return 'Re-scan or set this up again if the feature does not work.'
  return 'Set this up only if you need this feature.'
}

function advancedRows(card: DoctorCard): Array<[string, string]> {
  const rows: Array<[string, string]> = []
  if (card.version) rows.push(['Version', card.version])
  if (card.source && card.source !== 'missing') rows.push(['Source', card.source])
  const tier = card.kind === 'perception' ? String(card.details?.tier ?? '') : ''
  if (tier) rows.push(['Tier', tier])
  if (card.kind === 'service') {
    if (card.details?.model != null) rows.push(['Model', String(card.details.model)])
    rows.push(['Runner', card.details?.runner_available ? 'available' : 'not found'])
    if (card.details?.endpoint != null) rows.push(['Endpoint', String(card.details.endpoint)])
    rows.push(['Secret', card.details?.secret_set ? 'set' : 'none'])
  }
  if (card.id === 'perception') {
    if (card.details?.python != null) rows.push(['Runtime', String(card.details.python)])
    if (card.details?.stt_model != null) rows.push(['Model', String(card.details.stt_model)])
  }
  // Why this build is the one in use (details.selection, set by the engine).
  const why = card.id === 'ffmpeg' ? (card.details?.selection as { text?: unknown } | undefined)?.text : undefined
  if (typeof why === 'string') rows.push(['Why this ffmpeg', why])
  if (card.id === 'vc-runtime') {
    if (Array.isArray(card.details?.needed_by)) rows.push(['Needed by', card.details.needed_by.join(', ')])
    if (typeof card.details?.official_page === 'string') rows.push(['From Microsoft', card.details.official_page])
    rows.push(['Uninstalling ShellX Cut', 'keeps it (it belongs to Windows)'])
  }
  if (card.id === 'gpu-encode') {
    if (card.details?.override_setting != null) rows.push(['Selected ffmpeg', String(card.details.override_setting)])
    if (card.details?.resolved != null) rows.push(['Resolved ffmpeg', String(card.details.resolved)])
    if (typeof card.details?.hardware_detail === 'string') rows.push(['ffmpeg said', card.details.hardware_detail])
    rows.push(...encoderSizeRows(card.details))
  }
  if (card.hint) rows.push(['Diagnostic hint', card.hint])
  return rows
}

function AdvancedDetails({ card, rows }: { card: DoctorCard; rows: Array<[string, string]> }) {
  if (rows.length === 0) return null
  return (
    <details className="env-advanced" data-cut-env-advanced={card.id}>
      <summary className="env-advanced-summary" data-cut-env-advanced-toggle={card.id}>Advanced details</summary>
      <dl className="env-advanced-list">
        {rows.map(([k, v]) => (
          <div className="env-advanced-row" key={`${k}:${v}`}>
            <dt>{k}</dt>
            <dd title={v}>{v}</dd>
          </div>
        ))}
      </dl>
    </details>
  )
}

export default function EnvCardRow({
  card,
  os,
  arch,
  onChanged,
  requirements = [],
}: {
  card: DoctorCard
  os: string
  arch: string
  onChanged: () => void
  /** Registry entries this card reports, for its "Needed for" line. */
  requirements?: RequirementTool[]
}) {
  const { title, role } = cardLabel(card)
  const presentationStatus = environmentCardStatus(card)
  const chip = statusChip(card)
  const gpuSoftwareOnly = card.id === 'gpu-encode' && card.details?.hardware_available !== true
  const { job, runJob } = useEnvironmentSetupJob(onChanged)
  const overrideSetting =
    card.id === 'gpu-encode' ? ((card.details?.override_setting as string | null) ?? null) : null
  const [ffBusy, setFfBusy] = useState(false)
  const [ffNote, setFfNote] = useState<string | null>(null)
  const [serviceSetupOpen, setServiceSetupOpen] = useState(false)
  const offer = hasFetchAction(card, os, arch) ? ffmpegDownloadOffer(card) : null
  const fact = compactFact(card)
  const hint = compactHint(card)
  const details = advancedRows(card)

  const setFfmpeg = async (path: string | null) => {
    setFfBusy(true)
    setFfNote(null)
    const r = await callVerb('system.set_ffmpeg', { path })
    setFfBusy(false)
    if (!r.ok) {
      setFfNote(r.error?.message ?? 'Could not change the video tool.')
      return
    }
    if (path && (r.result as { restart_required?: boolean })?.restart_required) {
      setFfNote('Set — restart ShellX Cut to apply.')
    }
    onChanged()
  }

  const chooser = useAvailability('shell.desktop')
  const onChangeFfmpeg = async () => {
    if (chooser.state === 'absent') { setFfNote(availabilitySentence('shell.desktop')); return }
    const p = await pickFfmpeg()
    if (p) await setFfmpeg(p)
  }

  const onDownload = () =>
    void runJob(() => fetchTool(card.id === 'vc-runtime' ? 'vc-runtime' : 'ffmpeg'), 'could not start the download')
  const onSetupPerception = () =>
    void runJob(() => setupPerception(), 'could not start captions install')
  const onSetupMatte = () =>
    void runJob(
      () => setupMatte(card.id === 'matte_premium' ? 'matanyone' : 'rvm'),
      'could not start background-removal setup',
    )
  const openServiceSetup = (_id: string) => setServiceSetupOpen(true)

  return (
    <div
      className={`env-row env-row--${gpuSoftwareOnly ? 'unknown' : presentationStatus}`}
      data-cut-env-card={card.id}
      data-cut-env-status={presentationStatus}
    >
      <span className={`env-st ${chip.cls}`} data-cut-env-statuschip={presentationStatus}>
        {chip.label}
      </span>

      <div className="env-row-id">
        <span className="env-row-title" data-cut-env-title={card.id}>{title}</span>
        {role && <span className="env-row-role">{role}</span>}
      </div>

      <div className="env-row-facts">
        {fact && <span className="env-fact env-fact--summary" data-cut-env-fact>{fact}</span>}
      </div>

      <div className="env-row-action">
        <ServiceRuntimeActions
          card={card}
          busy={job.busy}
          onOpenSetup={openServiceSetup}
        />
        {!job.busy && hasFetchAction(card, os, arch) && (
          <button
            className="env-btn env-btn--primary env-btn--sm"
            data-cut-env-download={card.id}
            onClick={onDownload}
            title={
              card.id === 'vc-runtime'
                ? "Download Microsoft's installer from Microsoft and run it (Windows asks for administrator permission)"
                : offer ? `Download FFmpeg from ${offer.publisher}` : 'Download verified video-processing tools'
            }
          >
            {card.id === 'vc-runtime' ? 'Install' : 'Download'}
          </button>
        )}
        {!job.busy && hasSetupAction(card) && (
          <button
            className="env-btn env-btn--primary env-btn--sm"
            data-cut-env-setup-perception={card.id}
            onClick={onSetupPerception}
              title="Install captions and transcription tools (takes a few minutes)"
            >
            Install captions
          </button>
        )}
        {!job.busy && (hasMatteSetupAction(card) || (card.id === 'matte_premium' && card.details?.setup_supported === false)) && (
          <button
            className="env-btn env-btn--primary env-btn--sm"
            data-cut-env-setup-matte={card.id}
            disabled={card.details?.setup_supported === false} onClick={onSetupMatte}
            title={card.details?.setup_supported === false ? card.hint :
              card.id === 'matte_premium'
                ? 'Install premium background removal (NVIDIA GPU; models: 141 MB + 323 MB + 323 MB, plus several GB for tools; non-commercial license)'
                : 'Install standard background removal (~14 MB, runs on your device)'
            }
          >
            {card.id === 'matte_premium' ? 'Install premium' : 'Install (~14 MB)'}
          </button>
        )}
        {!job.busy && card.kind !== 'service' && presentationStatus === 'unknown' && (
          <button
            className="env-btn env-btn--sm"
            data-cut-env-rescan={card.id}
            onClick={onChanged}
            title={
              card.kind === 'service'
                ? 'Re-check whether this optional service is reachable (start it, or set its endpoint, then re-scan)'
                : "Re-check this dependency — the last check timed out, so it couldn't be verified"
            }
          >
            Re-scan
          </button>
        )}
      </div>

      <ServiceRuntimeDetail
        card={card}
        open={serviceSetupOpen}
        onOpenChange={setServiceSetupOpen}
        onChanged={onChanged}
      />

      <NeededFor cardId={card.id} tools={requirements} />

      {hint || job.busy || job.err || details.length > 0 ? (
        <div className="env-row-detail">
          {hint && !job.busy && !job.err && (
            <div className="env-card-hint" data-cut-env-hint>{hint}</div>
          )}
          {job.busy && (
            <div className="env-card-progress" data-cut-env-progress={job.pct ?? 0}>
              <div className="env-progress-track">
                <div className="env-progress-fill" style={{ width: `${job.pct ?? 0}%` }} />
              </div>
              <span className="env-progress-label">
                {job.msg ? `${job.msg} · ${job.pct ?? 0}%` : `${job.pct ?? 0}%`}
              </span>
            </div>
          )}
          {job.err && <div className="env-card-err" data-cut-env-error>{job.err}</div>}
          <AdvancedDetails card={card} rows={details} />
        </div>
      ) : null}

      {offer && !job.busy && <FfmpegDownloadConsent offer={offer} />}
      {card.id === 'vc-runtime' && hasFetchAction(card, os, arch) && !job.busy && typeof card.details?.about === 'string' && (
        <div className="env-row-detail env-ff" data-cut-env-vc-runtime-about>
          <div className="env-ff-note">{card.details.about}</div>
        </div>
      )}

      {needsManualFfmpegSetup(card) && (() => {
        const guide = ffmpegSetupGuide(card)
        return guide ? <FfmpegSetupGuide guide={guide} onChanged={onChanged} otherWays={!!offer} /> : null
      })()}

      <SttModelControl card={card} onChanged={onChanged} />

      {card.id === 'gpu-encode' && (
        <div className="env-row-detail env-ff" data-cut-env-ffmpeg-control>
          {typeof card.details?.enable_help === 'string' && (
            <details className="env-disclosure" data-cut-env-gpu-help>
              <summary className="env-disclosure-summary" data-cut-env-gpu-help-toggle>How to enable faster exports</summary>
              <p className="env-disclosure-body">{String(card.details.enable_help)}</p>
            </details>
          )}
          <div className="env-ff-row">
            {overrideSetting ? (
              <>
                <span className="env-ff-label">Custom video tool selected</span>
                <button
                  className="env-btn env-btn--sm"
                  data-cut-env-ffmpeg-auto
                  onClick={() => void setFfmpeg(null)}
                  disabled={ffBusy}
                >
                  Use automatic
                </button>
              </>
            ) : (
              <>
                <span className="env-ff-label">Video tool: automatic</span>
                <Availability rule="shell.desktop">
                  <button className="env-btn env-btn--sm" data-cut-action="settings-video-tool-change" data-cut-env-ffmpeg-change
                    onClick={() => void onChangeFfmpeg()} disabled={ffBusy}
                    title="Choose a different video-processing tool">Choose tool…</button>
                </Availability>
              </>
            )}
          </div>
          {ffNote && (
            <div className="env-ff-note" data-cut-env-ffmpeg-note>{ffNote}</div>
          )}
        </div>
      )}

      {card.id === 'matte_premium' &&
        (card.status === 'missing' || card.status === 'degraded') && (
        <div className="env-row-detail env-ff" data-cut-env-matte-consent>
          <div className="env-ff-note">
            Cleaner edges + temporal stability + click-to-pick which subject. NVIDIA GPU. Models: 141 MB + 323 MB + 323 MB (about 790 MB), plus several GB for tools —
            <strong> non-commercial license (NTU S-Lab 1.0)</strong>; installing accepts it.
          </div>
        </div>
      )}
    </div>
  )
}
