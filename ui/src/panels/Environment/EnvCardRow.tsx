import { useState } from 'react'
import { callVerb } from '../../lib/client'
import {
  cardLabel,
  fetchTool,
  hasFetchAction,
  hasMatteSetupAction,
  hasSetupAction,
  setupMatte,
  setupPerception,
  type DoctorCard,
} from '../../lib/doctor'
import { pickFfmpeg } from '../../lib/tauri'
import { ServiceRuntimeActions, ServiceRuntimeDetail } from './ServiceRuntime'
import { SttModelControl } from './SttModelControl'
import { useEnvironmentSetupJob } from './useEnvironmentSetupJob'

/** Status -> display token (label + color class). Fixed family semantics. */
function statusChip(card: DoctorCard): { label: string; cls: string } {
  if (card.id === 'gpu-encode') {
    return card.status === 'ok' && card.details?.hardware_available === true
      ? { label: 'Ready', cls: 'env-st--ok' }
      : card.details?.hardware_available === true
        ? { label: 'Needs attention', cls: 'env-st--degraded' }
        : { label: 'Ready', cls: 'env-st--ok' }
  }
  if (card.status === 'unknown') {
    return card.kind === 'service'
      ? { label: 'Optional', cls: 'env-st--unknown' }
      : { label: 'Check again', cls: 'env-st--unknown' }
  }
  switch (card.status) {
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
  if (card.id === 'gpu-encode') {
    return card.details?.hardware_available === true ? 'GPU acceleration' : 'Software export'
  }
  if (card.kind === 'disk' && card.details?.free_human != null) {
    return `${String(card.details.free_human)} free`
  }
  return null
}

function compactHint(card: DoctorCard): string | null {
  if (card.id === 'gpu-encode') {
    if (card.status === 'ok' && card.details?.hardware_available === true) return null
    if (card.details?.hardware_available === true) return 'Re-scan or choose a working video tool if faster exports are not available.'
    return 'Software export works. Add GPU support only if you need faster renders.'
  }
  if (card.status === 'ok' || card.status === 'unknown') return null
  if (card.id === 'ffmpeg') return 'Install video processing so imports, previews, and exports work.'
  if (card.id === 'perception') return 'Install captions when you need transcripts, word edits, silence cleanup, or search.'
  if (card.id === 'matte') return 'Install the standard cutout model for on-device background removal.'
  if (card.id === 'matte_premium') return 'Install only for cleaner edges and subject picking on supported NVIDIA machines.'
  if (card.status === 'degraded') return 'Re-scan or set this up again if the feature does not work.'
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
  if (card.id === 'gpu-encode') {
    if (card.details?.override_setting != null) rows.push(['Selected ffmpeg', String(card.details.override_setting)])
    if (card.details?.resolved != null) rows.push(['Resolved ffmpeg', String(card.details.resolved)])
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
  onChanged,
}: {
  card: DoctorCard
  os: string
  onChanged: () => void
}) {
  const { title, role } = cardLabel(card)
  const chip = statusChip(card)
  const gpuSoftwareOnly = card.id === 'gpu-encode' && card.details?.hardware_available !== true
  const { job, runJob } = useEnvironmentSetupJob(onChanged)
  const overrideSetting =
    card.id === 'gpu-encode' ? ((card.details?.override_setting as string | null) ?? null) : null
  const [ffBusy, setFfBusy] = useState(false)
  const [ffNote, setFfNote] = useState<string | null>(null)
  const [serviceSetupOpen, setServiceSetupOpen] = useState(false)
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

  const onChangeFfmpeg = async () => {
    const p = await pickFfmpeg()
    if (p) await setFfmpeg(p)
  }

  const onDownload = () => void runJob(() => fetchTool('ffmpeg'), 'could not start the download')
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
      className={`env-row env-row--${gpuSoftwareOnly ? 'unknown' : card.status}`}
      data-cut-env-card={card.id}
      data-cut-env-status={card.status}
    >
      <span className={`env-st ${chip.cls}`} data-cut-env-statuschip={card.status}>
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
        {!job.busy && hasFetchAction(card, os) && (
          <button
            className="env-btn env-btn--primary env-btn--sm"
            data-cut-env-download={card.id}
            onClick={onDownload}
            title="Install verified video-processing tools"
          >
            Install
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
        {!job.busy && hasMatteSetupAction(card) && (
          <button
            className="env-btn env-btn--primary env-btn--sm"
            data-cut-env-setup-matte={card.id}
            onClick={onSetupMatte}
            title={
              card.id === 'matte_premium'
                ? 'Install premium background removal (needs an NVIDIA GPU, ~135 MB, non-commercial license)'
                : 'Install standard background removal (~14 MB, runs on your device)'
            }
          >
            {card.id === 'matte_premium' ? 'Install premium' : 'Install (~14 MB)'}
          </button>
        )}
        {!job.busy && card.kind !== 'service' && card.status === 'unknown' && (
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
                <button
                  className="env-btn env-btn--sm"
                  data-cut-env-ffmpeg-change
                  onClick={() => void onChangeFfmpeg()}
                  disabled={ffBusy}
                  title="Choose a different video-processing tool"
                >
                  Choose tool…
                </button>
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
            Cleaner edges + temporal stability + click-to-pick which subject. NVIDIA GPU, ~135 MB —
            <strong> non-commercial license (NTU S-Lab 1.0)</strong>; installing accepts it.
          </div>
        </div>
      )}
    </div>
  )
}
