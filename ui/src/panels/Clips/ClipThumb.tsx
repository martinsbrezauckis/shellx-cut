import { useEffect, useState } from 'react'
import { Icon } from '../../icons'
import { scopeMediaUrl } from '../../lib/mediaScope'
import { takeThumbTurn } from './thumbTurns'

/** One suggestion's frame. It keeps the placeholder icon when the frame cannot be loaded. */
export default function ClipThumb({ atMs }: { atMs: number }) {
  const [src, setSrc] = useState<string | null>(null)
  const [failed, setFailed] = useState(false)
  const [release, setRelease] = useState<(() => void) | null>(null)

  useEffect(() => {
    let cancelled = false
    const turn = takeThumbTurn()
    setSrc(null)
    setFailed(false)
    setRelease(() => turn.release)
    void turn.ready.then(() => {
      if (!cancelled) setSrc(scopeMediaUrl(`/api/frame?at_ms=${atMs}&h=120`))
    })
    return () => {
      cancelled = true
      turn.release()
    }
  }, [atMs])

  return (
    <div className="clip-thumb-wrap" data-cut-clip-thumb-wrap data-cut-thumb-error={failed ? 'true' : undefined}>
      <span className="clip-thumb-fallback" data-cut-clip-thumb-fallback aria-hidden="true">
        <Icon name="image" size={20} />
      </span>
      {src && (
        <img
          className="clip-thumb"
          data-cut-clip-thumb
          alt={`frame at ${atMs}ms`}
          src={src}
          hidden={failed}
          onLoad={() => release?.()}
          onError={() => { setFailed(true); release?.() }}
        />
      )}
    </div>
  )
}
