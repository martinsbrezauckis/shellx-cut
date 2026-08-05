import { Icon } from '../../icons'
import type { LibItem } from '../../lib/client'
import { posterSrc } from './model'

export interface LibraryPosterProps {
  item: LibItem
  failed: boolean
  onFail: (id: string) => void
}

function KindGlyph({ kind }: { kind: string }) {
  if (kind === 'audio') return <Icon name="waveform" size={20} />
  if (kind === 'image') return <Icon name="image" size={20} />
  return <Icon name="video" size={20} />
}

export function LibraryPoster({ item, failed, onFail }: LibraryPosterProps) {
  const src = posterSrc(item)
  if (failed || !src) {
    return <div className="lb-thumb-glyph"><KindGlyph kind={item.type} /></div>
  }

  return (
    <img
      className="lb-thumb-img"
      src={src}
      alt={item.name}
      loading="lazy"
      draggable={false}
      onDragStart={(event) => event.preventDefault()}
      onError={() => onFail(item.id)}
    />
  )
}
