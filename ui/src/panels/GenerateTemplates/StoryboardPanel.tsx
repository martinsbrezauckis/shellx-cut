import type { GenerateStoryboardResult } from '../../lib/client'
import { Icon } from '../../icons'
import {
  formatDuration,
  optionValue,
  PROMPT_AGENTS,
  STORYBOARD_MODES,
  type PromptAgent,
  type PromptPolicy,
  type StoryboardMode,
} from './model'

interface StoryboardPanelProps {
  projectReady: boolean
  storyboardInput: string
  storyboardMode: StoryboardMode
  storyboardAgent: PromptAgent
  storyboardAnswers: Record<string, string>
  atMs: number
  canRunStoryboard: boolean
  storyboardBusy: PromptPolicy | null
  storyboardResult: GenerateStoryboardResult | null
  storyboardError: string | null
  onStoryboardInput: (value: string) => void
  onStoryboardMode: (mode: StoryboardMode) => void
  onStoryboardAgent: (agent: PromptAgent) => void
  onAnswer: (questionId: string, value: string) => void
  onAtMs: (value: number) => void
  onRun: (policy: PromptPolicy) => void
}

export default function StoryboardPanel({
  projectReady,
  storyboardInput,
  storyboardMode,
  storyboardAgent,
  storyboardAnswers,
  atMs,
  canRunStoryboard,
  storyboardBusy,
  storyboardResult,
  storyboardError,
  onStoryboardInput,
  onStoryboardMode,
  onStoryboardAgent,
  onAnswer,
  onAtMs,
  onRun,
}: StoryboardPanelProps) {
  const questions = storyboardResult?.questions ?? []

  return (
    <div className="gt-storyboard" data-cut-generate-storyboard>
      <section className="gt-storyboard__compose" aria-label="Generate storyboard">
        <label className="gt-field">
          <span className="gt-field__label">brief</span>
          <textarea
            className="gt-input gt-storyboard__input"
            data-cut-generate-storyboard-input
            value={storyboardInput}
            onChange={(e) => onStoryboardInput(e.target.value)}
            placeholder="Plan a 12 second launch video with title, lower third, and CTA"
          />
        </label>

        <div className="gt-prompt__controls">
          <label className="gt-field">
            <span className="gt-field__label">mode</span>
            <select
              className="gt-input"
              data-cut-generate-storyboard-mode
              value={storyboardMode}
              onChange={(e) => onStoryboardMode(optionValue(STORYBOARD_MODES, e.target.value, storyboardMode))}
            >
              {STORYBOARD_MODES.map((m) => <option key={m} value={m}>{m}</option>)}
            </select>
          </label>
          <label className="gt-field">
            <span className="gt-field__label">agent</span>
            <select
              className="gt-input"
              data-cut-generate-storyboard-agent
              value={storyboardAgent}
              onChange={(e) => onStoryboardAgent(optionValue(PROMPT_AGENTS, e.target.value, storyboardAgent))}
            >
              {PROMPT_AGENTS.map((a) => <option key={a} value={a}>{a}</option>)}
            </select>
          </label>
          <label className="gt-field">
            <span className="gt-field__label">timeline start</span>
            <input
              className="gt-input"
              data-cut-generate-storyboard-at-ms
              type="number"
              min={0}
              value={atMs}
              onChange={(e) => onAtMs(Math.max(0, Math.round(Number(e.target.value) || 0)))}
            />
          </label>
        </div>

        {questions.length > 0 && (
          <div className="gt-questions" data-cut-generate-storyboard-questions={questions.length}>
            {questions.map((question, index) => (
              <label
                className="gt-field gt-question"
                data-cut-generate-storyboard-question
                data-cut-generate-storyboard-question-id={question.id}
                key={question.id}
              >
                <span className="gt-field__label">question {index + 1}</span>
                <span className="gt-question__prompt">{question.prompt}</span>
                {question.choices?.length ? (
                  <select
                    className="gt-input"
                    data-cut-generate-storyboard-answer={question.id}
                    value={storyboardAnswers[question.id] ?? ''}
                    onChange={(e) => onAnswer(question.id, e.target.value)}
                  >
                    <option value="">Choose answer</option>
                    {question.choices.map((choice) => <option key={choice} value={choice}>{choice}</option>)}
                  </select>
                ) : (
                  <input
                    className="gt-input"
                    data-cut-generate-storyboard-answer={question.id}
                    value={storyboardAnswers[question.id] ?? ''}
                    onChange={(e) => onAnswer(question.id, e.target.value)}
                  />
                )}
              </label>
            ))}
          </div>
        )}

        <div className="gt-actions gt-actions--three">
          <button
            type="button"
            className="gt-btn gt-btn--secondary"
            data-cut-generate-storyboard-plan
            disabled={!canRunStoryboard}
            onClick={() => onRun('plan')}
          >
            {storyboardBusy === 'plan' ? 'Planning...' : <><Icon name="agent" size={14} /> Plan</>}
          </button>
          <button
            type="button"
            className="gt-btn gt-btn--secondary"
            data-cut-generate-storyboard-preview
            disabled={!canRunStoryboard}
            onClick={() => onRun('preview')}
          >
            {storyboardBusy === 'preview' ? 'Previewing...' : <><Icon name="eye" size={14} /> Preview</>}
          </button>
          <button
            type="button"
            className="gt-btn gt-btn--primary"
            data-cut-generate-storyboard-insert
            disabled={!canRunStoryboard}
            onClick={() => onRun('insert')}
          >
            {storyboardBusy === 'insert' ? 'Inserting...' : <><Icon name="effect" size={14} /> Insert</>}
          </button>
        </div>

        {!projectReady && <div className="gt-note">Open a project to plan, preview, or insert storyboards.</div>}
        {storyboardError && <div className="gt-error" data-cut-generate-storyboard-error role="alert">{storyboardError}</div>}
      </section>

      <aside className="gt-storyboard__evidence" aria-label="Generate storyboard evidence">
        <div className="gt-prompt__status" data-cut-generate-storyboard-status>
          <span>{storyboardResult ? storyboardResult.status : 'idle'}</span>
          <span>{storyboardResult ? `${storyboardResult.evidence.policy} · ${storyboardResult.evidence.mutated ? 'mutated' : 'no mutation'}` : storyboardMode}</span>
        </div>

        {storyboardResult?.reason && (
          <div className={storyboardResult.status === 'completed' ? 'gt-note' : 'gt-error'}>
            {storyboardResult.reason}
          </div>
        )}

        {storyboardResult && (
          <dl className="gt-result" data-cut-generate-storyboard-evidence>
            <dt>Scenes</dt><dd>{storyboardResult.evidence.scene_count}</dd>
            <dt>Duration</dt><dd>{formatDuration(storyboardResult.evidence.duration_ms)}</dd>
            <dt>Templates</dt><dd>{storyboardResult.evidence.template_ids.join(', ') || 'none'}</dd>
            <dt>Brief</dt><dd>{storyboardResult.evidence.brief_fields.stated.join(', ') || 'none'}</dd>
          </dl>
        )}

        <div className="gt-storyboard__scenes" data-cut-generate-storyboard-scenes>
          {storyboardResult?.storyboard?.scenes?.length ? (
            storyboardResult.storyboard.scenes.map((scene) => (
              <div
                key={scene.scene_id}
                className="gt-storyboard__scene"
                data-cut-generate-storyboard-scene
                data-cut-generate-storyboard-scene-id={scene.scene_id}
              >
                <span className="gt-storyboard__scene-top">
                  <strong>{scene.index}. {scene.role}</strong>
                  <em>{formatDuration(scene.range_ms[1] - scene.range_ms[0])}</em>
                </span>
                <span>{scene.template_id ?? scene.source}</span>
                {scene.screen_text && <small>{scene.screen_text}</small>}
              </div>
            ))
          ) : (
            <div className="gt-empty">Storyboard scene evidence appears here.</div>
          )}
        </div>

        {storyboardResult?.preview?.scenes?.length ? (
          <div className="gt-storyboard__previews" data-cut-generate-storyboard-preview-result>
            {storyboardResult.preview.scenes.map((scene) => (
              <figure key={scene.scene_id} className="gt-storyboard__preview">
                {scene.url ? (
                  <img
                    data-cut-generate-storyboard-preview-img
                    src={scene.url}
                    alt={`${scene.scene_id} preview`}
                  />
                ) : (
                  <div className="gt-note">Preview wrote {scene.path}, but no browser URL was returned.</div>
                )}
                <figcaption>
                  <span>{scene.scene_id}</span>
                  <code>{scene.preview_id}</code>
                </figcaption>
              </figure>
            ))}
          </div>
        ) : null}

        {storyboardResult?.insert && (
          <dl className="gt-result" data-cut-generate-storyboard-insert-result>
            <dt>Checkpoint</dt><dd>{storyboardResult.insert.checkpoints.join(', ')}</dd>
            <dt>Ops</dt><dd>{storyboardResult.insert.op_ids.join(', ')}</dd>
            <dt>Clips</dt><dd>{storyboardResult.insert.clips.join(', ') || 'none'}</dd>
            <dt>Assets</dt><dd>{storyboardResult.insert.assets.join(', ') || 'none'}</dd>
            <dt>Scenes</dt><dd>{storyboardResult.insert.scenes.length}</dd>
            <dt>Restore</dt><dd>{storyboardResult.insert.restore_hint}</dd>
          </dl>
        )}
      </aside>
    </div>
  )
}
