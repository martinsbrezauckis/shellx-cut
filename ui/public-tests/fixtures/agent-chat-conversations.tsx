import React, { useEffect, useState } from 'react'
import { createRoot } from 'react-dom/client'
import '../../src/theme.css'
import AppRightRail from '../../src/app/AppRightRail'
import { LAYOUT_DEFAULTS, type Layout } from '../../src/layout/useLayout'
import { events } from '../../src/lib/events'
import type { DoctorReport, Project } from '../../src/lib/client'

const project = {
  version: '0.6.122',
  name: 'Chat Conversations Fixture',
  settings: { width: 1280, height: 720, fps: 30, audio_rate: 48000 },
  assets: {},
  tracks: [{ id: 'v1', kind: 'video', clips: [] }],
  markers: [],
  captions: {},
  ignored_transcripts: [],
  project_identity: {
    schema: 'shellx-cut/project-identity/1',
    origin_path_sha256: `sha256:${'a'.repeat(64)}`,
    project_name: 'Chat Conversations Fixture',
  },
} as unknown as Project

const doctor: DoctorReport = {
  schema: 'shellx-cut/doctor/1',
  scanned_at: '2026-11-24T00:00:00Z',
  os: 'linux',
  arch: 'x86_64',
  app_version: '0.6.122',
  essential_ok: true,
  cards: [
    {
      id: 'claude',
      kind: 'judge',
      status: 'ok',
      details: { found: true, installed: true, chat: { ready: true, status: 'ok' } },
    },
  ],
}

function Fixture() {
  const [layout, setLayout] = useState<Layout>({
    ...LAYOUT_DEFAULTS,
    railCollapsed: false,
    rightTab: 'chat',
  })

  useEffect(() => {
    events.connect()
  }, [])

  return (
    <div style={{ width: 420, height: 780, display: 'flex' }}>
      <AppRightRail
        layout={layout}
        setLayout={setLayout}
        dragRail={() => {}}
        project={project}
        projectSession={1}
        doctor={doctor}
        ops={[]}
        receipts={[]}
        selectedClipId={null}
        playheadMs={0}
        onSeek={() => {}}
        agentChatPrefill={null}
        onUndo={() => {}}
        onRedo={() => {}}
      />
    </div>
  )
}

const rootEl = document.getElementById('root')
if (rootEl) createRoot(rootEl).render(<Fixture />)
