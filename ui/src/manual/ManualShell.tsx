import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import {
  MANUAL_CONTENT,
  MANUAL_FEATURE_BY_ID,
  MANUAL_FEATURES,
  resolveManualFeatureId,
  type ManualFeatureContent,
} from './content'
import {
  CUT_MANUAL_PROTOCOL,
  isManualFrontendMessage,
  manualPostMessageTargetOrigin,
} from './protocol'
import './manual-shell.css'

type RevealStatus =
  | { kind: 'idle'; text: string }
  | { kind: 'pending'; text: string }
  | { kind: 'shown'; text: string }
  | { kind: 'surface-only'; text: string }
  | { kind: 'unavailable'; text: string }
  | { kind: 'missing'; text: string }

const DEFAULT_FEATURE_ID = MANUAL_FEATURES[0]?.id ?? ''

function featureIdFromLocation(): string {
  // Keep the established ?feature= links valid while the interactive shell
  // uses hashes for cheap, static-host-safe in-page history.
  const requestedId = window.location.hash.slice(1)
    || new URLSearchParams(window.location.search).get('feature')
    || ''
  return resolveManualFeatureId(requestedId) ?? DEFAULT_FEATURE_ID
}

function manualEmbedUrl(): string {
  const url = new URL(window.location.href)
  url.search = ''
  url.searchParams.set('manual', 'embed')
  url.searchParams.set('mock', '1')
  url.hash = ''
  return `${url.pathname}${url.search}`
}

function requestId(): string {
  if (typeof crypto.randomUUID === 'function') return crypto.randomUUID()
  return `manual-${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`
}

function normalizeSearch(value: string): string {
  return value.trim().toLocaleLowerCase()
}

function matchesSearch(feature: ManualFeatureContent, query: string): boolean {
  if (!query) return true
  return [feature.label, feature.title, feature.group, feature.where, feature.description, feature.api]
    .some((value) => value.toLocaleLowerCase().includes(query))
}

function displayGroup(group: string): string {
  return group.replaceAll('_', ' ').replace(/\b\w/g, (letter) => letter.toUpperCase())
}

function initialRevealStatus(): RevealStatus {
  return { kind: 'idle', text: 'Select a feature to locate it in Cut.' }
}

export default function ManualShell() {
  const iframeRef = useRef<HTMLIFrameElement>(null)
  const pendingReveals = useRef(new Map<string, string>())
  const [query, setQuery] = useState('')
  const [selectedId, setSelectedId] = useState(featureIdFromLocation)
  const [embeddedReady, setEmbeddedReady] = useState(false)
  const [revealStatus, setRevealStatus] = useState<RevealStatus>(initialRevealStatus)
  const selectedFeature = MANUAL_FEATURE_BY_ID.get(selectedId) ?? MANUAL_FEATURES[0]
  const normalizedQuery = normalizeSearch(query)
  const visibleFeatures = useMemo(
    () => MANUAL_FEATURES.filter((feature) => matchesSearch(feature, normalizedQuery)),
    [normalizedQuery],
  )
  const groupedFeatures = useMemo(() => {
    const groups = new Map<string, ManualFeatureContent[]>()
    for (const feature of visibleFeatures) {
      const members = groups.get(feature.group)
      if (members) members.push(feature)
      else groups.set(feature.group, [feature])
    }
    return [...groups]
  }, [visibleFeatures])

  const postReveal = useCallback((featureId: string) => {
    if (!embeddedReady) {
      setRevealStatus({ kind: 'pending', text: 'Opening the editor…' })
      return
    }

    const targetWindow = iframeRef.current?.contentWindow
    if (!targetWindow) return

    const id = requestId()
    pendingReveals.current.set(id, featureId)
    setRevealStatus({ kind: 'pending', text: 'Locating this control in the editor…' })
    targetWindow.postMessage({
      schema: CUT_MANUAL_PROTOCOL,
      type: 'reveal',
      featureId,
      requestId: id,
    }, manualPostMessageTargetOrigin())
  }, [embeddedReady])

  const selectFeature = useCallback((requestedId: string, writeHistory: boolean) => {
    const featureId = resolveManualFeatureId(requestedId)
    if (!featureId) return null

    setSelectedId(featureId)
    if (writeHistory && window.location.hash.slice(1) !== featureId) {
      const nextUrl = new URL(window.location.href)
      nextUrl.hash = featureId
      window.history.pushState({ manualFeatureId: featureId }, '', nextUrl)
    }
    return featureId
  }, [])

  const activateFeature = useCallback((requestedId: string) => {
    const featureId = selectFeature(requestedId, true)
    if (featureId === selectedId) postReveal(featureId)
  }, [postReveal, selectFeature, selectedId])

  useEffect(() => {
    const syncLocation = () => {
      const featureId = featureIdFromLocation()
      setSelectedId(featureId)
    }
    window.addEventListener('popstate', syncLocation)
    window.addEventListener('hashchange', syncLocation)
    return () => {
      window.removeEventListener('popstate', syncLocation)
      window.removeEventListener('hashchange', syncLocation)
    }
  }, [])

  useEffect(() => {
    if (embeddedReady && selectedFeature) postReveal(selectedFeature.id)
  }, [embeddedReady, postReveal, selectedFeature])

  useEffect(() => {
    const receiveMessage = (event: MessageEvent<unknown>) => {
      if (event.origin !== window.location.origin || event.source !== iframeRef.current?.contentWindow) return
      if (!isManualFrontendMessage(event.data)) return

      if (event.data.type === 'ready') {
        setEmbeddedReady(true)
        return
      }

      if (event.data.type === 'selected') {
        const featureId = selectFeature(event.data.featureId, true)
        if (featureId) setRevealStatus({ kind: 'shown', text: 'Selected in the editor.' })
        return
      }

      if (event.data.type === 'reveal-result') {
        const featureId = pendingReveals.current.get(event.data.requestId)
        pendingReveals.current.delete(event.data.requestId)
        if (featureId !== selectedId || event.data.featureId !== selectedId) return

        const defaultText: Record<Exclude<RevealStatus['kind'], 'idle' | 'pending'>, string> = {
          shown: 'Opened and highlighted in the editor.',
          'surface-only': 'Opened the nearest editor surface.',
          unavailable: 'This entry explains a capability without one editor control.',
          missing: 'This control is unavailable in the embedded editor.',
        }
        setRevealStatus({
          kind: event.data.status,
          text: event.data.message ?? defaultText[event.data.status],
        })
      }
    }
    window.addEventListener('message', receiveMessage)
    return () => window.removeEventListener('message', receiveMessage)
  }, [selectedId, selectFeature])

  useEffect(() => {
    const focusSearch = (event: KeyboardEvent) => {
      if (event.key !== '/' || event.metaKey || event.ctrlKey || event.altKey) return
      const target = event.target as HTMLElement | null
      if (target?.matches('input, textarea, select, [contenteditable="true"]')) return
      const search = document.querySelector<HTMLInputElement>('[data-cut-manual-search]')
      if (!search) return
      event.preventDefault()
      search.focus()
    }
    window.addEventListener('keydown', focusSearch)
    return () => window.removeEventListener('keydown', focusSearch)
  }, [])

  const moveFocus = (event: React.KeyboardEvent<HTMLButtonElement>, featureId: string) => {
    const buttons = [...document.querySelectorAll<HTMLButtonElement>('[data-cut-manual-feature]')]
    const index = buttons.findIndex((button) => button.dataset.cutManualFeature === featureId)
    if (index < 0) return
    const nextIndex = event.key === 'Home'
      ? 0
      : event.key === 'End'
        ? buttons.length - 1
        : event.key === 'ArrowDown'
          ? Math.min(index + 1, buttons.length - 1)
          : event.key === 'ArrowUp'
            ? Math.max(index - 1, 0)
            : index
    if (nextIndex === index && !['Home', 'End', 'ArrowDown', 'ArrowUp'].includes(event.key)) return
    event.preventDefault()
    buttons[nextIndex]?.focus()
  }

  if (!selectedFeature) return null

  return (
    <main className="manual-shell" data-cut-manual-shell>
      <aside className="manual-shell__rail" aria-label="ShellX Cut manual">
        <header className="manual-shell__header">
          <p className="manual-shell__eyebrow">ShellX Cut</p>
          <h1>Manual</h1>
          <p>{MANUAL_CONTENT.featureCount} features · live editor guide</p>
        </header>

        <label className="manual-shell__search">
          <span className="manual-shell__search-label">Search the manual</span>
          <input
            data-cut-manual-search
            type="search"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            onKeyDown={(event) => {
              if (event.key === 'Escape' && query) {
                event.preventDefault()
                setQuery('')
              }
            }}
            placeholder="Find a tool or workflow"
            autoComplete="off"
          />
          <kbd aria-hidden="true">/</kbd>
        </label>

        <nav className="manual-shell__index" aria-label="Feature index" data-cut-manual-index>
          {groupedFeatures.length > 0 ? groupedFeatures.map(([group, features]) => (
            <section className="manual-shell__group" key={group} aria-labelledby={`manual-group-${group}`}>
              <h2 id={`manual-group-${group}`}>{displayGroup(group)}</h2>
              <ul>
                {features.map((feature) => {
                  const selected = feature.id === selectedFeature.id
                  return (
                    <li key={feature.id}>
                      <button
                        type="button"
                        data-cut-manual-feature={feature.id}
                        className={selected ? 'is-selected' : undefined}
                        aria-current={selected ? 'page' : undefined}
                        onClick={() => activateFeature(feature.id)}
                        onKeyDown={(event) => moveFocus(event, feature.id)}
                      >
                        <span>{feature.label}</span>
                        <small>{feature.where}</small>
                      </button>
                    </li>
                  )
                })}
              </ul>
            </section>
          )) : (
            <p className="manual-shell__empty">No manual entries match “{query}”.</p>
          )}
        </nav>

        <section className="manual-shell__explanation" aria-live="polite" data-cut-manual-explanation>
          <p className="manual-shell__eyebrow">{selectedFeature.where}</p>
          <h2>{selectedFeature.title}</h2>
          <p>{selectedFeature.description}</p>
          <dl>
            <div>
              <dt>Use when</dt>
              <dd>{selectedFeature.requirement}</dd>
            </div>
            <div>
              <dt>Interface</dt>
              <dd>{selectedFeature.api}</dd>
            </div>
          </dl>
          <p className={`manual-shell__reveal-status is-${revealStatus.kind}`} role="status">
            {revealStatus.text}
          </p>
        </section>
      </aside>

      <section className="manual-shell__editor" aria-label="Interactive ShellX Cut editor">
        <iframe
          ref={iframeRef}
          data-cut-manual-editor
          title="Interactive ShellX Cut editor"
          src={manualEmbedUrl()}
          sandbox="allow-scripts allow-same-origin"
        />
      </section>
    </main>
  )
}
