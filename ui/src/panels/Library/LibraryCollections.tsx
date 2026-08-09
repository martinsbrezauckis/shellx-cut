import type { LibraryCollection } from './model'

export interface LibraryCollectionsProps {
  active: LibraryCollection
  /** Only render globally truthful collection counts. Page-local counts are hidden. */
  favoriteCount?: number
  missingCount?: number
  tags: string[]
  activeTag: string | null
  allMediaActive: boolean
  onSelect: (collection: LibraryCollection) => void
  onSelectTag: (tag: string) => void
}

const COLLECTIONS: Array<{ id: LibraryCollection; label: string }> = [
  { id: 'all', label: 'All media' },
  { id: 'recent', label: 'Recent' },
  { id: 'favorites', label: 'Favorites' },
  { id: 'missing', label: 'Missing' },
]

export function LibraryCollections({
  active,
  favoriteCount,
  missingCount,
  tags,
  activeTag,
  allMediaActive,
  onSelect,
  onSelectTag,
}: LibraryCollectionsProps) {
  const countFor = (id: LibraryCollection) => {
    if (id === 'favorites') return favoriteCount
    if (id === 'missing') return missingCount
    return undefined
  }

  return (
    <div className="lb-collections" data-cut-library-collections>
      {COLLECTIONS.map(({ id, label }) => {
        const count = countFor(id)
        const pressed = active === id && !activeTag && (id !== 'all' || allMediaActive)
        return (
          <button
            type="button"
            key={id}
            className={`lb-collection ${pressed ? 'lb-collection--active' : ''}`}
            data-cut-library-collection={id}
            aria-pressed={pressed}
            onClick={() => onSelect(id)}
          >
            <span>{label}</span>
            {count !== undefined && <small>{count}</small>}
          </button>
        )
      })}
      {tags.length > 0 && (
        <div className="lb-collection-tags">
          <p>Tags</p>
          {tags.map((tag) => (
            <button
              type="button"
              key={tag}
              className={`lb-collection lb-collection--tag ${activeTag === tag ? 'lb-collection--active' : ''}`}
              data-cut-library-collection-tag={tag}
              aria-pressed={activeTag === tag}
              onClick={() => onSelectTag(tag)}
            >
              <span>#{tag}</span>
            </button>
          ))}
        </div>
      )}
    </div>
  )
}
