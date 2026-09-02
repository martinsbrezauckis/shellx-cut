import { Suspense, lazy, type Dispatch, type RefObject, type SetStateAction } from 'react'
import type { OpRecord, Project, Transcript as TranscriptData } from '../lib/client'
import type { DoctorReport } from '../lib/doctor'
import { Icon } from '../icons'
import Divider from '../layout/Divider'
import type { LayoutState } from '../layout/useLayout'
import type { GenerateWorkspaceTab } from '../panels/GenerateTemplates'
import type { SourceNavigationState } from './useSourceNavigationController'
import type { StockImportCoordinator } from '../panels/Stock/importCoordinator'
import Preview from '../panels/Preview'
import Timeline from '../panels/Timeline'

const Comments = lazy(() => import('../panels/Comments'))
const RecordWorkspace = lazy(() => import('../panels/Record'))
const LeftPanel = lazy(() => import('../panels/LeftPanel'))
const LibraryWorkspace = lazy(() => import('../panels/Library/LibraryWorkspace'))

interface AppWorkspaceProps {
  layout: LayoutState
  setLayout: Dispatch<SetStateAction<LayoutState>>
  sourceNavigation: SourceNavigationState | null
  mainRef: RefObject<HTMLDivElement | null>
  splitRef: RefObject<HTMLDivElement | null>
  txWidth: string
  dragSplit: (clientX: number, clientY: number) => void
  dragTimeline: (clientX: number, clientY: number) => void
  project: Project | null
  projectScope: number
  doctor: DoctorReport | null
  ops: OpRecord[]
  transcripts: Record<string, TranscriptData>
  playheadMs: number
  selectedClipIds: string[]
  exportRange: [number, number] | null
  clipboardHasContent: boolean
  clipboardKind: 'video' | 'audio' | null
  clipboardClipId: string | null
  commentsOpen: boolean
  focusComment: { id: string; n: number } | null
  generateTab: GenerateWorkspaceTab
  onGenerateTab: (tab: GenerateWorkspaceTab) => void
  onCutWords: (asset: string, wordRange: [number, number], rationale?: string) => void
  onRestore: (opId: string) => void
  onSeek: (atMs: number) => void
  onSelect: (clipIds: string[]) => void
  onExportRange: (range: [number, number] | null) => void
  onCopyClip: (clipId: string) => boolean
  onCutClip: (clipId: string) => void
  onPasteClip: (target?: { atMs: number; trackId: string }) => void
  onCollapseComments: () => void
  onReopenProject: () => void
  onLibraryAddedToProject: () => void
  onRecordClipAdded: () => void
  onOpenOutputSettings: () => void
  /** App-lifetime owner for an in-flight Find-media import across workspace remounts. */
  stockImportCoordinator: StockImportCoordinator
}

function SurfaceLoading({ label = 'Loading' }: { label?: string }) {
  return (
    <div className="app__loading" data-cut-loading>
      {label}
    </div>
  )
}

export default function AppWorkspace({
  layout,
  setLayout,
  sourceNavigation,
  mainRef,
  splitRef,
  txWidth,
  dragSplit,
  dragTimeline,
  project,
  projectScope,
  doctor,
  ops,
  transcripts,
  playheadMs,
  selectedClipIds,
  exportRange,
  clipboardHasContent,
  clipboardKind,
  clipboardClipId,
  commentsOpen,
  focusComment,
  generateTab,
  onGenerateTab,
  onCutWords,
  onRestore,
  onSeek,
  onSelect,
  onExportRange,
  onCopyClip,
  onCutClip,
  onPasteClip,
  onCollapseComments,
  onReopenProject,
  onLibraryAddedToProject,
  onRecordClipAdded,
  onOpenOutputSettings,
  stockImportCoordinator,
}: AppWorkspaceProps) {
  const recordTimelineDeferred = layout.workspaceMode === 'record'

  return (
    <>
      {commentsOpen && layout.workspaceMode === 'edit' && (
        <div className="app__comments">
          <Suspense fallback={<SurfaceLoading />}>
            <Comments
              project={project}
              playheadMs={playheadMs}
              onSeek={onSeek}
              onCollapse={onCollapseComments}
              focus={focusComment}
            />
          </Suspense>
        </div>
      )}
      <div
        className="app__main"
        ref={mainRef}
        data-cut-record-timeline-deferred={recordTimelineDeferred}
        data-cut-record-timeline-compact={false}
      >
        {layout.workspaceMode === 'library' ? (
          <Suspense fallback={<SurfaceLoading label="Opening Library" />}>
            <LibraryWorkspace
              project={project}
              playheadMs={playheadMs}
              sourceNavigation={sourceNavigation?.destination === 'library' ? sourceNavigation : null}
              onAddedToProject={onLibraryAddedToProject}
              onClose={() => setLayout((current) => ({ ...current, workspaceMode: 'edit' }))}
            />
          </Suspense>
        ) : layout.workspaceMode === 'record' ? (
          <div className="app__split app__split--record">
            <Suspense fallback={<SurfaceLoading />}>
              <RecordWorkspace
                project={project}
                onClipAdded={onRecordClipAdded}
                onOpenOutputSettings={onOpenOutputSettings}
              />
            </Suspense>
          </div>
        ) : (
          <div className="app__split" ref={splitRef}>
            <div
              className={`app__transcript ${layout.leftCollapsed ? 'app__transcript--collapsed' : ''}`}
              style={layout.leftCollapsed ? undefined : { width: txWidth }}
            >
              <Suspense fallback={<SurfaceLoading />}>
                <LeftPanel
                  project={project}
                  projectScope={projectScope}
                  doctor={doctor}
                  ops={ops}
                  playheadMs={playheadMs}
                  transcripts={transcripts}
                  selectedClipId={selectedClipIds[0] ?? null}
                  onCutWords={onCutWords}
                  onRestore={onRestore}
                  onSeek={onSeek}
                  tab={layout.leftTab}
                  onTab={(t) => setLayout((l) => ({ ...l, leftTab: t }))}
                  findSurface={layout.findSurface}
                  onFindSurface={(s) => setLayout((l) => ({ ...l, findSurface: s }))}
                  generateTab={generateTab}
                  onGenerateTab={onGenerateTab}
                  onReopenProject={onReopenProject}
                  onProjectChanged={onLibraryAddedToProject}
                  sourceNavigation={sourceNavigation?.destination === 'project' ? sourceNavigation : null}
                  onCollapse={() => setLayout((l) => ({ ...l, leftCollapsed: true }))}
                  stockImportCoordinator={stockImportCoordinator}
                />
              </Suspense>
            </div>
            {layout.leftCollapsed ? (
              <button
                className="app__side-expand app__side-expand--left"
                data-cut-action="expand-left"
                onClick={() => setLayout((l) => ({ ...l, leftCollapsed: false }))}
                title="Show sidebar (Projects, Assets, Transcript, Generate, Find)"
                aria-label="Show sidebar"
              >
                <Icon name="collapseRight" size={14} />
                <span className="app__side-expand-label">Sidebar</span>
              </button>
            ) : (
              <Divider orient="v" id="transcript-preview" onDrag={dragSplit} />
            )}
            <div className="app__preview">
              <Preview
                project={project}
                doctor={doctor}
                playheadMs={playheadMs}
                onSeek={onSeek}
                headOpId={ops.length ? ops[ops.length - 1].op_id : ''}
                selectedClipIds={selectedClipIds}
                exportRange={exportRange}
              />
            </div>
          </div>
        )}
        {layout.workspaceMode !== 'library' && !recordTimelineDeferred && (
          <>
            <Divider orient="h" id="timeline" onDrag={dragTimeline} />
            <div className="app__timeline" style={{ height: layout.tlH }}>
              <Timeline
                project={project}
                playheadMs={playheadMs}
                selectedClipIds={selectedClipIds}
                headOpId={ops.length ? ops[ops.length - 1].op_id : ''}
                onSeek={onSeek}
                onSelect={onSelect}
                exportRange={exportRange}
                onExportRange={onExportRange}
                onCopyClip={onCopyClip}
                onCutClip={onCutClip}
                onPasteClip={onPasteClip}
                clipboardHasContent={clipboardHasContent}
                clipboardKind={clipboardKind}
                clipboardClipId={clipboardClipId}
              />
            </div>
          </>
        )}
      </div>
    </>
  )
}
