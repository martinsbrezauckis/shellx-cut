import type { DoctorCard } from '../../lib/doctor'

interface ServiceRuntimeInfo {
  model: string
  verb: string
  outcome: string
  chatLabel: string
  chatPrompt: string
  setupPrompt: string
  readyCopy: string
  setupCopy: string
  requirementCopy: string
  connectorCopy: string
  setupSteps: string[]
}

export function serviceInfo(card: DoctorCard): ServiceRuntimeInfo | null {
  if (card.id === 'dub') {
    return {
      model: String(card.details?.model ?? 'OmniVoice TTS'),
      verb: String(card.details?.powers ?? 'audio.dub'),
      outcome: 'Creates a translated voice track',
      chatLabel: 'Ask Chat to dub',
      chatPrompt: 'Dub the timeline audio into Latvian',
      setupPrompt: 'Help me connect OmniVoice TTS for ShellX Cut dubbing',
      readyCopy: 'Ready for re-voicing translated speech into a new audio track.',
      setupCopy: 'Editing, captions, and export work without this optional service.',
      requirementCopy: 'External runtime required',
      connectorCopy: 'Connector included',
      setupSteps: [
        'Start the OmniVoice service for ShellX Cut.',
        'If it uses another address, set CUT_DUB_ENDPOINT before opening the app.',
        'Re-scan here, then ask Agent Chat or run audio.dub on a transcribed clip.',
      ],
    }
  }
  if (card.id === 'diarize') {
    return {
      model: String(card.details?.model ?? 'Sortformer v2'),
      verb: String(card.details?.powers ?? 'media.diarize'),
      outcome: 'Adds speaker labels to transcripts',
      chatLabel: 'Ask Chat to label speakers',
      chatPrompt: 'Label the speakers in this video — diarize who is talking and when',
      setupPrompt: 'Help me connect Sortformer v2 for ShellX Cut speaker labels',
      readyCopy: 'Ready to label who speaks when for transcripts, multicam, and dubbing.',
      setupCopy: 'Editing and export work without this optional service.',
      requirementCopy: 'External runtime required',
      connectorCopy: 'Connector included',
      setupSteps: [
        'Start the Sortformer v2 service for ShellX Cut.',
        'If it uses another address, set CUT_DIARIZE_ENDPOINT before opening the app.',
        'Re-scan here, then label speakers from Agent Chat or media.diarize.',
      ],
    }
  }
  return null
}

function openAgentTask(prompt: string) {
  document.dispatchEvent(new CustomEvent('cut:open-chat', { detail: { prompt } }))
}

export function ServiceRuntimeActions({
  card,
  busy,
  onOpenSetup,
}: {
  card: DoctorCard
  busy: boolean
  onOpenSetup: (id: string) => void
}) {
  const svc = serviceInfo(card)
  if (busy || !svc) return null
  if (card.status === 'ok') {
    return (
      <button
        className="env-btn env-btn--primary env-btn--sm"
        data-cut-env-service-primary={card.id}
        data-cut-env-service-chat={card.id}
        onClick={() => openAgentTask(svc.chatPrompt)}
        title={svc.chatLabel}
      >
        Use in Chat
      </button>
    )
  }
  return (
    <div className="env-service-actions" data-cut-env-service-primary={card.id}>
      <button
        className="env-btn env-btn--primary env-btn--sm"
        data-cut-env-service-connect={card.id}
        onClick={() => onOpenSetup(card.id)}
        title="Show the short connection steps for this optional model runtime"
      >
        Connect service
      </button>
    </div>
  )
}

export function ServiceRuntimeDetail({
  card,
  open,
  onOpenChange,
  onChanged,
}: {
  card: DoctorCard
  open: boolean
  onOpenChange: (open: boolean) => void
  onChanged: () => void
}) {
  const svc = serviceInfo(card)
  if (!svc) return null
  const connectorReady = card.details?.runner_available === true
  const runtimeReady = card.status === 'ok'
  return (
    <div className="env-row-detail env-service" data-cut-env-service={card.id}>
      <div className="env-service-card">
        <span className="env-service-model" data-cut-env-service-model={card.id}>
          <strong data-cut-env-service-outcome={card.id}>{svc.outcome}</strong>
          <em>{card.status === 'ok' ? svc.readyCopy : svc.setupCopy}</em>
        </span>
      </div>
      <details
        className="env-service-setup"
        data-cut-env-service-setup={card.id}
        open={open}
        onToggle={(e) => onOpenChange(e.currentTarget.open)}
      >
        <summary className="env-service-setup-summary" data-cut-env-service-setup-toggle={card.id}>Connection steps</summary>
        <ol className="env-service-setup-list">
          {svc.setupSteps.map((step) => (
            <li key={step} data-cut-env-service-setup-step={card.id}>{step}</li>
          ))}
        </ol>
        <dl className="env-advanced-list">
          <div className="env-advanced-row"><dt>Runtime</dt><dd>{svc.model}</dd></div>
          <div className="env-advanced-row"><dt>Capability</dt><dd data-cut-env-service-powered-by={card.id}>{svc.verb}</dd></div>
          <div className="env-advanced-row">
            <dt>Requirement</dt>
            <dd className="env-service-requirement" data-cut-env-service-requirement={card.id}>
              <span>{svc.requirementCopy}</span>
              <span>{svc.connectorCopy}</span>
            </dd>
          </div>
          <div className="env-advanced-row">
            <dt>Connection</dt>
            <dd className="env-service-metrics">
              <span
                className={`env-service-state ${connectorReady ? 'env-service-state--ok' : 'env-service-state--missing'}`}
                data-cut-env-service-connector={card.id}
                data-cut-env-service-runner={card.id}
              >
                Connector {connectorReady ? 'ready' : 'missing'}
              </span>
              <span
                className={`env-service-state ${runtimeReady ? 'env-service-state--ok' : 'env-service-state--missing'}`}
                data-cut-env-service-runtime={card.id}
              >
                External service {runtimeReady ? 'ready' : 'not connected'}
              </span>
            </dd>
          </div>
        </dl>
        {card.status !== 'ok' && (
          <div className="env-service-actions">
            <button
              className="env-btn env-btn--sm env-btn--ghost"
              data-cut-env-service-chat={card.id}
              onClick={() => openAgentTask(svc.setupPrompt)}
              title="Ask Agent Chat for connection help"
            >
              Ask Agent for help
            </button>
            <button
              className="env-btn env-btn--sm env-btn--ghost"
              data-cut-env-service-rescan={card.id}
              onClick={onChanged}
              title="Check the connection again"
            >
              Re-check connection
            </button>
          </div>
        )}
      </details>
    </div>
  )
}
