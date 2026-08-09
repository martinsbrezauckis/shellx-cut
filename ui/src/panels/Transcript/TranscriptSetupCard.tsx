interface TranscriptSetupCardProps {
  hint: string | null
  error: string | null
  busy: boolean
  message: string
  onInstall: () => void
}

export default function TranscriptSetupCard({
  hint,
  error,
  busy,
  message,
  onInstall,
}: TranscriptSetupCardProps) {
  return (
    <div className="tx__setup" data-cut-perception-setup>
      <p className="tx__setup-title">Captions and transcripts are not installed</p>
      <p className="tx__setup-body">
        {hint ??
          'Install the captions tools to create word-level transcripts, searchable speech, and caption edits. Core editing and export still work without them.'}
      </p>
      {error && <p className="tx__setup-err" data-cut-perception-setup-err>{error}</p>}
      {busy ? (
        <p className="tx__setup-progress" data-cut-perception-setup-progress>
          {message || 'installing captions tools... this can take a few minutes'}
        </p>
      ) : (
        <button
          type="button"
          className="tx__setup-btn"
          data-cut-action="setup-perception"
          onClick={() => onInstall()}
        >
          Install captions
        </button>
      )}
    </div>
  )
}
