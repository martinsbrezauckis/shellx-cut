import { useRef, useState } from 'react'
import { createRoot } from 'react-dom/client'
import { OverlayVideo } from '../../src/panels/Preview/PreviewLayers'
import { TransformHandles } from '../../src/panels/Preview/TransformHandles'
import type { ClipTransform } from '../../src/lib/client'
import '../../src/panels/Preview/preview.css'

function Fixture() {
  const stage = useRef<HTMLDivElement>(null)
  const shape = new URLSearchParams(location.search).get('shape') as 'circle' | 'rounded'
  const [transform, setTransform] = useState<ClipTransform & { opacity: number }>({ x: 0.2, y: 0.2, scale: 0.4, opacity: 1, shape, ring: { width: 0.04, colour: '#ffffff' } })
  return <div ref={stage} style={{ width: 960, height: 720, position: 'relative', background: 'green', containerType: 'size', overflow: 'hidden' }}>
    <OverlayVideo rate={0} playheadMs={500} layer={{ clipId: 'camera', trackId: 'v2', src: 'data:image/svg+xml,' + encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" width="320" height="240"><path fill="red" d="M0 0h320v240H0z"/></svg>'), kind: 'source', assetId: 'camera', startMs: 0, srcInMs: 0, srcOutMs: 1000, durMs: 1000, speed: 1, reverse: false, isImage: true, transform }} />
    <TransformHandles clipId="camera" transform={transform} stageRef={stage} onLive={setTransform} onCommit={setTransform} />
  </div>
}
createRoot(document.getElementById('root')!).render(<Fixture />)
