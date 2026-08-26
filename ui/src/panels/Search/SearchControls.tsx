import { useEffect, useRef, type FormEvent } from 'react'
import { Icon } from '../../icons'
import { EVIDENCE_MODES, type EvidenceMode, type EvidenceScope } from './model'

interface SearchControlsProps {
  query: string
  mode: EvidenceMode
  scope: EvidenceScope
  searching: boolean
  canSearch: boolean
  onQuery: (value: string) => void
  onMode: (value: EvidenceMode) => void
  onScope: (value: EvidenceScope) => void
  onSearch: () => void
}

export default function SearchControls({
  query, mode, scope, searching, canSearch,
  onQuery, onMode, onScope, onSearch,
}: SearchControlsProps) {
  const inputRef = useRef<HTMLInputElement>(null)

  useEffect(() => {
    const focusSearch = (event: KeyboardEvent) => {
      if (event.key !== '/' || event.metaKey || event.ctrlKey || event.altKey) return
      const target = event.target
      if (target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement || (target instanceof HTMLElement && target.isContentEditable)) return
      event.preventDefault()
      inputRef.current?.focus()
    }
    document.addEventListener('keydown', focusSearch)
    return () => document.removeEventListener('keydown', focusSearch)
  }, [])

  const submit = (event: FormEvent) => {
    event.preventDefault()
    if (canSearch && !searching) onSearch()
  }

  return (
    <form className="mi-search" data-cut-intelligence-controls onSubmit={submit}>
      <label className="mi-search__query">
        <span className="mi-sr-only">Search project evidence</span>
        <Icon name="search" size={14} />
        <input
          ref={inputRef}
          type="search"
          value={query}
          onChange={(event) => onQuery(event.target.value)}
          placeholder="Describe a shot or type words that were spoken"
          autoComplete="off"
          data-cut-intelligence-query
        />
        <kbd>/</kbd>
      </label>
      <div className="mi-search__filters">
        <label>
          <span className="mi-sr-only">Evidence type</span>
          <select value={mode} onChange={(event) => onMode(event.target.value as EvidenceMode)} data-cut-intelligence-kind>
            {EVIDENCE_MODES.map((option) => <option key={option.value} value={option.value}>{option.label}</option>)}
          </select>
        </label>
        <label>
          <span className="mi-sr-only">Search scope</span>
          <select value={scope} onChange={(event) => onScope(event.target.value as EvidenceScope)} data-cut-intelligence-scope>
            <option value="all_project_media">All project media</option>
            <option value="this_sequence">This sequence</option>
          </select>
        </label>
        <button type="submit" className="cd-btn cd-btn--primary mi-search__submit" disabled={!canSearch || searching || !query.trim()} data-cut-intelligence-search>
          {searching ? <><Icon name="spinner" size={14} /> Searching…</> : 'Search'}
        </button>
      </div>
    </form>
  )
}
