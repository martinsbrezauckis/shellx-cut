import { useEffect, useMemo, useState } from 'react'
import { Icon } from '../icons'
import { MANUAL_FEATURES, resolveManualFeatureId, type ManualFeatureContent } from './content'
import { CUT_MANUAL_PROTOCOL } from './protocol'
import { manualFeatureTarget } from './targets'
import './local-manual.css'

interface LocalManualPanelProps {
  open: boolean
  requestedFeatureId?: string
  requestId: number
  onClose: () => void
}

const GROUP_LABELS: Record<string, string> = {
  setup: 'Setup',
  top: 'Top bar',
  header: 'Header tools',
  left: 'Sidebar',
  preview: 'Preview',
  timeline: 'Timeline',
  context: 'Context menus',
  inspector: 'Inspector',
  review: 'Review',
  record: 'Recording',
  workflow: 'Workflows',
  api: 'API reference',
}

function matches(feature: ManualFeatureContent, query: string): boolean {
  const words = query.toLowerCase().trim().split(/\s+/).filter(Boolean)
  if (!words.length) return true
  const haystack = [feature.label, feature.title, feature.where, feature.description, feature.api]
    .join(' ')
    .toLowerCase()
  return words.every((word) => haystack.includes(word))
}

function revealFeature(featureId: string): void {
  document.dispatchEvent(new CustomEvent('cut:manual-reveal', {
    detail: {
      schema: CUT_MANUAL_PROTOCOL,
      type: 'reveal',
      featureId,
      requestId: `local-${Date.now()}`,
    },
  }))
}

export default function LocalManualPanel({ open, requestedFeatureId, requestId, onClose }: LocalManualPanelProps) {
  const [query, setQuery] = useState('')
  const [selectedId, setSelectedId] = useState('cut.left.assets')
  const visible = useMemo(() => MANUAL_FEATURES.filter((feature) => matches(feature, query)), [query])
  const selected = MANUAL_FEATURES.find((feature) => feature.id === selectedId) ?? MANUAL_FEATURES[0]
  const selectedTarget = selected ? manualFeatureTarget(selected.id) : null
  const groups = useMemo(() => {
    const next = new Map<string, ManualFeatureContent[]>()
    for (const feature of visible) {
      const entries = next.get(feature.group) ?? []
      entries.push(feature)
      next.set(feature.group, entries)
    }
    return next
  }, [visible])

  useEffect(() => {
    if (!open) return
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key !== 'Escape') return
      event.preventDefault()
      onClose()
    }
    window.addEventListener('keydown', closeOnEscape)
    return () => window.removeEventListener('keydown', closeOnEscape)
  }, [onClose, open])

  useEffect(() => {
    if (!open || !requestedFeatureId) return
    const featureId = resolveManualFeatureId(requestedFeatureId)
    if (featureId) setSelectedId(featureId)
  }, [open, requestedFeatureId, requestId])

  if (!open || !selected) return null

  const select = (feature: ManualFeatureContent) => {
    setSelectedId(feature.id)
  }
  const showInEditor = () => {
    if (selectedTarget?.unavailable) return
    onClose()
    // Close the manual before a real modal/drawer opens; otherwise that
    // surface's focus/scrim layer can correctly sit above this panel and make
    // the initiating control unreachable.
    window.requestAnimationFrame(() => revealFeature(selected.id))
  }

  return (
    <aside className="local-manual" data-cut-local-manual aria-label="ShellX Cut manual">
      <header className="local-manual__header">
        <div>
          <strong>Manual</strong>
          <span>{MANUAL_FEATURES.length} indexed features</span>
        </div>
        <button type="button" onClick={onClose} aria-label="Close manual" data-cut-local-manual-close>
          <Icon name="close" size={16} />
        </button>
      </header>

      <label className="local-manual__search">
        <span>Find a feature</span>
        <input
          type="search"
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          placeholder="Search tools, menus, or verbs"
          autoFocus
          data-cut-local-manual-search
        />
      </label>

      <article className="local-manual__detail" aria-live="polite">
        <span className="local-manual__where">{selected.where}</span>
        <h2>{selected.title}</h2>
        <p>{selected.description}</p>
        <dl>
          <div><dt>Requirement</dt><dd>{selected.requirement}</dd></div>
          <div><dt>Debug/API</dt><dd>{selected.api}</dd></div>
        </dl>
        {selectedTarget?.unavailable && (
          <p className="local-manual__availability" role="status">{selectedTarget.unavailable}</p>
        )}
        <button
          type="button"
          className="local-manual__reveal"
          onClick={showInEditor}
          disabled={Boolean(selectedTarget?.unavailable)}
          data-cut-local-manual-reveal
        >
          {selectedTarget?.unavailable ? 'Reference only' : 'Show in Cut'}
        </button>
      </article>

      <nav className="local-manual__index" aria-label="Manual index">
        {Array.from(groups, ([group, features]) => (
          <section key={group}>
            <h3>{GROUP_LABELS[group] ?? group}</h3>
            {features.map((feature) => (
              <button
                type="button"
                key={feature.id}
                className={feature.id === selected.id ? 'is-selected' : undefined}
                aria-current={feature.id === selected.id ? 'page' : undefined}
                onClick={() => select(feature)}
                data-cut-local-manual-feature={feature.id}
              >
                <span>{feature.label}</span>
                <small>{feature.where}</small>
              </button>
            ))}
          </section>
        ))}
        {!visible.length && <p className="local-manual__empty">No manual entries match that search.</p>}
      </nav>
    </aside>
  )
}
