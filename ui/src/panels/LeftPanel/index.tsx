// panels/LeftPanel — the tabbed left sidebar (Projects | Assets | Transcript | Generate | Find).
// Role: hosts the left-rail surfaces under a tab strip so they SHARE the
// sidebar width instead of competing for it (transcript + assets remain
// tabs in the left sidebar"). Both panes stay MOUNTED and are toggled with
// display:none — switching tabs never loses transcript scroll/search/reel state
// or the Assets selection. The Assets tab carries a live count badge so imports
// arriving while the user is on the Transcript tab are visible immediately.
// The strip also owns the sidebar COLLAPSE button (left half of the "collapse
// both side panels" request; the right rail's lives in Review's header).
// Callers: App.tsx. Dependencies: Transcript, Assets panels + lib/client types.

import { useEffect } from 'react'
import type { OpRecord, Project, Transcript as TranscriptData } from '../../lib/client'
import type { DoctorReport } from '../../lib/doctor'
import type { LeftTab, FindSurface } from '../../layout/useLayout'
import Transcript from '../Transcript'
import Assets from '../Assets'
import GenerateTemplatesWorkspace, { type GenerateWorkspaceTab } from '../GenerateTemplates'
import ProjectsPanel from '../Projects'
import StockDrawer from '../Stock'
import SearchDrawer from '../Search'
import SequenceIndex from '../SequenceIndex'
import { Icon } from '../../icons'
import './leftpanel.css'

export interface LeftPanelProps {
  project: Project | null
  doctor: DoctorReport | null
  /** App-owned complete durable history; shared with Review and Transcript. */
  ops: OpRecord[]
  playheadMs: number
  transcripts: Record<string, TranscriptData>
  /** The currently-selected clip (Timeline) — drives the Transcript SELECTED-CLIP view. */
  selectedClipId: string | null
  onCutWords: (asset: string, wordRange: [number, number], rationale?: string) => void
  onRestore: (opId: string) => void
  onSeek?: (atMs: number) => void
  /** Active tab (persisted in layout). */
  tab: LeftTab
  onTab: (t: LeftTab) => void
  /** Which Find surface the Find tab shows, + its setter. */
  findSurface: FindSurface
  onFindSurface: (s: FindSurface) => void
  /** Which native Generate sub-surface is visible. */
  generateTab: GenerateWorkspaceTab
  onGenerateTab: (tab: GenerateWorkspaceTab) => void
  /** Hard-reset + reload after a project reopen/create (App.onProjectSwitched). */
  onReopenProject: () => void
  /** Refresh after a child surface mutates project state (App.resync). */
  onProjectChanged: () => void
  /** Collapse the whole sidebar (App hides it, shows the expand strip). */
  onCollapse: () => void
}

export default function LeftPanel({
  project,
  doctor,
  ops,
  playheadMs,
  transcripts,
  selectedClipId,
  onCutWords,
  onRestore,
  onSeek,
  tab,
  onTab,
  findSurface,
  onFindSurface,
  generateTab,
  onGenerateTab,
  onReopenProject,
  onProjectChanged,
  onCollapse,
}: LeftPanelProps) {
  const assetCount = Object.keys(project?.assets ?? {}).length

  // Find Moment and Sequence Index both open media through the shared Source
  // Monitor event. Assets stays mounted so it can receive the payload; reveal
  // its tab as part of the same event so the resulting monitor is not hidden
  // behind the Find pane.
  useEffect(() => {
    const revealSourceMonitor = () => onTab('assets')
    document.addEventListener('cut:open-source-monitor', revealSourceMonitor)
    return () => document.removeEventListener('cut:open-source-monitor', revealSourceMonitor)
  }, [onTab])

  return (
    <div className="leftpanel" data-cut-leftpanel>
      <div className="lp__tabs" role="tablist" aria-label="Sidebar">
        <button
          role="tab"
          aria-selected={tab === 'projects'}
          className={`lp__tab ${tab === 'projects' ? 'lp__tab--active' : ''}`}
          data-cut-left-tab="projects"
          onClick={() => onTab('projects')}
        >
          Projects
        </button>
        <button
          role="tab"
          aria-selected={tab === 'assets'}
          className={`lp__tab ${tab === 'assets' ? 'lp__tab--active' : ''}`}
          data-cut-left-tab="assets"
          onClick={() => onTab('assets')}
        >
          Assets
          {assetCount > 0 && <span className="lp__tab-badge" data-cut-asset-badge={assetCount}>{assetCount}</span>}
        </button>
        <button
          role="tab"
          aria-selected={tab === 'transcript'}
          className={`lp__tab ${tab === 'transcript' ? 'lp__tab--active' : ''}`}
          data-cut-left-tab="transcript"
          onClick={() => onTab('transcript')}
        >
          Transcript
        </button>
        <button
          role="tab"
          aria-selected={tab === 'generate'}
          className={`lp__tab ${tab === 'generate' ? 'lp__tab--active' : ''}`}
          data-cut-left-tab="generate"
          onClick={() => onTab('generate')}
        >
          Generate
        </button>
        <button
          role="tab"
          aria-selected={tab === 'find'}
          className={`lp__tab ${tab === 'find' ? 'lp__tab--active' : ''}`}
          data-cut-left-tab="find"
          onClick={() => onTab('find')}
        >
          Find
        </button>
        <span className="lp__tabs-spacer" />
        <button
          className="lp__collapse"
          data-cut-action="collapse-left"
          onClick={onCollapse}
          title="Collapse sidebar"
          aria-label="Collapse sidebar"
        >
          <Icon name="collapseLeft" size={14} />
        </button>
      </div>

      <div className="lp__body">
        {/* Both panes stay mounted; display toggles so neither loses its state. */}
        <div className="lp__pane" style={{ display: tab === 'transcript' ? 'flex' : 'none' }}>
          <Transcript
            project={project}
            ops={ops}
            playheadMs={playheadMs}
            transcripts={transcripts}
            selectedClipId={selectedClipId}
            onCutWords={onCutWords}
            onRestore={onRestore}
            onSeek={onSeek}
            onProjectChanged={onProjectChanged}
          />
        </div>
        <div className="lp__pane" style={{ display: tab === 'assets' ? 'flex' : 'none' }}>
          <Assets project={project} doctor={doctor} playheadMs={playheadMs} />
        </div>
        <div className="lp__pane lp__pane--generate" style={{ display: tab === 'generate' ? 'flex' : 'none' }}>
          <GenerateTemplatesWorkspace
            project={project}
            playheadMs={playheadMs}
            selectedClipId={selectedClipId}
            onInserted={onProjectChanged}
            activeTab={generateTab}
            onTab={onGenerateTab}
          />
        </div>
        <div className="lp__pane" style={{ display: tab === 'projects' ? 'flex' : 'none' }}>
          <ProjectsPanel onReopen={onReopenProject} currentName={project?.name ?? null} active={tab === 'projects'} />
        </div>
        {/* The Find pane — a sub-toggle over the embedded media, moment, and
            cross-sequence metadata search surfaces. The tab is permanent; its body mounts only
            when active so embedded search inputs do not steal focus while hidden.
            Generated-media placement: Generate used to be a third Find sub-tab, but it CREATES media
            (assets.generate) rather than searching — it is now a permanent left tab
            beside project-local Assets. */}
        {tab === 'find' ? (
          <div className="lp__pane lp__pane--find" style={{ display: 'flex' }}>
            <div className="lp__subtabs" role="tablist" aria-label="Find" data-cut-find-surface={findSurface}>
              <button
                role="tab"
                aria-selected={findSurface === 'find-media'}
                className={`lp__subtab ${findSurface === 'find-media' ? 'lp__subtab--active' : ''}`}
                data-cut-find-tab="find-media"
                onClick={() => onFindSurface('find-media')}
              >
                <Icon name="search" size={14} tone="brand" /> Find media
              </button>
              <button
                role="tab"
                aria-selected={findSurface === 'find-moment'}
                className={`lp__subtab ${findSurface === 'find-moment' ? 'lp__subtab--active' : ''}`}
                data-cut-find-tab="find-moment"
                onClick={() => onFindSurface('find-moment')}
              >
                <Icon name="search" size={14} tone="asset" /> Find moment
              </button>
              <button
                role="tab"
                aria-selected={findSurface === 'sequence-index'}
                className={`lp__subtab ${findSurface === 'sequence-index' ? 'lp__subtab--active' : ''}`}
                data-cut-find-tab="sequence-index"
                title="Search clips and markers across every sequence"
                onClick={() => onFindSurface('sequence-index')}
              >
                <Icon name="list" size={14} tone="brand" /> Sequence
              </button>
            </div>
            <div className="lp__find-body">
              {findSurface === 'find-media'
                ? <StockDrawer project={project} />
                : findSurface === 'find-moment'
                  ? <SearchDrawer project={project} playheadMs={playheadMs} />
                  : <SequenceIndex project={project} onProjectChanged={onProjectChanged} />}
            </div>
          </div>
        ) : null}
      </div>
    </div>
  )
}
