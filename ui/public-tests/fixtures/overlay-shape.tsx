import { useRef, useState } from 'react'
import { createRoot } from 'react-dom/client'
import { OverlayVideo } from '../../src/panels/Preview/PreviewLayers'
import { TransformHandles } from '../../src/panels/Preview/TransformHandles'
import type { ClipTransform } from '../../src/lib/client'
import '../../src/panels/Preview/preview.css'

function Fixture() {
  const stage = useRef<HTMLDivElement>(null)
  const params = new URLSearchParams(location.search)
  const shape = params.get('shape') as 'circle' | 'rounded' | 'rect'
  const proof = shape === 'rect'
  const sw = Number(params.get('sw') ?? 320), sh = Number(params.get('sh') ?? 240)
  const pw = Number(params.get('pw') ?? 960), ph = Number(params.get('ph') ?? 720)
  const sourceAspect = sw / sh
  const [transform, setTransform] = useState<ClipTransform & { opacity: number }>({ x: proof ? 0.6 : 0.2, y: proof ? 0.1 : 0.2, scale: proof ? 0.25 : 0.4, opacity: 1, shape, ...(!proof ? { ring: { width: 0.04, colour: '#ffffff' } } : {}) })
  return <div style={{ display: 'flex', gap: 8 }}>
    <div><p>Live preview</p><div data-rect-stage ref={stage} style={{ width: pw, height: ph, position: 'relative', background: 'blue', containerType: 'size', overflow: 'hidden' }}>
      <OverlayVideo rate={0} playheadMs={500} layer={{ clipId: 'camera', trackId: 'v2', src: proof ? `/rect-evidence/camera-${sw}x${sh}.png` : 'data:image/svg+xml,' + encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" width="320" height="240"><path fill="red" d="M0 0h320v240H0z"/></svg>'), kind: 'source', sourceAspect, assetId: 'camera', startMs: 0, srcInMs: 0, srcOutMs: 1000, durMs: 1000, speed: 1, reverse: false, isImage: true, transform }} />
      {(!proof || params.has('handles')) && <TransformHandles clipId="camera" transform={transform} sourceAspect={sourceAspect} stageRef={stage} onLive={setTransform} onCommit={setTransform} />}
    </div></div>
    {proof && <div><p>Exported frame</p><img data-rect-export src={`/rect-evidence/export-${sw}x${sh}-${pw}x${ph}-false.png`} width={pw} height={ph} /></div>}
  </div>
}
createRoot(document.getElementById('root')!).render(<Fixture />)
