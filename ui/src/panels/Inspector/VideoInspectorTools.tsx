import { useEffect, useMemo, useState } from 'react'
import InspectorSection from '../../components/inspector/InspectorSection'
import { getEffectsCatalog, type EffectCatalogEntry } from '../../lib/catalogs'
import { type Project, type VerbResult } from '../../lib/client'
import type { DoctorReport } from '../../lib/doctor'
import { runUserVerb } from '../../lib/userActionFeedback'
import EffectChainControls, { type EffectOption } from './EffectChainControls'
import VideoColorSection from './VideoColorSection'
import {
  BLEND_MODES,
  REDACT_CENTRE_POINTS,
  REDACT_PRESETS,
  VIDEO_EFFECTS,
  ZOOM_INTENSITIES,
  blendModeFromInput,
  clipEffectFromCatalog,
  isObject,
  redactModeFromInput,
  type BlendMode,
  type InspectorMediaSelection,
  type RedactMode,
} from './model'
import {
  stabilizationReadiness,
  videoEffectsSummary,
  videoMotionSummary,
  videoPrivacySummary,
} from './inspectorTaskModel'
import type { useInspectorAutoVideoControls } from './useInspectorAutoVideoControls'
import type { useInspectorColorControls } from './useInspectorColorControls'

type AutoVideoControls = ReturnType<typeof useInspectorAutoVideoControls>
type ColorControls = ReturnType<typeof useInspectorColorControls>

interface VideoInspectorToolsProps {
  project: Project | null
  selection: InspectorMediaSelection
  doctor: DoctorReport | null
  isOverlayVideo: boolean
  trackBlendMode: BlendMode
  auto: AutoVideoControls
  color: ColorControls
  onOpenDrawer: (name: string) => void
}

function applyVisual(promise: Promise<VerbResult | null>): void {
  void promise.then((result) => {
    if (result?.ok) document.dispatchEvent(new CustomEvent('cut:show-composed'))
  })
}

function openVideoSetup() {
  document.dispatchEvent(new CustomEvent('cut:open-ui-surface', {
    detail: { id: 'settings-video-performance' },
  }))
}

export default function VideoInspectorTools({
  project,
  selection,
  doctor,
  isOverlayVideo,
  trackBlendMode,
  auto,
  color,
  onOpenDrawer,
}: VideoInspectorToolsProps) {
  const clip = selection.clip
  const [drawRedact, setDrawRedact] = useState(false)
  const [redactMode, setRedactMode] = useState<RedactMode>('blur')
  const [redactNote, setRedactNote] = useState<string | null>(null)
  const [effectCatalog, setEffectCatalog] = useState<EffectCatalogEntry[]>([])

  useEffect(() => {
    let live = true
    void getEffectsCatalog().then((catalog) => {
      if (live) setEffectCatalog(catalog)
    })
    return () => { live = false }
  }, [])

  const curatedVideoKeys = useMemo(
    () => new Set<string>(VIDEO_EFFECTS.map((option) => option.eff.type)),
    [],
  )
  const extraVideoEffects = useMemo<EffectOption[]>(
    () => effectCatalog
      .filter((entry) => entry.track === 'video' && !entry.overlay_only && !curatedVideoKeys.has(entry.key))
      .flatMap((entry) => {
        const effect = clipEffectFromCatalog(entry)
        return effect ? [{ eff: effect, label: entry.key, description: entry.description }] : []
      }),
    [curatedVideoKeys, effectCatalog],
  )

  useEffect(() => {
    if (drawRedact) {
      document.dispatchEvent(new CustomEvent('cut:redact-draw', {
        detail: { active: true, clip: clip.id, mode: redactMode },
      }))
    } else {
      document.dispatchEvent(new CustomEvent('cut:redact-draw', { detail: { active: false } }))
    }
    return () => {
      document.dispatchEvent(new CustomEvent('cut:redact-draw', { detail: { active: false } }))
    }
  }, [clip.id, drawRedact, redactMode])

  useEffect(() => {
    const onDone = (event: Event) => {
      const ok = event instanceof CustomEvent && isObject(event.detail) && 'ok' in event.detail
        ? event.detail.ok
        : false
      setDrawRedact(false)
      setRedactNote(ok ? 'Region obscured.' : 'Draw cancelled.')
      setTimeout(() => setRedactNote(null), 4000)
    }
    document.addEventListener('cut:redact-draw-done', onDone)
    return () => document.removeEventListener('cut:redact-draw-done', onDone)
  }, [])

  const motion = videoMotionSummary(clip)
  const effectsSummary = videoEffectsSummary(clip, trackBlendMode)
  const privacy = videoPrivacySummary(clip)
  const stabilization = stabilizationReadiness(doctor)
  const stabilizationApplied = Boolean(clip.stabilize)

  return (
    <>
      <InspectorSection
        title="Stabilization & auto zoom"
        sectionKey="video-motion"
        summary={motion.label}
        summaryTone={motion.tone}
      >
        <div className="insp__group" data-cut-inspector-group="video-motion">
          <div className="insp__group-title">Motion repair</div>
          <div className="insp__row">
            <button
              type="button"
              className={`insp__btn${stabilizationApplied ? ' insp__btn--on' : ''}`}
              data-cut-inspector-action="stabilize"
              data-cut-inspector-stabilize-applied={stabilizationApplied ? 'true' : 'false'}
              disabled={!stabilizationApplied && !stabilization.ready}
              onClick={() => void runUserVerb('edit.stabilize', {
                clip: clip.id,
                enabled: !stabilizationApplied,
                rationale: `inspector: ${stabilizationApplied ? 'remove' : 'apply'} stabilization`,
              }, stabilizationApplied ? 'Could not remove stabilization.' : 'Could not stabilize this clip.')}
            >
              {stabilizationApplied ? 'Remove stabilization' : 'Stabilize'}
            </button>
            <button
              type="button"
              className="insp__btn"
              data-cut-inspector-tool="layer"
              onClick={() => onOpenDrawer('layer')}
            >
              Layer motion
            </button>
          </div>
          {!stabilizationApplied && stabilization.reason && (
            <div className="insp__setup-blocker" data-cut-inspector-blocked="stabilize">
              <span>{stabilization.reason}</span>
              <button type="button" data-cut-inspector-open-video-setup onClick={openVideoSetup}>Open video setup</button>
            </div>
          )}

          <div className="insp__group-title insp__group-title--sub">Automatic punch-ins</div>
          <div className="insp__row" data-cut-inspector-autozoom>
            <select
              className="insp__select"
              data-cut-autozoom-intensity
              value={auto.zoomIntensity}
              disabled={auto.autoBusy !== ''}
              title="How far each punch zooms in"
              onChange={(event) => auto.setZoomIntensity(Number(event.currentTarget.value))}
            >
              {ZOOM_INTENSITIES.map(({ value, label }) => <option key={value} value={value}>{label}</option>)}
            </select>
            <button
              type="button"
              className="insp__btn"
              data-cut-action="auto-zoom"
              disabled={auto.autoBusy !== ''}
              title="Add punch-in zooms at detected emphasis beats"
              onClick={() => void auto.autoZoom()}
            >
              {auto.autoBusy === 'zoom' ? 'Adding zooms…' : 'Add auto zoom'}
            </button>
          </div>
        </div>
      </InspectorSection>

      {auto.autoNote && <p className="insp__hint" role="status" data-cut-inspector-auto-note>{auto.autoNote}</p>}

      <VideoColorSection
        project={project}
        selection={selection}
        auto={auto}
        color={color}
        onOpenDrawer={onOpenDrawer}
      />

      <InspectorSection
        title="Effects & compositing"
        sectionKey="video-effects"
        defaultCollapsed
        summary={effectsSummary.label}
        summaryTone={effectsSummary.tone}
      >
        <div className="insp__group" data-cut-inspector-group="video-effects">
          <EffectChainControls
            clipId={clip.id}
            effects={clip.effects}
            kind="video"
            options={VIDEO_EFFECTS}
            extraOptions={extraVideoEffects}
            onApplied={() => document.dispatchEvent(new CustomEvent('cut:show-composed'))}
          />
          {isOverlayVideo && (
            <>
              <div className="insp__group-title insp__group-title--sub">Blend mode</div>
              <div className="insp__row">
                <select
                  className="insp__select"
                  data-cut-inspector-blend
                  value={trackBlendMode}
                  onChange={(event) => {
                    const mode = blendModeFromInput(event.currentTarget.value, trackBlendMode)
                    applyVisual(runUserVerb('edit.blend', {
                      track: selection.trackId,
                      mode,
                      rationale: `inspector: blend ${mode}`,
                    }, 'Could not change the blend mode.'))
                  }}
                >
                  {BLEND_MODES.map((mode) => <option key={mode} value={mode}>{mode}</option>)}
                </select>
              </div>
            </>
          )}
        </div>
      </InspectorSection>

      <InspectorSection
        title="Privacy & redaction"
        sectionKey="video-privacy"
        defaultCollapsed
        summary={privacy.label}
        summaryTone={privacy.tone}
      >
        <div className="insp__group" data-cut-inspector-group="video-privacy">
          <div className="insp__tools">
            <button type="button" className="insp__tool" data-cut-inspector-tool="matte" onClick={() => onOpenDrawer('matte')}>
              Remove or replace background
            </button>
            <button type="button" className="insp__tool" data-cut-inspector-tool="shape" onClick={() => onOpenDrawer('shape')}>
              Add shape or mask
            </button>
          </div>
          <div className="insp__group-title insp__group-title--sub">Quick privacy</div>
          <div className="insp__effects" data-cut-inspector-privacy>
            <button type="button" className="insp__chip insp__chip--accent" data-cut-inspector-redact="faces"
              onClick={() => applyVisual(runUserVerb('edit.redact', {
                clip: clip.id,
                faces: true,
                mode: 'blur',
                strength: 30,
                rationale: 'inspector: blur faces',
              }, 'Could not blur faces in this clip.'))}>
              Blur faces
            </button>
            {REDACT_PRESETS.map(({ mode, label }) => (
              <button key={mode} type="button" className="insp__chip" data-cut-inspector-redact={mode}
                onClick={() => applyVisual(runUserVerb('edit.redact', {
                  clip: clip.id,
                  shape: 'rect',
                  points: REDACT_CENTRE_POINTS,
                  mode,
                  strength: mode === 'blur' ? 25 : 16,
                  rationale: `inspector: redact ${mode} centre`,
                }, `Could not apply ${label.toLocaleLowerCase()}.`))}>
                {label}
              </button>
            ))}
            <button type="button" className="insp__chip" data-cut-inspector-redact="clear"
              onClick={() => applyVisual(runUserVerb('edit.redact', {
                clip: clip.id,
                enabled: false,
                rationale: 'inspector: clear redaction',
              }, 'Could not clear redaction.'))}>
              Clear
            </button>
          </div>
          <div className="insp__group-title insp__group-title--sub">Draw a region</div>
          <div className="insp__row">
            <select className="insp__select" data-cut-inspector-redact-mode value={redactMode}
              onChange={(event) => setRedactMode(redactModeFromInput(event.currentTarget.value, redactMode))}>
              <option value="blur">Blur</option>
              <option value="pixelate">Pixelate</option>
              <option value="box">Solid box</option>
            </select>
            <button type="button" className={`insp__btn${drawRedact ? ' insp__btn--on' : ''}`}
              data-cut-action="redact-draw" data-cut-redact-drawing={drawRedact ? 'true' : 'false'}
              onClick={() => setDrawRedact((value) => !value)}>
              {drawRedact ? 'Drawing — drag on preview…' : 'Draw region'}
            </button>
          </div>
          {redactNote && <p className="insp__hint" role="status" data-cut-inspector-redact-note>{redactNote}</p>}
        </div>
      </InspectorSection>
    </>
  )
}
