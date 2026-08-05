// panels/Inspector — the context-revealed Inspector with the highest-value
// pattern). ONE right-side panel that RESHAPES to the selection: no selection →
// project properties; a video clip → Transform/Crop/Speed/Grade/Effects/Fade; an
// audio clip → Level/Fades. It surfaces quick inline facts + actions and launches
// the matching deep-edit drawer (the 12 drawers stay as the focused surfaces; the
// Inspector is the selection-aware launcher + at-a-glance state that replaces
// hunting for the right drawer).
//
// Non-breaking: dispatches the existing `cut:open-drawer` events the drawers
// already listen to, and a couple of one-shot verbs (edit.fade, edit.speed) for
// quick actions. Zero local mutation beyond firing verbs. Callers: App (Edit-mode
// right rail). Deps: lib/client (verbs + Clip shape).

import { useMemo } from 'react'
import type { Project } from '../../lib/client'
import type { DoctorReport } from '../../lib/doctor'
import { baseVideoTrackId, isTrackLocked } from '../../lib/layerStack'
import AudioInspectorTools from './AudioInspectorTools'
import CaptionEditSection from './CaptionEditSection'
import ClipActionsSection from './ClipActionsSection'
import CroppingSection from './CroppingSection'
import EngagementSection from './EngagementSection'
import FadesSection from './FadesSection'
import MotionLinkSection from './MotionLinkSection'
import ProjectCaptionsSection from './ProjectCaptionsSection'
import ShapeEditSection from './ShapeEditSection'
import SpeedSection from './SpeedSection'
import TitleEditSection from './TitleEditSection'
import TrackLockGate from './TrackLockGate'
import TransformSection from './TransformSection'
import VolumeSection from './VolumeSection'
import VideoInspectorTools from './VideoInspectorTools'
import {
  blendModeFromInput,
  fmtDur,
  isRangeMs,
  sourceDims,
  type CaptionSelectionClip,
  type InspectorMediaClip,
  type ShapeSelectionClip,
  type TitleSelectionClip,
} from './model'
import { useInspectorAutoVideoControls } from './useInspectorAutoVideoControls'
import { useInspectorClipActions } from './useInspectorClipActions'
import { useInspectorColorControls } from './useInspectorColorControls'
import './inspector.css'
import './inspectorTasks.css'
import './motion.css'
export interface InspectorProps {
  project: Project | null
  /** The selected clip id (Timeline selection). */
  selectedClipId: string | null
  /** Live playhead (timeline ms) — the start anchor for a placed caption card. */
  playheadMs?: number
  /** Installed capability truth used to explain or enable environment-dependent actions. */
  doctor: DoctorReport | null
}
/** Open a drawer the App already mounts (the generic open-drawer event). */
function openDrawer(name: string) {
  document.dispatchEvent(new CustomEvent('cut:open-drawer', { detail: name }))
}

export default function Inspector({ project, selectedClipId, playheadMs = 0, doctor }: InspectorProps) {
  // Resolve the selected clip + its track kind from project state.
  const sel = useMemo(() => {
    if (!project || !selectedClipId) return null
    for (const t of project.tracks ?? []) {
      if (t.kind !== 'video' && t.kind !== 'audio') continue
      for (const c of t.clips ?? []) {
        if ('id' in c && c.id === selectedClipId && 'asset' in c) {
          return { clip: c as InspectorMediaClip, trackKind: t.kind, trackId: t.id }
        }
      }
    }
    return null
  }, [project, selectedClipId])
  const colorControls = useInspectorColorControls({ project, sel })
  const autoVideoControls = useInspectorAutoVideoControls({ project, sel })

  // A selected CAPTION clip — resolved SEPARATELY because captions carry no asset
  // (the `sel` resolver above requires one), so without this they fell through to
  // the empty project view and the right-click "Edit text & style…" dead-ended.
  // When set, the body renders the per-caption text editor below.
  const capSel = useMemo(() => {
    if (!project || !selectedClipId) return null
    for (const t of project.tracks ?? []) {
      if (t.kind !== 'caption') continue
      for (const c of t.clips ?? []) {
        if ('id' in c && c.id === selectedClipId && 'text' in c) {
          if (typeof c.id !== 'string' || typeof c.text !== 'string' || !('range_ms' in c) || !isRangeMs(c.range_ms)) continue
          const clip: CaptionSelectionClip = {
            id: c.id,
            text: c.text,
            range_ms: c.range_ms,
            style_ref: 'style_ref' in c && typeof c.style_ref === 'string' ? c.style_ref : undefined,
          }
          return {
            clip,
            trackId: t.id,
          }
        }
      }
    }
    return null
  }, [project, selectedClipId])

  // A selected TITLE clip — a media clip on a `title*` overlay track that carries
  // the transient `title_text` annotation (project.state adds it for title.add-
  // created titles, recovered from the op-log). Resolved SEPARATELY so it gets the
  // in-place text editor instead of the generic "Video clip" inspector, which
  // had no way to change the title's words. Kinetic-caption titles have no
  // title_text → they fall through to the normal inspector (their text is the
  // transcript, not a single editable field). Takes precedence over `sel` below.
  const titleSel = useMemo(() => {
    if (!project || !selectedClipId) return null
    for (const t of project.tracks ?? []) {
      if (!t.id?.startsWith('title')) continue
      for (const c of t.clips ?? []) {
        if (
          'id' in c &&
          c.id === selectedClipId &&
          'title_text' in c &&
          typeof c.title_text === 'string'
        ) {
          if (typeof c.id !== 'string') continue
          const clip: TitleSelectionClip = { id: c.id, title_text: c.title_text }
          return {
            clip,
            trackId: t.id,
          }
        }
      }
    }
    return null
  }, [project, selectedClipId])

  // A selected SHAPE clip — a media clip on a `title*` overlay track (shapes share
  // the title tracks) carrying the transient `shape_kind` annotation (project.state
  // adds it for edit.add_shape-created clips, recovered from the op-log). Resolved
  // SEPARATELY so it gets the in-place shape editor instead of the generic
  // "Video clip" inspector, which had no way to change it. A title clip carries
  // `title_text` and NOT `shape_kind` (and vice versa), so titleSel and shapeSel
  // never shadow each other. Takes precedence over `sel` below.
  const shapeSel = useMemo(() => {
    if (!project || !selectedClipId) return null
    for (const t of project.tracks ?? []) {
      if (!t.id?.startsWith('title')) continue
      for (const c of t.clips ?? []) {
        if (
          'id' in c &&
          c.id === selectedClipId &&
          'shape_kind' in c &&
          typeof c.shape_kind === 'string'
        ) {
          if (typeof c.id !== 'string') continue
          const clip: ShapeSelectionClip = {
            id: c.id,
            shape_kind: c.shape_kind,
            shape_label: 'shape_label' in c && typeof c.shape_label === 'string' ? c.shape_label : undefined,
            shape_color: 'shape_color' in c && typeof c.shape_color === 'string' ? c.shape_color : undefined,
          }
          return {
            clip,
            trackId: t.id,
          }
        }
      }
    }
    return null
  }, [project, selectedClipId])

  const selectedTrackLocked = isTrackLocked(project?.tracks ?? [], capSel?.trackId ?? titleSel?.trackId ?? shapeSel?.trackId ?? sel?.trackId)
  const baseTrackId = baseVideoTrackId(project?.tracks ?? [])
  const isOverlayVideo = sel?.trackKind === 'video' && sel.trackId !== baseTrackId
  // Server truth for the Blend select: the SELECTED overlay track's stored blend
  // mode (engine Track.blend_mode; absent/null = 'normal'). Derived from the
  // project snapshot and keyed by sel.trackId so it re-reads when the selection
  // moves to another track — mirrors the controlled color-management selects that
  // read project.settings/clip fields directly instead of holding stale local state.
  const rawTrackBlendMode = (project?.tracks ?? []).find((t) => t.id === sel?.trackId)?.blend_mode ?? 'normal'
  const trackBlendMode = blendModeFromInput(rawTrackBlendMode, 'normal')
  // Ducking needs a real audio track as the speech reference. Do not offer the
  // action when the selected audio track is the only audio track; the backend
  // rejects video tracks and self-ducking.
  const speechTrackId = useMemo(() => {
    if (sel?.trackKind !== 'audio') return null
    return (project?.tracks ?? []).find((t) =>
      t.kind === 'audio' &&
      t.id !== sel.trackId &&
      (t.clips ?? []).some((c) => 'asset' in c && !!c.asset),
    )?.id ?? null
  }, [project, sel?.trackId, sel?.trackKind])

  // ── Derived selection facts ────────────────────────────────────────────────
  // Media clips support engagement analysis and stream-appropriate fades. Video
  // tracks contribute pixels only (visual fade); audio tracks contribute sound.
  const mediaCapable = sel?.trackKind === 'audio' || sel?.trackKind === 'video'
  const isVideoClip = sel?.trackKind === 'video'
  const {
    replaceOptions,
    replaceAssetId,
    clipActionNote,
    setReplaceAssetId,
    runReplaceSource,
    runDetachAudio,
  } = useInspectorClipActions({ project, sel })
  // Source pixel geometry for the Cropping section (null until the asset is probed).
  const cropDims = useMemo(
    () => (isVideoClip && sel ? sourceDims(project, sel.clip.asset) : null),
    [isVideoClip, sel, project],
  )

  return (
    <section className="insp" data-cut-panel="inspector" data-cut-inspector-kind={capSel ? 'caption' : titleSel ? 'title' : shapeSel ? 'shape' : sel ? sel.trackKind : 'none'}>
      <header className="insp__head">
        <h2 className="insp__title">Inspector</h2>
        <span className="insp__scope" data-cut-inspector-scope>
          {capSel
            ? `Caption · ${capSel.clip.id}`
            : titleSel
              ? `Title · ${titleSel.clip.id}`
              : shapeSel
                ? `Shape · ${shapeSel.clip.id}`
                : sel
                  ? `${sel.trackKind === 'audio' ? 'Audio' : 'Video'} clip · ${sel.clip.id}`
                  : 'No selection'}
        </span>
      </header>

      <div className="insp__body">
        <TrackLockGate locked={selectedTrackLocked}>
        {capSel ? (
          // A caption clip is selected → edit its text in place (caption-editing regression fix).
          <CaptionEditSection
            key={capSel.clip.id}
            clipId={capSel.clip.id}
            text={capSel.clip.text}
            rangeMs={capSel.clip.range_ms}
          />
        ) : titleSel ? (
          // A title clip is selected → edit its text in place (title-editing regression).
          <TitleEditSection
            key={titleSel.clip.id}
            clipId={titleSel.clip.id}
            text={titleSel.clip.title_text}
          />
        ) : shapeSel ? (
          // A shape clip is selected → edit its props in place (shape-editing regression).
          <ShapeEditSection
            key={shapeSel.clip.id}
            clipId={shapeSel.clip.id}
            kind={shapeSel.clip.shape_kind}
            label={shapeSel.clip.shape_label ?? ''}
            color={shapeSel.clip.shape_color ?? ''}
          />
        ) : !sel ? (
          // No selection → project / sequence properties.
          <>
            <div className="insp__group" data-cut-inspector-group="project">
              <div className="insp__group-title">Project</div>
              {project ? (
                <dl className="insp__props">
                  <dt>Name</dt><dd>{project.name}</dd>
                  <dt>Resolution</dt><dd>{project.settings?.width}×{project.settings?.height}</dd>
                  <dt>Frame rate</dt><dd>{project.settings?.fps} fps</dd>
                  <dt>Tracks</dt><dd>{project.tracks?.length ?? 0}</dd>
                </dl>
              ) : (
                <p className="insp__hint">Open a project, then select a clip to edit it here.</p>
              )}
              <p className="insp__hint">Select a clip on the timeline — its tools appear here.</p>
            </div>
            <ProjectCaptionsSection project={project} playheadMs={playheadMs} />
          </>
        ) : (
          <>
            {sel.clip.motion_link && <MotionLinkSection link={sel.clip.motion_link} />}
            {/* At-a-glance clip facts */}
            <div className="insp__group" data-cut-inspector-group="clip">
              <div className="insp__group-title">Clip</div>
              <dl className="insp__props">
                <dt>Source in/out</dt>
                <dd>{fmtDur(sel.clip.src_in_ms)} – {fmtDur(sel.clip.src_out_ms)}</dd>
                <dt>Length</dt><dd>{fmtDur(sel.clip.src_out_ms - sel.clip.src_in_ms)}</dd>
                {typeof sel.clip.speed === 'number' && sel.clip.speed !== 1 && (<><dt>Speed</dt><dd>{sel.clip.speed}×</dd></>)}
                {sel.clip.grade && (<><dt>Grade</dt><dd>applied</dd></>)}
                {sel.clip.crop && (<><dt>Crop</dt><dd>set</dd></>)}
                {sel.clip.fade && (<><dt>Fade</dt><dd>{fmtDur(sel.clip.fade.in_ms ?? 0)} / {fmtDur(sel.clip.fade.out_ms ?? 0)}</dd></>)}
              </dl>
            </div>

            {/* ENGAGEMENT (score.clip) — on-demand engagement readout for media
                clips (video/audio carry the speech+energy+motion the score reads;
                caption clips have no source media to score). Keyed by clip id so a
                new selection remounts it and clears the stale score. */}
            {mediaCapable && <EngagementSection key={sel.clip.id} clipId={sel.clip.id} />}

            {/* Fades section — scrubbable fade-in/out sliders replacing the
                old fixed "Fade 0.5s" buttons (→ edit.fade {in_ms|out_ms}). Gated to
                clips that carry audio/video that can fade (audio clip OR video clip);
                caption clips have nothing to fade. Seeds from sel.clip.fade. */}
            {mediaCapable && (
              <FadesSection
                clipId={sel.clip.id}
                fadeInMs={sel.clip.fade?.in_ms ?? 0}
                fadeOutMs={sel.clip.fade?.out_ms ?? 0}
                isVideo={isVideoClip}
                clipDurMs={Math.round((sel.clip.src_out_ms - sel.clip.src_in_ms) / (sel.clip.speed ?? 1))}
              />
            )}

            <ClipActionsSection
              trackKind={sel.trackKind}
              replaceOptions={replaceOptions}
              replaceAssetId={replaceAssetId}
              note={clipActionNote}
              onReplaceAssetChange={setReplaceAssetId}
              onReplaceSource={runReplaceSource}
              onDetachAudio={runDetachAudio}
            />

            {sel.trackKind === 'video' ? (
              // Video clip → Transform (flagship continuous UI) + the picture tools.
              <>
                {/* Continuous Transform section, gated to
                    VIDEO clips here (only rendered in the trackKind === 'video'
                    branch — never for audio/caption). Seeds from sel.clip.transform. */}
                <TransformSection clipId={sel.clip.id} stored={sel.clip.transform} isOverlay={isOverlayVideo} />
                {/* Cropping (source-px X/Y/W/H -> edit.crop) and speed/retime
                    (factor → edit.speed, Reverse → edit.reverse, Freeze →
                    edit.freeze). VIDEO clips only — same branch as Transform. */}
                <CroppingSection clipId={sel.clip.id} stored={sel.clip.crop} dims={cropDims} />
                <SpeedSection
                  clipId={sel.clip.id}
                  speed={typeof sel.clip.speed === 'number' ? sel.clip.speed : 1}
                  reverse={!!sel.clip.reverse}
                  frozen={!!sel.clip.freeze}
                  speedRampApplied={Boolean(sel.clip.speed_ramp)}
                  srcDurMs={sel.clip.src_out_ms - sel.clip.src_in_ms}
                />
                <VideoInspectorTools
                  project={project}
                  selection={sel}
                  doctor={doctor}
                  isOverlayVideo={isOverlayVideo}
                  trackBlendMode={trackBlendMode}
                  auto={autoVideoControls}
                  color={colorControls}
                  onOpenDrawer={openDrawer}
                />
              </>
            ) : (
              <>
                <VolumeSection
                  clipId={sel.clip.id}
                  gainDb={typeof sel.clip.gain_db === 'number' ? sel.clip.gain_db : 0}
                />
                <AudioInspectorTools
                  selection={sel}
                  speechTrackId={speechTrackId}
                  onOpenDrawer={openDrawer}
                />
              </>
            )}
          </>
        )}
        </TrackLockGate>
      </div>
    </section>
  )
}
