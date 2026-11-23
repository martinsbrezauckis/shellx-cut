import { Icon } from '../../icons'
import { Availability } from '../../lib/availability'

export interface LibraryBrowseControlProps {
  busy: string | null
  onBrowse: () => void
}

/** Desktop-only native file chooser for adding Library items, with a browser explanation. */
export default function LibraryBrowseControl({ busy, onBrowse }: LibraryBrowseControlProps) {
  return (
    <Availability
      rule="shell.desktop"
      replacement={
        <span className="cd-note" data-cut-library-desktop-note>
          Importing files needs the desktop app.
        </span>
      }
    >
      <button
        className="lb-browse"
        data-cut-library-browse
        data-cut-library-addbtn
        data-cut-action="library-browse"
        disabled={busy === 'add'}
        onClick={onBrowse}
      >
        {busy === 'add' ? (
          'Adding…'
        ) : (
          <>
            <Icon name="import" size={14} tone="asset" /> Browse files…
          </>
        )}
      </button>
    </Availability>
  )
}
