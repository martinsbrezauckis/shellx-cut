import type { GenerateFromPromptResult } from '../../lib/client'
import { Icon } from '../../icons'
import {
  optionValue,
  PROMPT_AGENTS,
  PROMPT_POLICIES,
  type PromptAgent,
  type PromptPolicy,
} from './model'

interface PromptPanelProps {
  projectReady: boolean
  selectedId: string | null
  promptText: string
  promptPolicy: PromptPolicy
  promptAgent: PromptAgent
  atMs: number
  canRunPrompt: boolean
  promptBusy: boolean
  promptResult: GenerateFromPromptResult | null
  error: string | null
  onPromptText: (value: string) => void
  onPromptPolicy: (policy: PromptPolicy) => void
  onPromptAgent: (agent: PromptAgent) => void
  onAtMs: (value: number) => void
  onRun: () => void
}

const PROMPT_POLICY_LABEL: Record<PromptPolicy, string> = {
  plan: 'Plan only',
  preview: 'Create preview',
  insert: 'Add to timeline',
}

function templateName(id: string): string {
  const words = id.split('.').slice(1).join(' ').replaceAll('-', ' ').trim()
  return words ? words.charAt(0).toUpperCase() + words.slice(1) : 'Generated design'
}

export default function PromptPanel({
  projectReady,
  selectedId,
  promptText,
  promptPolicy,
  promptAgent,
  atMs,
  canRunPrompt,
  promptBusy,
  promptResult,
  error,
  onPromptText,
  onPromptPolicy,
  onPromptAgent,
  onAtMs,
  onRun,
}: PromptPanelProps) {
  return (
    <div className="gt-prompt" data-cut-generate-prompt-panel>
      <section className="gt-prompt__compose" aria-label="Generate from prompt">
        <label className="gt-field">
          <span className="gt-field__label">Describe the result</span>
          <textarea
            className="gt-input gt-prompt__input"
            data-cut-generate-prompt-input
            value={promptText}
            onChange={(e) => onPromptText(e.target.value)}
            placeholder="Create a clean lower third for Marta"
          />
        </label>

        <div className="gt-prompt__controls">
          <label className="gt-field">
            <span className="gt-field__label">Result</span>
            <select
              className="gt-input"
              data-cut-generate-prompt-policy
              value={promptPolicy}
              onChange={(e) => onPromptPolicy(optionValue(PROMPT_POLICIES, e.target.value, promptPolicy))}
            >
              {PROMPT_POLICIES.map((p) => <option key={p} value={p}>{PROMPT_POLICY_LABEL[p]}</option>)}
            </select>
          </label>
          <label className="gt-field">
            <span className="gt-field__label">agent</span>
            <select
              className="gt-input"
              data-cut-generate-prompt-agent
              value={promptAgent}
              onChange={(e) => onPromptAgent(optionValue(PROMPT_AGENTS, e.target.value, promptAgent))}
            >
              {PROMPT_AGENTS.map((a) => <option key={a} value={a}>{a === 'auto' ? 'Choose automatically' : a.charAt(0).toUpperCase() + a.slice(1)}</option>)}
            </select>
          </label>
          <label className="gt-field">
            <span className="gt-field__label">timeline start</span>
            <input
              className="gt-input"
              data-cut-generate-prompt-at-ms
              type="number"
              min={0}
              value={atMs}
              onChange={(e) => onAtMs(Math.max(0, Math.round(Number(e.target.value) || 0)))}
            />
          </label>
        </div>

        <div className="gt-prompt__hint">
          <span>Template hint</span>
          <code>{selectedId ?? 'auto'}</code>
        </div>

        <button
          type="button"
          className="gt-btn gt-btn--primary gt-prompt__run"
          data-cut-generate-prompt-run
          disabled={!canRunPrompt}
          onClick={() => onRun()}
        >
          {promptBusy ? 'Generating...' : <><Icon name="agent" size={14} /> Generate from prompt</>}
        </button>

        {!projectReady && <div className="gt-note">Open a project to run prompt preview or insert policies.</div>}
        {error && <div className="gt-error" data-cut-generate-prompt-error role="alert">{error}</div>}
      </section>

      <aside className="gt-prompt__evidence" aria-label="Generate prompt evidence">
        <div className="gt-prompt__status" data-cut-generate-prompt-status>
          <span>{promptResult ? promptResult.status : 'idle'}</span>
          <span>{promptResult?.backend ? String(promptResult.backend.provider ?? 'backend') : promptAgent}</span>
        </div>

        {promptResult?.reason && (
          <div className={promptResult.status === 'completed' ? 'gt-note' : 'gt-error'}>
            {promptResult.reason}
          </div>
        )}

        {promptResult?.plan && (
          <dl className="gt-result" data-cut-generate-prompt-plan>
            <dt>Design</dt><dd>{templateName(promptResult.plan.template_id)}</dd>
            <dt>Confidence</dt><dd>{promptResult.plan.confidence == null ? 'Not scored' : `${Math.round(promptResult.plan.confidence * 100)}%`}</dd>
          </dl>
        )}

        <div className="gt-preview" data-cut-generate-prompt-preview-pane>
          {promptResult?.preview?.url ? (
            <img
              data-cut-generate-prompt-preview-img
              src={promptResult.preview.url}
              alt={`${promptResult.plan?.template_id ?? 'Generate'} prompt preview`}
            />
          ) : promptResult?.preview ? (
            <div className="gt-note">Preview wrote {promptResult.preview.path}, but no browser URL was returned.</div>
          ) : (
            <div className="gt-empty">Native prompt preview evidence appears here.</div>
          )}
        </div>

        {promptResult?.preview && (
          <dl className="gt-result" data-cut-generate-prompt-preview-result>
            <dt>Preview</dt><dd>{promptResult.preview.preview_id}</dd>
            <dt>Size</dt><dd>{promptResult.preview.width}x{promptResult.preview.height}</dd>
            <dt>Lowering</dt><dd>{promptResult.preview.lowering.verb}</dd>
          </dl>
        )}

        {promptResult?.insert && (
          <dl className="gt-result" data-cut-generate-prompt-insert-evidence>
            <dt>Checkpoint</dt><dd>{String(promptResult.insert.checkpoint.id ?? '')}</dd>
            <dt>Ops</dt><dd>{promptResult.insert.op_ids.join(', ')}</dd>
            <dt>Clips</dt><dd>{promptResult.insert.clips.join(', ') || 'none'}</dd>
            <dt>Assets</dt><dd>{promptResult.insert.assets.join(', ') || 'none'}</dd>
            <dt>Lowering</dt><dd>{promptResult.insert.lowering.verb}</dd>
            <dt>Restore</dt><dd>{promptResult.insert.restore_hint}</dd>
          </dl>
        )}
      </aside>
    </div>
  )
}
