import { useCallback, useEffect, useMemo, useState } from 'react'
import {
  callVerb,
  type MotionClipLink,
  type MotionTrackingInventory,
} from '../../lib/client'
import {
  defaultTrackingAnalysisId,
  normalizedTrackingRegion,
  trackingModelForMode,
  trackingVerificationLabel,
  type MotionTrackingMode,
  type MotionTrackingRegionPercent,
} from './motionTrackingModel'

export interface MotionTrackingSectionProps {
  link: MotionClipLink
}

type BusyAction = 'load' | 'analyze' | 'inspect' | 'apply' | 'verify' | 'detach'

const DEFAULT_REGION: MotionTrackingRegionPercent = { x: 25, y: 25, width: 50, height: 50 }

export default function MotionTrackingSection({ link }: MotionTrackingSectionProps) {
  const [inventory, setInventory] = useState<MotionTrackingInventory | null>(null)
  const [analysisId, setAnalysisId] = useState(link.tracking?.analysisId || defaultTrackingAnalysisId(link.clipId))
  const [assetId, setAssetId] = useState(link.tracking?.assetId || '')
  const [layerId, setLayerId] = useState(link.tracking?.attachedLayerId || '')
  const [mode, setMode] = useState<MotionTrackingMode>(link.tracking?.mode || 'point')
  const [region, setRegion] = useState(DEFAULT_REGION)
  const [everyMs, setEveryMs] = useState(100)
  const [busy, setBusy] = useState<BusyAction | null>(null)
  const [note, setNote] = useState<string | null>(null)

  const loadInventory = useCallback(async (preserveNote = false) => {
    if (!link.sourcePath) return
    setBusy((current) => current ?? 'load')
    try {
      const response = await callVerb('motion.link.tracking.inventory', { clip: link.clipId })
      if (!response.ok || !response.result) {
        setNote(response.error?.message ?? 'Tracking inventory is unavailable.')
        return
      }
      setInventory(response.result.inventory)
      setAssetId((current) => current || response.result!.inventory.videoAssets.find((asset) => asset.available)?.id || '')
      setLayerId((current) => current || response.result!.inventory.targetLayers[0]?.id || '')
      if (!preserveNote) setNote(null)
    } catch (error) {
      setNote(error instanceof Error ? error.message : 'Tracking inventory is unavailable.')
    } finally {
      setBusy((current) => current === 'load' ? null : current)
    }
  }, [link.clipId, link.sourcePath])

  useEffect(() => { void loadInventory(false) }, [loadInventory, link.sourceRevision])

  const normalizedRegion = useMemo(() => normalizedTrackingRegion(region), [region])
  const canAnalyze = !!inventory && !!assetId && !!analysisId && !!normalizedRegion && busy === null
  const canUseAnalysis = !!inventory && !!analysisId && !!layerId && busy === null
  const attachedLayerId = link.tracking?.attachedLayerId || inventory?.targetLayers.find((layer) => layer.trackingAttached)?.id || null

  const analyze = async () => {
    if (!canAnalyze || !normalizedRegion) return
    setBusy('analyze')
    setNote('Analyzing package-local footage…')
    try {
      const response = await callVerb('motion.link.tracking.request', {
        clip: link.clipId,
        analysis_id: analysisId,
        asset_id: assetId,
        mode,
        model: trackingModelForMode(mode),
        region: normalizedRegion,
        every_ms: everyMs,
        rationale: 'inspector: analyze linked Motion footage',
      })
      setNote(response.ok ? `Analysis ${response.result?.lifecycle.state || 'completed'}.` : response.error?.message ?? 'Analysis failed.')
      if (response.ok) await loadInventory(true)
    } catch (error) {
      setNote(error instanceof Error ? error.message : 'Analysis failed.')
    } finally {
      setBusy(null)
    }
  }

  const inspect = async () => {
    if (!analysisId || busy) return
    setBusy('inspect')
    setNote('Checking source identity…')
    try {
      const response = await callVerb('motion.link.tracking.inspect', { clip: link.clipId, analysis_id: analysisId })
      setNote(response.ok
        ? response.result?.current ? 'Analysis and source bytes are current.' : 'Source changed; rerun analysis before applying.'
        : response.error?.message ?? 'Inspection failed.')
    } catch (error) {
      setNote(error instanceof Error ? error.message : 'Inspection failed.')
    } finally {
      setBusy(null)
    }
  }

  const apply = async () => {
    if (!canUseAnalysis) return
    setBusy('apply')
    setNote('Compiling stabilization keyframes…')
    try {
      const response = await callVerb('motion.link.tracking.apply', {
        clip: link.clipId,
        analysis_id: analysisId,
        layer_id: layerId,
        rationale: 'inspector: apply linked Motion stabilization',
      })
      setNote(response.ok
        ? `Stabilization applied${response.result?.plan.fidelity ? ` (${response.result.plan.fidelity})` : ''}. Refresh the render to update pixels.`
        : response.error?.message ?? 'Apply failed.')
      if (response.ok) await loadInventory(true)
    } catch (error) {
      setNote(error instanceof Error ? error.message : 'Apply failed.')
    } finally {
      setBusy(null)
    }
  }

  const verify = async () => {
    const target = attachedLayerId || layerId
    if (!target || busy) return
    setBusy('verify')
    setNote('Verifying stabilization and source…')
    try {
      const response = await callVerb('motion.link.tracking.verify', {
        clip: link.clipId,
        layer_id: target,
        ...(analysisId ? { analysis_id: analysisId } : {}),
      })
      setNote(response.ok && response.result
        ? trackingVerificationLabel(response.result.verification)
        : response.error?.message ?? 'Verification failed.')
    } catch (error) {
      setNote(error instanceof Error ? error.message : 'Verification failed.')
    } finally {
      setBusy(null)
    }
  }

  const detach = async () => {
    const target = attachedLayerId || layerId
    if (!target || busy) return
    setBusy('detach')
    setNote('Restoring the pre-stabilization keyframes…')
    try {
      const response = await callVerb('motion.link.tracking.detach', {
        clip: link.clipId,
        layer_id: target,
        rationale: 'inspector: detach linked Motion stabilization',
      })
      setNote(response.ok ? 'Stabilization detached. Refresh the render to update pixels.' : response.error?.message ?? 'Detach failed.')
      if (response.ok) await loadInventory(true)
    } catch (error) {
      setNote(error instanceof Error ? error.message : 'Detach failed.')
    } finally {
      setBusy(null)
    }
  }

  const setRegionField = (field: keyof MotionTrackingRegionPercent, value: number) => {
    setRegion((current) => ({ ...current, [field]: value }))
  }

  if (!link.sourcePath) {
    return <p className="insp__motion-warning">Relink the editable Motion package to use tracking and stabilization.</p>
  }

  return (
    <div className="insp__motion-tracking" data-cut-motion-tracking data-cut-motion-tracking-state={link.tracking?.lifecycleState || 'idle'}>
      <div className="insp__subhead">Track & stabilize</div>
      <p className="insp__hint">Track a subject or surface in package footage, then compile ordinary Motion transform keyframes. Your current Cut render stays intact until Refresh render.</p>
      {inventory ? (
        <>
          <div className="insp__motion-grid">
            <label>Analysis <input data-cut-motion-tracking-analysis value={analysisId} list={`tracking-analyses-${link.clipId}`} onChange={(event) => setAnalysisId(event.currentTarget.value)} /></label>
            <datalist id={`tracking-analyses-${link.clipId}`}>{inventory.analyses.map((analysis) => <option key={analysis.analysisId} value={analysis.analysisId}>{analysis.state}</option>)}</datalist>
            <label>Footage <select data-cut-motion-tracking-asset value={assetId} onChange={(event) => setAssetId(event.currentTarget.value)}>{inventory.videoAssets.map((asset) => <option key={asset.id} value={asset.id} disabled={!asset.available}>{asset.name}{asset.available ? '' : ' (missing)'}</option>)}</select></label>
            <label>Target <select data-cut-motion-tracking-layer value={layerId} onChange={(event) => setLayerId(event.currentTarget.value)}>{inventory.targetLayers.map((layer) => <option key={layer.id} value={layer.id}>{layer.name} · {layer.kind}</option>)}</select></label>
            <label>Mode <select data-cut-motion-tracking-mode value={mode} onChange={(event) => setMode(event.currentTarget.value as MotionTrackingMode)}><option value="point">Point · position</option><option value="planar">Planar · perspective</option></select></label>
            <label>Sample <select data-cut-motion-tracking-sample value={everyMs} onChange={(event) => setEveryMs(Number(event.currentTarget.value))}><option value={50}>50 ms</option><option value={100}>100 ms</option><option value={200}>200 ms</option><option value={500}>500 ms</option></select></label>
          </div>
          <fieldset className="insp__motion-region" data-cut-motion-tracking-region>
            <legend>Seed region · frame %</legend>
            {(['x', 'y', 'width', 'height'] as const).map((field) => <label key={field}>{field === 'width' ? 'W' : field === 'height' ? 'H' : field.toUpperCase()}<input type="number" data-cut-motion-tracking-region-field={field} min={field === 'width' || field === 'height' ? 1 : 0} max={100} step={1} value={region[field]} onChange={(event) => setRegionField(field, Number(event.currentTarget.value))} /></label>)}
          </fieldset>
          {!normalizedRegion && <p className="insp__motion-warning">The seed region must stay fully inside the 0–100% frame.</p>}
          <div className="insp__motion-actions">
            <button className="insp__btn insp__btn--primary" data-cut-motion-tracking-analyze disabled={!canAnalyze} onClick={() => void analyze()}>{busy === 'analyze' ? 'Analyzing…' : 'Analyze'}</button>
            <button className="insp__btn" data-cut-motion-tracking-inspect disabled={!analysisId || busy !== null} onClick={() => void inspect()}>Inspect</button>
            <button className="insp__btn" data-cut-motion-tracking-apply disabled={!canUseAnalysis} onClick={() => void apply()}>{busy === 'apply' ? 'Applying…' : 'Apply stabilization'}</button>
            <button className="insp__btn" data-cut-motion-tracking-verify disabled={!attachedLayerId || busy !== null} onClick={() => void verify()}>Verify</button>
            <button className="insp__btn" data-cut-motion-tracking-detach disabled={!attachedLayerId || busy !== null} onClick={() => void detach()}>Detach</button>
          </div>
          {attachedLayerId && <p className="insp__motion-note">Attached to <strong>{attachedLayerId}</strong>{link.tracking?.fidelity ? ` · ${link.tracking.fidelity}` : ''}</p>}
        </>
      ) : <p className="insp__hint">{busy === 'load' ? 'Reading tracking controls…' : 'Tracking controls are unavailable.'}</p>}
      {note && <p className="insp__motion-note" role="status" data-cut-motion-tracking-status>{note}</p>}
    </div>
  )
}
