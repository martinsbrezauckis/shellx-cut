import { useState } from 'react'
import type { MotionClipLink } from '../../lib/client'
import { callVerb } from '../../lib/client'
import { pickMotionPackage } from '../../lib/tauri'
import MotionEffectsSection from './MotionEffectsSection'
import MotionTrackingSection from './MotionTrackingSection'

export interface MotionLinkSectionProps {
  link: MotionClipLink
}

function shortDigest(value: string): string {
  return value.length > 14 ? `${value.slice(0, 12)}…` : value
}

export default function MotionLinkSection({ link }: MotionLinkSectionProps) {
  const [busy, setBusy] = useState<'edit' | 'refresh' | 'relink' | null>(null)
  const [note, setNote] = useState<string | null>(null)
  const canRefresh = link.availability?.canRefresh ?? !!link.sourcePath
  const canEditInMotion = link.availability?.canEditInMotion === true

  const editInMotion = async () => {
    if (busy || !canEditInMotion) return
    setBusy('edit')
    setNote('Opening verified package in Canvas Motion Studio…')
    try {
      const result = await callVerb('motion.link.edit', { clip: link.clipId })
      setNote(result.ok ? 'Opened in Canvas Motion Studio.' : result.error?.message ?? 'Canvas launch failed')
    } catch (error) {
      setNote(error instanceof Error ? error.message : 'Canvas launch failed')
    } finally {
      setBusy(null)
    }
  }

  const refresh = async () => {
    if (busy || !canRefresh) return
    setBusy('refresh')
    setNote('Rendering a verified replacement…')
    try {
      const result = await callVerb('motion.link.refresh', {
        clip: link.clipId,
        preset: 'mp4-h264',
        rationale: 'inspector: refresh linked Motion clip',
      })
      setNote(result.ok ? 'Linked render refreshed. Undo restores the previous render.' : result.error?.message ?? 'Refresh failed')
    } catch (error) {
      setNote(error instanceof Error ? error.message : 'Refresh failed')
    } finally {
      setBusy(null)
    }
  }

  const relink = async () => {
    if (busy) return
    const packageDir = await pickMotionPackage()
    if (!packageDir) return
    setBusy('relink')
    setNote('Validating Motion package identity…')
    try {
      const result = await callVerb('motion.link.relink', {
        clip: link.clipId,
        package_dir: packageDir,
        rationale: 'inspector: relink Motion package',
      })
      setNote(result.ok ? 'Source relinked. Refresh when ready to update pixels.' : result.error?.message ?? 'Relink failed')
    } catch (error) {
      setNote(error instanceof Error ? error.message : 'Relink failed')
    } finally {
      setBusy(null)
    }
  }

  return (
    <div className="insp__group insp__motion-link" data-cut-inspector-group="motion-link" data-cut-motion-link-state={link.state}>
      <div className="insp__group-title">ShellX Motion</div>
      <div className={`insp__motion-state insp__motion-state--${link.state}`} data-cut-motion-status>
        <span aria-hidden="true" />
        <strong>{link.state === 'linked-current' ? 'Linked and current' : link.state.replaceAll('-', ' ')}</strong>
      </div>
      <dl className="insp__props">
        <dt>Source</dt><dd title={link.motionSourceId}>{link.motionId}</dd>
        <dt>Package</dt><dd title={link.packageId}>{link.packageId}</dd>
        <dt>Source revision</dt><dd title={link.sourceRevision}>{shortDigest(link.sourceRevision)}</dd>
        <dt>Render</dt><dd title={link.render.sha256}>{shortDigest(link.render.sha256)}</dd>
        <dt>Mode</dt><dd>{link.mode === 'rendered_media' ? 'Rendered fallback' : 'Native lowering'}</dd>
        {link.lastReceiptId && (<><dt>Receipt</dt><dd title={link.lastReceiptId}>{shortDigest(link.lastReceiptId)}</dd></>)}
      </dl>
      <p className="insp__hint">Cut preserves this clip’s timeline identity while Motion owns rain, water, snow, shaders, 3D, particles, blur, and film rendering. Edit the source in Canvas, then refresh this render.</p>
      <div className="insp__motion-actions">
        <button
          type="button"
          className="insp__btn insp__btn--primary"
          data-cut-motion-edit={link.clipId}
          disabled={busy !== null || !canEditInMotion}
          title={canEditInMotion ? 'Open weather, effects, curves, and keyframes in Canvas Motion Studio' : 'Install/configure ShellX Canvas and relink the source first'}
          onClick={() => void editInMotion()}
        >
          {busy === 'edit' ? 'Opening…' : 'Edit in Motion'}
        </button>
        <button
          type="button"
          className="insp__btn"
          data-cut-motion-refresh={link.clipId}
          disabled={busy !== null || !canRefresh}
          title={canRefresh ? 'Render a new immutable Motion artifact and replace this clip in place' : 'Relink the original Motion package first'}
          onClick={() => void refresh()}
        >
          {busy === 'refresh' ? 'Rendering…' : 'Refresh render'}
        </button>
        <button
          type="button"
          className="insp__btn"
          data-cut-motion-relink={link.clipId}
          disabled={busy !== null}
          onClick={() => void relink()}
        >
          {busy === 'relink' ? 'Relinking…' : 'Relink source…'}
        </button>
      </div>
      {!canRefresh && <p className="insp__motion-warning">The rendered fallback is still usable, but its editable Motion source is not connected on this PC.</p>}
      {note && <p className="insp__motion-note" role="status" data-cut-motion-action-status>{note}</p>}
      <MotionEffectsSection link={link} />
      <MotionTrackingSection link={link} />
    </div>
  )
}
