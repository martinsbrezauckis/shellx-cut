interface TimelineSaveActionsProps {
  canSaveRange: boolean
  canSaveGif: boolean
  savingRange: boolean
  savingGif: boolean
  saveNote: string | null
  onSaveRange: () => void
  onSaveGif: () => void
}

export default function TimelineSaveActions({
  canSaveRange,
  canSaveGif,
  savingRange,
  savingGif,
  saveNote,
  onSaveRange,
  onSaveGif,
}: TimelineSaveActionsProps) {
  return (
    <>
      <button
        type="button"
        className="tl-tool"
        data-cut-action="save-range"
        disabled={savingRange || !canSaveRange}
        title="Render the selected timeline span as a reusable asset"
        onClick={() => void onSaveRange()}
      >
        {savingRange ? 'Saving…' : 'Save to Assets'}
      </button>
      <button
        type="button"
        className="tl-tool"
        data-cut-action="save-gif"
        disabled={savingGif || !canSaveGif}
        title="Export the selected video clips as a looping GIF, up to 30 seconds"
        onClick={() => void onSaveGif()}
      >
        {savingGif ? 'GIF…' : 'GIF'}
      </button>
      {saveNote && <span className="tl-save-note" data-cut-save-note>{saveNote}</span>}
    </>
  )
}
