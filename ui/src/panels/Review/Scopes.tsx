import { useCallback, useEffect, useMemo, useState } from 'react'
import { callVerb, exportUrl } from '../../lib/client'
import { Icon } from '../../icons'

type ScopeKind = 'vectorscope' | 'waveform' | 'histogram'

interface ScopeReport {
  at_ms: number
  source?: string
  frame?: string
  pass?: boolean
  luma?: { min?: number; avg?: number; max?: number }
  clipping?: { highlights?: boolean; shadows?: boolean }
  broadcast_legal?: boolean
  saturation?: { avg?: number; max?: number }
  white_balance?: { u_avg?: number; v_avg?: number; cast?: string }
  hue?: { avg?: number; med?: number }
  flags?: string[]
  scopes?: Partial<Record<ScopeKind, string>>
}

const KIND_LABELS: Record<ScopeKind, string> = {
  vectorscope: 'Vectorscope',
  waveform: 'Waveform',
  histogram: 'Histogram',
}

const FLAG_COPY: Record<string, string> = {
  clipped_highlights: 'clipped highlights',
  crushed_shadows: 'crushed shadows',
  illegal_levels: 'broadcast levels outside range',
  colour_cast: 'visible colour cast',
  over_saturated: 'over-saturated colour',
}

function scopeWarning(flag: string): string {
  return FLAG_COPY[flag] ?? flag.replaceAll('_', ' ')
}

function fmt(value: unknown, suffix = ''): string {
  return typeof value === 'number' && Number.isFinite(value) ? `${value.toFixed(value % 1 === 0 ? 0 : 1)}${suffix}` : 'n/a'
}

function imageHref(path: string): string {
  return path.startsWith('/api/') || path.startsWith('http') ? path : exportUrl(path)
}

export default function Scopes({ playheadMs }: { playheadMs: number }) {
  const [atMs, setAtMs] = useState(() => Math.max(0, Math.round(playheadMs)))
  const [includeImages, setIncludeImages] = useState(false)
  const [kinds, setKinds] = useState<Set<ScopeKind>>(new Set(['vectorscope', 'waveform', 'histogram']))
  const [busy, setBusy] = useState(false)
  const [report, setReport] = useState<ScopeReport | null | undefined>(undefined)
  const [error, setError] = useState<string | null>(null)

  const selectedKinds = useMemo(() => Array.from(kinds), [kinds])

  useEffect(() => {
    setAtMs(Math.max(0, Math.round(playheadMs)))
    setReport(undefined)
    setError(null)
  }, [playheadMs])

  const toggleKind = useCallback((kind: ScopeKind) => {
    setKinds((prev) => {
      const next = new Set(prev)
      if (next.has(kind)) next.delete(kind)
      else next.add(kind)
      return next.size ? next : new Set([kind])
    })
  }, [])

  const runScopes = useCallback(async () => {
    setBusy(true)
    setError(null)
    try {
      const r = await callVerb('verify.scopes', {
        at_ms: Math.max(0, Math.round(atMs)),
        scope_images: includeImages,
        kinds: selectedKinds,
      })
      if (!r.ok || !r.result) {
        setReport(null)
        setError(r.error?.message ?? 'Scopes check failed')
        return
      }
      setReport(r.result as ScopeReport)
    } catch (err) {
      setReport(null)
      setError(err instanceof Error ? err.message : 'Scopes check failed')
    } finally {
      setBusy(false)
    }
  }, [atMs, includeImages, selectedKinds])

  const warnings = report?.flags?.map(scopeWarning) ?? []
  const sourceLabel = report?.source === 'asset' ? 'Source frame' : 'Timeline frame'

  return (
    <div className="scopes" data-cut-scopes>
      <div className="scopes__bar" data-cut-scopes-bar>
        <label className="scopes__time">
          <span>Frame</span>
          <input
            type="number"
            min={0}
            step={250}
            value={atMs}
            data-cut-scopes-at-ms
            onChange={(e) => setAtMs(Number(e.target.value) || 0)}
          />
          <span>ms</span>
        </label>
        <label className="scopes__toggle">
          <input
            type="checkbox"
            checked={includeImages}
            data-cut-scopes-images
            onChange={(e) => setIncludeImages(e.target.checked)}
          />
          Images
        </label>
        <button className="qc__run" data-cut-action="scopes-run" disabled={busy} onClick={() => void runScopes()}>
          {busy ? 'checking…' : <><Icon name="waveform" size={14} /> Check scopes</>}
        </button>
      </div>

      <div className="scopes__kinds" aria-label="Scope images" data-cut-scopes-kinds>
        {(Object.keys(KIND_LABELS) as ScopeKind[]).map((kind) => (
          <button
            key={kind}
            type="button"
            className={`scopes__kind${kinds.has(kind) ? ' scopes__kind--on' : ''}`}
            data-cut-scopes-kind={kind}
            aria-pressed={kinds.has(kind)}
            onClick={() => toggleKind(kind)}
          >
            {KIND_LABELS[kind]}
          </button>
        ))}
      </div>

      {error && <div className="qc__err" data-cut-scopes-error>{error}</div>}

      {report === undefined ? (
        <div className="qc__card" data-cut-scopes-empty>
          <div className="qc__card-head">
            <span className="qc__card-title">Video Scopes</span>
            <span className="qc__card-tag">Colour scopes</span>
          </div>
          <div className="qc__dim">not run</div>
        </div>
      ) : report === null ? (
        <div className="qc__card qc__card--fail" data-cut-scopes-result="error">
          <div className="qc__card-head">
            <span className="qc__card-title">Video Scopes</span>
            <span className="qc__verdict">CHECK FAILED</span>
            <span className="qc__card-tag">Colour scopes</span>
          </div>
        </div>
      ) : (
        <div className={`qc__card ${report.pass ? 'qc__card--pass' : 'qc__card--fail'}`} data-cut-scopes-result={report.pass ? 'pass' : 'warn'}>
          <div className="qc__card-head">
            <span className="qc__card-title">Video Scopes</span>
            <span className="qc__verdict">{report.pass ? 'PASS' : 'CHECK'}</span>
            <span className="qc__card-tag">{sourceLabel} · {fmt(report.at_ms, 'ms')}</span>
          </div>

          {warnings.length > 0 ? (
            <div className="scopes__warnings" data-cut-scopes-warnings>
              {warnings.map((warning) => <span key={warning}>{warning}</span>)}
            </div>
          ) : (
            <div className="qc__dim" data-cut-scopes-warnings>no clipping or broadcast-level warnings</div>
          )}

          <div className="scopes__grid">
            <Metric title="Luma" value={`${fmt(report.luma?.min)} / ${fmt(report.luma?.avg)} / ${fmt(report.luma?.max)}`} detail="min / avg / max" />
            <Metric title="Saturation" value={`${fmt(report.saturation?.avg)} / ${fmt(report.saturation?.max)}`} detail="avg / max" />
            <Metric title="White balance" value={report.white_balance?.cast ?? 'n/a'} detail={`Cb ${fmt(report.white_balance?.u_avg)} · Cr ${fmt(report.white_balance?.v_avg)}`} />
            <Metric title="Broadcast" value={report.broadcast_legal ? 'legal' : 'check'} detail={report.broadcast_legal ? 'Rec.709 range' : 'outside range'} />
          </div>

          {report.scopes && Object.keys(report.scopes).length > 0 && (
            <div className="scopes__images" data-cut-scopes-image-links>
              {(Object.entries(report.scopes) as Array<[ScopeKind, string]>).map(([kind, path]) => (
                <a key={kind} href={imageHref(path)} target="_blank" rel="noreferrer" data-cut-scopes-image={kind}>
                  <img src={imageHref(path)} alt={`${KIND_LABELS[kind]} for frame ${report.at_ms}ms`} />
                  <span>{KIND_LABELS[kind]}</span>
                </a>
              ))}
            </div>
          )}
        </div>
      )}
    </div>
  )
}

function Metric({ title, value, detail }: { title: string; value: string; detail: string }) {
  return (
    <div className="scopes__metric">
      <span className="scopes__metric-title">{title}</span>
      <span className="scopes__metric-value">{value}</span>
      <span className="scopes__metric-detail">{detail}</span>
    </div>
  )
}
