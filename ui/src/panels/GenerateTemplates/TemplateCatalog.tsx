import type { GenerateTemplateSummary } from '../../lib/client'
import { KIND_FILTERS, type KindFilter } from './model'

interface TemplateCatalogProps {
  kind: KindFilter
  query: string
  templates: GenerateTemplateSummary[]
  selectedId: string | null
  loadingList: boolean
  onKind: (kind: KindFilter) => void
  onQuery: (query: string) => void
  onSelected: (id: string) => void
}

export default function TemplateCatalog({
  kind,
  query,
  templates,
  selectedId,
  loadingList,
  onKind,
  onQuery,
  onSelected,
}: TemplateCatalogProps) {
  return (
    <aside className="gt-catalog" aria-label="Generate template catalog">
      <div className="gt-toolbar">
        <label className="gt-search">
          <span>Search</span>
          <input
            className="gt-input"
            data-cut-generate-template-search
            value={query}
            onChange={(e) => onQuery(e.target.value)}
            placeholder="lower third, callout, captions"
          />
        </label>
        <div className="gt-seg" role="tablist" aria-label="Template kind">
          {KIND_FILTERS.map((k) => (
            <button
              key={k}
              role="tab"
              aria-selected={kind === k}
              className={kind === k ? 'gt-seg__btn gt-seg__btn--on' : 'gt-seg__btn'}
              data-cut-generate-kind={k}
              onClick={() => onKind(k)}
            >
              {k}
            </button>
          ))}
        </div>
      </div>

      <div className="gt-list" data-cut-generate-template-list>
        {loadingList && <div className="gt-empty">Loading templates...</div>}
        {!loadingList && templates.length === 0 && <div className="gt-empty">No templates match this filter.</div>}
        {templates.map((template) => (
          <button
            key={template.id}
            className={template.id === selectedId ? 'gt-card gt-card--active' : 'gt-card'}
            data-cut-generate-template-card
            data-cut-generate-template-id={template.id}
            onClick={() => onSelected(template.id)}
          >
            <span className="gt-card__top">
              <strong>{template.title}</strong>
              <em>{template.kind}</em>
            </span>
            <span className="gt-card__summary">{template.summary}</span>
            <span className="gt-card__badges">
              {template.capabilities.map((cap) => <span key={cap}>{cap}</span>)}
            </span>
          </button>
        ))}
      </div>
    </aside>
  )
}
