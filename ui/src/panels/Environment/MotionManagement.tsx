// MotionManagement.tsx — read-only ShellX Motion posture in Settings > Video.
//
// This surface intentionally separates a detected local runtime from a managed
// install. Until MOTION-DIST-01 exists, lifecycle controls are disabled facts,
// not inert promises: each names the exact prerequisite in nearby copy.

import { useCallback, useEffect, useRef, useState } from 'react'
import {
  fetchMotionManagementStatus,
  type MotionManagementStatus,
} from '../../lib/doctor'
import './motion-management.css'

type MotionAction = 'install' | 'repair' | 'update' | 'remove'

// The server bounds each of its three fixed discovery commands to 15 seconds.
// This client deadline still prevents Settings from staying in a loading state
// if the local server or transport stops responding before it returns a status.
const MOTION_STATUS_UI_TIMEOUT_MS = 50_000
const MOTION_STATUS_TIMEOUT_MESSAGE =
  'Motion verification did not finish within 50 seconds. Cut did not make any lifecycle change; re-check to try again.'

async function fetchMotionStatusWithinDeadline(): Promise<MotionManagementStatus | null> {
  let timer: number | undefined
  try {
    return await Promise.race([
      fetchMotionManagementStatus(),
      new Promise<never>((_resolve, reject) => {
        timer = window.setTimeout(() => reject(new Error('motion-status-timeout')), MOTION_STATUS_UI_TIMEOUT_MS)
      }),
    ])
  } finally {
    if (timer !== undefined) window.clearTimeout(timer)
  }
}

function runtimeChip(status: MotionManagementStatus['runtime']['status']): { label: string; cls: string } {
  switch (status) {
    case 'discovered-unmanaged':
      return { label: 'Detected', cls: 'env-st--degraded' }
    case 'unverified':
      return { label: 'Check again', cls: 'env-st--unknown' }
    case 'not-discovered':
      return { label: 'Not detected', cls: 'env-st--unknown' }
  }
}

function runtimeSummary(status: MotionManagementStatus): string {
  if (status.runtime.status === 'discovered-unmanaged') {
    return 'Local runtime found. Catalog and Template-to-Cut descriptor are verified; connector execution is not qualified.'
  }
  if (status.runtime.status === 'unverified') {
    return 'A local Motion candidate was found, but Cut could not verify its read-only discovery contract.'
  }
  return 'No local ShellX Motion runtime was detected. Cut cannot install it until the verified distribution prerequisite is available.'
}

function AdvancedDetails({ status }: { status: MotionManagementStatus }) {
  const cli = status.runtime.cli
  const engine = status.runtime.engine
  return (
    <details className="env-advanced motion-management-advanced" data-cut-motion-advanced>
      <summary className="env-advanced-summary" data-cut-motion-advanced-toggle>Advanced details</summary>
      <dl className="env-advanced-list">
        <div className="env-advanced-row"><dt>Runtime</dt><dd>{status.runtime.status}</dd></div>
        <div className="env-advanced-row"><dt>CLI</dt><dd>{cli ? `${cli.name} ${cli.version}` : 'not verified'}</dd></div>
        <div className="env-advanced-row"><dt>Engine</dt><dd>{engine ? `${engine.name} ${engine.version}` : 'not verified'}</dd></div>
        <div className="env-advanced-row"><dt>Platform</dt><dd>{status.runtime.platform ?? 'not verified'}</dd></div>
        <div className="env-advanced-row"><dt>Catalog</dt><dd>{status.connector.catalog}</dd></div>
        <div className="env-advanced-row"><dt>Descriptor</dt><dd>{status.connector.descriptor}</dd></div>
        <div className="env-advanced-row"><dt>Capability</dt><dd>{status.connector.capabilityId}</dd></div>
        <div className="env-advanced-row"><dt>Managed candidate</dt><dd>none</dd></div>
        <div className="env-advanced-row"><dt>Managed version</dt><dd>none</dd></div>
      </dl>
    </details>
  )
}

function LifecycleControls({ status }: { status: MotionManagementStatus }) {
  const blocker = status.distribution.blocker
  const actions: MotionAction[] = ['install', 'repair', 'update', 'remove']
  return (
    <div className="motion-management-actions" aria-label="ShellX Motion lifecycle actions">
      {actions.map((action) => {
        const available = status.distribution.actions[action].available
        const label = action[0].toUpperCase() + action.slice(1)
        return (
          <button
            key={action}
            type="button"
            className="env-btn env-btn--sm"
            data-cut-motion-action={action}
            disabled={!available}
            aria-describedby="motion-management-blocker"
            title={`${blocker.id}: ${blocker.message}`}
          >
            {label}
          </button>
        )
      })}
    </div>
  )
}

/** Compact Settings > Video surface for a local Motion runtime and its explicit
 * distribution-management boundary. It owns no mutation path. */
export default function MotionManagement() {
  const [status, setStatus] = useState<MotionManagementStatus | null>(null)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const requestSequence = useRef(0)

  const refresh = useCallback(async () => {
    const request = ++requestSequence.current
    setLoading(true)
    setError(null)
    try {
      const next = await fetchMotionStatusWithinDeadline()
      if (request !== requestSequence.current) return
      if (!next) {
        setError('Could not check ShellX Motion right now.')
        return
      }
      setStatus(next)
    } catch (reason) {
      if (request !== requestSequence.current) return
      setError(reason instanceof Error && reason.message === 'motion-status-timeout'
        ? MOTION_STATUS_TIMEOUT_MESSAGE
        : 'Could not check ShellX Motion right now.')
    } finally {
      if (request === requestSequence.current) setLoading(false)
    }
  }, [])

  useEffect(() => {
    void refresh()
    return () => { requestSequence.current += 1 }
  }, [refresh])

  if (loading && !status) {
    return <div className="motion-management-empty" data-cut-motion-status-loading>Checking ShellX Motion…</div>
  }
  if (!status) {
    const timedOut = error === MOTION_STATUS_TIMEOUT_MESSAGE
    return (
      <div className="motion-management-empty" role="alert" data-cut-motion-status-error data-cut-motion-status-timeout={timedOut ? 'true' : undefined}>
        <span>{error ?? 'Could not check ShellX Motion right now.'}</span>
        <button type="button" className="env-btn env-btn--sm" data-cut-motion-refresh onClick={() => void refresh()}>Try again</button>
      </div>
    )
  }

  const chip = runtimeChip(status.runtime.status)
  const cliVersion = status.runtime.cli?.version
  const blocker = status.distribution.blocker
  const timedOut = error === MOTION_STATUS_TIMEOUT_MESSAGE

  return (
    <section className="motion-management" data-cut-motion-management data-cut-motion-runtime={status.runtime.status}>
      <div className={`motion-management-row motion-management-row--${status.runtime.status}`}>
        <span className={`env-st ${chip.cls}`} data-cut-motion-status>{chip.label}</span>
        <div className="motion-management-copy">
          <span className="env-row-title">ShellX Motion</span>
          <span className="env-row-role">Optional motion graphics and rendered templates</span>
        </div>
        <div className="env-row-facts">
          {cliVersion && <span className="env-fact env-fact--summary" data-cut-motion-version>CLI {cliVersion}</span>}
        </div>
        <button type="button" className="env-btn env-btn--sm" data-cut-motion-refresh onClick={() => void refresh()} disabled={loading}>
          {loading ? 'Checking…' : 'Re-check'}
        </button>
      </div>

      <div className="motion-management-detail">
        <p className="env-card-hint" data-cut-motion-summary>{runtimeSummary(status)}</p>
        {error && <p className="motion-management-verification-error" role="alert" data-cut-motion-verification-error data-cut-motion-status-timeout={timedOut ? 'true' : undefined}>{error}</p>}
        <p className="motion-management-blocker" id="motion-management-blocker" data-cut-motion-blocker>
          <strong>{blocker.id}</strong> — {blocker.message}
        </p>
        <LifecycleControls status={status} />
        <AdvancedDetails status={status} />
      </div>
    </section>
  )
}
