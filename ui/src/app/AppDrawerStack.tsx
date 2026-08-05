import { Suspense, lazy } from 'react'
import type { Project } from '../lib/client'

const MusicBed = lazy(() => import('../panels/MusicBed'))
const MatteDrawer = lazy(() => import('../panels/Matte'))
const ShapeDrawer = lazy(() => import('../panels/Shape'))
const LayerDrawer = lazy(() => import('../panels/Layer'))
const KineticDrawer = lazy(() => import('../panels/Kinetic'))
const ClipsDrawer = lazy(() => import('../panels/Clips'))
const AutopilotDrawer = lazy(() => import('../panels/Autopilot'))
const AssembleDrawer = lazy(() => import('../panels/Assemble'))
const RecipesDrawer = lazy(() => import('../panels/Recipes'))
const MaskDrawer = lazy(() => import('../panels/Mask'))
const TitleDrawer = lazy(() => import('../panels/Title'))

export type AppDrawer =
  | 'music'
  | 'title'
  | 'kinetic'
  | 'layer'
  | 'clips'
  | 'autopilot'
  | 'assemble'
  | 'recipes'
  | 'matte'
  | 'shape'
  | 'mask'

interface AppDrawerStackProps {
  activeDrawer: AppDrawer | null
  project: Project | null
  selectedClipId: string | null
  playheadMs: number
  onSeek: (atMs: number) => void
  onProjectSwitched: () => void | Promise<void>
  onClose: () => void
}

function SurfaceLoading({ label = 'Loading' }: { label?: string }) {
  return <div className="app__loading" data-cut-loading>{label}</div>
}

export default function AppDrawerStack({
  activeDrawer,
  project,
  selectedClipId,
  playheadMs,
  onSeek,
  onProjectSwitched,
  onClose,
}: AppDrawerStackProps) {
  return (
    <>
      {activeDrawer === 'music' && (
        <Suspense fallback={<SurfaceLoading />}>
          <MusicBed project={project} onClose={onClose} />
        </Suspense>
      )}
      {activeDrawer === 'title' && (
        <Suspense fallback={<SurfaceLoading />}>
          <TitleDrawer project={project} defaultInMs={playheadMs} onClose={onClose} />
        </Suspense>
      )}
      {activeDrawer === 'kinetic' && (
        <Suspense fallback={<SurfaceLoading />}>
          <KineticDrawer project={project} onClose={onClose} />
        </Suspense>
      )}
      {activeDrawer === 'matte' && (
        <Suspense fallback={<SurfaceLoading />}>
          <MatteDrawer project={project} clipId={selectedClipId} playheadMs={playheadMs} onClose={onClose} />
        </Suspense>
      )}
      {activeDrawer === 'shape' && (
        <Suspense fallback={<SurfaceLoading />}>
          <ShapeDrawer project={project} defaultInMs={playheadMs} onClose={onClose} />
        </Suspense>
      )}
      {activeDrawer === 'layer' && (
        <Suspense fallback={<SurfaceLoading />}>
          <LayerDrawer project={project} clipId={selectedClipId} onClose={onClose} />
        </Suspense>
      )}
      {activeDrawer === 'clips' && (
        <Suspense fallback={<SurfaceLoading />}>
          <ClipsDrawer onClose={onClose} />
        </Suspense>
      )}
      {activeDrawer === 'autopilot' && (
        <Suspense fallback={<SurfaceLoading />}>
          <AutopilotDrawer onClose={onClose} />
        </Suspense>
      )}
      {activeDrawer === 'recipes' && (
        <Suspense fallback={<SurfaceLoading />}>
          <RecipesDrawer project={project} onProjectSwitched={onProjectSwitched} onClose={onClose} />
        </Suspense>
      )}
      {activeDrawer === 'mask' && (
        <Suspense fallback={<SurfaceLoading />}>
          <MaskDrawer
            project={project}
            clipId={selectedClipId}
            playheadMs={playheadMs}
            onSeek={onSeek}
            onClose={onClose}
          />
        </Suspense>
      )}
      {activeDrawer === 'assemble' && (
        <Suspense fallback={<SurfaceLoading />}>
          <AssembleDrawer project={project} playheadMs={playheadMs} onSeek={onSeek} onClose={onClose} />
        </Suspense>
      )}
    </>
  )
}
