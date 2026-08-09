import { Icon } from '../../icons'
import { TYPE_TABS, sortKeyFromInput, type SortKey, type TypeFilter } from './model'

export interface LibraryFiltersProps {
  type: TypeFilter
  sort: SortKey
  search: string
  activeTagFilter: string | null
  onTypeChange: (type: TypeFilter) => void
  onSortChange: (sort: SortKey) => void
  onSearchChange: (value: string) => void
  onClearTagFilter: () => void
}

export function LibraryFilters({
  type,
  sort,
  search,
  activeTagFilter,
  onTypeChange,
  onSortChange,
  onSearchChange,
  onClearTagFilter,
}: LibraryFiltersProps) {
  return (
    <>
      <div className="lb-toolbar">
        <div className="lb-tabs" data-cut-library-tabs>
          {TYPE_TABS.map((tab) => (
            <button
              key={tab.key}
              className={`lb-tab ${type === tab.key ? 'lb-tab--on' : ''}`}
              data-cut-library-tab={tab.key}
              data-cut-on={type === tab.key}
              onClick={() => onTypeChange(tab.key)}
            >
              {tab.label}
            </button>
          ))}
        </div>
        <select
          className="lb-sort"
          data-cut-library-sort
          value={sort}
          onChange={(event) => onSortChange(sortKeyFromInput(event.target.value, sort))}
          title="Sort"
        >
          <option value="added">Newest</option>
          <option value="recent">Recently used</option>
          <option value="uses">Most used</option>
          <option value="name">Name</option>
        </select>
        <input
          className="lb-search"
          data-cut-library-search
          placeholder={'Search name or #tag\u2026'}
          value={search}
          onChange={(event) => onSearchChange(event.target.value)}
        />
      </div>

      {activeTagFilter && (
        <div className="lb-tagfilter" data-cut-library-tagfilter>
          <Icon name="bookmark" size={14} />
          <span>#{activeTagFilter}</span>
          <button className="lb-tagfilter-x" data-cut-library-tagfilter-clear title="Clear tag filter" onClick={onClearTagFilter}>
            <Icon name="close" size={14} label="Clear tag filter" />
          </button>
        </div>
      )}
    </>
  )
}
