import type { LibItem } from '../../lib/client'
import { LibraryPoster } from './LibraryPoster'
import { shortDur } from './model'

export interface LibraryDetailsProps {
  item: LibItem | null
  selectedCount: number
  failedPoster: boolean
  inProject: boolean
  onPosterFail: (id: string) => void
}

export function LibraryDetails({
  item,
  selectedCount,
  failedPoster,
  inProject,
  onPosterFail,
}: LibraryDetailsProps) {
  if (!item) {
    return (
      <aside className="lb-details" aria-label="Library item details" data-cut-library-details>
        <p className="lb-details__eyebrow">Details</p>
        <h3>{selectedCount > 1 ? `${selectedCount} items selected` : 'Choose media'}</h3>
        <p>
          {selectedCount > 1
            ? 'Use the bulk action bar to organize or add the selected media.'
            : 'Click an item to inspect its source, portability, tags, and project status. Use checkboxes for bulk actions.'}
        </p>
      </aside>
    )
  }

  const dimensions = item.probe?.width && item.probe?.height
    ? `${item.probe.width} × ${item.probe.height}`
    : 'Not measured'
  const location = item.blob
    ? 'Managed copy in Library'
    : item.src_path ?? 'Source location unavailable'

  return (
    <aside className="lb-details" aria-label={`${item.name} details`} data-cut-library-details={item.id}>
      <p className="lb-details__eyebrow">Details</p>
      <div className={`lb-details__poster lb-thumb--${item.type}`}>
        <LibraryPoster item={item} failed={failedPoster} onFail={onPosterFail} />
      </div>
      <h3 title={item.name}>{item.name}</h3>
      <dl>
        <div>
          <dt>Status</dt>
          <dd>{item.media_ok === false ? item.blob ? 'Managed copy missing' : 'Linked source missing' : inProject ? 'In this project' : 'Ready to add'}</dd>
        </div>
        <div>
          <dt>Storage</dt>
          <dd>{item.blob ? 'Managed copy' : 'Linked source'}</dd>
        </div>
        <div>
          <dt>Type</dt>
          <dd>{item.type}</dd>
        </div>
        <div>
          <dt>Duration</dt>
          <dd>{shortDur(item.probe?.duration_ms) || 'Not applicable'}</dd>
        </div>
        <div>
          <dt>Dimensions</dt>
          <dd>{dimensions}</dd>
        </div>
        <div>
          <dt>Folder</dt>
          <dd>{item.folder ?? 'All media'}</dd>
        </div>
        <div>
          <dt>Used</dt>
          <dd>{item.uses ?? 0} time{item.uses === 1 ? '' : 's'}</dd>
        </div>
      </dl>
      <div className="lb-details__location">
        <span>Location</span>
        <code title={location}>{location}</code>
      </div>
      <div className="lb-details__tags">
        <span>Tags</span>
        <p>{item.tags.length ? item.tags.map((tag) => `#${tag}`).join(' ') : 'No tags yet'}</p>
      </div>
    </aside>
  )
}
