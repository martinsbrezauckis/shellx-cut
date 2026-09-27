import { useCallback, useEffect, useRef, useState } from 'react'
import { useBlockingOverlay } from '../../components/overlay/useBlockingOverlay'
import NativeFolderPicker from '../../components/NativeFolderPicker'
import { callVerb, type JobRecord, type PortableB5RelinkReceipt } from '../../lib/client'
import type { PortablePackagePlan } from '../../lib/clientResults'
import {
  formatPortableBytes,
  PORTABLE_NAME_HINT,
  portableErrorMessage,
  portableNameForProject,
  portablePackageCancellationMessage,
  portablePackageNameError,
  portablePreviewNeedsInvalidation,
  type PortableCopyPhase,
} from './portableCopyModel'
import './portableCopy.css'

interface PortableCopyDialogProps {
  projectName: string
  b5Receipt: PortableB5RelinkReceipt | null
  scopeKey: string
  onClose: () => void
}

function CopyDialog({ projectName, b5Receipt, scopeKey, onClose }: PortableCopyDialogProps) {
  const overlay = useBlockingOverlay<HTMLDivElement>(onClose)
  const [name, setName] = useState(() => portableNameForProject(projectName))
  const [destination, setDestination] = useState<string | null>(null)
  const [phase, setPhase] = useState<PortableCopyPhase>('form')
  const [plan, setPlan] = useState<PortablePackagePlan | null>(null)
  const [planHash, setPlanHash] = useState('')
  const [reviewedSources, setReviewedSources] = useState(false)
  const [jobId, setJobId] = useState<string | null>(null)
  const [job, setJob] = useState<JobRecord | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [cancelPending, setCancelPending] = useState(false)
  const activeScope = useRef<string | null>(scopeKey)
  useEffect(() => {
    activeScope.current = scopeKey
    return () => { activeScope.current = null }
  }, [scopeKey])
  const nameError = portablePackageNameError(name)
  const targetOccupied = plan?.target_status === 'occupied'
  const sourceInventoryComplete = !!plan && Array.isArray(plan.assets)
    && plan.assets.length === plan.source_file_count
    && plan.assets.every((asset) => typeof asset.asset === 'string' && asset.asset.length > 0
      && typeof asset.source_path === 'string' && asset.source_path.length > 0)
  const canPreview = !!destination && !nameError && phase === 'form'
  const b5ReceiptIdentity = b5Receipt
    ? `${b5Receipt.grouped_op_id}:${b5Receipt.post_revision}:${b5Receipt.plan_hash}`
    : ''

  const resetPreview = useCallback(() => {
    setPlan(null)
    setPlanHash('')
    setReviewedSources(false)
    setJob(null)
    setJobId(null)
    setPhase('form')
  }, [])

  const preview = useCallback(async () => {
    if (!destination || nameError) return
    setError(null)
    setPlan(null)
    setPlanHash('')
    setReviewedSources(false)
    setPhase('previewing')
    try {
      const response = await callVerb('project.package_plan', {
        destination,
        name,
        ...(b5Receipt ? { b5_receipt: b5Receipt } : {}),
      })
      if (activeScope.current !== scopeKey) return
      if (!response.ok || !response.result) {
        setError(portableErrorMessage(response.error, 'Cut could not inspect this copy.'))
        setPhase('form')
        return
      }
      const nextPlan = response.result.plan
      if (!Array.isArray(nextPlan.assets) || nextPlan.assets.length !== nextPlan.source_file_count
        || nextPlan.assets.some((asset) => typeof asset.source_path !== 'string' || !asset.source_path)) {
        setError('Cut could not show every source file. No copy was created; preview again after updating Cut.')
        setPhase('form')
        return
      }
      setPlan(nextPlan)
      setPlanHash(response.result.plan_hash)
      setPhase('ready')
    } catch {
      if (activeScope.current !== scopeKey) return
      setError('Cut could not reach the local engine. No copy was created.')
      setPhase('form')
    }
  }, [b5Receipt, destination, name, nameError, scopeKey])

  const create = useCallback(async () => {
    if (!destination || !plan || !planHash || targetOccupied || !sourceInventoryComplete || !reviewedSources) return
    setError(null)
    setPhase('creating')
    try {
      const response = await callVerb('project.package_create', {
        destination,
        name,
        plan_hash: planHash,
        ...(b5Receipt ? { b5_receipt: b5Receipt } : {}),
      })
      if (activeScope.current !== scopeKey) return
      if (!response.ok || !response.result) {
        setError(portableErrorMessage(response.error, 'Cut did not start the copy.'))
        resetPreview()
        return
      }
      setJobId(response.result.job_id)
      setJob(null)
    } catch {
      if (activeScope.current !== scopeKey) return
      setError('Cut could not reach the local engine. No copy was confirmed.')
      resetPreview()
    }
  }, [b5Receipt, destination, name, plan, planHash, resetPreview, reviewedSources, scopeKey, sourceInventoryComplete, targetOccupied])

  // A B5 receipt is current-revision-bound. Drop any plan created under an
  // earlier value instead of allowing a stale review dialog to imply it can
  // still publish.
  useEffect(() => {
    resetPreview()
  }, [b5ReceiptIdentity, resetPreview])

  useEffect(() => {
    if (!jobId) return
    let disposed = false
    let timer: number | null = null
    const poll = async () => {
      try {
        const response = await callVerb('jobs.status', { job_id: jobId })
        if (disposed) return
        if (!response.ok || !response.result || response.result.job_id !== jobId) {
          setError(portableErrorMessage(response.error, 'Copy status could not be verified. It may still be running.'))
          timer = window.setTimeout(() => { void poll() }, 1_000)
          return
        }
        const record = response.result
        setJob(record)
        const cancellation = portablePackageCancellationMessage(record)
        if (cancellation) {
          setJobId(null)
          setError(cancellation)
          resetPreview()
          return
        }
        if (record.state === 'done') {
          setJobId(null)
          setPhase('complete')
          return
        }
        if (record.state === 'failed') {
          setJobId(null)
          setError(portableErrorMessage(record.error, 'The copy did not finish.'))
          resetPreview()
          return
        }
        timer = window.setTimeout(() => { void poll() }, 400)
      } catch {
        if (!disposed) {
          setError('Copy status could not be verified. It may still be running.')
          timer = window.setTimeout(() => { void poll() }, 1_000)
        }
      }
    }
    void poll()
    return () => {
      disposed = true
      if (timer !== null) window.clearTimeout(timer)
    }
  }, [jobId, resetPreview])

  const cancel = async () => {
    if (!jobId) return
    setCancelPending(true)
    setError(null)
    try {
      const response = await callVerb('jobs.cancel', { job_id: jobId })
      if (!response.ok) setError(portableErrorMessage(response.error, 'Cut could not cancel the copy.'))
    } catch {
      setError('Cut could not cancel the copy.')
    } finally {
      setCancelPending(false)
    }
  }

  const updateName = (next: string) => {
    setName(next)
    if (portablePreviewNeedsInvalidation(phase)) resetPreview()
  }
  const updateDestination = (next: string) => {
    setDestination(next)
    if (portablePreviewNeedsInvalidation(phase)) resetPreview()
  }

  const duplicateReferences = plan ? plan.source_file_count - plan.unique_media_count : 0
  const savedBytes = plan ? plan.total_source_bytes - plan.total_package_bytes : 0

  return (
    <div className="pj-portable-scrim" data-cut-portable-scrim onMouseDown={overlay.onScrimMouseDown}>
      <div ref={overlay.dialogRef} className="pj-portable-dialog" data-cut-portable-copy data-cut-portable-state={phase} data-cut-blocking-overlay role="dialog" aria-modal="true" aria-label="Make a portable copy" tabIndex={-1} onKeyDown={overlay.onDialogKeyDown}>
        <header className="pj-portable-head">
          <div>
            <p className="pj-portable-eyebrow">Project copy</p>
            <h2>Take this project with you</h2>
            <p>Collect only media used in this project. Originals and Cut&apos;s rebuildable cache stay where they are.</p>
          </div>
          <button type="button" className="pj-portable-close" data-cut-action="portable-copy" data-cut-portable-close onClick={onClose} aria-label="Close portable copy">×</button>
        </header>

        <div className="pj-portable-body">
          <label className="pj-portable-name">
            <span>Copy name</span>
            <input data-cut-action="portable-copy" data-cut-portable-name value={name} disabled={phase === 'previewing' || phase === 'creating' || phase === 'complete'} onChange={(event) => updateName(event.target.value)} aria-invalid={!!nameError} />
            <small>{nameError ?? `${PORTABLE_NAME_HINT}. Cut adds .cutproj.`}</small>
          </label>
          <NativeFolderPicker
            kind="portable"
            label="Destination folder"
            dialogTitle="Choose a folder for the portable copy — ShellX Cut"
            value={destination ?? ''}
            disabled={phase === 'previewing' || phase === 'creating' || phase === 'complete'}
            onChooseStart={() => setError(null)}
            onDesktopRequired={() => setError('Portable copies need the desktop app to choose a local destination folder.')}
            onSelected={updateDestination}
          />
          {error && <p className="pj-portable-error" data-cut-portable-error role="alert">{error}</p>}

          {plan && (
            <section className="pj-portable-preview" data-cut-portable-preview>
              <div className="pj-portable-preview-title"><strong>Copy preview</strong><span>{plan.name}.cutproj</span></div>
              <dl>
                <div><dt>Used media references</dt><dd data-cut-portable-source-files>{plan.source_file_count}</dd></div>
                <div><dt>Unique files to copy</dt><dd data-cut-portable-unique-files>{plan.unique_media_count}</dd></div>
                <div><dt>Media copy size</dt><dd data-cut-portable-size>{formatPortableBytes(plan.total_package_bytes)}</dd></div>
                <div><dt>Duplicates avoided</dt><dd>{duplicateReferences > 0 ? `${duplicateReferences} · ${formatPortableBytes(savedBytes)} saved` : 'None'}</dd></div>
              </dl>
              <div className="pj-portable-sources" data-cut-portable-source-list>
                <strong>Source files included</strong>
                {plan.assets.length === 0 ? <p>No media files are referenced by this project.</p> : (
                  <ul>{plan.assets.map((asset) => (
                    <li key={asset.asset}><span>{asset.asset}</span><code>{asset.source_path}</code></li>
                  ))}</ul>
                )}
              </div>
              <ul className="pj-portable-policy" aria-label="Copy safeguards">
                <li>Only timeline-referenced media is included.</li>
                <li>All referenced media is online now; an offline source refuses the copy before it begins.</li>
                <li>Caches are excluded; original files are never changed.</li>
                <li>{targetOccupied ? 'That copy name already exists in this folder.' : 'The name is free now; publishing checks again and never replaces a folder.'}</li>
              </ul>
            </section>
          )}

          {phase === 'confirming' && plan && !targetOccupied && (
            <section className="pj-portable-confirm" data-cut-portable-confirm>
              <strong>Ready to create this copy?</strong>
              <p>Cut will copy {plan.unique_media_count} used media file{plan.unique_media_count === 1 ? '' : 's'}, verify the manifest, then publish {plan.name}.cutproj. Your open project will not change.</p>
              <label className="pj-portable-source-review"><input type="checkbox" data-cut-action="portable-copy" data-cut-portable-source-reviewed checked={reviewedSources} onChange={(event) => setReviewedSources(event.target.checked)} /> I reviewed the source files listed above.</label>
            </section>
          )}

          {phase === 'creating' && (
            <section className="pj-portable-progress" data-cut-portable-job data-cut-portable-job-state={job?.state ?? 'queued'} aria-live="polite">
              <strong>{job?.message ?? 'Copy is queued.'}</strong>
              <span>{job ? `${Math.round(job.progress * 100)}%` : 'Waiting to start'}</span>
              <p>You can close this window and keep editing; the copy continues in the job indicator.</p>
              <button type="button" className="pj-portable-secondary" data-cut-action="portable-copy" data-cut-portable-cancel disabled={cancelPending} onClick={() => void cancel()}>{cancelPending ? 'Cancelling…' : 'Cancel copy'}</button>
            </section>
          )}

          {phase === 'complete' && (
            <section className="pj-portable-success" data-cut-portable-success role="status">
              <strong>Portable copy is ready.</strong>
              <p>{plan?.name}.cutproj was published in the selected folder. The original project and media were not changed.</p>
            </section>
          )}
        </div>

        <footer className="pj-portable-actions">
          {phase === 'form' && <button type="button" className="pj-portable-primary" data-cut-action="portable-copy" data-cut-portable-preview disabled={!canPreview} onClick={() => void preview()}>{error ? 'Try preview again' : 'Preview copy'}</button>}
          {phase === 'previewing' && <span className="pj-portable-wait" aria-live="polite">Checking used media…</span>}
          {phase === 'ready' && <>
            <button type="button" className="pj-portable-secondary" data-cut-action="portable-copy" data-cut-portable-edit onClick={resetPreview}>Edit copy</button>
            <button type="button" className="pj-portable-primary" data-cut-action="portable-copy" data-cut-portable-review disabled={targetOccupied} onClick={() => setPhase('confirming')}>{targetOccupied ? 'Choose another name' : 'Review copy'}</button>
          </>}
          {phase === 'confirming' && <>
            <button type="button" className="pj-portable-secondary" data-cut-action="portable-copy" data-cut-portable-confirm-cancel onClick={() => setPhase('ready')}>Back</button>
            <button type="button" className="pj-portable-primary" data-cut-action="portable-copy" data-cut-portable-create disabled={!reviewedSources || !sourceInventoryComplete} onClick={() => void create()}>Create portable copy</button>
          </>}
          {phase === 'creating' && <button type="button" className="pj-portable-secondary" data-cut-action="portable-copy" data-cut-portable-background onClick={onClose}>Continue in background</button>}
          {phase === 'complete' && <button type="button" className="pj-portable-primary" data-cut-action="portable-copy" data-cut-portable-done onClick={onClose}>Done</button>}
        </footer>
      </div>
    </div>
  )
}

/** Discoverable Projects route for the verified, non-destructive package flow. */
export default function PortableCopy({ projectName, b5Receipt, scopeKey }: { projectName: string | null; b5Receipt: PortableB5RelinkReceipt | null; scopeKey: string }) {
  const [open, setOpen] = useState(false)
  useEffect(() => setOpen(false), [scopeKey])
  if (!projectName) return null
  return (
    <section className="pj-portable-launch" data-cut-portable-launch>
      <div><strong>Take this project with you</strong><span>Collect the media this project uses into a new copy.</span></div>
      <button type="button" className="pj-portable-launch-btn" data-cut-action="portable-copy" data-cut-portable-open onClick={() => setOpen(true)}>Make a copy…</button>
      {open && <CopyDialog projectName={projectName} b5Receipt={b5Receipt} scopeKey={scopeKey} onClose={() => setOpen(false)} />}
    </section>
  )
}
