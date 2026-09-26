import { useState } from 'react'
import { createRoot } from 'react-dom/client'
import AppRightRail from '../../src/app/AppRightRail'
import { LAYOUT_DEFAULTS } from '../../src/layout/useLayout'
import type { Project } from '../../src/lib/client'

const project = { name: 'model-remount-fixture', tracks: [], assets: {} } as Project

function Fixture() {
  const [projectSession, setProjectSession] = useState(1)
  const [layout, setLayout] = useState({
    ...LAYOUT_DEFAULTS,
    railCollapsed: false,
    railPinned: true,
    rightTab: 'chat' as const,
  })
  return <>
    <button data-test-timeline onClick={() => setLayout(current => ({ ...current, railCollapsed: true }))}>Timeline</button>
    <button data-test-return-chat onClick={() => setLayout(current => ({ ...current, railCollapsed: false, rightTab: 'chat' }))}>Return to Chat</button>
    <button data-test-other-project onClick={() => setProjectSession(2)}>Other project</button>
    <button data-test-original-project onClick={() => setProjectSession(1)}>Original project</button>
    <AppRightRail
      layout={layout} setLayout={setLayout} dragRail={() => {}}
      project={project} projectSession={projectSession} doctor={null} ops={[]} receipts={[]}
      selectedClipId={null} playheadMs={0} onSeek={() => {}}
      agentChatPrefill={null} onUndo={() => {}} onRedo={() => {}}
    />
  </>
}

createRoot(document.getElementById('root')!).render(<Fixture />)
