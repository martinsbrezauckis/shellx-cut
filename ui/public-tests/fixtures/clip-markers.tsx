import { useRef, useState } from 'react'
import { createRoot } from 'react-dom/client'
import '../../src/theme.css'
import '../../src/panels/Timeline/timeline.css'
import { OfflineMediaContext } from '../../src/app/OfflineMediaContext'
import ClipView from '../../src/panels/Timeline/ClipView'
import { layoutTrack } from '../../src/panels/Timeline/layout'
import type { Project } from '../../src/lib/client'

function Fixture() {
  const params = new URLSearchParams(location.search)
  const theme = params.get('theme') === 'light' ? 'light' : 'dark'
  document.documentElement.setAttribute('data-theme', theme)
  const [trimLeftMs, setTrimLeftMs] = useState(params.get('mode') === 'trimmed' ? 2_000 : 0)
  const [splitAtMs, setSplitAtMs] = useState<number | null>(params.get('mode') === 'split' ? 3_000 : null)
  const [selectedId, setSelectedId] = useState<string | null>(null)
  const [dragCount, setDragCount] = useState(0)
  const dragStartX = useRef<number | null>(null)

  const clips = splitAtMs === null
    ? [{
        id: 'rec-clip-1',
        asset: 'Take 1 (paused + 2 markers)',
        src_in_ms: trimLeftMs,
        src_out_ms: 6_000,
        markers: [
          { id: 'cm1', source_ms: 1_500, label: 'Intro highlight' },
          { id: 'cm2', source_ms: 4_200, label: 'Key takeaway' },
        ],
      }]
    : [
        {
          id: 'rec-clip-1',
          asset: 'Take 1 (left)',
          src_in_ms: 0,
          src_out_ms: splitAtMs,
          markers: [
            { id: 'cm1', source_ms: 1_500, label: 'Intro highlight' },
            { id: 'cm2', source_ms: 4_200, label: 'Key takeaway' },
          ],
        },
        {
          id: 'rec-clip-2',
          asset: 'Take 1 (right)',
          src_in_ms: splitAtMs,
          src_out_ms: 6_000,
          markers: [
            { id: 'cm1', source_ms: 1_500, label: 'Intro highlight' },
            { id: 'cm2', source_ms: 4_200, label: 'Key takeaway' },
          ],
        },
      ]

  const project = {
    tracks: [{ id: 'v1', kind: 'video', clips }],
  } as unknown as Project
  const items = layoutTrack(project.tracks[0])

  return (
    <div
      style={{
        padding: 20,
        background: 'var(--bg)',
        color: 'var(--ink)',
        fontFamily: 'var(--sans)',
        minHeight: '100vh',
      }}
    >
      <div style={{ marginBottom: 12, display: 'flex', gap: 8, alignItems: 'center', fontSize: 13 }}>
        <strong>Clip markers ({theme})</strong>
        <button type="button" data-cut-test="trim" onClick={() => { setSplitAtMs(null); setTrimLeftMs(2_000) }}>Trim 2s</button>
        <button type="button" data-cut-test="untrim" onClick={() => { setSplitAtMs(null); setTrimLeftMs(0) }}>Untrim</button>
        <button type="button" data-cut-test="split" onClick={() => { setTrimLeftMs(0); setSplitAtMs(3_000) }}>Split 3s</button>
        <span data-cut-test="selected">{selectedId ?? 'none'}</span>
        <span data-cut-test="drags">{dragCount}</span>
      </div>
      <div
        className="tl-lane"
        data-cut-track="v1"
        style={{
          width: 720,
          height: 52,
          position: 'relative',
          border: '1px solid var(--hairline)',
          background: 'var(--surface)',
        }}
      >
        <OfflineMediaContext.Provider
          value={{ offlineAssets: new Set<string>(), relinkingAssets: new Set<string>(), onRelinkAsset: async () => {} }}
        >
          {items.map((item) => (
            <ClipView
              key={item.id}
              item={item}
              zoom={2}
              fps={30}
              selected={selectedId === item.id}
              dragging={false}
              locked={false}
              onClipDown={(e, it) => {
                setSelectedId(it.id)
                dragStartX.current = e.clientX
                const onMove = (ev: MouseEvent) => {
                  if (dragStartX.current !== null && Math.abs(ev.clientX - dragStartX.current) >= 4) {
                    setDragCount((c) => c + 1)
                    window.removeEventListener('mousemove', onMove)
                  }
                }
                const onUp = () => {
                  window.removeEventListener('mousemove', onMove)
                  window.removeEventListener('mouseup', onUp)
                }
                window.addEventListener('mousemove', onMove)
                window.addEventListener('mouseup', onUp)
              }}
              onRelinkAsset={async () => {}}
            />
          ))}
        </OfflineMediaContext.Provider>
      </div>
    </div>
  )
}

createRoot(document.getElementById('root')!).render(<Fixture />)
