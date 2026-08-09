import InspectorSection from '../../components/inspector/InspectorSection'
import type { Project } from '../../lib/client'
import {
  ADJ_LOOKS,
  COLOR_SPACES,
  GRADE_STACK_LAYERS,
  WINDOW_LOOKS,
  WINDOW_REGIONS,
  colorSpaceFromInput,
  gradeSummary,
  type InspectorMediaSelection,
} from './model'
import { videoColorSummary } from './inspectorTaskModel'
import type { useInspectorAutoVideoControls } from './useInspectorAutoVideoControls'
import type { useInspectorColorControls } from './useInspectorColorControls'

interface VideoColorSectionProps {
  project: Project | null
  selection: InspectorMediaSelection
  auto: ReturnType<typeof useInspectorAutoVideoControls>
  color: ReturnType<typeof useInspectorColorControls>
  onOpenDrawer: (name: string) => void
}

export default function VideoColorSection({
  project,
  selection,
  auto,
  color,
  onOpenDrawer,
}: VideoColorSectionProps) {
  const clip = selection.clip
  const summary = videoColorSummary(clip)

  return (
    <InspectorSection
      title="Color"
      sectionKey="video-color"
      defaultCollapsed
      summary={summary.label}
      summaryTone={summary.tone}
    >
      <div className="insp__group" data-cut-inspector-group="video-color">
        <div className="insp__tools">
          <button
            type="button"
            className="insp__tool"
            data-cut-inspector-tool="grade"
            onClick={() => onOpenDrawer('grade')}
          >
            Open full color controls
          </button>
        </div>

        <div className="insp__group-title insp__group-title--sub">Automatic color</div>
        <div className="insp__row" data-cut-inspector-autocolor>
          <button
            type="button"
            className="insp__btn insp__btn--accent"
            data-cut-action="auto-balance"
            disabled={auto.autoBusy !== ''}
            onClick={() => void auto.autoBalance()}
          >
            {auto.autoBusy === 'balance' ? 'Balancing…' : 'Auto balance'}
          </button>
        </div>
        <div className="insp__row" data-cut-inspector-colormatch>
          <select
            className="insp__select"
            data-cut-colormatch-ref
            value={auto.matchRef}
            disabled={auto.refCandidates.length === 0 || auto.autoBusy !== ''}
            onChange={(event) => auto.setMatchRef(event.currentTarget.value)}
          >
            <option value="">Match colour to…</option>
            {auto.refCandidates.map(({ id }) => <option key={id} value={id}>{id}</option>)}
          </select>
          <button
            type="button"
            className="insp__btn"
            data-cut-action="color-match"
            disabled={!auto.matchRef || auto.autoBusy !== ''}
            onClick={() => void auto.colorMatch()}
          >
            {auto.autoBusy === 'match' ? 'Matching…' : 'Match'}
          </button>
        </div>
        {auto.refCandidates.length === 0 && (
          <p className="insp__hint" data-cut-colormatch-hint>Add a second video clip to match colour toward it.</p>
        )}

        <div className="insp__group-title insp__group-title--sub">Adjustment layer</div>
        <div className="insp__row" data-cut-inspector-adjustment>
          <select
            className="insp__select"
            data-cut-adjustment-look
            value={auto.adjLook}
            disabled={auto.autoBusy !== ''}
            onChange={(event) => auto.setAdjLook(event.currentTarget.value)}
          >
            {ADJ_LOOKS.map(({ key, label }) => <option key={key} value={key}>{label}</option>)}
          </select>
          <button
            type="button"
            className="insp__btn"
            data-cut-action="adjustment"
            disabled={auto.autoBusy !== ''}
            onClick={() => void auto.addAdjustment()}
          >
            {auto.autoBusy === 'adjust' ? 'Adding…' : 'Add over clip'}
          </button>
        </div>

        <div className="insp__group-title insp__group-title--sub">Color management</div>
        <div className="insp__row" data-cut-inspector-colormgmt>
          <label className="insp__label" htmlFor="cm-working">Working</label>
          <select
            id="cm-working"
            className="insp__select"
            data-cut-color-working
            value={project?.settings.color?.working ?? 'rec709'}
            disabled={color.colorBusy}
            onChange={(event) => {
              const space = colorSpaceFromInput(event.currentTarget.value)
              if (space) void color.setProjectSpace('working', space)
            }}
          >
            {COLOR_SPACES.map(({ value, label }) => <option key={value} value={value}>{label}</option>)}
          </select>
        </div>
        <div className="insp__row">
          <label className="insp__label" htmlFor="cm-output">Output</label>
          <select
            id="cm-output"
            className="insp__select"
            data-cut-color-output
            value={project?.settings.color?.output ?? 'rec709'}
            disabled={color.colorBusy}
            onChange={(event) => {
              const space = colorSpaceFromInput(event.currentTarget.value)
              if (space) void color.setProjectSpace('output', space)
            }}
          >
            {COLOR_SPACES.map(({ value, label }) => <option key={value} value={value}>{label}</option>)}
          </select>
        </div>
        <div className="insp__row">
          <label className="insp__label" htmlFor="cm-input">Clip input</label>
          <select
            id="cm-input"
            className="insp__select"
            data-cut-color-input
            value={clip.input_color_space ?? ''}
            disabled={color.colorBusy}
            onChange={(event) => void color.setClipInputSpace(event.currentTarget.value)}
          >
            <option value="">Untagged (= working)</option>
            {COLOR_SPACES.map(({ value, label }) => <option key={value} value={value}>{label}</option>)}
          </select>
        </div>

        <div className="insp__group-title insp__group-title--sub">Saved looks</div>
        <div className="insp__row" data-cut-inspector-gallery>
          <input
            className="insp__text insp__text--grow"
            data-cut-grade-save-name
            type="text"
            placeholder="Look name"
            value={color.saveName}
            disabled={color.colorBusy}
            onChange={(event) => color.setSaveName(event.currentTarget.value)}
          />
          <button
            type="button"
            className="insp__btn"
            data-cut-action="grade-save"
            disabled={color.colorBusy || !clip.grade}
            title={clip.grade ? 'Save this clip grade as a reusable look' : 'Apply a grade first, then save it'}
            onClick={() => void color.saveLook()}
          >
            Save look
          </button>
        </div>
        {!clip.grade && <p className="insp__hint" data-cut-grade-save-hint>Apply a grade first to save its look.</p>}
        <div className="insp__row">
          <select
            className="insp__select"
            data-cut-grade-preset
            value={color.selPreset}
            disabled={color.colorBusy || color.presets.length === 0}
            onChange={(event) => color.setSelPreset(event.currentTarget.value)}
          >
            {color.presets.length === 0
              ? <option value="">No saved looks yet</option>
              : color.presets.map((preset) => <option key={preset.name} value={preset.name}>{preset.name}</option>)}
          </select>
          <button
            type="button"
            className="insp__btn"
            data-cut-action="grade-apply"
            disabled={color.colorBusy || !color.selPreset}
            onClick={() => void color.applyLook()}
          >
            Apply
          </button>
        </div>

        <div className="insp__group-title insp__group-title--sub">Layered grades</div>
        <div className="insp__row" data-cut-inspector-grade-stack>
          <select
            className="insp__select"
            data-cut-grade-stack-layer
            value={color.stackLayer}
            disabled={color.colorBusy}
            onChange={(event) => color.setStackLayer(event.currentTarget.value)}
          >
            {GRADE_STACK_LAYERS.map(({ key, label }) => <option key={key} value={key}>{label}</option>)}
          </select>
          <button
            type="button"
            className="insp__btn"
            data-cut-action="grade-stack-add"
            disabled={color.colorBusy}
            onClick={() => void color.addStackLayer()}
          >
            Add layer
          </button>
        </div>
        {(clip.grade_stack ?? []).length > 0 ? (
          <ul className="insp__list" data-cut-grade-stack-list>
            {(clip.grade_stack ?? []).map((grade, index) => (
              <li key={index} className="insp__list-row" data-cut-grade-stack-row>
                <span className="insp__list-label">Layer {index + 1}: {gradeSummary(grade)}</span>
                <button
                  type="button"
                  className="insp__btn insp__btn--mini"
                  data-cut-action="grade-stack-remove"
                  disabled={color.colorBusy}
                  onClick={() => void color.removeStackLayer(index)}
                >
                  Remove
                </button>
              </li>
            ))}
          </ul>
        ) : <p className="insp__hint" data-cut-grade-stack-empty>No grade layers yet.</p>}

        <div className="insp__group-title insp__group-title--sub">Power windows</div>
        <div className="insp__row" data-cut-inspector-grade-window>
          <select
            className="insp__select"
            data-cut-grade-window-region
            value={color.winRegion}
            disabled={color.colorBusy}
            onChange={(event) => color.setWinRegion(event.currentTarget.value)}
          >
            {WINDOW_REGIONS.map(({ key, label }) => <option key={key} value={key}>{label}</option>)}
          </select>
          <select
            className="insp__select"
            data-cut-grade-window-look
            value={color.winLook}
            disabled={color.colorBusy}
            onChange={(event) => color.setWinLook(event.currentTarget.value)}
          >
            {WINDOW_LOOKS.map(({ key, label }) => <option key={key} value={key}>{label}</option>)}
          </select>
          <button
            type="button"
            className="insp__btn"
            data-cut-action="grade-window-add"
            disabled={color.colorBusy}
            onClick={() => void color.addWindow()}
          >
            Add window
          </button>
        </div>
        {color.clipWindows.length > 0 ? (
          <>
            <ul className="insp__list" data-cut-grade-window-list>
              {color.clipWindows.map((window, index) => (
                <li key={index} className="insp__list-row" data-cut-grade-window-row>
                  <span className="insp__list-label">{window.window.shape} · {gradeSummary(window.grade)}</span>
                  <button
                    type="button"
                    className="insp__btn insp__btn--mini"
                    data-cut-action="grade-window-remove"
                    disabled={color.colorBusy}
                    onClick={() => void color.removeWindow(index)}
                  >
                    Remove
                  </button>
                </li>
              ))}
            </ul>
            <div className="insp__row">
              <button
                type="button"
                className="insp__btn"
                data-cut-action="grade-window-clear"
                disabled={color.colorBusy}
                onClick={() => void color.clearWindows()}
              >
                Clear all windows
              </button>
            </div>
          </>
        ) : <p className="insp__hint" data-cut-grade-window-empty>No power windows yet.</p>}
        {color.colorNote && (
          <p className="insp__hint" role="status" data-cut-inspector-color-note>{color.colorNote}</p>
        )}
      </div>
    </InspectorSection>
  )
}
