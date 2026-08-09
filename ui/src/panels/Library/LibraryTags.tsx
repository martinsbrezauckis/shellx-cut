import type { LibItem } from '../../lib/client'

type SaveTagsAction = (item: LibItem, draft: string) => void | Promise<void>

export interface LibraryTagsProps {
  item: LibItem
  editing: boolean
  draft: string
  activeTagFilter: string | null
  className: string
  hideWhenEmpty?: boolean
  exposeTagsHook?: boolean
  onDraftChange: (value: string) => void
  onSave: SaveTagsAction
  onCancelEdit: () => void
  onToggleTagFilter: (tag: string) => void
}

export function LibraryTags({
  item,
  editing,
  draft,
  activeTagFilter,
  className,
  hideWhenEmpty = false,
  exposeTagsHook = false,
  onDraftChange,
  onSave,
  onCancelEdit,
  onToggleTagFilter,
}: LibraryTagsProps) {
  if (editing) {
    return (
      <input
        className="lb-taginput"
        data-cut-library-taginput
        autoFocus
        value={draft}
        placeholder="comma, separated, tags"
        onChange={(event) => onDraftChange(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === 'Enter') {
            event.preventDefault()
            void onSave(item, event.currentTarget.value)
          }
          if (event.key === 'Escape') onCancelEdit()
        }}
        onBlur={(event) => { void onSave(item, event.currentTarget.value) }}
      />
    )
  }

  if (item.tags.length === 0 && hideWhenEmpty) return null

  const hookProps = exposeTagsHook ? { 'data-cut-library-tags': true } : {}
  return (
    <div className={className} {...hookProps}>
      {item.tags.map((tag) => (
        <button
          className={`lb-tag ${activeTagFilter === tag ? 'lb-tag--on' : ''}`}
          data-cut-library-tag={tag}
          key={tag}
          title={`Filter by #${tag}`}
          onClick={() => onToggleTagFilter(tag)}
        >
          {tag}
        </button>
      ))}
    </div>
  )
}
